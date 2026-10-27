# Development

How to build Nexus, run it, test it, package it and release it. Every command runs from the
repo root. The README's "Build, run, check" has the short version; how the code is laid out is
in [architecture.md](architecture.md).

## Toolchain

- **Rust 1.99 or newer** (`rust-version` in `Cargo.toml`), on the stable channel.
  `rust-toolchain.toml` asks for `stable` with the `rustfmt` and `clippy` components, so
  [rustup](https://rustup.rs) installs both on the first `cargo` call in the repo. To add them
  by hand: `rustup component add rustfmt clippy`. With a distribution's Rust instead of rustup,
  install them as packages: `sudo dnf install rustfmt clippy` (Fedora) or
  `sudo apt-get install rustfmt rust-clippy` (Debian), and check that `rustc --version` is 1.99
  or newer.
- **Format**: `rustfmt.toml` sets `max_width = 120` and `use_small_heuristics = "Max"`.
- **Lints**: the workspace denies `unsafe_code` and warns on `clippy::all`. `make lint` and CI
  turn the warnings into errors.
- **Git**, **Bash** for `scripts/` (Git Bash on Windows) and **make** for the `Makefile`.
- **cargo-nextest** runs the tests as `make check` and CI do, each test in its own process:
  `cargo install --locked cargo-nextest`, or the prebuilt binary:
  `curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C ${CARGO_HOME:-~/.cargo}/bin`
  (macOS and Windows: see [nexte.st](https://nexte.st/docs/installation/pre-built-binaries/)).
  Without it, `scripts/check.sh` falls back to `cargo test`.
- Optional tools:

| Tool | For | Install |
|---|---|---|
| cargo-deny | `make deny`: advisories, licences and sources (`deny.toml`) | `cargo install --locked cargo-deny` |
| cargo-about | The crates' licences in a package (`about.toml`, `about.hbs`) | `cargo install --locked cargo-about --version 0.9.2 --features cli` |
| pre-commit | The git hooks | `pipx install pre-commit`, or `brew install pre-commit` |
| shellcheck | Checking the shell scripts | `sudo dnf install ShellCheck`, `sudo apt-get install shellcheck` |

`third_party/oozextract` is outside the workspace (`exclude` in `Cargo.toml`): it isn't
formatted or linted with the rest. See [third-party-notices.md](third-party-notices.md).

## Build profiles

`Cargo.toml` sets the debug profile:

```toml
[profile.dev]
opt-level = 1

[profile.dev.package."*"]
opt-level = 2
debug = "line-tables-only"
```

The workspace's own crates build at `opt-level = 1`, so a change rebuilds fast. Every dependency
builds at `opt-level = 2`, so image decoding, JSON reading and Slint run near release speed under
`make run` and in the tests. The first build after `cargo clean` takes longer for it.
Dependencies keep only line tables for debug info: their backtraces still name files and lines,
while a debug build is about a quarter smaller and a cold build about a fifth faster. Release
builds (`make build`, `make package`) use Cargo's default release profile.

## System packages

### Linux

To build: a C toolchain, `pkg-config`, and the development files for fontconfig and xkbcommon
(Slint), udev (gilrs, for controllers) and ALSA (rodio, for sounds).

| | Fedora | Debian and Ubuntu |
|---|---|---|
| Build | `gcc gcc-c++ make pkgconf-pkg-config fontconfig-devel libxkbcommon-devel systemd-devel alsa-lib-devel` | `build-essential pkg-config libfontconfig1-dev libxkbcommon-dev libudev-dev libasound2-dev` |
| Run (all optional) | `xdotool xprop mpv-libs libarchive vulkan-loader mesa-vulkan-drivers wine NetworkManager gtk2 libpng12 unar` | `xdotool x11-utils libmpv2 libarchive13 libvulkan1 mesa-vulkan-drivers wine network-manager libgtk2.0-0t64 unar` |
| Package | `cmake patchelf xz-devel libzstd-devel bzip2-devel lz4-devel zlib-ng-compat-devel mpv mpv-devel` | `cmake patchelf liblzma-dev libzstd-dev libbz2-dev liblz4-dev zlib1g-dev libmpv-dev mpv` |

Nexus starts without any of the run packages. Each turns on one feature:

- `xdotool` brings games and Nexus to the front on X11; under Gamescope it needs `xprop` too
  (`x11-utils` on Debian). On KDE Plasma's Wayland session KWin does it over D-Bus, with nothing
  to install. On another Wayland desktop only games under XWayland come to the front.
- libmpv plays trailers in the window. Without it, trailers open in the browser.
- unar unpacks a RAR that libarchive can't read (a RAR 5 with a big dictionary). The packages
  recommend it.
- GTK 2 and libpng 1.2 run old Linux games' own launchers, which their start scripts open
  (Frozenbyte's, for Trine 3 from GOG). The packages depend on them.
- 32-bit libraries (the C library, X11, OpenGL, Vulkan, sound, fonts: the `.i686` packages)
  run a game that brings its own 32-bit Wine. The rpm depends on them; the deb and Arch
  packages don't, since those systems need the i386 architecture or the multilib repository
  turned on first.
- libarchive unpacks RAR, tar.xz, tar.bz2, tar.zst and ISO downloads. The release packages ship
  their own copy.
- The Vulkan loader and a driver give the "This PC" check each card's Vulkan version.
- `wine` runs Windows PC games. On Linux the PC plugin prefers `umu-run`
  ([umu-launcher](https://github.com/Open-Wine-Components/umu-launcher/releases), which runs them
  with Proton), then `wine`, then `wine64`. umu-launcher isn't in Fedora's repositories.
- NetworkManager (`nmcli`) lists, connects and adds the saved VPN connections in Settings.
  Without it only the VPNs already up show.

How Nexus raises windows on each desktop is in [reference/desktop.md](reference/desktop.md);
archives, Wine and umu-launcher are in
[reference/archives-and-pc-games.md](reference/archives-and-pc-games.md).

The package scripts' Linux parts read Debian's package data (`dpkg-query` for the FFmpeg
version, `/usr/share/doc/<package>/copyright` for licences), so on Fedora they stop. There, make
a package without the libraries, `NEXUS_NO_LIBMPV=1 NEXUS_NO_LIBARCHIVE=1 scripts/package.sh`, or
package on Ubuntu as CI does. The AppImage needs
[linuxdeploy](https://github.com/linuxdeploy/linuxdeploy) as well (see [Packaging](#packaging)).

### macOS

- To build: the Xcode Command Line Tools, `xcode-select --install`.
- To run: the Accessibility permission for the app that starts Nexus (Terminal, your editor), in
  System Settings, Privacy & Security, Accessibility. Nexus drives System Events through
  `osascript` to bring a game and itself to the front. Without the permission it says so once.
- Optional at run time: `brew install mpv` (trailers in the window), `brew install libarchive`
  (RAR and the other archives, for a build from source), Wine (Windows PC games). The "This PC"
  check reads Vulkan through a system loader (`libvulkan.dylib`) or MoltenVK, and also finds the
  MoltenVK that the installed KytyPS5 and shadPS4 builds carry.
- To package: `brew install mpv dylibbundler cmake zstd lz4`.

### Windows

- To build: the Visual Studio Build Tools with the "Desktop development with C++" workload (MSVC
  and the Windows SDK), and Rust's `x86_64-pc-windows-msvc` toolchain. `crates/nexus-app/build.rs`
  puts the icon and version info into `nexus.exe` with the SDK's resource compiler; without it a
  local build warns and goes on, and a CI build fails.
- To run: nothing more. The GPU driver ships the Vulkan loader (`vulkan-1.dll`). For trailers,
  put `libmpv-2.dll` from a [shinchiro mpv-dev build](https://github.com/shinchiro/mpv-winbuild-cmake/releases)
  beside `nexus.exe` (`target\debug\` while developing).
- To package: Git Bash, 7-Zip (`7z` on the `PATH`) for the `.zip`, an unpacked mpv-dev build named
  by `LIBMPV_DIR`, and MSYS2's libarchive DLLs with a `licenses/` folder named by
  `LIBARCHIVE_DIR`. The `libmpv` and `libarchive` actions in `.github/actions/` show how CI fills
  both.

## Make targets

`make` alone lists them.

| Target | What it runs |
|---|---|
| `make check` | `scripts/check.sh`: `cargo fmt --all --check`, clippy with warnings as errors, and every test, as CI runs them |
| `make fmt` | `cargo fmt --all` |
| `make fmt-check` | `cargo fmt --all --check` |
| `make lint` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `make test` | `cargo test --workspace` |
| `make run` | `cargo run -p nexus-app`: full screen |
| `make run-windowed` | `cargo run -p nexus-app -- --windowed` |
| `make build` | `cargo build -p nexus-app --release` |
| `make package` | `scripts/package.sh`: a release package in `dist/nexus-<os>-<arch>/` |
| `make linux-packages` | `make package`, then the deb, rpm and Arch packages in `dist/packages/`, checked. Linux. It bundles libmpv and libarchive and checks the packages, so it needs, on Fedora: `sudo dnf install mpv-libs patchelf cmake xz-devel libzstd-devel bzip2-devel lz4-devel zlib-ng-compat-devel dpkg bsdtar zstd` and `cargo install --locked cargo-about --version 0.9.2 --features cli`; on Ubuntu, what `.github/actions/libmpv`, `libarchive` and `linux-packages` install. nfpm comes from `scripts/nfpm.sh`, the version CI pins |
| `make appimage` | `make package`, then `dist/Nexus-x86_64.AppImage`. Linux; needs `LINUXDEPLOY` |
| `make test-rpm` | `scripts/test-rpm.sh`: a quick rpm of this checkout in `dist/test-rpm/`, to try a change on this PC, such as the Nexus session. It builds the launcher and takes the rest from the latest release's Linux folder. Its version, `<version>^local.<UTC time>`, installs over that release, and the next release replaces it |
| `make os-image` | The Nexus OS image as `localhost/nexus-os:minimal` (`OS_IMAGE`), with `sudo podman`. It bundles the latest release's rpm, or `RPM=<file>`, such as `make test-rpm`'s |
| `make os-iso` | The USB stick, `nexus-os-x86_64-<version>.iso`, from that image ([os/README.md](../os/README.md#build-and-test-locally)) |
| `make os-vm` | Starts the stick in QEMU with Secure Boot and a 64 GB disk to install on (needs `edk2-ovmf`) |
| `make deny` | `cargo deny check` |
| `make hooks-install` | `pre-commit install --hook-type pre-commit --hook-type pre-push` |
| `make hooks-run` | `pre-commit run --all-files` |
| `make catalog` | Reads the catalog forums again in Chrome and writes `assets/catalog/` (`CONSOLE=pc` for one catalog, `STEAM=1` for Steam data) |
| `make catalog-refresh` | Updates `assets/catalog/` from the forums' feed and API, without a browser (`CONSOLE=pc` for one catalog) |
| `make clean` | `cargo clean`, then removes `dist/` and `target/` |

The catalog targets are in [scripts/catalog/README.md](../scripts/catalog/README.md).

`make clean` removes all of `dist/`: the catalog backups in `dist/catalog/backup/`, and the
collector's files in `dist/catalog/`, which git tracks. `git checkout -- dist` brings the
tracked ones back.

## Running

```sh
make run                                  # full screen
make run-windowed                         # a normal window
cargo run -p nexus-app -- --help          # every option
cargo run -p nexus-app -- --windowed --dev --extensions ~/nexus-themes
```

| Option | What it does |
|---|---|
| `--windowed` | A normal window for this run. Full screen is the default. |
| `--monitor <name>` | Opens on this monitor for this run, such as `DP-2`. A screen picked in Settings wins. |
| `--theme <id>` | Shows this theme now, without saving the choice. |
| `--dev` | Reloads a theme loaded from a folder when one of its files changes. A broken edit logs an error and keeps the last good version on screen. The built-in default theme is compiled into the binary, so an edit to it needs `make run` again. |
| `--safe-mode` | Loads the built-in plugins and the default theme only. |
| `--data-dir <path>` | Where settings and plugin data live. |
| `--extensions <path>` | The folder of third-party plugins and themes. Default: `<data-dir>/extensions`. Nexus also reads `extensions/` beside the binary (`Contents/Resources/extensions` in `Nexus.app`). |
| `--folder <name>=<path>` | Maps a named host folder, such as `games=/srv/games`. It is saved. Repeat it for more folders under one name. |
| `--supports <feature>` | Exits 0 when this launcher has the feature (`gamescope`, `system-settings`, `os-install`), else 1. |
| `--version` | Prints `nexus <version>` on one line, which Nexus OS keeps in its release marker. |

The default data folder is `$XDG_DATA_HOME/nexus` (`~/.local/share/nexus`) on Linux,
`~/Library/Application Support/nexus` on macOS and `%APPDATA%\nexus` on Windows. It holds
`config.json`, each plugin's folder under `plugins/<id>/`, and each game's output in
`logs/<plugin>/<game>.log`. For a clean start, point `--data-dir` at an empty folder.

`NEXUS_SCALE=<n>` draws everything at that size for this run, from 0.8 to 1.4, in place of "Size
of everything" in Settings: `NEXUS_SCALE=1.4 make run-windowed`.

Three switches make Nexus behave as it does on Nexus OS, for this run, on any Linux desktop:

| Switch | What shows |
|---|---|
| `NEXUS_SESSION=1` | Nexus as the whole screen, as the Nexus session sets it: the network, sound and Bluetooth controls in the top bar, the on-screen keyboard for text fields, Log out, Restart and Shut down, and "Restart Nexus" in place of "Quit Nexus" |
| `NEXUS_OS=1` | Also Nexus OS: the launcher updates with the system, and Settings has System updates |
| `NEXUS_USB=1` | Also Nexus OS's USB stick: "Install Nexus OS" |

```bash
NEXUS_SESSION=1 make run-windowed
```

The controls act on this computer through `nmcli`, `bluetoothctl` and `pactl`, so joining a
Wi-Fi network, pairing and the volume are real, and Restart and Shut down are too. System updates
and the install need `rpm-ostree` and the stick's installer: elsewhere they fail and show their
error. The install and its first start are tested on the OS itself, in a VM
([os/README.md](../os/README.md)).

Ctrl-C, SIGTERM and SIGHUP quit the way the Quit button does: the torrents stop first.

Theme work is in [themes.md](themes.md) and
[reference/default-theme.md](reference/default-theme.md); plugins are in [plugins.md](plugins.md).

## Logging

Nexus logs to stderr through `tracing`. The level is `info` unless `RUST_LOG` says otherwise.
Targets are module paths: the binary is `nexus` (`nexus::gamepad`, `nexus::desktop`,
`nexus::torrents`), and each library crate goes by its name with underscores (`nexus_core`,
`nexus_ui_host`, `release_catalog`, `emu_shadps4`). What a plugin logs through its context goes
to `nexus_core::facades`, with the plugin's id in `plugin_id`.

```sh
RUST_LOG=debug make run                                 # everything
RUST_LOG=info,nexus::gamepad=debug make run             # each controller press and repeat
RUST_LOG=info,nexus_core::facades=debug make run        # the plugins' own debug lines
```

## Tests

```sh
cargo test --workspace                       # every test, as `make test`
cargo test -p nexus-tests --test seams themes::    # one area: a module of tests/tests/seams/
cargo nextest run --workspace                # every test, each in its own process, all at once
cd scripts/catalog && npm test               # the catalog collector's tests (Node 20+)
```

`scripts/check.sh test` runs the tests with [cargo-nextest](https://nexte.st) when it's installed
(see [Toolchain](#toolchain)), as CI does, and with `cargo test` otherwise. Its settings are in
`.config/nextest.toml`. The seam tests are one test binary, `seams` (`tests/tests/seams/main.rs`,
one module per area), so their shared code builds and links once; `spans.rs` is its own binary.

The tests need no window, GPU, controller or sound device: they use Slint's testing backend and
fakes. `tests/` (the `nexus-tests` crate) holds the seam tests: the core's API, the plugin
contracts and the composed host. `cargo test -p nexus-tests --test seams screenshots::` renders each
default theme screen to PNG files in `target/tmp/screenshots/`.

Some tests touch the real system, the network or files you supply. Each one skips, or returns at
once, unless its variable is set:

| Variable | What it runs | Needs |
|---|---|---|
| `NEXUS_DESKTOP_TESTS=1` | `cargo test -p nexus-app --test desktop`: shows real windows, reads which one is in front, and writes and removes this user's start-at-login entry (it puts back what was there) | A desktop. On Linux, X11 with a window manager, `xdotool` and `xprop`. On macOS, the Accessibility permission. |
| `NEXUS_DESKTOP_REQUIRE=1` | With the above, a skipped desktop test fails | |
| `NEXUS_KWIN_TESTS=1` | `cargo test -p nexus-app kwin_says`: asks the real KWin which window is in front | A KDE Plasma Wayland session |
| `NEXUS_TRAILER_TESTS=1` | `cargo test -p nexus-app libmpv_plays`: plays a made-up video through the real libmpv | libmpv, and `ffmpeg` (`ffmpeg-free` on Fedora) to make the video |
| `NEXUS_TORRENT_TESTS=1` | `cargo test -p nexus-app a_torrent_downloads`: a real BitTorrent transfer between two sessions on this computer, with no DHT, tracker or network | |
| `NEXUS_VPN_TESTS=1` | `cargo test -p nexus-app lists_this_computer_s_networks`: lists this computer's networks; it connects nothing | |
| `NEXUS_REAL_RELEASES=1` | `cargo test -p nexus-app real -- --nocapture`: downloads and installs the newest real KytyPS5 and shadPS4 releases | The network, and some hundreds of megabytes. Anonymous GitHub API calls can hit the rate limit. |
| `NEXUS_REAL_ARCHIVES=<folder>` | `cargo test -p nexus-tests --test seams real_archives::`: installs real release archives through the plugins on a fake web | The archives, named as GitHub has them (`KytyPS5-*-Linux-x86_64.tar.gz`, `shadps4-linux*.zip` and so on). A missing one is skipped. |
| `NEXUS_CATALOG_FILES=<file>[:<file>…]` | `cargo test -p release-catalog catalog_files -- --nocapture`: reads catalog files through the plugin's reader and prints what each holds | Catalog files, such as the collector's |
| `NEXUS_TIMING=1` | `cargo test -p nexus-tests --test seams timing:: -- --nocapture`: times the Catalog view with the bundled catalogs | |
| `NEXUS_COMPARE_OUT=<folder>` | `cargo test -p nexus-tests --test seams compare::`: renders the default theme's screens on a real library into that folder | A KytyPS5 game library in `NEXUS_COMPARE_GAMES` (default `~/Games`); optionally `NEXUS_COMPARE_BUILDS` (an `emu-kyty` plugin folder) and `NEXUS_COMPARE_WEB` (a folder with a `manifest.json` of saved web answers) |
| `PS4_TEST_PKG=<file>` | `cargo test --release -p emu-shadps4 real_ps4_package -- --ignored --nocapture`: lists a real PS4 package and copies `param.sfo` | A PS4 fake package |
| `PS5_TEST_PKG=<file>` | `cargo test -p emulator-kit real_package -- --ignored`: lists a real PS5 debug package and decodes its largest file; `PS5_TEST_FULL=1` decodes every file | A PS5 debug package |

CI runs the desktop tests on Linux under Xvfb with openbox. To do the same:

```sh
sudo apt-get install xvfb openbox xdotool x11-utils     # Fedora: xorg-x11-server-Xvfb openbox xdotool xprop
NEXUS_DESKTOP_TESTS=1 NEXUS_DESKTOP_REQUIRE=1 xvfb-run -a -s "-screen 0 1280x800x24" \
  bash -c 'openbox & sleep 1; cargo test -p nexus-app --test desktop'
```

## Git hooks

`.pre-commit-config.yaml` sets the hooks for [pre-commit](https://pre-commit.com):

- On each commit: merge-conflict markers, YAML and TOML syntax, a newline at the end of each file,
  trailing spaces (not in `.md` files), no file over 2 MB, and `cargo fmt --all --check` when Rust
  files change.
- Before each push: `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace`.

`make hooks-install` installs them; `make hooks-run` runs them on every file. Each cargo hook moves
into `nexus/` first when the repo root has a `nexus/Cargo.toml`, so the hooks also work while
Nexus is a folder of a larger repo. There, name the config:
`pre-commit run -c nexus/.pre-commit-config.yaml`.

## Packaging

`make package` (`scripts/package.sh`) makes a release build with `--locked`, then
`dist/nexus-<os>-<arch>/` with `README.md`, `LICENSE`, an empty `extensions/` and a `licenses/`
folder, and runs the packaged `nexus --help` to check that it starts.

| System | What it makes |
|---|---|
| Linux | The binary, libraries in `lib/`, `nexus.desktop`, `nexus.png` and `install.sh`, packed as `dist/nexus-linux-<arch>.tar.gz` |
| macOS | `Nexus.app` (the binary, libraries in `Contents/Frameworks`, the icon, `extensions/` and the licences in `Contents/Resources`, and an `Info.plist` with the workspace version), signed ad hoc and packed as `dist/nexus-macos-<arch>.zip` with `ditto`. `NEXUS_BUNDLE_ID` sets the bundle id (default `io.github.mohamed-rekiba.nexus`). |
| Windows | `nexus.exe`, the DLLs beside it and `nexus.ico`. The folder isn't zipped: CI zips it with 7-Zip. |

From those folders, the release and CI also make:

| File | Made by | What it is |
|---|---|---|
| `nexus_<version>-1_amd64.deb`, `nexus-<version>-1.x86_64.rpm`, `nexus-<version>-1-x86_64.pkg.tar.zst` | `packaging/linux/build-packages.sh <folder> <version> <out>` (nfpm, `packaging/linux/nfpm.yaml`) | System packages. The release folder goes to `/usr/lib/nexus/` with `/usr/bin/nexus` linking to it, the app menu entry, the icon, the Nexus session, and `/usr/lib/nexus/packaged` naming the format. `packaging/linux/check-packages.sh <out>` checks the files, modes, marker and dependencies (it needs `dpkg-deb`, `rpm` and `zstd`). |
| `nexus-macos-universal.zip` and `.dmg` | `packaging/macos/universal.sh <arm64 app> <x86_64 app> <out app>`, then `packaging/macos/make-dmg.sh <app> <dmg>` | One `Nexus.app` for both Mac processors: each Mach-O file is joined with `lipo`. The disk image holds the app and an Applications shortcut. |
| `nexus-windows-x86_64-setup.exe` | `scripts/windows-installer.sh <version>` (Inno Setup 6, `packaging/windows/nexus.iss`) | A per-user installer: no administrator, a Start menu entry, an uninstaller in Windows' apps, and the marker `installed-by-setup` beside `nexus.exe`. |

### The Nexus session

`packaging/linux/nexus-session` runs Nexus as the whole session, with no desktop: display
managers list it as "Nexus" (`nexus-session.desktop`), and Nexus OS's greetd starts it. It runs
gamescope in its Steam mode with Nexus inside, starts Nexus again when it ends or prints no
`window shown` line within the watchdog time, and runs it in cage when gamescope can't drive
the graphics card. It sets `NEXUS_SESSION=1`, which tells Nexus nothing is behind it. Its
settings are in `/usr/share/nexus/session.conf` (`packaging/linux/session.conf`), and
`/etc/nexus/session.conf` overrides them. Output goes to `~/.cache/nexus-session.log`. Before each start of Nexus,
a log over `NEXUS_LOG_MAX_MB` (10 by default) becomes `nexus-session.previous.log`, in place of
the one before.

What goes into `licenses/`:

- The fonts' licences, `OFL-Inter.txt` and `OFL-Tajawal.txt`.
- `THIRD-PARTY.md`: every Rust crate in the build and its licence text, made by
  `cargo about generate` from `about.toml` and `about.hbs`. Without cargo-about a local package
  warns and goes on; with `CI` set it fails.
- `libmpv/` and `libarchive/` from the bundle scripts below.

The bundle scripts:

- `scripts/bundle-libmpv.sh` copies libmpv and the libraries it needs that a desktop doesn't
  already have (Linux: from apt, with `patchelf` setting `$ORIGIN`; macOS: from Homebrew, through
  `dylibbundler`; Windows: `libmpv-2.dll` from `LIBMPV_DIR`). It fetches mpv's and FFmpeg's licence
  texts at the shipped versions and writes `NOTICE` and `SOURCES`. `NEXUS_NO_LIBMPV=1` leaves it
  out.
- `scripts/bundle-libarchive.sh` builds libarchive 3.8.9 from its release (pinned by SHA-256) on
  Linux and macOS with only its compression codecs: no XML, ACL, crypto or iconv. On Windows it
  copies MSYS2's DLLs from `LIBARCHIVE_DIR`. It writes the licences, `NOTICE` and `SOURCES`.
  `NEXUS_NO_LIBARCHIVE=1` leaves it out.

Then, on Linux:

- `scripts/appimage.sh` makes `dist/Nexus-<arch>.AppImage` from the package, with linuxdeploy
  (named by `LINUXDEPLOY`) picking the libraries to bundle. Run it after `package.sh`. CI pins the
  linuxdeploy release and its SHA-256 in `.github/actions/libmpv`.
- `install.sh`, in the tarball, installs the package for the user without sudo: the folder goes
  to `~/.local/lib/nexus` (`PREFIX` changes `~/.local`), with a link in `~/.local/bin`, a menu
  entry and the icon. `./install.sh --uninstall` removes it and keeps the settings. Check the
  script with `shellcheck scripts/install.sh`.

`scripts/fetch-sources.sh dist/nexus-<os>-<arch>` downloads every source archive the `SOURCES`
files name into `dist/nexus-<os>-<arch>-third-party-sources.tar`.

Other scripts:

- `scripts/make-icons.sh` makes `assets/nexus.png`, `nexus.ico` and `nexus.icns` from
  `assets/logo.svg`. It needs `rsvg-convert` (`librsvg2-tools` on Fedora, `librsvg2-bin` on
  Debian), and macOS's `iconutil` for the `.icns`. The icons are in git, so a build needs no
  image tools.
- `scripts/make-sounds.py` writes the default theme's WAV sounds with Python's standard library.

## CI

The workflows are in `.github/workflows/`:

| Workflow | When | What it does |
|---|---|---|
| `ci.yml` | Pull requests, pushes to `main`, by hand | On Linux, side by side: `Format` (`cargo fmt --check`), `Clippy` (`-D warnings`), `Test` (cargo-nextest) and `cargo-deny` |
| `weekly.yml` | Mondays at 05:17 UTC, by hand | The tests on Windows; on Linux: the real emulator releases install (`NEXUS_REAL_RELEASES=1`), the desktop adapter under Xvfb and openbox (`NEXUS_DESKTOP_TESTS=1`), the trailer player plays a real video (`NEXUS_TRAILER_TESTS=1`), and `cargo deny check advisories` |
| `release.yml` | Pushes to `main` | Release Please; then it starts `package.yml` or `os.yml` on the new tag ([Releases](#releases)) |
| `package.yml` | On a Nexus tag (started by `release.yml`), by hand | The packages on Linux, both Mac processors and Windows; on a tag it publishes the release, then starts `os.yml` |
| `os.yml` | Daily, on a Nexus OS tag, after a Nexus release, by hand | The Nexus OS images and USB stick ([os/README.md](../os/README.md#releases)) |
| `mirror-deps.yml` | By hand | Copies the pinned Windows libmpv build into the `deps` release |

`ci.yml` skips what can't change the build: pushes and pull requests that only touch docs,
Markdown, `os/` or the other workflows, and Release Please's pull requests (a version and a
changelog). A new push cancels the older run of the same branch.

`ci.yml` runs on Linux only. macOS and Windows build in `package.yml`: run it by hand on a
branch to try a change on them. Its files stay as the run's artifacts for 7 days.

For branch protection, require `Format`, `Clippy`, `Test` and `cargo-deny`.

The shared steps are in `.github/actions/`:

- `setup`: the stable toolchain, the Linux build packages, and the
  [rust-cache](https://github.com/Swatinem/rust-cache) cache.
- `libmpv`: Homebrew's mpv and dylibbundler on macOS; apt's `libmpv-dev`, `mpv`, `patchelf` and a
  pinned linuxdeploy on Linux; a pinned shinchiro build on Windows, checked against its SHA-256.
  It sets `LINUXDEPLOY`, `LIBMPV_DIR` and `LIBMPV_SOURCE`.
- `linux-packages`: a pinned nfpm (checked against its SHA-256), `build-packages.sh` and
  `check-packages.sh`.
- `libarchive`: the tools and codecs that `bundle-libarchive.sh` needs on macOS and Linux, and
  MSYS2's libarchive with its DLLs, licences and source list on Windows. It sets `LIBARCHIVE_DIR`
  and `LIBARCHIVE_PACKAGES`.

shinchiro keeps about six weeks of builds, so the Windows build is mirrored in this repo's `deps`
release, and the `libmpv` action takes it from there first. After changing the pin (`FILE`,
`UPSTREAM` and `SHA256` together), run the `mirror-deps` workflow by hand.

The rules every workflow follows:

- Every third-party action is pinned to a commit, with its version in a comment. Dependabot
  updates the pins.
- Permissions are `contents: read` or none at the top; a job that writes asks for what it needs.
- Checkouts don't keep the token (`persist-credentials: false`).
- Every job has a `timeout-minutes`.
- Cargo runs with `--locked`, so CI fails when `Cargo.lock` is out of date.
- A new push to a pull request cancels its older CI run.

The cache: only `main` saves it, and a pull request restores `main`'s. So a pull request builds
only what it changed, and pull requests don't fill GitHub's 10 GB cache for the repo. CI builds
have no debug info (`CARGO_PROFILE_DEV_DEBUG=0`; a failing test still says its file and line),
which makes the builds and the cache about three times smaller.

## Releases

Releases are made by [Release Please](https://github.com/googleapis/release-please), from
Conventional Commit messages: `feat:` raises the minor version, `fix:` the patch version (before
1.0, a breaking change raises the minor version). Other prefixes (`docs:`, `ci:`, `refactor:`,
`test:`, `chore:`) don't make a release.

Release Please, packaging and Nexus OS are separate workflows, each with its own runs.
`.github/workflows/release.yml` runs on every push to `main`:

1. `Release Please` keeps a release pull request open for Nexus with the next version: it bumps
   `version` in `[workspace.package]` and the workspace's crates in `Cargo.lock`, `version.txt`,
   and writes `CHANGELOG.md` (`.github/release-please-config.json` and
   `.github/release-please-manifest.json`). Changes under `os/` are left out; Nexus OS has its own
   release pull request. A pull request opened with the default token doesn't start CI; set a
   token as the `RELEASE_PLEASE_TOKEN` secret to get checks on it.
2. Merging that pull request makes a draft release and its tag `v<version>`
   (`force-tag-creation`, so Release Please finds the release on the next push). Since Nexus's
   package leaves `os/` out, a Nexus tag means the app changed. `release.yml` then starts
   `package.yml` on the tag (`gh workflow run --ref <tag>`: GitHub doesn't start workflows for a
   tag made with a workflow's own token, but does start one asked for by name).
3. Once the release is published, `package.yml` starts `os.yml`, so the images get the new
   Nexus.
4. Merging Nexus OS's release pull request makes the tag `nexus-os-v<version>`, and
   `release.yml` starts `os.yml` on it: the images and the stick from the tag, and the OS
   release. When the same push made a Nexus tag too, `package.yml` starts the OS release instead,
   once Nexus's release is published, since the images bundle it.

`package.yml` doesn't run the tests: CI did, on the commit, before it reached `main`.

- `Version` fails when the tag isn't `v` and the workspace version.
- `Build`, on Linux, both Mac processors and Windows: the libmpv and libarchive actions,
  cargo-about, `scripts/package.sh`; on Linux the AppImage (run with `--version`) and the deb,
  rpm and Arch packages with their checks; on Windows the signatures when their secrets are set
  ([signing.md](signing.md)), the zip, and the installer, installed silently, started, and
  uninstalled; the third-party sources tars.
- `macOS universal app` joins the two Mac apps, signs and notarizes the app when the secrets are
  set, and makes the zip and the disk image (signed, notarized and checked).
- `Publish the release` (on a tag only) writes a `.sha256` file for each file and
  `latest.json` (`scripts/update-manifest.sh`), signs the build provenance
  (`gh attestation verify <file> --repo <owner>/<repo>` checks it), attaches them all to the
  draft, and publishes it as the latest.
- `Start Nexus OS` starts `os.yml`: on the OS tag made in the same push (its `os-release`
  input), or on `main` for new images.

When `package.yml` fails for a release, run it again on the same tag: Actions → Package → Run
workflow, with the tag under "Use workflow from" (and the OS tag as `os-release`, when that push
made one). Don't make a new release.

## The launcher's own update

`crates/nexus-core/src/launcher_update.rs` and `apply_launcher_update` in
`crates/nexus-app/src/update.rs` do it.

- `latest.json` holds the version and, for each kind of install, its package's URL and SHA-256:
  `linux-x86_64` (the tar.gz), `linux-x86_64-appimage`, `linux-x86_64-deb`, `linux-x86_64-rpm`,
  `linux-x86_64-arch`, `macos-aarch64` and `macos-x86_64` (both the universal zip),
  `windows-x86_64` (the zip) and `windows-x86_64-setup` (the installer).
- The kind of install (`Install::detect`) comes from the files beside the binary: `packaged`
  (deb, rpm, Arch; on Nexus OS, which has `/usr/lib/nexus-os/release`, the OS's), then
  `installed-by-setup` (Windows' installer), `Contents/MacOS` (the macOS app), and `LICENSE` with
  `extensions/` (the unpacked tar.gz or zip). `APPIMAGE` in the environment means the AppImage.
- Settings, UPDATES, "Nexus updates" checks
  `https://github.com/<repo>/releases/latest/download/latest.json`. `<repo>` is `app.update_repo`
  in `config.json` ("owner/name"), else `mohamed-rekiba/nexus`. An empty `app.update_repo` turns
  the checks off. A draft or a pre-release isn't "latest", so it isn't offered.
- A newer version (by semver) downloads only from GitHub's release hosts, with at most five
  redirects. Its SHA-256 must match `latest.json`. It is kept in `<data-dir>/updates/` with
  `ready.json`.
- The row's button reads "Check now", then "Download" ("Update" for a deb, rpm or Arch
  package), then "Restart".
- A deb, rpm or Arch package is handed to the system's package manager at once, through polkit:
  `pkexec dnf install -y`, `pkexec apt-get install -y` or `pkexec pacman -U --noconfirm` with the
  downloaded file. The system asks for the password; a closed prompt or the package manager's
  error leaves this version and says why. Then the row offers "Restart".
- Windows' installer: the downloaded installer runs silently (`/VERYSILENT`) at the next start
  and starts the new version when it's done.
- On Nexus OS nothing is downloaded: the row says the new version comes with the next system
  update.
- Otherwise, at the next start, or after "Restart", Nexus swaps the package in for its install: the
  AppImage file, `Nexus.app`, or the package folder (the folder with `LICENSE` and
  `extensions/`). The old one is renamed aside, comes back when the swap fails, and is removed at
  the start after. On Windows each file is replaced, and the one in use is renamed to `.old`. An
  update is tried once.
- A build that isn't installed from a package, such as `cargo run`'s, is never replaced.

"Update automatically" in the same section is for the emulators, not for Nexus.
