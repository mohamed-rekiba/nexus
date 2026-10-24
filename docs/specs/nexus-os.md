# Spec: Nexus OS

Status: ready for implementation, after the ISO proof (the first step under Implementation
Decisions).

Related specs:
- [releases-and-packaging.md](releases-and-packaging.md): the Linux package and the Nexus
  session that Nexus OS installs, and the release stream it shares.
- [system-plugin.md](system-plugin.md): the SystemPlugin that provides network, Bluetooth,
  sound and power, plus the on-screen keyboard. This spec adds its two OS capabilities.

## Problem Statement

People want to turn a spare PC into a console that starts straight into Nexus: no desktop, no
passwords, no pop-ups, just the launcher on the TV with a controller.

Doing that today means:

- installing a Linux distribution;
- setting up automatic login;
- arranging for Nexus to start full screen;
- keeping the system and the launcher updated by hand.

Most players can't or won't do that. azrak-os proves the idea works, but it ships a different
launcher and lives outside this repository.

Installing also needs two different paths:

- **Erase a disk:** most players want to wipe a disk and be done.
- **Choose partitions:** others want to keep Windows or another system on the same disk, and
  pick partitions themselves.

Players also want to try the system before installing anything.

## Solution

**Nexus OS** is a small, read-only Fedora system (bootc) that starts straight into Nexus. It's
delivered as **one ISO** to write to a USB stick. From the stick a player can:

- **Try it:** Nexus OS starts from the stick, nothing on the PC changes, and it's all
  controller-driven.
- **Install by erasing a disk:** from the launcher's "Install on this PC". Pick a disk, confirm,
  wait a few minutes.
- **Install with your own partitions:** with Fedora's installer (Anaconda), from a second boot
  entry or the launcher's "Choose partitions instead". Keep Windows, use free space, or
  partition by hand. This needs a keyboard and mouse.

Both installs copy the system stored on the stick, so neither needs internet.

The installed PC:

- updates the whole system, Nexus included, in the background;
- applies an update at the next restart, with the previous version kept in the boot menu;
- on PCs with an NVIDIA card, gets NVIDIA's driver by itself.

Nexus OS is released from this repository as its own release stream. Each release has the ISO,
split into parts when it's over 2 GiB.

## User Stories

