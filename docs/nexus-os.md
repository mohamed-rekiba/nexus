# Nexus and Nexus OS

Nexus OS is a PC image that starts straight into Nexus. No desktop is behind Nexus. So the
screens the OS needs (internet, controllers, sound, system updates, installing from the USB
stick) are part of Nexus, and they show only where the OS needs them.

This page lists what Nexus expects from the OS and what the OS expects from Nexus. Change both
sides together. How the images are built and released is in [os/README.md](../os/README.md).
The code around it is in [architecture.md](architecture.md) and [plugins.md](plugins.md).

## Where Nexus runs

`Place::detect` in `crates/nexus-core/src/system.rs` decides where Nexus runs. It reads three
facts.

| Place field | True when | Pretend with |
| --- | --- | --- |
| `nexus_os` | The file `/usr/lib/nexus-os/release` exists, or the stick is true. | `NEXUS_OS=1` |
| `stick` | `nexus_os` is true, the kernel command line (`/proc/cmdline`) has `rd.live.image`, and the file `/usr/libexec/nexus-os/install` exists. | `NEXUS_USB=1` |
| `whole_screen` | Nexus is the whole screen: `NEXUS_SESSION=1` is set, or `nexus_os` or `stick` is true. | `NEXUS_SESSION=1` |

The three variables count only when their value is exactly `1`. They exist for tests and
screenshots. `NEXUS_SESSION=1` is also what the session script sets for the real thing.

What each field changes in the default theme (`plugins/themes/default/theme/`):

| Field | Effect |
| --- | --- |
| `whole_screen` | The top bar shows the status buttons. Settings shows the SYSTEM section, with Restart and Shut down when the plugin allows them. "Quit Nexus" reads "Restart Nexus". |
| `nexus_os` | Log out is hidden, because greetd would start the session again at once. |
| `stick` | The "Welcome to Nexus OS" and install screens show. Settings shows the install row. |

The system update row in Settings shows when the plugin reports an update state. The plugin
does that only where the marker file exists.

The marker file holds one line, the output of `nexus --version` of the Nexus in the image. The
image writes it. Nexus only tests that it exists.

The image also writes `/usr/lib/nexus-os/image`. It holds where the PC updates from, without a
tag: `ghcr.io/<owner>/nexus-os`. The OS scripts read it. Nexus does not.

## What Nexus runs on Nexus OS

The built-in system plugin (`plugins/system/host/`) does the work. It declares the
capabilities `network`, `vpn`, `bluetooth`, `audio`, `power`, `os_updates` and `os_install`.
On macOS and Windows it keeps only `vpn` and `power`. The tools it may run are the
`permissions.process` list in `plugins/system/host/plugin.toml`.

| Nexus feature | Nexus runs | The image provides |
| --- | --- | --- |
| Internet, Join Wi-Fi | `nmcli` | NetworkManager, `wpa_supplicant`, and a polkit rule (see below) |
| Controllers and headsets | `bluetoothctl` | BlueZ |
| Sound output | `pactl` | PipeWire with its PulseAudio server |
| Restart, Shut down | `systemctl reboot`, `systemctl poweroff` | systemd; logind lets the active local session do both |
| System update state | `rpm-ostree status --json` | `rpm-ostree` |
| System update, check now | `systemctl start --no-block nexus-os-update.service` | the update service and a polkit rule |
| Installing from the stick | `nexus-os-install` | the stick's installer |

If a tool is missing, the setting says which one, such as "NetworkManager (nmcli) isn't
installed". It does not crash. The test is in `plugins/system/host/src/linux.rs`.

### System updates

The OS updates as a whole image with bootc. The previous version stays in the boot menu.

`nexus-os-update.timer` runs `nexus-os-update.service`. The timer fires 3 minutes after boot
(`OnBootSec`) and then every day (`OnUnitActiveSec`), with up to 10 minutes of random delay.
If the service fails, for example with no internet, systemd runs it again after 15 minutes
(`RestartSec`). The service does not run on the USB stick (`ConditionKernelCommandLine=!rd.live.image`).

The script `/usr/libexec/nexus-os/update` only stages the next version. It never restarts the
PC, because a game may be running. The image masks Fedora's own `bootc-fetch-apply-updates.timer`
for the same reason: that timer restarts the PC by itself.

