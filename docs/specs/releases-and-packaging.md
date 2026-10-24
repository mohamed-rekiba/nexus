# Spec: Nexus releases and packages

Status: ready for implementation.

Related specs:
- [system-plugin.md](system-plugin.md): the SystemPlugin kind, the top bar's status icons,
  Settings → System and the on-screen keyboard.
- [nexus-os.md](nexus-os.md): Nexus OS, which uses the Linux package and the release stream
  described here.

## Problem Statement

Making a Nexus release is a manual job. Someone bumps the version in the workspace, pushes a
`v*` tag, waits for the build and publishes the draft by hand. Nothing writes a changelog, and
nothing ties a release to the changes it contains.

What a player can download is narrow:

- **Linux:** a tar.gz to unpack and an AppImage. There's no package for Fedora, Debian/Ubuntu
  or Arch, so Nexus isn't installed the way the rest of the system is: no package manager owns
  it, removes it or lists it.
- **Linux, as a console:** no way to choose Nexus at the login screen and use the PC as a
  console without a desktop.
- **macOS:** Apple Silicon only, so Intel Macs can't run it. It comes as a bare zip, not the
  usual disk image.
- **Windows:** a zip to unpack anywhere, with no Start menu entry and no entry in Windows'
  installed-apps list to uninstall from.

Packaged copies, once they exist, also need to update. The launcher's own updater swaps files
in place, which is wrong for files a package manager owns.

## Solution

Nexus is released by Release Please. Every change lands on `main` with a Conventional Commit
message. Release Please keeps a release pull request open with the next version and its
changelog. Merging it creates a draft release, CI builds and attaches every file, and the
release is published automatically as the repository's **latest** release.

Each release offers:

- **Linux:**
  - the tar.gz;
  - the AppImage;
  - a **deb**, an **rpm** and an **Arch** package. Each installs Nexus system-wide, with its
    app menu entry, icon, and a **Nexus session** at the login screen that runs Nexus full
    screen in gamescope without a desktop.
- **macOS:** one **universal** `Nexus.app` (Apple Silicon and Intel), as a **`.dmg`** to drag
  into Applications, and as a `.zip`.