1. As a player, I want to download one ISO and write it to a USB stick with balenaEtcher, so that getting Nexus OS is simple.
2. As a player, I want to start the PC from the stick and use Nexus OS without changing anything on the PC, so that I can try it first.
3. As a player trying it, I want Nexus to greet me with "Try it" or "Install on this PC", so that I know what I can do.
4. As a player trying it, I want to join Wi-Fi, pair a controller and play from the stick, so that I can see whether my PC runs games well before installing.
5. As a player, I want "Install on this PC" to list my disks with their size and name, and leave out the stick, so that I pick the right one.
6. As a player, I want a disk with Windows on it marked clearly, so that I don't erase Windows by accident.
7. As a player, I want a firm confirmation that names the disk before it's erased, so that one wrong press can't wipe it.
8. As a player, I want to see the install's progress with what it's doing, so that I know it's working.
9. As a player, I want the Wi-Fi I joined on the stick to come along to the installed PC, so that it's online at its first start.
10. As a player, I want an install error to explain what went wrong with a detail I can send, so that I can get help.
11. As a player who wants to keep Windows, I want to choose my own partitions, so that both systems stay on one disk.
12. As a player, I want "Choose partitions instead" in the launcher to take me to the partitioning installer, so that I don't have to know the boot menu.
13. As a player, I want a boot menu entry "Install Nexus OS (choose partitions)", so that I can start the partitioning installer directly.
14. As a player using the partitioning installer, I want only the screens I need (disk, keyboard, time zone), so that installing is short.
15. As a player, I want both installs to work without internet, so that I can install anywhere.
16. As a player, I want the installed PC to start straight into Nexus with no login or desktop, so that it works like a console.
17. As a player, I want a boot screen with the Nexus logo and no text, so that starting looks like a console.
18. As a player with an NVIDIA card, I want NVIDIA's driver set up by itself, so that games run at full speed.
19. As a player with an NVIDIA card and Secure Boot, I want clear instructions for the one-time key screen, so that the driver can load.
20. As a player, I want the system and Nexus updated in the background, so that I never have to update by hand.
21. As a player, I want updates downloaded but not applied until I restart, so that an update never interrupts a game.
22. As a player, I want Settings → System to say "Restart to update" when an update is ready, so that I know when one is waiting.
23. As a player, I want a new Nexus release to reach my PC soon after it's published, so that I get fixes quickly.
24. As a player, I want a bad update to be undoable from the boot menu, so that one broken update can't brick my console.
25. As a player, I want emulators to keep updating themselves as on any system, so that I get emulator fixes without a system update.
26. As a player, I want Nexus restarted by itself if it closes or hangs, so that the TV never shows an empty screen.
27. As a player whose graphics card gamescope can't drive, I want Nexus to still show and explain the problem, so that I'm not left with a black screen.
28. As a player, I want restart and shut down in Settings, so that I can turn the console off from the couch.
29. As a maintainer, I want the OS released by its own release pull request with its own version and changelog, so that OS changes are released separately from Nexus.
30. As a maintainer, I want an OS release to build and check the images and the ISO before publishing, so that a broken ISO is never released.
31. As a maintainer, I want an ISO over GitHub's 2 GiB limit split into parts automatically, so that every release can be published.
32. As a player, I want the README to show how to join split parts on Linux, macOS and Windows, so that I can rebuild the ISO before flashing.
33. As a player, I want a SHA-256 for the whole ISO and for each part, so that I can check my download.
34. As a maintainer, I want OS releases published as "not latest", so that Nexus's updater keeps reading Nexus releases.
35. As a maintainer, I want the OS images rebuilt daily and after every Nexus release, so that installed PCs get Fedora's fixes and new launchers.
36. As a maintainer, I want the image build to fail when a tool or library the OS needs is missing, so that a broken image is never published.
37. As a maintainer, I want the launcher and the OS to share one written contract, so that changing either side doesn't break the other.
38. As a maintainer, I want a test mode that builds the images and ISO under test tags no PC follows, so that I can try changes safely.

## Implementation Decisions

**First step: prove the ISO**
- Build one stick from the current azrak-os minimal image with Anaconda added, using osbuild's
  `image-builder` and its `bootc-generic-iso` type. Boot it in a UEFI VM with Secure Boot.
- It must show:
  1. a live start;
  2. the existing erase-disk installer working;
  3. Anaconda installing the system stored on the stick, offline, keeping an existing Windows
     partition;
  4. the installed system booting and tracking the right image, with Fedora's known
     `systemd-remount-fs` problem for Anaconda-installed bootc systems handled;
  5. the ISO's size;
  6. how the launcher's "Choose partitions instead" reaches Anaconda, in order of preference:
     - switching the running live system to Anaconda's target;
     - `kexec` into the Anaconda boot entry;
     - otherwise, instructions to restart and pick the entry.
- If `image-builder` can't produce a live start, the stick is built by extending azrak-os's
  hand-made live ISO script, still as one ISO with both boot entries.
- The results decide the details of the ISO below before it's built.

**The images**
- **The minimal edition:**
  - Fedora bootc's minimal set;
  - graphics drivers and firmware, gamescope, cage and greetd;
  - NetworkManager, BlueZ and PipeWire;
  - the boot screen and fonts;
  - the Nexus rpm from the latest Nexus release.

  Ported from azrak-os and renamed to Nexus OS throughout: os-release, paths, marker, services
  and polkit rules.