The script also picks the edition by the graphics card. An NVIDIA card (PCI vendor `0x10de`,
class `0x03`) gets `:minimal-nvidia`. Other cards get `:minimal`. With Secure Boot on, the
NVIDIA edition loads only once Universal Blue's key is enrolled. Until then the script queues the
key with `mokutil` (the blue screen at the next restart asks for it) and stays on `:minimal`.

Nexus reads the result with `rpm-ostree status --json` (`OsUpdates::status`):

| Result | Nexus shows |
| --- | --- |
| A staged deployment exists | "Restart to update to `<version>`" |
| The booted deployment exists and none is staged | "Nexus OS is up to date" |
| The output cannot be read | "Couldn't look for an update: rpm-ostree's status couldn't be read" |

`OsUpdates::status` fails with "this isn't Nexus OS" when the marker file is missing. The
Settings row then does not show.

### Nexus's own update

Nexus does not download its own update on Nexus OS. `Install::detect` in
`crates/nexus-core/src/launcher_update.rs` returns `NexusOs` when the file `packaged` sits
beside the binary and the marker file exists. `package_key` then returns nothing. The check
still reads `latest.json`, and if a newer version exists, Nexus shows "Nexus `<version>` is
out · it comes with the next system update". It then starts the same check as "check now", so
the OS looks for the new image.

The image installs the rpm from a Nexus release (`os/fetch-nexus.sh`, `os/Containerfile`).
The OS release process rebuilds the image after each Nexus release.

## The session

greetd logs in the user `player` at boot and runs `/usr/bin/nexus-session`. The file is
`os/files/etc/greetd/config.toml`: terminal 1, and the same command for the initial and the
default session. `nexus-session` comes from the Nexus package (`packaging/linux/nfpm.yaml`),
and the desktop entry `packaging/linux/nexus-session.desktop` lists it as "Nexus" in display
managers.

The script `packaging/linux/nexus-session`:

1. Reads `/usr/share/nexus/session.conf`, then `/etc/nexus/session.conf`.
2. Rotates the log (below).
3. Exports `NEXUS_SESSION=1`, and `SDL_VIDEO_DRIVER=x11`, `SDL_VIDEODRIVER=x11`,
   `QT_QPA_PLATFORM=xcb` and `GDK_BACKEND=x11`. Games inherit these. Gamescope's X server is
   where Nexus and the games run, and Steam mode's focus needs that.
4. Waits for the network with `nm-online`, up to `NEXUS_WAIT_NETWORK` seconds, then starts
   anyway. Nexus retries what failed once it is online.
5. Starts `gamescope <NEXUS_GAMESCOPE_ARGS> -- <NEXUS_BIN>` and appends all output to the log.
6. Starts again when Nexus ends.

It runs Nexus in `cage` instead of gamescope when any of these is true: gamescope is not
installed, `nexus --supports gamescope` fails, or the last two runs in gamescope ended early
(the watchdog killed them, or they ended in under 15 seconds). If neither gamescope nor cage is
installed, it runs Nexus in the current desktop when there is one, else it exits with an error.

Settings in `packaging/linux/session.conf`:

| Name | Default | Meaning |
| --- | --- | --- |
| `NEXUS_BIN` | `/usr/bin/nexus` | The launcher to run. |
| `NEXUS_GAMESCOPE_ARGS` | `-e --xwayland-count 1` | Options for gamescope. `-e` is its Steam mode: Nexus and games swap with gamescope's focus atoms. |
| `NEXUS_WATCHDOG` | `90` | Seconds to wait for `window shown` before it starts Nexus again. |
| `NEXUS_WAIT_NETWORK` | `5` | Seconds to wait for the network at the start. |
| `NEXUS_LOG` | `${XDG_CACHE_HOME:-$HOME/.cache}/nexus-session.log` | Where the session and Nexus write. |
| `NEXUS_LOG_MAX_MB` | `10` | Size at which the log becomes `nexus-session.previous.log`. |

The package ships these defaults. A player copies a line to `/etc/nexus/session.conf`, which
package upgrades leave alone. Before each start, a log over `NEXUS_LOG_MAX_MB` replaces the
previous log, so the log never fills the disk and no logrotate is needed.

### Lines between Nexus and the session

