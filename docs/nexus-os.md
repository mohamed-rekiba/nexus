# Nexus on Nexus OS

On Nexus OS, Nexus is the whole screen: there's no desktop behind it. So the screens the OS
needs (internet, controllers, sound, system updates, installing from the USB stick) are part of
Nexus, and show only on Nexus OS. Everywhere else none of it shows.

This page is the contract between Nexus (`crates/`, `plugins/`) and Nexus OS (`os/`): what Nexus
expects from the OS, and what the OS expects from Nexus. Change both sides together, and keep
older versions working (see the end). How the OS is built is in [os/README.md](../os/README.md);
the reasons behind it are in [specs/nexus-os.md](specs/nexus-os.md).

## How Nexus knows where it is

| | Detected by | Shows |
|---|---|---|
| Nexus OS | `/usr/lib/nexus-os/release` exists. The image writes the bundled Nexus's `nexus --version` into it. | Settings → System, with system updates |
| The Nexus OS USB stick | Nexus OS, plus `rd.live.image` on the kernel command line (`/proc/cmdline`) and the stick's installer, `/usr/libexec/nexus-os/install` | "Welcome to Nexus OS" first (Try it / Install on this PC), and the install screens |

Tests and screenshots pretend either with environment variables, which Nexus defines.

Other files Nexus may read:

| File | Holds |
|---|---|
| `/usr/lib/nexus-os/image` | Where the PC updates from, without a tag: `ghcr.io/<owner>/nexus-os` |
| `/usr/lib/os-release` | `NAME="Nexus OS"`, and `PRETTY_NAME="Nexus OS <os version> (<build date>)"` |

## What the OS provides

| Feature (Nexus) | Nexus runs | The OS provides |
|---|---|---|
| Internet, Join Wi-Fi | `nmcli` | NetworkManager, and a polkit rule letting `player` manage the network: joining a new network saves a system-wide connection, which polkit otherwise reserves for an administrator. |
| Controllers and headsets | `bluetoothctl` | BlueZ |
| Sound output | `pactl` | PipeWire with its PulseAudio server |
| System updates: state | `rpm-ostree status --json` (a staged deployment means "Restart to update"), `systemctl is-active nexus-os-update.service` (checking) | `nexus-os-update.timer` runs `nexus-os-update.service` (`/usr/libexec/nexus-os/update`) daily, from 3 minutes after start, and again every 15 minutes while it fails. It stages the next image (and moves an NVIDIA PC to `:minimal-nvidia`), and never restarts the PC. |
| System updates: check now | `systemctl start --no-block nexus-os-update.service` | A polkit rule letting `player` start that unit, and only start it |
| Nexus's own updates | nothing: "Comes with the next system update" | The image installs the nexus rpm from the latest Nexus release, and is rebuilt after each Nexus release |
| Restart, shut down | `systemctl reboot` / `poweroff` | logind (the active local session may) |
| This PC | `lspci` | pciutils |
| The session | Prints `window shown` on standard output as soon as its window is up | greetd logs `player` in and runs `/usr/bin/nexus-session`, from the nexus package. It sets `NEXUS_SESSION=1` (nothing is behind Nexus: it shows the system's settings, Restart and Shut down), starts Nexus in gamescope, or in cage when gamescope can't run, and appends all output to `~/.cache/nexus-session.log`. If no `window shown` line comes within the watchdog time (90 s by default, `NEXUS_WATCHDOG`), it starts Nexus again. |

The polkit rules are `os/files/usr/share/polkit-1/rules.d/50-nexus-os.rules` (installed PC and
stick) and `os/live/files/usr/share/polkit-1/rules.d/50-nexus-os-live.rules` (stick only). Each
applies only to `player` in a local, active session:

| polkit action | Allowed | Where |
|---|---|---|
| `org.freedesktop.NetworkManager.*` | everything | everywhere |
| `org.freedesktop.systemd1.manage-units` | unit `nexus-os-update.service`, verb `start` | everywhere |
| `org.freedesktop.policykit.exec` | program `/usr/bin/nexus-os-install` or `/usr/libexec/nexus-os/install` | the stick |

`systemctl start` needs no pkexec: systemd asks polkit itself. pkexec's `program` is the
resolved path in current polkit (`/usr/libexec/nexus-os/install`) and the given path in older
ones (`/usr/bin/nexus-os-install`), so the rule allows both.

## The installer (USB stick only)

`/usr/libexec/nexus-os/install` (`os/live/files/`) does the work; Nexus only shows it. Nexus runs
it by its name on the PATH, `nexus-os-install` (`/usr/bin/nexus-os-install` links to it), since
plugins only run `system:<name>` programs. Its presence at `/usr/libexec/nexus-os/install` is
part of how Nexus detects the stick.

### Erase a disk

- `nexus-os-install --disks` (as `player`): one line per disk it can install on (not the stick,
  nothing under 40 GB):
  `disk PATH SIZE_BYTES WINDOWS(0/1) USB(0/1) MODEL…`
  - WINDOWS is 1 when the disk has an NTFS or BitLocker partition;
  - MODEL is the rest of the line, and may hold spaces.
- `pkexec /usr/bin/nexus-os-install DISK` (no password, by the polkit rule): erases DISK and
  installs Nexus OS from the image stored on the stick. It prints, one per line:
  - `progress PERCENT TEXT` as it goes;
  - then `done TAG [mok] [nvidia-later]`:
    - TAG is the edition installed, `minimal` or `minimal-nvidia`;
    - `mok`: Secure Boot's key screen comes at the restart (password `universalblue`);
    - `nvidia-later`: the PC has an NVIDIA card and gets its driver on the installed system;
  - or `error TEXT`, and a non-zero exit.

  Anything else it prints is ignored, on standard output or standard error (bootc's own output
  goes to standard error).

### Choose partitions

Fedora's installer (Anaconda) installs the same stored image with the partitions the player
picks. It needs a keyboard and a mouse. Two ways in:

- the boot entry "Install Nexus OS (choose partitions)";
- Nexus's "Choose partitions instead": `pkexec /usr/bin/nexus-os-install --choose-partitions`
  (no password, by the polkit rule). It switches the running stick to
  `nexus-os-anaconda.target` (`systemctl isolate`), and exits 0 once the switch has started;
  greetd and Nexus are stopped right after, and Anaconda takes the screen. When it can't (not
  root, not on the stick, the switch refused) it prints one `error TEXT` line and exits
  non-zero.

When Anaconda ends, the PC restarts.

## What the OS asks of Nexus

`nexus --supports FEATURE` exits 0 when Nexus has it, 1 otherwise. Releases from before the
option reject it, which also counts as no.

| FEATURE | Means | Checked by |
|---|---|---|
| `gamescope` | It tags its window, so gamescope shows it | the session (otherwise cage), and the image checks in `os.yml` |
| `system-settings` | Settings → System for the OS (network, Bluetooth, sound, power, system updates) | — |
| `os-install` | The stick's welcome and install screens | `os/build-iso.sh`, which refuses to build a stick whose Nexus can't install |

`nexus --version` prints one line, which the image stores in the marker.

## Changing it

- A new capability gets a new `--supports` name, and the OS checks it before relying on it.
- The installer may add words to its lines (Nexus ignores what it doesn't know); it doesn't
  change or remove the ones above.
- Nexus keeps working on an older OS: a missing tool, file or rule shows as unavailable in
  Settings, never as a crash.