- **The NVIDIA edition:** the minimal edition plus NVIDIA's driver from Universal Blue's
  prebuilt modules, built for the minimal image's exact kernel. Until modules exist for a new
  kernel, the previous NVIDIA image stays.
- The images are published to the GitHub container registry under the repository owner, with
  the tags `minimal` and `minimal-nvidia`. Dated tags and version tags keep earlier versions at
  hand. Packages must be public, and the workflow checks this.
- **The image adds only what's OS-specific:**
  - greetd logs the `player` user in and runs the Nexus session from the package;
  - the Nexus OS marker, which holds the bundled Nexus version;
  - the update timer and its service;
  - polkit rules letting `player` manage the network, run the update check, and run the
    installer on the stick;
  - the boot screen;
  - kernel arguments for a quiet boot.
- After building, each image is checked: required tools present (gamescope, cage, nmcli,
  bluetoothctl, pactl, bootc, rpm-ostree), required libraries present, the services enabled,
  Fedora's own auto-restarting update timer masked, and Nexus answering `--version` and
  `--supports` for the OS features.
- The minimal edition is rechunked into one layer per package, so a daily rebuild downloads only
  what changed.

**Updates**
- The update service runs daily, from a few minutes after start, and every 15 minutes while
  offline. It stages the next image and never restarts the PC. It also moves a PC with an
  NVIDIA card to the NVIDIA image, once Secure Boot allows it.
- Nexus on the OS is installed by the image's rpm. Its own update method is "comes with the
  next system update".
- When Nexus sees a newer Nexus release than its own, it asks the OS update capability to check
  now, rather than waiting for the daily run.
- Settings → System shows "Restart to update" when a staged image exists.
- Emulators update themselves in the home folder as on any system.

**The stick**
- A live container built from the minimal image. It adds dracut's live module, Anaconda, and
  the boot entries. The minimal image is stored inside for offline installs.
- **Boot entries:**
  - "Nexus OS" (the default): starts live, with changes kept in memory;
  - "Install Nexus OS (choose partitions)";
  - a basic-graphics entry.

  The boot menu stays hidden unless Esc or Shift is held. Fedora's signed shim, GRUB and kernel
  keep Secure Boot working.
- **The erase-disk installer** keeps azrak-os's protocol word for word, renamed to Nexus OS:
  - a disk list: one line per disk with path, size, whether Windows is on it, whether it's USB,
    and the model. The stick and disks under 40 GB are left out;
  - an install run as root through polkit, printing progress lines, then done (with the edition
    installed and flags for the Secure Boot key screen and a later NVIDIA driver), or an error.
- It installs the stored image with bootc. On an NVIDIA PC with internet it installs the NVIDIA
  edition instead, downloading only the driver layers. It queues Universal Blue's Secure Boot
  key when needed, and copies saved Wi-Fi to the installed system.
- **The Anaconda install:**
  - a kickstart with no partitioning, so Anaconda's disk screen decides;
  - the user and root password screens turned off, because the image already has `player`;
  - defaults for language and time zone;
  - a post-install step that queues the Secure Boot key on NVIDIA PCs and makes the installed
    system track the minimal image in the registry.

**SystemPlugin capabilities for the OS** (see [system-plugin.md](system-plugin.md))
- `os_updates`:
  - the update state: up to date, checking, staged (with its version), or failed;
  - "check now", which starts the update service through polkit.

  Provided on Nexus OS by the Linux system plugin, using `rpm-ostree status` and `systemctl`.
- `os_install`:
  - the disk list;
  - the erase-disk install as a job, with the protocol's progress, done and error;
  - the way to start the partitioning installer chosen by the ISO proof.

  Provided only on the stick.
- **Detection:**
  - the Nexus OS marker means Nexus OS;
  - the live kernel argument plus the installer means the stick;
  - environment variables pretend either, for tests and screenshots;
  - new `--supports` names: `system-settings` and `os-install`.

  The ISO build refuses a Nexus without `os-install`.