Nexus writes two lines to standard output, and the session reads them in the log.

| Line | Written when | The session |
| --- | --- | --- |
| `window shown` | The app shell's tick says the window shows (`crates/nexus-ui-host/src/shell.rs`, through `Platform::announce`). | Marks the start as good. Without it after the watchdog time, it kills the display server and starts again. |
| `log out` | The player chooses Log out, just before Nexus quits. | Ends the session, back to the login screen. |

Under gamescope, the window shows only once it carries gamescope's id. `nexus-app` sets it with
`xprop` (`crates/nexus-app/src/desktop.rs`). `nexus-app` needs the tools `xdotool` and `xprop` for this, and the image installs both.

## polkit rules

Two rule files grant `player` what Settings needs. Each rule applies only when the subject is
`player` in a local, active session.

| Action | Allowed | File |
| --- | --- | --- |
| `org.freedesktop.NetworkManager.*` | everything | `os/files/usr/share/polkit-1/rules.d/50-nexus-os.rules` |
| `org.freedesktop.systemd1.manage-units` | unit `nexus-os-update.service`, verb `start` | the same file |
| `org.freedesktop.policykit.exec` | program `/usr/bin/nexus-os-install` or `/usr/libexec/nexus-os/install` | `os/live/files/usr/share/polkit-1/rules.d/50-nexus-os-live.rules` (stick only) |

Joining a new Wi-Fi network saves a system-wide connection, which polkit reserves for an
administrator. The first rule lifts that for `player`. `systemctl start` needs no `pkexec`,
because systemd asks polkit itself. For `pkexec`, current polkit reports the resolved path as
`program` and older versions report the given path, so the rule allows both.

The system plugin lists `system:nexus-os-install` under `permissions.polkit`. Only a system
plugin may do that. Nexus then runs the installer through `pkexec` and the path it finds on the
`PATH`.

## The USB stick installer

The stick is the minimal image plus a live boot layer (`os/live/Containerfile`). It keeps a
copy of the minimal image, so installs need no internet. Two ways to install exist. The
installer script `os/live/files/usr/libexec/nexus-os/install` does the first. Nexus only shows it.

Plugins run only `system:<name>` programs. So Nexus calls the installer by the name
`nexus-os-install`. That name is a link, `os/live/files/usr/bin/nexus-os-install`, to
`../libexec/nexus-os/install`. The file `/usr/libexec/nexus-os/install` is also part of how
Nexus detects the stick.

### Erase a disk

`nexus-os-install --disks` runs as `player`. It prints one line per disk it can install on:

```
disk PATH SIZE_BYTES WINDOWS(0/1) USB(0/1) MODEL…
```

- It leaves out the stick, read-only disks, and names that start with `zram`, `loop` or `sr`.
- It leaves out disks smaller than `NEXUS_OS_MIN_DISK`, which is 40 × 10^9 bytes by default.
- `WINDOWS` is 1 when the disk has an NTFS or BitLocker partition.
- `USB` is 1 when the disk's transport is USB.
- `MODEL` is the rest of the line and may hold spaces.

`pkexec /usr/bin/nexus-os-install DISK` erases the disk and installs Nexus OS from the stored
image with `bootc install to-disk --wipe`. It must run as root and refuses a disk that `--disks`
does not list. It prints one line at a time:

| Line | Meaning |
| --- | --- |
| `progress PERCENT TEXT` | Progress. |
| `done TAG [mok] [nvidia-later]` | The install ended. `TAG` is the edition, `minimal` or `minimal-nvidia`. `mok`: the key screen of Secure Boot comes at the restart. `nvidia-later`: the PC has an NVIDIA card and gets its driver on the installed system. |
| `error TEXT` | The install failed. The exit code is not zero. |

Nexus ignores every other line, on standard output or standard error (`parse::install_line`
in `plugins/system/host/src/parse.rs`). bootc writes its own output to standard error.

On a PC with an NVIDIA card, internet and more than 6 GiB of available memory, the installer
downloads the NVIDIA edition, only the driver layers. With Secure Boot on and the key not enrolled, it queues
the key and installs `minimal`. In both cases it lets the installed system switch to the NVIDIA
edition later (`nexus-os/update`).

The installer copies the Wi-Fi connections saved on the stick into the new system.