- **Windows:** an **installer** (per user, no admin rights, Start menu entry, uninstaller in
  Windows' installed-apps list), and the portable zip.

Every install keeps itself up to date in the way that fits how it was installed:

- **tar.gz, AppImage, macOS app, Windows zip:** swapped in at the next start, as today.
- **Windows installer:** the new installer runs silently at the next start.
- **deb/rpm/Arch:** the new package is handed to the system's package manager. The system asks
  for the administrator password the usual way.

## User Stories

1. As a maintainer, I want a release pull request that always shows the next version and its changelog, so that I can see what a release will contain before making it.
2. As a maintainer, I want merging the release pull request to be the only step of a release, so that releasing takes no manual tagging or publishing.
3. As a maintainer, I want the version bumped in the workspace and the lock file in the same change, so that a release always builds with `--locked`.
4. As a maintainer, I want the changelog written from commit messages, so that release notes need no separate writing.
5. As a maintainer, I want `feat:` to raise the minor version and `fix:` the patch version, so that version numbers say what changed.
6. As a maintainer, I want a release to stay a draft until every file is attached, so that the updater never sees a release without its files.
7. As a maintainer, I want the release build to run the full check first, so that a release never ships what the check would refuse.
8. As a maintainer, I want changes under the OS folder left out of Nexus's changelog and version, so that OS changes don't produce launcher releases.
9. As a maintainer, I want every package format built and checked on every pull request, so that a wrong file list or dependency fails before a release, not after.
10. As a maintainer, I want each package's file list and dependencies checked automatically, so that a package missing a file can't be published.
11. As a maintainer, I want signing, notarization and build provenance kept as they are, so that the new formats are as trustworthy as the current ones.
12. As a maintainer, I want unsigned builds to still be produced when signing secrets are missing, so that forks and test runs work.
13. As a Fedora user, I want to install Nexus with `dnf install ./nexus-….rpm`, so that it is installed like the rest of my system.
14. As a Debian or Ubuntu user, I want to install Nexus with `apt install ./nexus_….deb`, so that apt pulls in what it needs.
15. As an Arch user, I want to install Nexus with `pacman -U`, so that pacman owns its files.
16. As a Linux user, I want a package to pull in the libraries Nexus loads, so that trailers, archives and sound work after installing.
17. As a Linux user, I want gamescope recommended rather than required, so that I can install Nexus on a desktop without it.
18. As a Linux user, I want Nexus in my app menu with its icon after installing the package, so that I can start it like any app.
19. As a Linux user, I want removing the package to remove Nexus completely, so that nothing is left behind in system folders.
20. As a Linux user, I want my games, settings and emulators kept when I remove or upgrade the package, so that nothing I own is lost.
21. As a living-room PC owner, I want to choose "Nexus" at the login screen, so that the PC starts as a console with no desktop.
22. As a living-room PC owner, I want the Nexus session to restart Nexus if it closes or crashes, so that the TV never shows an empty screen.
23. As a living-room PC owner, I want the session to restart Nexus when its window doesn't appear in time, so that a hung start fixes itself.
24. As a living-room PC owner, I want the session to fall back to a simpler compositor (cage) when gamescope can't drive my graphics card, so that Nexus still shows and can explain the problem.
25. As a living-room PC owner, I want the session's output written to one log file in my cache folder, so that I can send it when something goes wrong.
26. As an advanced user, I want to change the session's settings (which binary, extra gamescope options, the watchdog time) in a file under `/etc`, so that my change survives package upgrades.
27. As a packaged-install user, I want Nexus to tell me an update is available, so that I don't have to watch the releases page.
28. As a packaged-install user, I want the update downloaded and checked against its SHA-256 before anything is installed, so that a damaged or altered file is never installed.
29. As a packaged-install user, I want the update installed by my package manager after asking for my password, so that the package manager still owns every file.
30. As a packaged-install user, I want to be offered "Restart Nexus" after the update installs, so that the new version starts right away.
31. As a packaged-install user, I want a failed or cancelled install to leave my current version working and tell me why, so that an update can't break Nexus.
32. As a tar.gz or AppImage user, I want updates to keep working as they do today, so that nothing changes for me.
33. As an Intel Mac user, I want Nexus to run natively on my Mac, so that I'm not left out.
34. As an Apple Silicon Mac user, I want the same app to stay native on my Mac, so that the universal app costs me nothing.
35. As a Mac user, I want to download a `.dmg` and drag Nexus into Applications, so that installing works like any other Mac app.
36. As a Mac user, I want the app signed and notarized, so that macOS opens it without warnings.
37. As a Mac user on an older Apple Silicon release, I want my in-app update to bring me the universal app, so that the change of format is invisible to me.
38. As a Mac user, I want trailers and archives to work on both kinds of Mac, so that the bundled libraries aren't for one processor only.
39. As a Windows user, I want an installer that needs no administrator rights, so that I can install Nexus on any account.
40. As a Windows user, I want a Start menu entry, and a desktop shortcut if I choose one, so that I can find Nexus.
41. As a Windows user, I want to uninstall Nexus from Windows' installed-apps list, so that removing it works like any other app.
42. As a Windows user, I want the installer and the app signed, so that SmartScreen doesn't warn me.
43. As a Windows user who used the installer, I want updates to run the new installer silently at the next start, so that Windows' app list always shows the right version.
44. As a Windows user who prefers the zip, I want the portable zip to keep updating itself, so that I don't need the installer.
45. As a Windows user, I want my games, settings and emulators kept when I uninstall or update, so that nothing I own is lost.
46. As any user, I want every download to have a SHA-256 file and verifiable build provenance, so that I can check what I downloaded.
47. As a fork maintainer, I want the release and update flow to follow my own repository, so that my fork releases and updates independently.

## Implementation Decisions

**Release stream**
- Release Please runs on every push to `main`. It has two components in one manifest: `nexus`
  (the repository root, excluding the OS folder) and `nexus-os` (specified in
  [nexus-os.md](nexus-os.md)). Each component gets its own release pull request.
- Nexus keeps its tag form `v<version>`, so the existing tags and the updater's reading of the
  latest release are unchanged. It starts from the workspace's current version, 0.1.0. Before
  1.0, `feat:` raises the minor version.
- The version lives in the workspace's shared package version. The release pull request also
  updates the lock file entries of the workspace's own crates. This is checked with a Release
  Please dry run before it's relied on. If Release Please can't update the lock file, a workflow
  step refreshes it on the release branch.
- When Release Please creates a release, a reusable package workflow runs with the tag and
  commit. The current tag-push trigger goes away, because tags created by GitHub's workflow
  token don't start workflows.
- **Release flow:**
  1. The package workflow builds every system's files from the tag's commit.
  2. It attaches them, their SHA-256 files and `latest.json` to the draft.
  3. A final job publishes the release and marks it latest.
  4. A published Nexus release then starts a rebuild of the Nexus OS images.
- The full check, build provenance, macOS signing and notarization, and Windows signing work as
  they do today. Each is skipped with a warning when its secrets are missing.

**`latest.json`**
- It keeps its shape (version, and per-key address and SHA-256). The keys are:
  - `linux-x86_64`, `linux-x86_64-appimage`, `linux-x86_64-deb`, `linux-x86_64-rpm`,
    `linux-x86_64-arch`;
  - `macos-aarch64` and `macos-x86_64` (both the universal zip);
  - `windows-x86_64` (the zip) and `windows-x86_64-setup` (the installer).
- Older launchers only read the keys they know, so they keep working.

**Linux packages**
- Built with nfpm from the release build's Linux folder. Nothing is compiled again. The
  architecture is x86_64.
- **Layout:**
  - one folder under the system library directory with the binary, the bundled libraries, the
    `extensions` folder and the licences;
  - a command on the PATH that links to the binary;
  - the app menu entry and the icon;
  - the session command, its login-screen entry and the session defaults;
  - a packaging marker naming the format (`rpm`, `deb` or `arch`).
- Dependencies are what the code loads at run time, listed with each distribution's own package
  names. Gamescope and cage are recommended (suggested on Arch). The tools the Linux system
  plugin runs (NetworkManager, BlueZ, PipeWire's `pactl`) are recommended too: without them the
  matching settings show as unavailable.
- The packaging marker is how Nexus recognises a packaged install. The launcher's install
  detection gains the kinds `deb`, `rpm` and `arch`.

**The Nexus session**
- A single session command with its defaults file is shared with Nexus OS, and only the Linux
  package ships it. A local override file under `/etc` takes precedence over the defaults.
  - The defaults cover the binary to run, extra gamescope options, the watchdog time and the
    log file.
- **Behaviour:**
  - it starts gamescope in its embedded (Steam) mode running Nexus;
  - it starts cage instead when gamescope can't start on the graphics card;
  - it restarts Nexus when it exits;
  - it restarts Nexus when the "window shown" line doesn't appear within the watchdog time;
  - it appends all output to one log file in the user's cache folder.
- Nexus prints the line "window shown" on standard output once its window is up. This is the
  contract the watchdog relies on.

**Updates for packaged installs**
- The launcher's update gains a per-install method. Checking (`latest.json`), downloading and
  verifying the SHA-256 stay in the core as they are. What differs is how a verified file is put
  in place:

  | Install kind | Method |
  |---|---|
  | AppImage, macOS app, Linux folder, Windows zip | Swap at the next start (existing) |
  | deb, rpm, Arch | Hand the verified file to the system's package manager through polkit (`pkexec`): `dnf install`, `apt install` or `pacman -U` on the local file. Report the outcome, then offer Restart |
  | Windows installer | Run the verified installer silently at the next start, then start the new version |
  | Nexus OS | Specified in [nexus-os.md](nexus-os.md): the OS updates the launcher |

- Installing the launcher's own package is a host function of the core (a method on a core
  port), not part of the SystemPlugin. It concerns the launcher itself, not the machine's
  settings. It reports success, a cancelled password prompt, or the package manager's error. A
  failure leaves the current version installed and shows the reason as a notice.
- The view model reports the method, so the theme words the update button right ("Update",
  "Update and restart", "Comes with the next system update").

**macOS**
- The release builds both Apple processors, joins the binaries with `lipo`, and bundles libmpv
  and libarchive built for both processors, joined the same way.
- **The `.dmg`:**
  - contains `Nexus.app` and an Applications shortcut, made with `hdiutil`;
  - is signed and notarized with the existing secrets;
  - is only for first installs. The updater keeps using the zip.

**Windows**
- **An Inno Setup installer:**
  - installs per user into the user's local programs folder, with no administrator rights;
  - adds a Start menu entry, an optional desktop shortcut and an uninstaller registered in
    Windows' installed-apps list;
  - writes a marker the launcher uses to recognise an installer install;
  - is signed with the existing certificate.
- The uninstaller removes only the program folder. The data folder (settings, plugins'
  storage, emulators) stays.

## Testing Decisions

- **What makes a good test:** it drives a public seam and checks what a player or a maintainer
  would see. Commands go in; view models, notices and the commands sent to the system come out.
  No test reaches into internals or re-computes an expected value the way the code does.
- **Seam 1: the core through `FakeHost`** (prior art: the launcher update tests and the
  `tests/tests` suites built on `TestCore`):
  - each install kind picks its `latest.json` entry and its update method;
  - a packaged update downloads, verifies, and asks the host to install the right file with the
    right package manager;
  - a cancelled or failed install leaves the version and shows the reason;
  - a wrong SHA-256 never reaches the installer;
  - the Windows installer method stages a silent run at the next start.
- **Seam 2: the composed host on Slint's testing backend** (prior art: the default theme
  tests): the update row's wording and Restart offer for each method.
- **Seam 3: the built files in CI** (prior art: the release workflow's package step that runs
  `nexus --help`):
  - a check script lists each Linux package's files and dependencies and fails on a missing or
    extra one;
  - the universal app holds both architectures (`lipo -archs`) and starts;
  - the `.dmg` mounts and contains the app;
  - the Windows installer installs silently, starts Nexus with `--version`, and uninstalls
    cleanly on a CI runner;
  - the AppImage starts and reports the right version.
- **Release Please** is verified with its dry run against the repository, not with a test.
- **"window shown"** is covered by a test of the app's start sequence.

## Out of Scope

- A signed package repository (Fedora COPR, an apt repository), and GPG signatures on the
  Linux packages.
- Homebrew and winget.
- ARM Linux and ARM Windows builds.
- Flatpak and Snap.
- Changes to the launcher's update check or download (only the per-install method is new).
- The SystemPlugin and Nexus OS: see their own specs.

## Further Notes

- ps5-launcher's Release Please setup, nfpm configuration, package checks and universal macOS
  bundle are the working reference for this spec.
- Conventional Commit messages are required from now on. Without them Release Please proposes
  no release.
- The Nexus session is shared with Nexus OS on purpose: there is one implementation, tested on
  ordinary distributions as well as on the OS.