- The theme shows "Welcome to Nexus OS" first on the stick, and the install screens: disk list,
  Windows warning, firm confirmation, progress, done, the Secure Boot key instructions, the
  error, and "Choose partitions instead".

**The OS release stream**
- Release Please component `nexus-os`, with the OS folder's own version file and changelog, and
  tags `nexus-os-v<version>`. Its changes are left out of Nexus's stream.
- **Merging its release pull request:**
  1. builds and checks the images, and tags them with the version;
  2. builds the ISO;
  3. splits it into 1500 MB parts when it's over 2 GiB;
  4. writes a SHA-256 for the whole ISO and for each part;
  5. attaches them to the draft;
  6. publishes the release as not latest.
- The README's "Download Nexus OS" section carries the OS version, which Release Please bumps in
  the same pull request. It shows how to join parts (`cat` on Linux and macOS, `copy /b` on
  Windows), check the SHA-256, and flash with balenaEtcher.
- The images are rebuilt on a daily schedule and after each published Nexus release. A new ISO
  is made only by an OS release, or by hand in test mode.
- In test mode the images and ISO are built from a branch under test tags no PC follows. The
  ISO is kept as a workflow artifact, not released.
- A contract document in the docs describes the launcher–OS contract: the marker, the
  installer's protocol, the polkit rules, the session's "window shown" line and the
  `--supports` names. It replaces azrak-launcher's AzrakOS document for Nexus.

## Testing Decisions

- **What makes a good test:** it drives a public seam and checks what a player would see, or
  what was sent to the system. Expected results come from recorded tool output and the
  installer protocol's known lines, not from re-computing them the way the code does.
- **Seam 1: the core through `FakeHost`, with the fake system plugin** (prior art: the
  `TestCore` suites):
  - on the stick: the welcome, then the disk list, then an install job's progress lines, then
    done with the Secure Boot key flag, or an error with its text;
  - on Nexus OS: the "comes with the next system update" method, the check-now request when a
    newer Nexus release appears, and "Restart to update" for a staged image.
- **The installer protocol parsers** are tested with the ported azrak-launcher test set.
- **Seam 2: the composed host on Slint's testing backend** (prior art: the default theme
  tests):
  - the welcome and install screens driven with only a controller;
  - the Windows warning;
  - the confirmation that names the disk;
  - "Choose partitions instead";
  - screenshots in pretend-stick mode.
- **Seam 3: the image checks in CI** (prior art: azrak-os's image checks): required tools,
  libraries, services and masked timers, and Nexus's `--version` and `--supports`.
- **Seam 4: the ISO in a UEFI VM with Secure Boot** (QEMU with OVMF):
  - live start;
  - an erase-disk install to an empty disk;
  - an Anaconda install that keeps an existing partition;
  - the first boot of each installed system.

  This runs for the ISO proof and before releases. It runs in CI if the runner supports KVM, or
  else locally with a documented script. A VM has no GPU Vulkan, so the session falls back to
  cage there.

## Out of Scope

- The Bazzite-based editions azrak-os still builds.
- Changes to the azrak-os and azrak-launcher repositories.
- ARM PCs.
- Legacy BIOS boot (UEFI only).
- Installing to a disk with the launcher's installer while keeping other partitions (that's
  Anaconda's job).
- A graphical first-start wizard beyond the welcome screen.

## Further Notes

- azrak-os (minimal, NVIDIA and live editions, the installer, the update service and the
  session) and azrak-launcher's OS module are the working references. Their behaviour is kept
  unless this spec says otherwise.
- One ISO with both install paths depends on the ISO proof. Its fallback is still one ISO.
- The launcher updates with the OS on purpose: updates are atomic, can be rolled back from the
  boot menu, and package files are never swapped in place on a read-only system.
- A new Nexus release reaches installed PCs after the image rebuild it triggers, about an hour
  later, and applies at the next restart.