If the installer ends without a `done` or `error` line, Nexus shows "The installer stopped
without finishing". Exit codes 126 and 127 show "The installer wasn't allowed to run".

### Choose partitions

Fedora's installer, Anaconda, installs the same stored image with partitions the player picks.
It needs a keyboard and a mouse. Two ways lead to it:

- The boot entry "Install Nexus OS (choose partitions)" starts
  `systemd.unit=nexus-os-anaconda.target`.
- Nexus's "Choose partitions instead" runs `pkexec /usr/bin/nexus-os-install --choose-partitions`.
  The script switches the running stick to `nexus-os-anaconda.target` with `systemctl isolate`,
  and exits 0 once the switch has started. greetd and Nexus stop right after. If it cannot
  (not root, not on the stick, the switch refused), it prints one `error TEXT` line and exits
  with a non-zero code.

The service `nexus-os-anaconda.service` runs `/usr/libexec/nexus-os/anaconda`, which ends the
`player` session and runs `liveinst` full screen in `cage`. When Anaconda ends after installing
(the log `/tmp/packaging.log` holds `ostree container image deploy`), the PC restarts. When it
ends without installing, the script returns to `graphical.target`, so Nexus comes back and a
broken start cannot loop through restarts.

The kickstart is `os/live/files/usr/share/anaconda/interactive-defaults.ks`. It sets no
partitioning, so the disk screen decides. It installs the stored image with
`ostreecontainer --transport=containers-storage`, then, in `%post`:

- It points the installed PC at the registry (`container-image-reference=ostree-unverified-registry:…:minimal`), not at the stick.
- It comments out the `/` line Anaconda writes in `/etc/fstab`, which makes
  `systemd-remount-fs` fail on a bootc system.
- On an NVIDIA PC with Secure Boot on, it queues Universal Blue's key.

### Boot menu of the stick

`os/live/files/usr/lib/image-builder/bootc/iso.yaml` holds the entries. All start the same live
system with changes kept in memory.

| Entry | What starts |
| --- | --- |
| Nexus OS (default) | Nexus. It shows "Welcome to Nexus OS": try it, or install it. |
| Install Nexus OS (choose partitions) | Anaconda, through `systemd.unit=nexus-os-anaconda.target`. |
| Nexus OS (basic graphics) | The first entry with `nomodeset`. |

## What the image sets

| What | Where | Why |
| --- | --- | --- |
| Base | `os/Containerfile`: `bootc-base-imagectl build-rootfs --manifest=minimal` | Fedora's smallest bootable set. The standard base is a headless server that a console never uses. |
| Packages | `os/Containerfile` | Gamescope, cage, greetd, Mesa, firmware, NetworkManager, BlueZ, PipeWire, GameMode, the libraries Nexus loads, `libarchive`, `mpv-libs`, `yt-dlp`, `xdotool`, `xprop`, `rpm-ostree`, `chrony`, `mokutil`, plymouth. |
| Services | `systemctl enable` in `os/Containerfile` | `greetd`, `NetworkManager`, `bluetooth`, `chronyd`, `nexus-os-update.timer`. chrony keeps the clock right, because HTTPS needs the right time. |
| os-release | `sed` in `os/Containerfile` | `NAME="Nexus OS"`, `PRETTY_NAME="Nexus OS <version> (<build date>)"`. The rest stays Fedora's, which tools like dnf and bootc read. |
| User `player` | `os/files/usr/lib/sysusers.d/nexus-os.conf` | Uid 1000, no password, in the `gamemode` group. The session logs in on its own, so there is no login screen. |
| Boot arguments | `os/files/usr/lib/bootc/kargs.d/10-nexus-os.toml` | A quiet boot with the splash screen. `systemd.gpt_auto=0`, because otherwise systemd mounts the EFI partition over `/boot` and updates fail to finish. |
| Root file system | `os/files/usr/lib/bootc/install/50-nexus-os.toml` | XFS. |
| Boot menu | `os/files/usr/lib/bootupd/grub2-static/configs.d/05_nexus-os.cfg` | Hidden unless the player holds Esc or Shift. |
| Boot screen | `os/files/usr/share/plymouth/themes/nexus-os/` | The Nexus logo. The build copies Fedora's spinner images in. |

The image runs `bootc container lint` at the end of its build. `nexus --version` must print
something, or the build fails.

