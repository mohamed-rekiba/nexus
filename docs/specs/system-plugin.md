# Spec: SystemPlugin, the top bar's status icons, Settings → System and the on-screen keyboard

Status: ready for implementation.

Related specs:
- [releases-and-packaging.md](releases-and-packaging.md): the Linux package that recommends
  the tools the Linux system plugin runs.
- [nexus-os.md](nexus-os.md): Nexus OS, the main user of the capabilities described here. The
  stick's install screens and OS updates are specified there.

## Problem Statement

Nexus is meant for a TV and a controller, but it can't manage the machine it runs on:

- **Nothing about the machine can be changed from the launcher.** Joining a Wi-Fi network,
  pairing a controller or headset, changing the volume or the sound output, restarting and
  shutting down all happen elsewhere. On a desktop that means leaving the launcher for the
  desktop's own settings, with a mouse. On a PC that runs Nexus as its whole screen (the Nexus
  session, Nexus OS) there is nowhere else to go.
- **The top bar shows only the clock.** Nexus doesn't show whether the PC is online, how loud
  it is, whether a controller is connected, or how much battery a laptop has left.
- **Text can't be typed with a controller.** The Library's search, Settings' text rows and,
  soon, Wi-Fi passwords need a physical keyboard.
- **VPN code is a core port.** It's built into the core as a system adapter for each platform,
  apart from the plugin system. Adding Wi-Fi, Bluetooth and sound the same way would grow the
  core with system code it shouldn't own, and no one could replace a backend for another
  system.

## Solution

**A new plugin kind, the SystemPlugin**, provides the machine's settings and devices to the
core. Its capabilities are network (with Wi-Fi), VPN, Bluetooth, audio, power and battery, plus
two Nexus OS capabilities (OS updates and the OS installer) specified in
[nexus-os.md](nexus-os.md). Nexus ships one built-in system plugin with a backend for each
system:

- on Linux, the standard tools: NetworkManager, BlueZ, PipeWire, systemd and the kernel's power
  supply files;
- on macOS and Windows, the VPN support that exists today, plus battery.

Each capability appears only where a system plugin provides it, so a missing tool shows the
matching setting as unavailable, never as an error.

What the player gets:

- **Status icons in the top bar,** beside the clock:
  - **network:** Wi-Fi signal, wired, or offline;
  - **sound:** volume level or muted;
  - **Bluetooth:** off, on, or the number of connected devices;
  - **battery:** on any PC that has one.

  Opening an icon with the controller or a click shows a small panel: networks to join, a volume
  slider with mute and the output, devices to pair or connect.
- **Settings → System** with the full versions: Internet, Bluetooth, sound, restart and shut
  down, and This PC.
- **An on-screen keyboard** for every text field, opened with the controller, in English.

The network, sound and Bluetooth icons and Settings → System appear when Nexus is the whole
screen (Nexus OS or the Nexus session), where nothing else offers them. A desktop already has
these in its own panel. The battery icon and the keyboard appear everywhere. VPN keeps its
current place in Settings on every system.

## User Stories

