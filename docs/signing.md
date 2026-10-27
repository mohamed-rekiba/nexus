# Signing the release

This page is for maintainers. It says how the release workflow signs and notarizes Nexus
packages, which secrets it reads, and how the launcher checks an update. How to build and test
packages is in [development.md](development.md).

Signing is optional in the workflow. Without the secrets, the build works and the packages
carry only the signing the build does by itself (macOS: ad hoc; Windows: none). The workflow
prints a warning for each missing set of secrets.

## What is signed

| Package | Signed with | When |
| --- | --- | --- |
| macOS app, zip and disk image | Apple Developer ID, notarized and stapled | The macOS secrets are set. |
| macOS app without secrets | Ad hoc (`codesign --sign -`) | Always. Apple Silicon runs nothing unsigned. |
| Windows `nexus.exe` and `.dll` files | Authenticode | The two Windows secrets are set. |
| Windows installer (`-setup.exe`) | Authenticode | The two Windows secrets are set. |
| Linux tar.gz, AppImage, deb, rpm, Arch package | Not signed | Never. |
| Every package | A `.sha256` file and a signed build provenance attestation | Release tags only. |

## The workflow

`.github/workflows/package.yml` is named "Package". It runs only by hand (`workflow_dispatch`)
or from `release.yml` on a release tag. It does not run the tests.

| Job | Does |
| --- | --- |
| `version` | Reads the version from `Cargo.toml`. On a tag, fails when the tag is not `v<version>`. |
| `build` | One job each for Linux, macOS arm64, macOS x86-64 and Windows. Builds, packs, signs the Windows files, and uploads artifacts. |
| `macos` | Joins the two Mac apps into one universal app, signs and notarizes it, and makes the disk image. |
| `publish` | On a tag only. Writes checksums and `latest.json`, attests, attaches the files to the draft release, and publishes it as the latest. |
| `os` | On a tag only. Names this release in `os/nexus.txt` and starts `os.yml`. See [os/README.md](../os/README.md). |

On a branch, the workflow only builds. The files stay as artifacts of the run. Use that to try
a packaging change.

The `publish` job has write access to the release (`contents: write`) and may attest
(`id-token: write`, `attestations: write`). The `os` job may write `os/nexus.txt` and start
workflows (`contents: write`, `actions: write`). The other jobs have `contents: read`.

## macOS

### Without secrets

`scripts/package.sh` signs each `Nexus.app` ad hoc: `codesign --force --deep --sign -`.
`packaging/macos/universal.sh` joins the arm64 and the x86-64 app with `lipo` and signs the
result ad hoc again. Libraries that `scripts/bundle-libmpv.sh` and `scripts/bundle-libarchive.sh`
add are also signed ad hoc. Gatekeeper asks before the first start of such an app.

### With secrets

The `macos` job reads these secrets:

| Secret | Holds |
| --- | --- |
| `MACOS_CERT_P12` | The Developer ID certificate as a `.p12` file, in base64. |
| `MACOS_CERT_PASSWORD` | The password of the `.p12` file. |
| `APPLE_ID` | The Apple ID used to notarize. |
| `APPLE_TEAM_ID` | The Apple team ID. |
| `APPLE_APP_PASSWORD` | An app-specific password for that Apple ID. |

Set them all together. The workflow tests only `MACOS_CERT_P12`. If that one is set and the
others are not, `notarytool` fails and so does the job.

The step "Sign and notarize the app" does this:

1. Makes a keychain in `$RUNNER_TEMP`, decodes the certificate with `base64 --decode`, and
   imports it for `codesign`.
2. Signs every `.dylib` in `Contents/Frameworks`, then the app, with
   `--timestamp --options runtime --sign "Developer ID Application"`. The hardened runtime and
   a timestamp are what notarization asks for.
3. Zips the app with `ditto` and submits it with `xcrun notarytool submit … --wait`.
4. Staples the ticket to the app with `xcrun stapler staple`.
5. Zips the app again. This zip is the release file `nexus-macos-universal.zip`.

The step "Disk image" runs `packaging/macos/make-dmg.sh`, which makes a disk image holding
`Nexus.app` and a link to `/Applications`. With `MACOS_CERT_P12` set, it signs the image,
notarizes it, and staples it. It then checks the image with `hdiutil verify`, mounts it, and
tests that the app's program and the link are there.

The workflow passes no entitlements file to `codesign`.

## Windows

The build job signs with `scripts/sign-windows.ps1`, in three steps:

1. Before packing: the `.exe` and `.dll` files in `dist\nexus-windows-*`.
2. The zip and the installer are built from those signed files.
3. After building: the installer `dist\nexus-windows-x86_64-setup.exe`.

The zip itself is not signed.

The script reads these secrets:

| Secret | Holds |
| --- | --- |
| `WINDOWS_CERT_PFX` | The code-signing certificate as a `.pfx` file, in base64. |
| `WINDOWS_CERT_PASSWORD` | The password of the `.pfx` file. |

Without `WINDOWS_CERT_PFX`, the script prints a warning and signs nothing. SmartScreen then
warns on the first start.

