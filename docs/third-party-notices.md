# Third-party code and notices

Nexus is under the GNU General Public License, version 2 or (at your option) any later
version. The file `LICENSE` holds the text of version 2. The workspace declares
`license = "GPL-2.0-or-later"` in `Cargo.toml`. The comment there gives the reason: the PS4
package reader is a port of shadPS4 code, which is GPL-2.0-or-later.

This page is for anyone who redistributes Nexus. It lists what comes from other projects: code
in this repository, Rust crates, libraries inside the packages, and fonts. Each licence named
here was read from a file in the repository or from a script that fetches it. Where the
repository holds no licence information, this page says so.

## Code in this repository

| Where | What | Licence |
| --- | --- | --- |
| `plugins/emulators/shadps4/src/pkg4.rs` | The PS4 fake package reader. Ported from shadPS4 v0.7.0 (`src/core/file_format/pkg.cpp`, `src/core/crypto/crypto.cpp`), with the file system walk of LibOrbisPkg in place of shadPS4's guesses. | shadPS4: GPL-2.0-or-later (the file's SPDX line). LibOrbisPkg: MIT (the file's header). |
| `plugins/emulators/shadps4/src/pkg4_data.rs` | Key data for that reader, generated from shadPS4 v0.7.0 (`src/core/crypto/keys.h`, `src/core/file_format/pkg_type.cpp`). Do not edit by hand. | GPL-2.0-or-later (the file's SPDX line). Copyright 2024 shadPS4 Emulator Project. |
| `third_party/oozextract` | Vendored copy of the `oozextract` crate 0.5.5, a Kraken decompressor that the PS5 package reader uses. | MIT, as the crate declares in `third_party/oozextract/Cargo.toml`. |

Everything else in the repository is this project's own code. That includes the PS5 package
reader (`plugins/shared/emulator-kit/src/pkg5.rs`) and the exFAT reader (`exfat.rs`). The header
of `pkg5.rs` says the package format is not documented by its vendor, and that its offsets were
checked against packages built by other open tools. It does not name code it copies.

### oozextract

`third_party/oozextract/NOTICE.md` records the origin. The crate is by lvlvllvlvllvlvl
(<https://github.com/lvlvllvlvllvlvl/oozextract>). It is a Rust port of `ooz` by powzix
(<https://github.com/powzix/ooz>).

Nexus changed the copy in these ways, as `NOTICE.md` lists them:

- It removed the command line tool, the tests, the benches, and the optional `tokio`, `wasm` and
  `clap` dependencies.
- `src/algorithm/kraken.rs` and `src/decoder/mod.rs` now read the "excess bytes" form of a
  Kraken table. Upstream refuses it. PS5 packages use it.
- `decode_bytes_type12` returned the wrong byte count for a table with one symbol. It now
  returns the bytes it read, as `ooz` does.

The workspace excludes this folder (`exclude` in `Cargo.toml`), so it is not formatted or linted
with the rest.

## Rust crates

Each crate keeps its own licence. `deny.toml` allows these licences, and `cargo deny check`
fails on any other. CI runs the check (`deny` job in `.github/workflows/ci.yml`).

```
0BSD, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, BSL-1.0,
CC0-1.0, CDLA-Permissive-2.0, GPL-3.0-only, ISC, MIT, MIT-0, MPL-2.0, Unicode-3.0, Zlib
```

The comments in `deny.toml` explain three of them:

- The launcher is GPL-2.0-or-later, so every licence must allow use in a GPL work.
- Apache-2.0 and GPL-3.0-only are fine because of "or later".
- Slint is GPL-3.0-only (or its own licences). MPL-2.0 is the symphonia audio decoders.
  CDLA-Permissive-2.0 is the webpki root certificates.

`deny.toml` also denies wildcard versions, unknown registries and unknown git sources. Only
crates.io is allowed. It lists a few RustSec advisories that it accepts, each with its reason.

`about.toml` lists the same licences for `cargo about`. `about.hbs` is the template. A
package gets `licenses/THIRD-PARTY.md`, made by `scripts/package.sh` with
`cargo about generate --locked --workspace --fail`. The file lists each licence with the crates
that use it, then each licence text. CI builds fail if `cargo-about` is missing. A local build
warns and goes on without the file.

`about.toml` sets `private = { ignore = true }`, and so does `deny.toml`. Crates with
`publish = false` count as private. `third_party/oozextract` sets `publish = false`. So
`THIRD-PARTY.md` and the licence check most likely skip it. This page did not run
`cargo about` to confirm. See Limitations.

## What the packages ship

`scripts/package.sh` builds each package. Besides the binary, it copies `README.md` and
`LICENSE`, then fills a `licenses/` folder. The folder sits here:

| Package | `licenses/` is in |
| --- | --- |
| Linux tar.gz | The package folder, beside the binary. |
| deb, rpm, Arch | `/usr/lib/nexus/licenses` (`packaging/linux/nfpm.yaml`) |
| AppImage | `usr/share/licenses/nexus/`, with `LICENSE` (`scripts/appimage.sh`) |
| macOS | `Nexus.app/Contents/Resources/licenses` |
| Windows | The package folder, beside `nexus.exe`. |

The folder holds:

- `OFL-Inter.txt` and `OFL-Tajawal.txt`, the licences of the fonts.
- `THIRD-PARTY.md`, for the Rust crates.
- `libmpv/`, from `scripts/bundle-libmpv.sh`.
- `libarchive/`, from `scripts/bundle-libarchive.sh`.

Setting `NEXUS_NO_LIBMPV=1` or `NEXUS_NO_LIBARCHIVE=1` leaves a library, and its folder, out of
a package.

Nexus loads both libraries at run time with `libloading`. It does not link to them. The
packages still ship them. The search order puts the package's copy first: `lib/` beside the
binary on Linux, `Contents/Frameworks` in the macOS bundle, and the binary's own folder on
Windows.

### Fonts

The default theme carries Inter and Tajawal. `plugins/themes/default/theme/theme.toml` lists the
font files, and `package.sh` says the theme carries them in the binary.

| Font | Copyright line in the licence file | Licence |
| --- | --- | --- |
| Inter | Copyright (c) 2016 The Inter Project Authors (<https://github.com/rsms/inter>) | SIL Open Font License 1.1 (`OFL-Inter.txt`) |
| Tajawal | Copyright 2018 Boutros International (<http://www.boutrosfonts.com>) | SIL Open Font License 1.1 (`OFL-Tajawal.txt`) |

The licence files sit beside the fonts in `plugins/themes/default/theme/fonts/`.

### libmpv

libmpv plays the trailers. `scripts/bundle-libmpv.sh` copies it and the libraries it needs into
the package. It leaves out the system's own libraries (C runtime, graphics, sound and windowing
on Linux).

| System | libmpv comes from |
| --- | --- |
| Linux | The build machine's package. On Ubuntu, the runner's `libmpv`; on Fedora, `mpv-libs`. The CI runner is Ubuntu 22.04 (`.github/workflows/package.yml`). |
| macOS | Homebrew's `mpv` formula, with its libraries copied by `dylibbundler`. |
| Windows | shinchiro's `mpv-winbuild-cmake` build, a pinned file with a pinned SHA-256 (`.github/actions/libmpv/action.yml`). |

The script writes `licenses/libmpv/` with:

- `Copyright`, `LICENSE.GPL` and `LICENSE.LGPL`, fetched from mpv's source at the version
  shipped.
- `ffmpeg/` with FFmpeg's `LICENSE.md`, `COPYING.GPLv2`, `COPYING.GPLv3`, `COPYING.LGPLv2.1`
  and `COPYING.LGPLv3`, fetched from FFmpeg's source.
- `NOTICE`, which names the build and lists the files.
- `SOURCES`, which gives the address of the source of mpv and FFmpeg.

The `NOTICE` text says: libmpv is mpv's library, under the GNU General Public License, version 2
or later, with some parts under the LGPL named in `Copyright`. FFmpeg is under the LGPL or the
GPL, and the libraries libmpv uses each keep their own licence.

### libarchive

libarchive unpacks RAR, tar.xz, tar.bz2, tar.zst, ISO and cab archives and some 7z archives. See
[reference/archives-and-pc-games.md](reference/archives-and-pc-games.md). The reason for using
it for RAR instead of the `unrar` crate is in `crates/nexus-app/Cargo.toml`: UnRAR's licence
adds terms the GPL does not allow.

| System | libarchive comes from |
| --- | --- |
| Linux and macOS | Built from source by `scripts/bundle-libarchive.sh`: the release named by `VERSION` in the script, checked against `SHA256` in the script. |
| Windows | MSYS2's `mingw-w64-ucrt-x86_64-libarchive` package and the DLLs it loads. `.github/actions/libarchive/action.yml` does not pin these packages, so it records their names and versions in the package's `NOTICE` and `SOURCES`. |

The build on Linux and macOS turns off XML, ACL, crypto, iconv and other optional parts
(see the `cmake` flags in the script). The script says this leaves only compression codecs, so
nothing under the LGPL comes with it.

The script writes `licenses/libarchive/` with:

- `COPYING`, libarchive's licence. The source file `crates/nexus-app/src/libarchive.rs` and the
  script's `NOTICE` text name it as BSD 2-clause.
- The licences of the codecs. On macOS: zstd's and lz4's `LICENSE`. On Linux: Debian's
  `copyright` files for `liblzma5`, `libzstd1`, `libbz2-1.0` and `liblz4-1`, or on an rpm
  system the licence files of the owning packages. On Windows: the `licenses` folder of the
  MSYS2 build.
- `NOTICE`, which names the build and lists the files. It says zstd and lz4 are BSD, xz is 0BSD,
  bzip2 has its own BSD-style licence, and that on Windows other libraries come too.
- `SOURCES`, with the address of the libarchive release. On Windows the action writes the MSYS2
  source package addresses.

## Source of the shipped libraries

`scripts/fetch-sources.sh <package folder>` reads every `licenses/*/SOURCES` file in a package
and downloads each address. It writes `dist/<package>-third-party-sources.tar`. The release
workflow runs it for each package, and attaches the tar to the release beside the package. On
macOS it runs once for the universal app.

## Test data

The folder `crates/nexus-app/testdata` holds small archives for the tests. Its `README.md` says
`version.rar` and `crypted.rar` come from the test data of the `unrar` crate 0.5.8
(<https://github.com/muja/unrar.rs>, MIT OR Apache-2.0). No UnRAR code is in the repository. The
other files there were made for these tests with bsdtar, 7-Zip and Info-ZIP.

## Programs Nexus downloads at run time

The emulators and tools below are not part of any package. Nexus downloads them when the
player installs them. They keep their own licences, and this repository holds none of them.

- KytyPS5, from the GitHub releases of `KytyPS5/KytyPS5`.
- shadPS4, from the GitHub releases of `shadps4-emu/shadPS4`.
- umu-launcher, from the GitHub releases of `Open-Wine-Components/umu-launcher`. It downloads
  Proton (GE-Proton) by itself when a Windows game runs.

## Nexus OS images

The Nexus OS images (`os/`) ship Nexus and Fedora's packages. The label in `os/Containerfile`
sets `org.opencontainers.image.licenses="GPL-2.0-or-later"` for the image. Each Fedora package
keeps its own licence, and the repository holds no list of them.

The NVIDIA edition (`os/nvidia/Containerfile`) adds NVIDIA's driver packages and kernel modules
from Universal Blue's `akmods-nvidia-lts` image. Both images carry Universal Blue's public
Secure Boot key (`os/Containerfile` downloads it and checks its SHA-256). The repository holds
no licence text for the driver or the key.

## Limitations

- **`oozextract` may be missing from the shipped licence list.** `THIRD-PARTY.md` most likely
  leaves it out (see "Rust crates"), and no package ships `third_party/oozextract/NOTICE.md`.
  The crate declares MIT, but this repository holds no MIT licence text file for it. This page
  did not run `cargo about` to check.
- **The `libmpv/` folder holds licence texts for mpv and FFmpeg only.** Other libraries that
  libmpv needs are copied into the package, but the script fetches no licence text for them.
  The `NOTICE` says they each keep their own licence.
- **Source for libmpv's other libraries is not in the sources tar.** `SOURCES` lists mpv and
  FFmpeg only. The `NOTICE` says the other libraries' source is that of the named build, at the
  same versions.
- **On Windows the FFmpeg source and licence texts come from `master`.** The script says the
  build's own FFmpeg commit is not in the file name.
- **Windows libarchive packages are not pinned.** The MSYS2 packages are the ones the runner
  installs on that day. The package's `NOTICE` and `SOURCES` record the versions.
- **The theme's images and sounds have no recorded licence.** `plugins/themes/default/theme/images/`
  holds `logo.png`, and `sounds/` holds five `.wav` files. The icons in `assets/` (`nexus.png`,
  `nexus.ico`, `nexus.icns`, `logo.svg`) have none either. This page does not guess who owns
  them.
- **The bundled catalogs have no recorded licence.** `assets/catalog/` and
  `assets/catalog-sources.json` hold lists of releases. The repository states no licence for
  them.
- **Not checked.** This page was written from the files. No package was built, and no crate
  licence was checked against the lock file.
