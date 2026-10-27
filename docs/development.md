# Development

This page shows how to build Nexus, run it, test it, package it and release it. It is for
contributors. Every command runs from the repository root. How the code is laid out is in
[architecture.md](architecture.md).

## Toolchain

- **Rust 1.99 or newer** (`rust-version` in `Cargo.toml`), on the stable channel.
  `rust-toolchain.toml` asks for `stable` with the `rustfmt` and `clippy` components, so
  [rustup](https://rustup.rs) installs both on the first `cargo` call in the repository. With a
  distribution's Rust, install the two components as packages and check that `rustc --version`
  is 1.99 or newer.
- **Format**: `rustfmt.toml` sets `max_width = 120` and `use_small_heuristics = "Max"`.
- **Lints**: the workspace denies `unsafe_code` and warns on `clippy::all`. `make lint` and CI turn
  the warnings into errors.
- **Git**, **Bash** for `scripts/` (Git Bash on Windows) and **make** for the `Makefile`.
- **cargo-nextest** runs the tests as `make check` and CI do, each test in its own process:
  `cargo install --locked cargo-nextest`. Without it, `scripts/check.sh` falls back to
  `cargo test`. The settings are in `.config/nextest.toml`.
- Optional tools:

| Tool | For | Install |
|---|---|---|
| cargo-deny | `make deny`: advisories, licences and sources (`deny.toml`) | `cargo install --locked cargo-deny` |
| cargo-about | The crates' licences in a package (`about.toml`, `about.hbs`) | `cargo install --locked cargo-about --version 0.9.2 --features cli` |
| pre-commit | The git hooks (`.pre-commit-config.yaml`) | `pipx install pre-commit`, or `brew install pre-commit` |
| shellcheck | Checking the shell scripts | your package manager |

`third_party/oozextract` is outside the workspace (`exclude` in `Cargo.toml`): it is not
formatted or linted with the rest. See [third-party-notices.md](third-party-notices.md).

## System packages

To build on Linux you need a C toolchain, `pkg-config`, and the development files for fontconfig
and xkbcommon (Slint), udev (gilrs, for controllers) and ALSA (rodio, for sounds). These are the
packages CI installs (`.github/actions/setup`):

| | Fedora | Debian and Ubuntu |
|---|---|---|
| Build | `gcc gcc-c++ make pkgconf-pkg-config fontconfig-devel libxkbcommon-devel systemd-devel alsa-lib-devel` | `build-essential pkg-config libfontconfig1-dev libxkbcommon-dev libudev-dev libasound2-dev` |
| Package | `cmake patchelf xz-devel libzstd-devel bzip2-devel lz4-devel zlib-ng-compat-devel mpv mpv-devel` | `cmake patchelf liblzma-dev libzstd-dev libbz2-dev liblz4-dev zlib1g-dev libmpv-dev mpv` |

CI installs the Debian and Ubuntu names. The Fedora names are the equivalents; CI does not test
them.

The packages to run Nexus with all its features (xdotool, libmpv, libarchive, Vulkan, Wine and
more) are optional. They are listed in [user-guide.md](user-guide.md).

The package scripts' Linux parts read Debian's package data (`dpkg-query` for the FFmpeg version,
`/usr/share/doc/<package>/copyright` for licences), so on Fedora they stop. There, make a package
without the libraries with `NEXUS_NO_LIBMPV=1 NEXUS_NO_LIBARCHIVE=1 scripts/package.sh`, or
package on Ubuntu as CI does. The AppImage also needs
[linuxdeploy](https://github.com/linuxdeploy/linuxdeploy) (see [Packaging](#packaging)).

- **macOS**: the Xcode Command Line Tools (`xcode-select --install`) to build. To package:
  `brew install mpv dylibbundler cmake zstd lz4`.
- **Windows**: the Visual Studio Build Tools with the "Desktop development with C++" workload,
  and Rust's `x86_64-pc-windows-msvc` toolchain. `crates/nexus-app/build.rs` puts the icon and
  version info into `nexus.exe` with the SDK's resource compiler. Without it a local build warns
  and goes on, and a CI build fails. To package: Git Bash, 7-Zip (`7z` on the `PATH`), an unpacked
  mpv-dev build named by `LIBMPV_DIR`, and MSYS2's libarchive DLLs named by `LIBARCHIVE_DIR`. The
  `libmpv` and `libarchive` actions in `.github/actions/` show how CI fills both.

## Build profiles

`[profile.dev]` in `Cargo.toml` builds the workspace's own crates at `opt-level = 1`, so a change
rebuilds fast. `[profile.dev.package."*"]` builds every dependency at `opt-level = 2` with line
tables only for debug info, so image decoding, JSON reading and Slint run near release speed under
`make run` and in the tests. The first build after `cargo clean` takes longer for it. Release
builds (`make build`, `make package`) use Cargo's default release profile.

## Make targets

`make help` lists them. This table is its output.

| Target | What it runs |
|---|---|
| `make check` | `scripts/check.sh`: format check, clippy with warnings as errors, and every test, as CI runs them |
| `make fmt` / `make fmt-check` | `cargo fmt --all` / `cargo fmt --all --check` |
| `make lint` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `make test` | `cargo test --workspace` |
| `make run` / `make run-windowed` | `cargo run -p nexus-app` (full screen) / the same with `--windowed` |
| `make build` | `cargo build -p nexus-app --release` |
| `make package` | `scripts/package.sh`: a release package in `dist/nexus-<os>-<arch>/` |
| `make linux-packages` | `make package`, then the deb, rpm and Arch packages in `dist/packages/`, checked (Linux) |
| `make appimage` | `make package`, then `dist/Nexus-x86_64.AppImage` (Linux; needs `LINUXDEPLOY`) |
| `make test-rpm` | `scripts/test-rpm.sh`: a quick rpm of this checkout in `dist/test-rpm/`, to try a change on this PC |
| `make os-image`, `make os-iso`, `make os-vm` | The Nexus OS image, USB stick and a QEMU run of it ([../os/README.md](../os/README.md)) |
| `make deny` | `cargo deny check` |
| `make hooks-install` / `make hooks-run` | Install the git hooks (commit and push) / run them on every file |
| `make catalog`, `make catalog-refresh` | Make or update `assets/catalog/` ([../scripts/catalog/README.md](../scripts/catalog/README.md)) |
| `make clean` | `cargo clean`, then remove `dist/` and `target/` |

`make clean` removes all of `dist/`, including the catalog collector's tracked files in
`dist/catalog/`. `git checkout -- dist` brings the tracked ones back.

## Running

```sh
make run                                  # full screen
make run-windowed                         # a normal window
cargo run -p nexus-app -- --help          # every option
cargo run -p nexus-app -- --windowed --dev --extensions ~/nexus-themes
```

The options are the ones `nexus --help` prints (`USAGE` in `crates/nexus-app/src/args.rs`):

| Option | What it does |
|---|---|
| `--theme <id>` | Shows this theme now. Not saved. |
| `--safe-mode` | Loads the built-in plugins and the default theme only. |
| `--dev` | Reloads a theme loaded from a folder when one of its files changes. A broken edit logs an error and keeps the last good version on screen. The built-in `default` theme is compiled into the binary, so an edit to it needs `make run` again. |
| `--windowed` | A normal window for this run. Full screen is the default. |
| `--monitor <name>` | Opens on this monitor for this run, such as `DP-2`. A screen picked in Settings wins. |
| `--data-dir <path>` | Where settings and plugin data live. |
| `--extensions <path>` | The folder of third-party themes. Default: `<data-dir>/extensions`. Nexus also reads the `extensions/` folder beside the binary. |
| `--folder <name>=<path>` | Maps a named host folder, such as `games=/srv/games`. It is saved. Repeat it for more folders under one name. |
| `--supports <feature>` | Exits 0 when this launcher has the feature (`gamescope`, `system-settings`, `os-install`), else 1. |
| `--version` | Prints `nexus <version>` on one line. |

The default data folder is `$XDG_DATA_HOME/nexus` (`~/.local/share/nexus`) on Linux,
`~/Library/Application Support/nexus` on macOS and `%APPDATA%\nexus` on Windows. For a clean start,
point `--data-dir` at an empty folder.

`NEXUS_SCALE=<n>` draws everything at that size for this run, from 0.8 to 1.4, in place of "Size
of everything" in Settings: `NEXUS_SCALE=1.4 make run-windowed`.

Three switches make Nexus behave as it does on Nexus OS, for this run, on any Linux desktop:

| Switch | What shows |
|---|---|
| `NEXUS_SESSION=1` | Nexus as the whole screen: the network, sound and Bluetooth controls in the top bar, the on-screen keyboard, Log out, Restart and Shut down |
| `NEXUS_OS=1` | Also Nexus OS: the launcher updates with the system, and Settings has System updates |
| `NEXUS_USB=1` | Also Nexus OS's USB stick: "Install Nexus OS" |

```bash
NEXUS_SESSION=1 make run-windowed
```

The controls act on this computer through `nmcli`, `bluetoothctl` and `pactl`, so joining a Wi-Fi
network, pairing and the volume are real, and so are Restart and Shut down. System updates and the
install need `rpm-ostree` and the stick's installer: elsewhere they fail and show their error. See
[nexus-os.md](nexus-os.md).

Ctrl-C, SIGTERM and SIGHUP quit as the Quit button does: the torrents stop first.

## Logging

Nexus logs to stderr through `tracing`. The level is `info` unless `RUST_LOG` says otherwise.
Targets are module paths: the binary is `nexus` (`nexus::gamepad`, `nexus::desktop`,
`nexus::torrents`), and each library crate goes by its name with underscores (`nexus_core`,
`nexus_ui_host`, `release_catalog`, `emu_shadps4`). What a plugin logs through its context goes to
`nexus_core::facades`, with the plugin's id in `plugin_id`.

```sh
RUST_LOG=debug make run                                 # everything
RUST_LOG=info,nexus::gamepad=debug make run             # each controller press and repeat
RUST_LOG=info,nexus_core::facades=debug make run        # the plugins' own debug lines
```

## Tests

```sh
cargo test --workspace                       # every test, as `make test`
cargo test -p nexus-tests --test seams themes::    # one area: a module of tests/tests/seams/
cargo nextest run --workspace                # every test, each in its own process
cd scripts/catalog && npm test               # the catalog collector's tests (Node 20 or newer)
```

`scripts/check.sh test` runs the tests with cargo-nextest when it is installed, as CI does, and with
`cargo test` otherwise. The seam tests are one test binary, `seams`
(`tests/tests/seams/main.rs`, one module per area), so their shared code builds and links once.
`spans.rs` is its own binary. The seams and their conventions are in the Tests section of
[architecture.md](architecture.md#tests).

The tests need no window, GPU, controller or sound device: they use Slint's testing backend and
fakes. `cargo test -p nexus-tests --test seams screenshots::` renders each default theme screen to
PNG files in `target/tmp/screenshots/`.

Some tests touch the real system, the network or files you supply. Each one skips, or returns at
once, unless its variable is set:

| Variable | What it runs | Needs |
|---|---|---|
| `NEXUS_DESKTOP_TESTS=1` | `cargo test -p nexus-app --test desktop`: shows real windows and reads which one is in front | A desktop. On Linux, X11 with a window manager, `xdotool` and `xprop`. On macOS, the Accessibility permission. |
| `NEXUS_DESKTOP_REQUIRE=1` | With the above, a skipped desktop test fails | |
| `NEXUS_KWIN_TESTS=1` | `cargo test -p nexus-app kwin_says`: asks the real KWin which window is in front | A KDE Plasma Wayland session |
| `NEXUS_TRAILER_TESTS=1` | `cargo test -p nexus-app libmpv_plays`: plays a made-up video through the real libmpv | libmpv, and `ffmpeg` to make the video |
| `NEXUS_TORRENT_TESTS=1` | `cargo test -p nexus-app a_torrent_downloads`: a real BitTorrent transfer between two sessions on this computer | |
| `NEXUS_VPN_TESTS=1` | `cargo test -p nexus-app lists_this_computer_s_networks`: lists this computer's networks and connects nothing | |
| `NEXUS_REAL_RELEASES=1` | `cargo test -p nexus-app real -- --nocapture`: downloads and installs the newest real KytyPS5 and shadPS4 releases | The network and hundreds of megabytes. Anonymous GitHub API calls can hit the rate limit. |
| `NEXUS_REAL_ARCHIVES=<folder>` | `cargo test -p nexus-tests --test seams real_archives::`: installs real release archives through the plugins on a fake web | The archives, named as GitHub names them. A missing one is skipped. |
| `NEXUS_CATALOG_FILES=<file>[:<file>…]` | `cargo test -p release-catalog catalog_files -- --nocapture`: reads catalog files through the plugin's reader and prints what each holds | Catalog files, such as the collector's |
| `NEXUS_TIMING=1` | `cargo test -p nexus-tests --test seams timing:: -- --nocapture`: times the Catalog view with the bundled catalogs | |
| `NEXUS_COMPARE_OUT=<folder>` | `cargo test -p nexus-tests --test seams compare::`: renders the default theme's screens on a real library into that folder | A KytyPS5 game library in `NEXUS_COMPARE_GAMES` (default `~/Games`). Optional: `NEXUS_COMPARE_BUILDS`, `NEXUS_COMPARE_WEB`. |
| `PS4_TEST_PKG=<file>` | `cargo test --release -p emu-shadps4 real_ps4_package -- --ignored --nocapture`: lists a real PS4 package | A PS4 fake package |
| `PS5_TEST_PKG=<file>` | `cargo test -p emulator-kit real_package -- --ignored`: lists a real PS5 debug package and decodes its largest file (`PS5_TEST_FULL=1` decodes every file) | A PS5 debug package |

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

`make hooks-install` installs both stages. `make hooks-run` runs the hooks on every file. Each cargo
hook moves into `nexus/` first when the repository root has a `nexus/Cargo.toml`, so the hooks also
work while Nexus is a folder of a larger repository. There, name the config:
`pre-commit run -c nexus/.pre-commit-config.yaml`.

## Packaging

`make package` (`scripts/package.sh`) makes a release build with `--locked`, then
`dist/nexus-<os>-<arch>/` with `README.md`, `LICENSE`, an empty `extensions/` and a `licenses/`
folder, and runs the packaged `nexus --help` to check that it starts.

| System | What it makes |
|---|---|
| Linux | The binary, libraries in `lib/`, `nexus.desktop`, the icon and `install.sh`, packed as `dist/nexus-linux-<arch>.tar.gz` |
| macOS | `Nexus.app` (the binary, libmpv in `Contents/Frameworks`, the icon, `extensions/` and the licences in `Contents/Resources`, an `Info.plist` with the workspace version), signed ad hoc and packed as a `.zip`. `NEXUS_BUNDLE_ID` sets the bundle id. |
| Windows | `nexus.exe`, the DLLs beside it and the icon. CI zips the folder with 7-Zip. |

CI and the release also make these files from those folders:

| File | Made by |
|---|---|
| deb, rpm and Arch packages | `packaging/linux/build-packages.sh <folder> <version> <out>` (nfpm, `packaging/linux/nfpm.yaml`), checked by `packaging/linux/check-packages.sh <out>` |
| `Nexus-<arch>.AppImage` | `scripts/appimage.sh`, with linuxdeploy named by `LINUXDEPLOY` |
| `nexus-macos-universal.zip` and `.dmg` | `packaging/macos/universal.sh`, then `packaging/macos/make-dmg.sh` |
| `nexus-windows-x86_64-setup.exe` | `scripts/windows-installer.sh <version>` (Inno Setup 6, `packaging/windows/nexus.iss`) |

`scripts/update-manifest.sh` writes `latest.json`, which the launcher's own update reads. The
signing steps are in [signing.md](signing.md).

The `licenses/` folder holds the fonts' licences, `THIRD-PARTY.md` (made by `cargo about generate`
from `about.toml` and `about.hbs`; without cargo-about a local package warns and goes on, and with
`CI` set it fails), and the `libmpv/` and `libarchive/` licence folders from the bundle scripts.
`scripts/bundle-libmpv.sh` and `scripts/bundle-libarchive.sh` copy or build those libraries;
`NEXUS_NO_LIBMPV=1` and `NEXUS_NO_LIBARCHIVE=1` leave them out. `scripts/fetch-sources.sh` downloads
the source archives that the `SOURCES` files name.

`install.sh`, in the tarball, installs the package for the user without sudo. Check it with
`shellcheck scripts/install.sh`.

## The Nexus session

`packaging/linux/nexus-session` runs Nexus as the whole session, with no desktop. Display managers
list it as "Nexus" (`nexus-session.desktop`), and Nexus OS's greetd starts it. It runs gamescope,
starts Nexus again when it ends or shows no `window shown` line within the watchdog time, and runs
Nexus in cage when gamescope cannot drive the graphics card. It sets `NEXUS_SESSION=1`. Its
settings are in `/usr/share/nexus/session.conf` (`packaging/linux/session.conf`), and
`/etc/nexus/session.conf` overrides them. The variables it reads are `NEXUS_BIN`,
`NEXUS_GAMESCOPE_ARGS`, `NEXUS_WAIT_NETWORK`, `NEXUS_WATCHDOG`, `NEXUS_LOG` and
`NEXUS_LOG_MAX_MB`. Output goes to `~/.cache/nexus-session.log`.

## CI

The workflows are in `.github/workflows/`:

| Workflow | When | What it does |
|---|---|---|
| `ci.yml` | Pull requests, pushes to `main`, by hand | On Linux, side by side: Format (`cargo fmt --check`), Clippy (`-D warnings`), Test (cargo-nextest) and cargo-deny |
| `weekly.yml` | Mondays, by hand | The tests on Windows; on Linux: the real emulator releases install, the desktop adapter under Xvfb and openbox, the trailer player on a real video, and `cargo deny` with the newest advisories |
| `release.yml` | Pushes to `main`, by hand | Release Please keeps a release pull request open for Nexus and one for Nexus OS, makes the draft release and tag when one merges, then starts `package.yml` or `os.yml` on the tag |
| `package.yml` | A Nexus tag (started by `release.yml`), by hand | The packages on Linux, both Mac processors and Windows. On a tag it publishes the release, records it in `os/nexus.txt`, and starts `os.yml` |
| `os.yml` | Daily, on a Nexus OS tag, after a Nexus release, by hand | The Nexus OS images and USB stick ([../os/README.md](../os/README.md)) |
| `mirror-deps.yml` | By hand | Copies the pinned Windows libmpv build into this repository's `deps` release |

`ci.yml` skips what cannot change the build: pushes and pull requests that only touch Markdown,
`docs/`, `os/`, `version.txt`, `LICENSE` or the other workflows. It runs on Linux only. macOS and
Windows build in `package.yml`: run it by hand on a branch to try a change there. Its files stay as
the run's artifacts.

For branch protection, require Format, Clippy, Test and cargo-deny.

The shared steps are in `.github/actions/`: `setup` (the toolchain, the Linux build packages and
the dependency cache), `libmpv`, `libarchive` and `linux-packages` (a pinned nfpm, checked against
its SHA-256).

The workflows follow these rules:

- Every third-party action is pinned to a commit, with its version in a comment.
- Permissions are `contents: read` or none at the top. A job that writes asks for what it needs.
- Every job has a `timeout-minutes`.
- Cargo runs with `--locked`, so CI fails when `Cargo.lock` is out of date.
- A new push to a pull request cancels its older CI run.
- Only `main` saves the dependency cache. A pull request restores `main`'s cache.

## Releases

Releases come from [Release Please](https://github.com/googleapis/release-please), which reads
Conventional Commit messages. `feat:` raises the minor version and `fix:` the patch version (before
1.0, a breaking change raises the minor version). The other prefixes (`docs:`, `ci:`, `refactor:`,
`test:`, `chore:`) make no release. The settings are in `.github/release-please-config.json` and
`.github/release-please-manifest.json`.

1. `release.yml` keeps one release pull request open for Nexus, which bumps `version` in
   `[workspace.package]`, the workspace's crates in `Cargo.lock` and `version.txt`, and writes
   `CHANGELOG.md`. Changes under `os/` belong to Nexus OS, which has its own pull request.
2. Merging that pull request makes a draft release and the tag `v<version>`. `release.yml` then
   starts `package.yml` on the tag.
3. `package.yml` checks that the tag is `v` and the workspace version, builds on Linux, both Mac
   processors and Windows, joins the Mac apps, signs what it can ([signing.md](signing.md)),
   attaches every file with a `.sha256` file and signed build provenance, writes `latest.json`, and
   publishes the release as the latest. It does not run the tests: CI did that on the commit.
4. After the release is published, `package.yml` names it in `os/nexus.txt` and starts `os.yml`, so
   the Nexus OS images get the new Nexus.
5. Merging Nexus OS's release pull request makes the tag `nexus-os-v<version>`, and `release.yml`
   starts `os.yml` on it: the images and the stick, and the OS release.

When `package.yml` fails for a release, run it again on the same tag: Actions, Package, Run
workflow, with the tag under "Use workflow from". Do not make a new release.

## The launcher's own update

The launcher checks `https://github.com/<repo>/releases/latest/download/latest.json`, where
`<repo>` is `app.update_repo` in `config.json` (`owner/name`) and `mohamed-rekiba/nexus` by
default. An empty `app.update_repo` turns the checks off. A draft or a pre-release is not "latest",
so it is not offered. `latest.json` holds the version and, for each kind of install, the package's
URL and SHA-256 (`scripts/update-manifest.sh`). The code is `launcher_update.rs` in `nexus-core` and
`update.rs` in `nexus-app`; [architecture.md](architecture.md#the-launchers-update) describes the
flow. A build that is not installed from a package, such as `cargo run`'s, is never replaced.
