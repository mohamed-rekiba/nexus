# Third-party code and notices

Nexus is under the GNU General Public License, version 2 or (at your option) any later version:
see `LICENSE`. The PS4 package reader is a port of GPL code, which sets that licence for the
whole program.

## Code in this repo

| Where | What | Licence |
|---|---|---|
| `plugins/emulators/shadps4/src/pkg4.rs`, `pkg4_data.rs` | The PS4 fake package reader and its key data, ported from shadPS4 v0.7.0 (`src/core/file_format/pkg.cpp`, `src/core/crypto/crypto.cpp`, `keys.h`, `pkg_type.cpp`), with LibOrbisPkg's file system walk | GPL-2.0-or-later (shadPS4); LibOrbisPkg is MIT |
| `third_party/oozextract` | The Kraken decompressor that the PS5 package reader uses: the `oozextract` crate 0.5.5, with fixes listed in its `NOTICE.md`. It is a Rust port of powzix's `ooz`. | MIT, as the crate declares |
| `plugins/themes/default/theme/fonts/` | Inter and Tajawal | SIL Open Font License 1.1 (`OFL-Inter.txt`, `OFL-Tajawal.txt` beside them) |
| `crates/nexus-app/testdata/version.rar`, `crypted.rar` | Test archives from the `unrar` crate 0.5.8's test data; no UnRAR code | MIT OR Apache-2.0 |

The rest, the PS5 package and exFAT readers among it, is this project's own code.

## Rust crates

Each crate keeps its own licence. `deny.toml` allows only licences that fit a GPL work, and
`cargo deny check` fails on any other. Slint is used under its GPL-3.0-only option, which the
"or any later version" in Nexus's licence allows.

Each package carries `licenses/THIRD-PARTY.md`: every crate in the build, with its licence text.
`scripts/package.sh` makes it with `cargo about generate` from `about.toml` and `about.hbs`, which
accept the same licences as `deny.toml`.

## Libraries in the packages

A package's `licenses/` folder (`Contents/Resources/licenses` in `Nexus.app`) holds:

- `OFL-Inter.txt` and `OFL-Tajawal.txt`, for the fonts compiled into the binary.
- `THIRD-PARTY.md`, for the Rust crates.
- `libmpv/`, from `scripts/bundle-libmpv.sh`. libmpv plays the trailers. It ships with the
  libraries it needs that a desktop doesn't already have, FFmpeg's among them: in `lib/` on Linux,
  `Contents/Frameworks` on macOS, and as `libmpv-2.dll` beside `nexus.exe` on Windows. The folder
  has mpv's `Copyright`, `LICENSE.GPL` and `LICENSE.LGPL`, FFmpeg's licence texts in `ffmpeg/`, a
  `NOTICE` that names the build and the files, and `SOURCES`. mpv is GPL-2.0-or-later, with the
  parts under the LGPL named in its `Copyright`. FFmpeg is LGPL or GPL, by how it was built. Each
  other library keeps its own licence.
- `libarchive/`, from `scripts/bundle-libarchive.sh`. libarchive unpacks RAR, tar.xz, tar.bz2,
  tar.zst, ISO and cab archives. On Linux and macOS it is libarchive 3.8.9, built from its release
  with only its compression codecs, so nothing under the LGPL comes with it. On Windows it is
  MSYS2's `mingw-w64-ucrt-x86_64-libarchive` with the DLLs it loads. The folder has libarchive's
  `COPYING` (BSD-2-Clause), the codecs' licences (zstd and lz4 BSD, xz 0BSD, bzip2 its own
  BSD-style licence; on Windows each MSYS2 package's), a `NOTICE` that names the files, and
  `SOURCES`.

The AppImage keeps the same folder in `usr/share/licenses/nexus/`, with `LICENSE`.

RAR goes through libarchive, loaded at run time, and not through the `unrar` crate: UnRAR's
licence adds terms the GPL doesn't allow.

## Source of the shipped libraries

The `SOURCES` files list where the source of mpv, FFmpeg and libarchive is, at the versions
shipped; on Windows also each MSYS2 source package. The release runs `scripts/fetch-sources.sh`
on each package and publishes the result beside it as
`nexus-<os>-<arch>-third-party-sources.tar`.
