# Nexus OS: how it's built

A PC that starts straight into Nexus. This page is for people working on it; players start at the
[README](../README.md#download-nexus-os). What Nexus and the OS expect from each other (the
marker, the installer's protocol, the polkit rules, `nexus --supports`) is in
[docs/nexus-os.md](../docs/nexus-os.md), and the reasons are in
[docs/specs/nexus-os.md](../docs/specs/nexus-os.md).

There are two images and one USB stick:

| | What | Published as |
|---|---|---|
| Minimal edition ([Containerfile](Containerfile), [files/](files/)) | Fedora's smallest bootable system, gamescope and Nexus: no desktop | `ghcr.io/<owner>/nexus-os:minimal` |
| NVIDIA edition ([nvidia/](nvidia/)) | The minimal edition plus NVIDIA's driver, as layers on top | `:minimal-nvidia` (only for NVIDIA PCs) |
| USB stick ([live/](live/), [build-iso.sh](build-iso.sh)) | Nexus OS started from the stick: try it, or install it by erasing a disk or with your own partitions | `nexus-os-x86_64.iso` in each `nexus-os-v*` release |

Each image also gets a dated tag (`minimal-20261024`) and, from an OS release, a version tag
(`minimal-0.2.0`).

## Minimal edition

| File | What it does |
|---|---|
| `Containerfile` | Fedora's minimal bootc set (`bootc-base-imagectl build-rootfs --manifest=minimal`: bootc, systemd, the kernel, dnf), then drivers and firmware, gamescope, cage, greetd, NetworkManager, BlueZ, PipeWire, the boot screen, and what Nexus and the emulators load. Then the nexus rpm from `nexus-rpm/`. Universal Blue's kernel-module key is stored for the NVIDIA edition. os-release says Nexus OS. |
| `fetch-nexus.sh` | Downloads the nexus rpm from the latest Nexus release (or a given tag) into `nexus-rpm/`, checked against its `.sha256`. |
| `files/etc/greetd/config.toml` | Logs `player` in at boot and runs `/usr/bin/nexus-session`, the session from the nexus package (`packaging/linux/nexus-session`). |
| `files/usr/lib/nexus-os/` (made at build time) | `release`, the marker, holds `nexus --version`. `image` holds where the PC updates from. |
| `files/usr/share/plymouth/themes/nexus-os/` | The boot screen: the Nexus logo, with Fedora's spinner images copied in at build time. |
| `files/usr/share/polkit-1/rules.d/50-nexus-os.rules` | `player`, at the PC, manages the network and may start the update service ("check now"). |
| `files/usr/lib/sysusers.d/`, `tmpfiles.d/` | The `player` user (uid 1000, no password) and its home. |
| `files/usr/libexec/nexus-os/update`, `files/usr/lib/systemd/system/nexus-os-update.*` | Daily, from 3 minutes after start (and every 15 minutes while offline): stage the next image, and the right one for the graphics card (an NVIDIA PC switches to `:minimal-nvidia`; with Secure Boot on, only once Universal Blue's key is enrolled, which it queues). Downloads only: Fedora's own timer, which restarts the PC, is masked. |
| `files/usr/lib/bootc/{install,kargs.d}/` | XFS root, a quiet boot with the splash, and `systemd.gpt_auto=0` (otherwise the EFI partition is mounted over `/boot` and updates fail to finish). |
| `files/usr/lib/bootupd/grub2-static/configs.d/` | The boot menu stays hidden unless Esc or Shift is held. |

The build is a `FROM scratch` image, so the workflow splits it into one layer per package
(`bootc-base-imagectl rechunk`, timestamps zeroed): a daily rebuild only changes the layers whose
packages changed, and that's all a PC downloads.

Nexus updates with the system: the image is rebuilt after every Nexus release, and installed PCs
stage it with their next update.

## NVIDIA edition

NVIDIA's 580 driver series (today's cards and the GTX 900/1000 ones newer series dropped) from
Universal Blue's `akmods-nvidia-lts`, built for the minimal edition's exact kernel and signed
with their key. The workflow builds it right after `:minimal`, for that image's kernel; until
Universal Blue has modules for a new kernel (usually within a day) it keeps the previous one.
No CUDA tools, settings app or 32-bit libraries; nouveau off, kernel modesetting on.

Nobody picks it: the erase-disk installer installs it on an NVIDIA PC with internet (downloading
only the driver layers), and otherwise the installed PC switches to it by itself
(`nexus-os/update`).

## USB stick

`live/Containerfile` is the minimal edition plus dracut's live module, Anaconda, and the boot
files. It's never pushed. `build-iso.sh` turns it into one ISO, with the minimal image stored
inside it for offline installs.

Boot menu (`live/files/usr/lib/image-builder/bootc/iso.yaml`):

| Entry | What starts |
|---|---|
| Nexus OS (the default) | Nexus OS from the stick, changes kept in memory. Nexus shows "Welcome to Nexus OS": try it, or install it. |
| Install Nexus OS (choose partitions) | The same live system, with `systemd.unit=nexus-os-anaconda.target`: Anaconda full screen in cage instead of Nexus. |
| Nexus OS (basic graphics) | The first entry with `nomodeset`. |

- The minimal image is stored as a read-only container store in the stick's root filesystem
  (`/var/lib/containers/storage` there). The live system sees it at
  `/run/rootfsbase/var/lib/containers/storage` (`live/files/etc/containers/storage.conf`).
  Podman's own storage and `/var/tmp` are in memory on the stick.
- **Erase a disk:** `live/files/usr/libexec/nexus-os/install` (run by Nexus as `nexus-os-install`, a link in `/usr/bin`) lists the disks and installs
  (`bootc install to-disk` from the stored image), with NVIDIA's driver when it applies, and
  copies saved Wi-Fi to the installed system. A polkit rule lets `player` run it, on the stick
  only.
- **Choose partitions:** `live/files/usr/share/anaconda/interactive-defaults.ks` is Anaconda's
  kickstart. It has no partitioning, so the disk screen decides. It installs the stored image
  (`ostreecontainer --transport=containers-storage`), sets English and UTC as defaults, and
  locks root; `live/files/etc/anaconda/conf.d/90-nexus-os.conf` hides the user, root password
  and network screens. Its `%post` makes the installed PC follow `ghcr.io/<owner>/nexus-os:minimal`,
  drops the `/` line Anaconda writes in `/etc/fstab` (it makes `systemd-remount-fs` fail on a
  bootc system), and queues Universal Blue's key on an NVIDIA PC with Secure Boot.
  `nexus-os-install --choose-partitions` switches a running stick to Anaconda (`systemctl isolate nexus-os-anaconda.target`),
  for Nexus's "Choose partitions instead".
- `build-iso.sh` refuses a Nexus without the install screens (`nexus --supports os-install`),
  so release Nexus before an OS change that needs it.
- In a test build the stored image is the `-test` one, but it's still called `:minimal` on the
  stick, so the installers find it. A PC installed from it moves to the real `:minimal` with
  its first update.

### Two ways to make the ISO

`build-iso.sh` makes the ISO with osbuild's
[image-builder](https://osbuild.org/docs/developer-guide/projects/image-builder/advanced/bootc/isos/)
and its `bootc-generic-iso` type by default: the live container becomes the squashfs, GRUB's
menu comes from `iso.yaml`, and `--bootc-installer-payload-ref` copies the minimal image into it.

If image-builder can't make a stick that starts live (or isn't available), `METHOD=xorriso`
makes the same ISO by hand, as azrak-os did: the live container's files in a squashfs, the
minimal image pushed into its `/var/lib/containers/storage`, Fedora's shim and GRUB on an EFI
partition, and xorriso. Its GRUB menu is made from the same `iso.yaml`, hidden unless Esc or
Shift is held. Both make one ISO with the same entries.

image-builder's `iso.yaml` has no hidden-menu setting, so its menu shows for 5 seconds.

## Releases

The OS is its own Release Please package (`os/`, component `nexus-os`): its own release pull
request, `version.txt`, `CHANGELOG.md` and tags `nexus-os-v<version>`. Nexus's releases leave
`os/` out. Merging the OS release pull request makes a draft release and its tag
(`.github/workflows/release.yml`), which starts `.github/workflows/os.yml` on the tag, in a run of
its own, which:

1. builds, checks and pushes both images from the tag, tagged with the version too;
2. builds the ISO from the minimal image just pushed;
3. splits it into 1500 MB parts when it's over 2 GiB, with a `.sha256` for the whole ISO and for
   each part;
4. attaches them to the draft and publishes it, not as the latest, so Nexus's updater keeps
   reading Nexus releases.

The release pull request also bumps the version in the README's "Download Nexus OS".

The images bundle the latest published Nexus release, so the first OS release needs one: merge
Nexus's release pull request first. When one push releases both, `package.yml` starts the OS
release once Nexus's release is published. When no Nexus release is published, the OS run stops at its start with
"release Nexus first". If an OS run fails, its draft and tag stay: run the failed jobs again (or
Actions → Nexus OS → Run workflow, with the tag under "Use workflow from") rather than making a
new release.

`os.yml` also runs daily, and after each Nexus release (`package.yml` starts it), for the images
only. By hand (Actions → Nexus OS → Run workflow) it runs in test mode by default: images under `-test` tags no PC
follows, and the ISO kept as the run's `nexus-os-iso` artifact for 3 days.

The images must be public packages: the installers and installed PCs pull without logging in.
`os.yml` checks this; GitHub creates packages private, so change it once in the package's
settings.

## Build and test locally

The images and the stick need podman as root (bootc-base-imagectl, rechunking and image-builder
need real privileges, loop devices included). From the repository root:

```bash
# The nexus rpm: from the latest release, or a local one (packaging/linux/build-packages.sh) copied to os/nexus-rpm/.
os/fetch-nexus.sh
sudo podman build --cap-add=all --security-opt=label=disable --device /dev/fuse \
  -f os/Containerfile -t localhost/nexus-os:minimal-build .
sudo podman run --rm --privileged -v /var/lib/containers:/var/lib/containers quay.io/fedora/fedora-bootc:44 \
  /usr/libexec/bootc-base-imagectl rechunk localhost/nexus-os:minimal-build ghcr.io/<owner>/nexus-os:minimal
sudo podman run --rm --privileged -v /var/lib/containers:/var/lib/containers -v "$PWD:/src" -w /src \
  -e CORE=ghcr.io/<owner>/nexus-os:minimal quay.io/fedora/fedora-bootc:44 os/build-iso.sh nexus-os-x86_64.iso
```

Add `-e METHOD=xorriso` to the last command for the hand-made ISO.

Boot the ISO in a UEFI VM as a USB stick, with Secure Boot, 6 GB of memory, and a disk to install
on. With QEMU and Fedora's `edk2-ovmf`:

```bash
qemu-img create -f qcow2 disk.qcow2 64G
cp /usr/share/edk2/ovmf/OVMF_VARS.secboot.fd vars.fd
qemu-system-x86_64 -enable-kvm -machine q35,smm=on -m 6G -smp 4 \
  -global driver=cfi.pflash01,property=secure,value=on \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/ovmf/OVMF_CODE.secboot.fd \
  -drive if=pflash,format=raw,file=vars.fd \
  -device qemu-xhci -drive if=none,id=stick,format=raw,file=nexus-os-x86_64.iso \
  -device usb-storage,drive=stick,bootindex=1 \
  -drive file=disk.qcow2,if=virtio -nic user,model=virtio
```

The ISO from a test run of `os.yml` works too: its `nexus-os-iso` artifact. To check the
partitioning install, give that disk an NTFS partition first and keep it in Anaconda's disk
screen. A VM has no GPU Vulkan, so Nexus shows in cage there, and draws slowly; nor has it Wi-Fi
or Bluetooth. Those controls are tested on a PC from the stick, or on a desktop with
`NEXUS_SESSION=1` ([development.md](../docs/development.md)).

What to check before a release:

- the live start, and Nexus's welcome;
- an erase-disk install to an empty disk, then its first start;
- an Anaconda install that keeps an existing partition, then its first start:
  `systemctl status systemd-remount-fs` is fine, and `bootc status` shows
  `ghcr.io/<owner>/nexus-os:minimal`;
- "Choose partitions instead" from the running stick reaches Anaconda.