With it, the script writes the certificate to `$RUNNER_TEMP\cert.pfx`, finds the newest
`signtool.exe` in the Windows Kits, and signs each file with SHA-256 for the file digest
(`/fd sha256`) and DigiCert's timestamp server (`/tr http://timestamp.digicert.com /td
sha256`). It deletes the certificate file at the end.

The workflow then installs the signed installer silently, checks that the file
`installed-by-setup` and `nexus.exe --help` work, and runs the uninstaller. The installer is
`packaging/windows/nexus.iss`.

## Linux

Nothing is signed. The Linux job builds the tar.gz, the AppImage (`scripts/appimage.sh`), and
the deb, rpm and Arch packages (`.github/actions/linux-packages` with nfpm). The action checks
the downloaded nfpm against a pinned SHA-256, then checks the packages
(`packaging/linux/check-packages.sh`).

Tools that other actions download are pinned and checked the same way: linuxdeploy and the
Windows mpv build in `.github/actions/libmpv`. The Windows libarchive comes from MSYS2's
packages, which the workflow does not pin.

## Checksums, attestations and `latest.json`

The `publish` job does this for each package file (`*.tar.gz`, `*.zip`, `*.tar`, `*.AppImage`,
`*.deb`, `*.rpm`, `*.pkg.tar.zst`, `*.exe`, `*.dmg`):

1. Writes `<file>.sha256` with `sha256sum`.
2. Attests the file with `actions/attest-build-provenance`, which signs a build provenance
   record with GitHub's identity. The record is not a code-signing certificate.
3. Attaches it to the release with `gh release upload --clobber`.

It also writes `latest.json` with `scripts/update-manifest.sh`. Last, it publishes the draft
with `gh release edit --draft=false --latest`. The launcher's update reads the latest release,
so it never sees a release without its files.

The files `nexus-macos-aarch64.zip` and `nexus-macos-x86_64.zip` are removed first. They were
only input for the universal app.

`scripts/update-manifest.sh <folder> <tag> <owner/repo>` takes each package's SHA-256 from its
`.sha256` file. The keys are one per kind of install:

| Key | File |
| --- | --- |
| `linux-<arch>` | `nexus-linux-<arch>.tar.gz` |
| `linux-<arch>-appimage` | `Nexus-<arch>.AppImage` |
| `linux-x86_64-deb` | `nexus_<version>-1_amd64.deb` |
| `linux-x86_64-rpm` | `nexus-<version>-1.x86_64.rpm` |
| `linux-x86_64-arch` | `nexus-<version>-1-x86_64.pkg.tar.zst` |
| `macos-aarch64`, `macos-x86_64` | `nexus-macos-universal.zip` (one app for both) |
| `windows-<arch>` | `nexus-windows-<arch>.zip` |
| `windows-<arch>-setup` | `nexus-windows-<arch>-setup.exe` |

Each entry has a `url` into the release and a `sha256`. Files such as
`nexus-*-third-party-sources.tar` and the disk image get no entry.

## The launcher's own update

The launcher checks its update in `crates/nexus-core/src/launcher_update.rs`. The startup code
that applies it is `crates/nexus-app/src/update.rs`.

The default theme's Settings row sends the command `LauncherUpdate` with `Check`, then with
`Download` when an update is available. This page found no automatic check at startup.

### Check

1. The repo is the setting `app.update_repo`, `owner/name`. The default is
   `mohamed-rekiba/nexus` (`DEFAULT_REPO`). An empty value turns the checks off.
2. The core reads `https://github.com/<repo>/releases/latest/download/latest.json`.
3. It compares the version in the file with its own, as semver. It reports an update only when
   the file's version is newer.
4. It picks the entry for this install. `Install::detect` decides the key: the AppImage
   (`APPIMAGE` is set), a deb, rpm or Arch package (the file `packaged` beside the binary),
   the Windows installer (the file `installed-by-setup`), `Nexus.app`, the unpacked folder, or
   Nexus OS. If the file has no entry, the state is "Nexus `<version>` has no package for this
   system".

### Download and check

The core downloads only from `github.com`, `objects.githubusercontent.com` and
`release-assets.githubusercontent.com` (`HOSTS`), over HTTPS. It follows at most 5 redirects
(`MAX_REDIRECTS`). Any other host stops the download.

It then computes the SHA-256 of the file and compares it with the `sha256` in `latest.json`. A
mismatch fails the update. The core keeps the package in `updates/` in the data folder, with
`ready.json`.

### Applying

`Install::method` decides how an update goes in.

| Install | Method | What happens |
| --- | --- | --- |
| tar.gz folder, AppImage, `Nexus.app`, Windows zip | `swap` | At the next start, `apply_staged` swaps the new package in and starts it. It renames the old one aside first and puts it back if anything fails. It tries once. |
| Windows installer | `installer` | At the next start, Nexus runs the new installer with `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART` and ends. The installer starts the new version. |
| deb, rpm, Arch | `package` | The core runs `pkexec` with `apt-get install -y`, `dnf install -y` or `pacman -U --noconfirm` on the file. The system asks for the administrator password. |
| Nexus OS | `system` | Nothing downloads. The new version comes with the next system update. |

A build that is not installed (`cargo run`, for example) never swaps in an update. A package
manager or Nexus OS owns its files, so `apply_launcher_update` returns early for the methods
`package` and `system`.

## Limitations

- **The update check proves the download, not the publisher.** `latest.json` and the package
  come from the same release. The SHA-256 catches a damaged or swapped download, but a
  release that someone else controls would carry its own matching hash. The launcher does not
  check the Authenticode signature, the Apple signature or the build attestation.
- **Linux packages are not signed.** No GPG key signs the deb, rpm or Arch package, or the
  AppImage.
- **The Windows zip and the macOS zip are not signed as files.** Their contents are.
- **Notarization needs all the macOS secrets.** See above.
- **Not tested here.** This page was written from the workflow and the scripts. No build,
  signing or notarization ran for it. How the hardened runtime treats a library that Nexus
  loads from outside the app, such as a system libarchive, was not tested.