1. As a player on a TV, I want to see in the top bar whether the PC is online, so that I know whether downloads and the Catalog will work.
2. As a player, I want the network icon to show Wi-Fi signal strength, a wired connection or no connection, so that I can tell a weak signal from no signal.
3. As a player, I want to open the network icon and see nearby Wi-Fi networks with their signal and whether they're secured, so that I can pick the right one.
4. As a player, I want to join an open network with one press, so that getting online is quick.
5. As a player, I want joining a secured network to ask for the password with an on-screen keyboard, so that a controller is all I need.
6. As a player, I want a wrong password to tell me it was wrong and let me try again, so that I'm not left guessing.
7. As a player, I want saved networks to rejoin by themselves, and a way to forget one, so that the PC stays online without asking again.
8. As a player, I want to see which network I'm on and its signal in Settings → System, so that I can check my connection in detail.
9. As a player, I want the sound icon to show the volume level or muted, so that I know why I hear nothing.
10. As a player, I want to change the volume and mute from the top bar with the controller, so that I don't need the TV remote.
11. As a player, I want to choose where sound goes (TV over HDMI, a headset, speakers), so that I can switch to headphones at night.
12. As a player, I want a new output, like a headset I just connected, to appear in the list by itself, so that I don't have to look for it.
13. As a player, I want the Bluetooth icon to show whether Bluetooth is on and how many devices are connected, so that I know my controller is connected.
14. As a player, I want to pair a new controller or headset from the top bar, so that adding one takes a few presses.
15. As a player, I want pairing to show its progress and say clearly whether it worked, so that I know when to stop pressing the controller's pairing button.
16. As a player, I want to connect, disconnect and remove paired devices, so that I can manage what's connected.
17. As a player, I want to turn Bluetooth on or off, so that I can save power or stop a device connecting.
18. As a laptop player, I want the battery level in the top bar, so that I know when to plug in.
19. As a laptop player, I want the battery icon to turn amber at 20% and red at 10%, and show when it's charging, so that a low battery catches my eye.
20. As a player, I want restart and shut down in Settings → System, with a confirmation, so that I can turn the PC off from the couch without doing it by accident.
21. As a player, I want This PC in Settings → System to name the graphics card and processor, so that I can tell support what I have.
22. As a player, I want the icons to update by themselves within a few seconds when something changes, so that they never show something stale.
23. As a player, I want a setting whose tool isn't installed to show as unavailable with the reason, so that I understand why it can't be used.
24. As a desktop user, I want Nexus not to duplicate my desktop's network, sound and Bluetooth controls, so that the top bar stays uncluttered.
25. As a player using Nexus as the whole screen, I want those controls to appear, so that the launcher is all I need.
26. As a player, I want the on-screen keyboard to open when I select a text field with the controller, so that I can type a search or a password without a keyboard.
27. As a player, I want the keyboard's keys laid out in rows I can move through with the d-pad, with letters, numbers and symbols, so that I can type anything a password needs.
28. As a player, I want ✕ to type the highlighted key, □ to delete, △ to add a space, L1 for Shift, R1 for symbols, R2 or Options for Done, and ○ to close, so that typing is fast with a controller.
29. As a player, I want to show or hide a password while I type it, so that I can check it without everyone in the room seeing it.
30. As a player with a physical keyboard, I want to type straight into the field without the on-screen keyboard getting in the way, so that the keyboard I have works as usual.
31. As a player, I want closing the keyboard to keep what I typed, so that I don't lose it by mistake.
32. As a player, I want the keyboard to work in every text field (search, Settings, Wi-Fi), so that it behaves the same everywhere.
33. As a VPN user, I want "Download only through a VPN" to work exactly as before, so that the change to plugins doesn't put my privacy at risk.
34. As a VPN user on macOS or Windows, I want my VPN list, import, connect and disconnect to keep working, so that moving VPN code changes nothing for me.
35. As a plugin author, I want a system plugin kind with documented capabilities, so that I can write a backend for another system or desktop.
36. As a plugin author, I want long operations (scans, pairing, installs) to report progress as jobs, so that they don't hit the plugin call timeout.
37. As a plugin author, I want my manifest to list exactly the programs my system plugin runs, so that the host enforces what it may do.
38. As a plugin author, I want a contract test suite for system plugins, so that I know my plugin behaves as the core expects.
39. As a player, I want a system plugin that fails or hangs to leave the launcher working, so that a broken tool can't freeze Nexus.
40. As a player, I want to turn the system plugin off in Settings → Plugins like any other, so that I can rule it out when something goes wrong.

## Implementation Decisions

**The SystemPlugin kind**
- A fourth plugin kind beside emulators, services and themes. Like the others it has the root
  plugin trait plus an `init` that returns its provided capabilities.
- The kernel accepts these capabilities only from a system plugin, and rejects them from the
  other kinds. A system plugin declares no game capabilities.
- New capabilities, one trait each:

  | Capability | What it offers |
  |---|---|
  | `network` | Current connection (Wi-Fi, wired, none) and signal. Wi-Fi scan as a job; join (open or with a password), forget, list saved networks |
  | `vpn` | What the core's VPN port offers today: problem, whether VPNs can be told apart, list, import, connect, disconnect |
  | `bluetooth` | Availability, power on/off, paired devices with connected state, scan as a job, pair as a job, connect, disconnect, remove |
  | `audio` | Outputs with the current one, set output, volume (0–100), mute |
  | `power` | Restart, shut down, battery (present, percent, charging) |
  | `os_updates` | Specified in [nexus-os.md](nexus-os.md) |
  | `os_install` | Specified in [nexus-os.md](nexus-os.md) |

- **One provider per capability:** the first active system plugin, in load order, that
  provides it. Without a provider, the core reports the capability as unavailable to the view
  model.
- **Reasons:** every capability can report why it's unavailable (for example "NetworkManager
  isn't installed" or "no Bluetooth adapter"), which the theme shows.

**Long operations are jobs**
- Scans, pairing and installs start in the background and return at once. They report through a
  job handle the host passes in, as the game installer capability does today: progress with
  text, then done with a result, or failed with a reason. A job can be cancelled.
- Every quick call still goes through the kernel's guard (timeout and panic catching). No quick
  call may block for long.

**Programs a system plugin may run**
- The manifest's process permission gains two flags for an entry:
  - **long-running with streamed output:** lines are delivered to the plugin as they come,
    there's no time limit, and it's cancellable with its job;
  - **through polkit:** run with `pkexec`, for actions that need it.
- Each program is still named one by one (for example `system:nmcli`). The host refuses
  anything else. Quick runs keep today's limits.

