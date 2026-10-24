#!/usr/bin/bash
# Builds the Nexus OS USB stick image: Nexus OS itself, started from the stick (os/live/
# Containerfile), with the minimal image stored on it, so both installs (erase a disk, or choose
# partitions with Anaconda) need no internet. UEFI, with Secure Boot working (Fedora's signed
# shim, GRUB and kernel).
# Runs in a privileged Fedora bootc container with the minimal image in its container storage,
# from the repository root:
#   podman run --rm --privileged -v /var/lib/containers:/var/lib/containers -v "$PWD:/src" -w /src \
#     quay.io/fedora/fedora-bootc:44 os/build-iso.sh nexus-os-x86_64.iso
# Settings, from the environment:
#   CORE           the minimal image to start from and to store (default
#                  ghcr.io/mohamed-rekiba/nexus-os:minimal; a test build passes its :minimal-test)
#   METHOD         image-builder (default): osbuild's image-builder, bootc-generic-iso type, with
#                  the boot menu from os/live/files/usr/lib/image-builder/bootc/iso.yaml.
#                  xorriso: the hand-made ISO (squashfs, shim, GRUB, xorriso) with the same boot
#                  menu, for when image-builder can't be used.
#   IMAGE_BUILDER  the image-builder command (default: Fedora's image-builder package, installed here)
#   WORK           a scratch folder (default: a new one in /var/tmp)
set -euo pipefail
out=${1:?usage: os/build-iso.sh OUTPUT.iso}
core=${CORE:-ghcr.io/mohamed-rekiba/nexus-os:minimal}
method=${METHOD:-image-builder}
work=${WORK:-$(mktemp -d /var/tmp/nexus-os-iso.XXXXXX)}
live=localhost/nexus-os:live
mkdir -p "$work"
case "$out" in /*) ;; *) out=$PWD/$out ;; esac

case "$method" in
    image-builder)
        if [ -z "${IMAGE_BUILDER:-}" ]; then
            dnf5 -y -q install --setopt=install_weak_deps=False image-builder >/dev/null
            IMAGE_BUILDER="image-builder"
        fi ;;
    xorriso)
        dnf5 -y -q install --setopt=install_weak_deps=False xorriso squashfs-tools dosfstools mtools >/dev/null ;;
    *) echo "error: METHOD is image-builder or xorriso, not $method" >&2; exit 1 ;;
esac

# The stored system is always called IMAGE:minimal, whatever CORE was (a test build's
# :minimal-test too): the installer, Anaconda's kickstart and the installed PC use that name.
image=$(podman run --rm --cgroups=disabled --network=none --entrypoint cat "$core" /usr/lib/nexus-os/image)
payload=$image:minimal
[ "$core" = "$payload" ] || podman tag "$core" "$payload"

podman build --network=host --build-arg CORE="$payload" -f os/live/Containerfile -t "$live" .

# The install screens are Nexus's (docs/nexus-os.md): a stick whose Nexus has none (an older
# release) couldn't install anything.
# (No cgroups: this already runs in a container, where podman can't make its own.)
status=0
podman run --rm --cgroups=disabled --network=none --entrypoint /usr/bin/nexus "$live" --supports os-install || status=$?
case $status in
    0) ;;
    1) echo "error: the Nexus in $core can't install Nexus OS (nexus --supports os-install): release one that can, rebuild $core, then the stick" >&2
       exit 1 ;;
    *) echo "error: couldn't run Nexus in $core to check it can install Nexus OS (podman exit $status)" >&2
       exit 1 ;;
esac

if [ "$method" = image-builder ]; then
    # The live container becomes the ISO's squashfs, and the minimal image is copied into its
    # /var/lib/containers/storage (the live system reads it at /run/rootfsbase/var/lib/containers/
    # storage, see os/live/files/etc/containers/storage.conf).
    rm -rf "$work/out"
    "$IMAGE_BUILDER" build --output-dir "$work/out" --bootc-ref "$live" --bootc-default-fs ext4 \
        --bootc-installer-payload-ref "$payload" bootc-generic-iso
    iso=$(find "$work/out" -name '*.iso' | head -1)
    [ -n "$iso" ] || { echo "error: image-builder made no ISO in $work/out" >&2; exit 1; }
    mv "$iso" "$out"
    ls -l "$out"
    exit 0
fi

# The hand-made ISO, with the same boot menu as iso.yaml.
yaml=os/live/files/usr/lib/image-builder/bootc/iso.yaml
label=$(sed -n 's/^label: *"\(.*\)"$/\1/p' "$yaml")
rm -rf "$work/rootfs" "$work/iso" && mkdir -p "$work/rootfs" "$work/iso/LiveOS" "$work/iso/images/pxeboot" "$work/iso/EFI/BOOT"
cid=$(podman create "$live" /bin/true)
podman export "$cid" | tar -C "$work/rootfs" -xf - --xattrs --numeric-owner
podman rm "$cid" >/dev/null

# The system the installers copy, as a read-only image store where image-builder would put it.
# Its files are mostly the live system's own, which mksquashfs stores once.
store=$work/rootfs/var/lib/containers/storage
mkdir -p "$store"
podman push "$payload" "containers-storage:[overlay@$store+/run/nexus-iso-storage]$payload"
rm -rf "$store"/overlay/*/merged "$store"/overlay/*/work 2>/dev/null || :

kver=$(cd "$work/rootfs/usr/lib/modules" && echo *)
cp "$work/rootfs/usr/lib/modules/$kver/vmlinuz" "$work/iso/images/pxeboot/vmlinuz"
cp "$work/rootfs/usr/lib/modules/$kver/initramfs.img" "$work/iso/images/pxeboot/initrd.img"

mksquashfs "$work/rootfs" "$work/iso/LiveOS/squashfs.img" -noappend -comp zstd -Xcompression-level 19 -b 1M \
    -e sysroot -e ostree -quiet

# UEFI boot: Fedora's signed shim starts its signed GRUB, which reads grub.cfg next to it.
efi=$work/rootfs/boot/efi/EFI/fedora
# shellcheck disable=SC2012 # one known file name
cp "$(ls "$work"/rootfs/usr/lib/efi/shim/*/EFI/BOOT/BOOTX64.EFI | head -1)" "$work/iso/EFI/BOOT/BOOTX64.EFI"
cp "$efi/mmx64.efi" "$work/iso/EFI/BOOT/mmx64.efi"
cp "$efi/gcdx64.efi" "$work/iso/EFI/BOOT/grubx64.efi"
{
    # No menu in sight unless Esc or Shift is held, as on the installed system.
    echo "set default=0"
    echo "set timeout_style=hidden"
    echo "set timeout=1"
    echo "search --no-floppy --set=root --label $label"
    awk '
        /^ *- name:/ { sub(/^ *- name: *"/, ""); sub(/" *$/, ""); name = $0 }
        /^ *linux:/  { sub(/^ *linux: *"/, ""); sub(/" *$/, ""); linux = $0 }
        /^ *initrd:/ { sub(/^ *initrd: *"/, ""); sub(/" *$/, "")
                       printf "menuentry '\''%s'\'' {\n    linux %s\n    initrd %s\n}\n", name, linux, $0 }
    ' "$yaml"
} > "$work/iso/EFI/BOOT/grub.cfg"
cat "$work/iso/EFI/BOOT/grub.cfg"
grep -q "menuentry 'Install Nexus OS (choose partitions)'" "$work/iso/EFI/BOOT/grub.cfg"
mkdir -p "$work/iso/EFI/fedora" && cp "$work/iso/EFI/BOOT/grub.cfg" "$work/iso/EFI/fedora/grub.cfg"

# The EFI system partition image (El Torito, and appended as a GPT partition for USB sticks).
esp=$work/efiboot.img
rm -f "$esp" && truncate -s 16M "$esp" && mkfs.vfat -n NEXUSEFI "$esp" >/dev/null
mmd -i "$esp" ::/EFI ::/EFI/BOOT ::/EFI/fedora
mcopy -i "$esp" "$work/iso/EFI/BOOT/"* ::/EFI/BOOT/
mcopy -i "$esp" "$work/iso/EFI/fedora/grub.cfg" ::/EFI/fedora/
cp "$esp" "$work/iso/images/efiboot.img"

rm -f "$out"
xorriso -as mkisofs -o "$out" -V "$label" -iso-level 3 -full-iso9660-filenames -joliet -joliet-long -rational-rock \
    -partition_offset 16 -append_partition 2 0xef "$esp" -appended_part_as_gpt \
    -e --interval:appended_partition_2:all:: -no-emul-boot \
    "$work/iso"
ls -l "$out"
