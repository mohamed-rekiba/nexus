# Fedora's installer (Anaconda) on the Nexus OS USB stick, installing with your own partitions.
# No partitioning here: Anaconda's disk screen decides (keep Windows, use free space, or
# partition by hand). @IMAGE@ is replaced at build time (os/live/Containerfile) with where
# installed PCs update from, ghcr.io/<owner>/nexus-os.
lang en_US.UTF-8
# Both the console and the graphical layout: with one missing, Anaconda on a live system asks
# the desktop's live user for it, and the stick has none.
keyboard --vckeymap=us --xlayouts='us'
timezone Etc/UTC --utc
# The image has its own user, "player", logged in automatically with no password.
rootpw --lock
firstboot --disable

# The system stored on the stick (os/build-iso.sh), so no internet is needed.
ostreecontainer --url=@IMAGE@:minimal --transport=containers-storage --no-signature-verification

# No bootloader line: the image's own kernel arguments (os/files/usr/lib/bootc/kargs.d) are
# applied by the deployment, and Anaconda adds root= and rw.

# The installed PC updates from the registry, not from the stick's storage. And
# systemd-remount-fs: Anaconda writes a / line in /etc/fstab, which fails to remount a bootc
# system's root ("overlay: No changes allowed in reconfigure"); `bootc install` writes no such
# line, so neither do we. /mnt/sysimage is the disk's root here, and the deployment is under it.
%post --nochroot --erroronfail --log=/tmp/nexus-os-post.log
set -eux
shopt -s nullglob
origins=(/mnt/sysimage/ostree/deploy/*/deploy/*.origin)
[ "${#origins[@]}" -gt 0 ]
for origin in "${origins[@]}"; do
    sed -i 's|^container-image-reference=.*|container-image-reference=ostree-unverified-registry:@IMAGE@:minimal|' "$origin"
    cat "$origin"
done
for fstab in /mnt/sysimage/ostree/deploy/*/deploy/*.0/etc/fstab; do
    awk '$1 !~ /^#/ && $2 == "/" { print "# " $0 " (not for bootc: systemd-remount-fs)"; next } { print }' "$fstab" > "$fstab.new"
    cat "$fstab.new" > "$fstab"
    rm "$fstab.new"
    cat "$fstab"
done
%end

# NVIDIA's driver is signed with Universal Blue's key. On an NVIDIA PC with Secure Boot on, queue
# the key now (the blue screen at the first restart, password universalblue); the installed system
# switches to the NVIDIA version once it's enrolled (nexus-os/update).
%post --nochroot --log=/tmp/nexus-os-secureboot.log
key=/usr/share/nexus-os/secureboot.der
nvidia=
for dev in /sys/bus/pci/devices/*; do
    [ "$(cat "$dev/vendor" 2>/dev/null)" = 0x10de ] && grep -q '^0x03' "$dev/class" 2>/dev/null && nvidia=1
done
if [ -n "$nvidia" ] && [ -d /sys/firmware/efi ] && mokutil --sb-state 2>/dev/null | grep -qi "SecureBoot enabled" &&
    ! mokutil --test-key "$key" 2>/dev/null | grep -qi "already enrolled"; then
    echo "NVIDIA card and Secure Boot: queueing Universal Blue's key for the next restart"
    printf 'universalblue\nuniversalblue\n' | mokutil --import "$key" && mokutil --timeout -1 || :
fi
%end