The image rebuilds in layers, one per package, so a daily rebuild changes only the layers whose
packages changed and a PC downloads only those. `os/build-image.sh` does this with
`bootc-base-imagectl rechunk`.

## What the OS asks of Nexus

`nexus --supports FEATURE` exits 0 when Nexus has the feature, and 1 when it does not
(`supports` in `crates/nexus-app/src/args.rs`). A Nexus without the option exits with an error
for an unknown option, which the OS counts as no.

| Feature | Means | Checked by |
| --- | --- | --- |
| `gamescope` | Nexus runs in a Gamescope session and sets the window ids Gamescope shows. | `nexus-session` before each start in gamescope; `.github/workflows/os.yml`. |
| `system-settings` | Settings → System for the OS. | Nothing in the repository. |
| `os-install` | The stick's welcome and install screens. | `os/build-iso.sh`. It stops when the Nexus in the image answers 1. |

`nexus --version` prints one line, `nexus <version>`. The image stores it in the marker file.

## Scripts that build and test the OS

Run these from the repository root. The images and the stick need `podman` as root, and the
scripts use `sudo` for it.

| Script | Does |
| --- | --- |
| `os/fetch-nexus.sh [TAG]` | Downloads the Nexus rpm of a release with `gh`, checks its `.sha256`, and leaves it in `os/nexus-rpm/`. |
| `os/build-image.sh [TAG]` | Builds `os/Containerfile`, then rechunks the image. The default tag is `localhost/nexus-os:minimal`. It stops when `os/nexus-rpm/` has no x86-64 rpm. |
| `os/make-iso.sh [IMAGE] [OUTPUT]` | Runs `os/build-iso.sh` in a privileged Fedora bootc container. The output is `nexus-os-x86_64-<version>.iso`, with the version from `os/version.txt`. |
| `os/run-vm.sh [ISO]` | Starts the ISO in QEMU as a USB stick. |

`os/build-iso.sh` has two methods. The default, `image-builder`, uses osbuild's image-builder
with the `bootc-generic-iso` type. `METHOD=xorriso` makes the same ISO by hand, from the same
`iso.yaml`.

`os/run-vm.sh` starts a UEFI machine with Secure Boot firmware, 6 GB of memory (`-m 6G`),
4 processors (`-smp 4`), and a 64 GB disk, `os-vm.qcow2`, to install on. It keeps the disk and
the firmware variables (`os-vm-vars.fd`) between starts. Delete both files to start over. The
script needs `qemu-kvm`, `qemu-img` and `edk2-ovmf`.

The VM has no GPU with Vulkan, so Nexus runs in cage there and draws slowly. It has no Wi-Fi or
Bluetooth. Test those controls on a real PC from the stick, or on a desktop with
`NEXUS_SESSION=1` (see [development.md](development.md)).

## Changing it

- A new capability gets a new `--supports` name. The OS checks it before it relies on it.
- The installer may add words to its lines. Nexus ignores words it does not know. The installer
  does not change or remove the words above.
- Nexus must keep working on an older OS. A missing tool, file or rule shows as unavailable in
  Settings.

## Limitations

- **x86-64 only.** `os/fetch-nexus.sh` and `os/build-image.sh` look for `nexus-*.x86_64.rpm`,
  and the stick and the VM are x86-64.
- **`OsUpdate::Checking` is never set.** `crates/nexus-core/src/view.rs` has a line for
  "Looking for an update…", but the Linux provider returns only `Current`, `Staged` or
  `Failed`. A running check shows no state of its own.
- **The image has no `unar`.** The image installs with weak dependencies off, and `unar` is a
  recommended package of the Nexus rpm. A RAR 5 with a big dictionary fails to unpack on
  Nexus OS. See [reference/archives-and-pc-games.md](reference/archives-and-pc-games.md).
- **Updates are staged, never applied.** The PC restarts only when the player restarts it.
- **NVIDIA with Secure Boot needs one manual step.** The player must enroll the key at the
  blue screen of the next restart. Until then the PC stays on `:minimal`.
- **The stick's live system runs with `enforcing=0`.** Every boot entry passes it.
- **This page was not checked on real hardware.** It was written from the code and the build
  files. No image, ISO or VM was built or started for it.