**The built-in system plugin**
- One crate with a backend for each system, chosen at build time.
- **Linux backend:**
  - `nmcli` (network, Wi-Fi, VPN);
  - `bluetoothctl` (Bluetooth);
  - `pactl` (outputs, volume, mute);
  - `systemctl` (restart, shut down);
  - the power supply folder (battery);
  - `lspci` and the existing hardware probe for This PC.

  Each call has a time limit, because `bluetoothctl` waits forever without an adapter. Output
  is parsed by pure functions ported from azrak-launcher with their tests.
- **macOS and Windows backends:** today's VPN adapters, moved from the app into the plugin, and
  battery. Other capabilities aren't provided there.
- The VPN capability replaces the core's VPN port. The core keeps "download only through a VPN"
  itself: the gate, binding torrents to the VPN's interface, and pausing downloads when the VPN
  drops. It reads the VPN list and state from the capability.
- The hardware probe (specs for "how it should run on this PC") stays a core port.

**The core and the view model**
- The core aggregates the system capabilities into a system view model:
  - the network state and Wi-Fi list;
  - the audio state and outputs;
  - the Bluetooth state and devices;
  - battery;
  - power actions;
  - each capability's availability and reason;
  - running jobs.
- **Refresh:** every few seconds while anything shows it, faster while a panel is open, and
  right after an action.
- **Where the controls appear:** the view model carries whether Nexus is the whole screen.
  That's true on Nexus OS and in the Nexus session; the session tells Nexus through its
  environment. The theme shows the network, sound and Bluetooth icons and Settings → System
  only then. Battery and the keyboard don't depend on it.
- New commands from the theme:
  - Wi-Fi: scan, join with an optional password, forget;
  - Bluetooth: power, scan, pair, connect, disconnect, remove;
  - audio: set output, set volume, mute;
  - power: restart, shut down;
  - jobs: cancel.

**The default theme**
- Top-bar status icons beside the clock, each opening a panel under it with the controller or a
  click. New icons:
  - Wi-Fi in four strengths, wired and offline;
  - the speaker at three levels and muted;
  - Bluetooth off, on and connected;
  - battery at several levels and charging.

  They're drawn at whole-pixel sizes so they stay crisp at every scale.
- Settings → System: Internet, Bluetooth, sound, power, This PC.
- **The on-screen keyboard:**
  - a theme component used by every text field;
  - the layout is ten keys per row: a numbers row, three letter rows, and an action row (Shift,
    symbols, Space, Delete, Show, Done);
  - the controller keys are those in the user stories above;
  - it opens when a field is focused with the controller and the last input was a controller;
  - typing on a physical keyboard goes straight into the field, and Enter right after typing
    means Done;
  - English only.

## Testing Decisions

- **What makes a good test:** it drives a public seam and checks what a player would see, or
  what was sent to the system. No test reaches into a plugin's internals or re-computes the
  expected result the way the code does. Expected outputs come from recorded tool output and
  known-good literals.
- **Seam 1: the core through `FakeHost`** (prior art: the VPN, settings and plugin lifecycle
  suites built on `TestCore`):
  - the test kit gains a fake system plugin;
  - tests drive commands and check the system view model: joining Wi-Fi with a password,
    pairing as a job with progress and a result, volume and output changes, restart and shut
    down asked for, a capability without a provider shown as unavailable with its reason;
  - a hanging or panicking system plugin leaves the core working;
  - the existing VPN tests keep passing, now against the VPN capability.
- **Seam 2: a contract suite for system plugins** (prior art: the emulator capability contract
  in the test kit), run on the built-in plugin with fake programs:
  - each capability answers within the guard's time;
  - jobs always end in done or failed;
  - missing programs give a reason, not an error.
- **The Linux backend's parsers** are tested on recorded output of the real tools: Wi-Fi lists,
  devices, outputs, battery folders. This is the ported azrak-launcher test set.
- **Seam 3: the composed host on Slint's testing backend** (prior art: the default theme tests
  using accessible labels, and the screenshot tests):
  - the icons for each state;
  - a panel opened with the controller;
  - typing a Wi-Fi password and a Library search with only controller input on the on-screen
    keyboard;
  - a physical keyboard typing straight in;
  - the icons hidden on a desktop and shown when Nexus is the whole screen;
  - battery colours at 20% and 10%;
  - renders of the new icons at several scales.

## Out of Scope

- Keyboard layouts other than English.
- Network, sound and Bluetooth backends for macOS and Windows.
- Moving the hardware probe into the SystemPlugin.
- Wi-Fi enterprise (802.1X) networks, hidden networks and static IP settings.
- Display settings (resolution, refresh rate, HDR).
- OS updates and the OS installer, which are in [nexus-os.md](nexus-os.md).

## Further Notes

- azrak-launcher's OS module (system tools, Wi-Fi join with its keyboard, power, battery) is the
  working reference for the Linux backend and the keyboard's behaviour. Its UI is rebuilt as
  default-theme components fed by the view model.
- Third-party system plugins (for example a backend for another desktop) become possible
  through the same discovery as other plugins, once the API is published.
- `docs/architecture.md` and `docs/plugins.md` gain the new kind, its capabilities, the job
  model and the new process permission flags.
