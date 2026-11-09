# Signing the release

This page is for maintainers who release Nexus. It lists what the release signs, the secrets
it reads, the checksums and `latest.json` it publishes, and how the launcher checks an update.
Building packages is in [development.md](development.md#packaging). The release steps are in
[development.md](development.md#commits-and-releases).

Signing is optional. Without the secrets, `package.yml` still builds every package, prints a
warning when `MACOS_CERT_P12` or `WINDOWS_CERT_PFX` is missing (the other secrets get no
warning), and the packages carry only the signing the build does by
itself: ad hoc on macOS, none on Windows.

## What is signed

| Package | Signed with | When |
| --- | --- | --- |
| `Nexus.app` and `nexus-macos-universal.dmg` | Apple Developer ID, notarized and stapled | `MACOS_CERT_P12` is set |
| `nexus-macos-universal.zip` | Not signed itself: it holds the signed, stapled app | `MACOS_CERT_P12` is set |
| `Nexus.app` without the secrets | Ad hoc (`codesign --sign -`) | Always: Apple Silicon runs nothing unsigned |
| `nexus.exe` and the `.dll` files | Authenticode | `WINDOWS_CERT_PFX` is set |
| `nexus-windows-x86_64-setup.exe` | Authenticode | `WINDOWS_CERT_PFX` is set |
| Linux tar.gz, AppImage, deb, rpm, Arch package | Nothing | Never |
| Every release file | A `.sha256` file and a build provenance attestation | Release tags only |

## Secrets

Set them in the repository's Actions secrets. Never print or commit their values.

| Secret | Holds | Read by |
| --- | --- | --- |
| `MACOS_CERT_P12` | The Developer ID Application certificate as a `.p12` file, in base64 | `macos` job |
| `MACOS_CERT_PASSWORD` | The password of the `.p12` file | `macos` job |
| `APPLE_ID` | The Apple ID that notarizes | `macos` job |
| `APPLE_TEAM_ID` | The Apple team id | `macos` job |
| `APPLE_APP_PASSWORD` | An app-specific password for that Apple ID | `macos` job |
| `WINDOWS_CERT_PFX` | The code-signing certificate as a `.pfx` file, in base64 | `scripts/sign-windows.ps1` |
| `WINDOWS_CERT_PASSWORD` | The password of the `.pfx` file | `scripts/sign-windows.ps1` |
| `RELEASE_PLEASE_TOKEN` | Optional: a personal access token or a GitHub App's token, so the release pull requests start CI | `release.yml` |

Set the five macOS secrets together. The workflow tests only `MACOS_CERT_P12` before it signs
and notarizes.

## The workflow

`.github/workflows/package.yml` ("Package") runs by hand (`workflow_dispatch`). `release.yml`
starts it on each Nexus tag. On a branch it only builds, and the files stay as the run's
artifacts: use that to try a signing change.

| Job | Permissions | Does |
| --- | --- | --- |
| `version` | `contents: read` | Reads the version from `Cargo.toml`. On a tag, fails when the tag is not `v<version>`. |
| `build` | `contents: read` | Linux and Windows. Builds, packs, and signs the Windows files. |
| `build-mac` | `contents: read` | Apple Silicon (`macos-15`) and Intel (`macos-15-intel`). Builds and packs each app as a `mac-*` artifact. |
| `macos` | `contents: read` | Joins the two apps into one universal app, signs and notarizes it, makes the disk image. |
| `sources` | `contents: read` | Fetches the third-party sources the builds list, one tar per package. |
| `publish` | `contents: write`, `id-token: write`, `attestations: write` | On a tag only. Checksums, `latest.json`, attestations, upload, publish. |
| `os` | `contents: write`, `actions: write` | After `publish`. Names the release in `os/nexus.txt` and starts `os.yml`. |

`publish` downloads only the `build-*` artifacts. The per-processor Mac zips are `mac-*`
artifacts, so they never reach the release.

## macOS

`scripts/package.sh` signs each `Nexus.app` ad hoc: `codesign --force --deep --sign -`. The
bundle scripts sign the libraries they copy ad hoc too. `packaging/macos/universal.sh` joins the
Apple Silicon and the Intel app with `lipo` and signs the result ad hoc again. Without the
secrets, the release ships that app, and Gatekeeper asks before its first start.

With `MACOS_CERT_P12` set, the step "Sign and notarize the app" in the `macos` job does this:

1. Makes a keychain in `$RUNNER_TEMP`, decodes the certificate, imports it for `codesign`, and
   deletes the decoded file.
2. Signs every `.dylib` in `Contents/Frameworks`, then the app, with
   `codesign --force --timestamp --options runtime --sign "Developer ID Application"`.
   Notarization asks for the hardened runtime and a timestamp.
3. Zips the app with `ditto` and sends it with `xcrun notarytool submit … --wait`.
4. Staples the ticket to the app with `xcrun stapler staple`.
5. Zips the stapled app again as `nexus-macos-universal.zip`.

The step "Disk image" runs `packaging/macos/make-dmg.sh`: a disk image with `Nexus.app` and a
link to `/Applications`. With `MACOS_CERT_P12` set, it signs, notarizes and staples the image.
Then it checks the image with `hdiutil verify`, mounts it, and tests that the app's binary and
the link are there.

The workflow passes no entitlements file to `codesign`.

## Windows

`scripts/sign-windows.ps1 <file or wildcard>…` signs files with Authenticode. The `build` job
runs it twice:

1. Before packing: `dist\nexus-windows-*\*.exe` and `dist\nexus-windows-*\*.dll`. The zip
   and the installer are then built from the signed files.
2. After `scripts/windows-installer.sh`: `dist\nexus-windows-x86_64-setup.exe`.

The zip itself is not signed.

Without `WINDOWS_CERT_PFX`, the script prints a warning and signs nothing. SmartScreen then
warns on the first start.

With it, the script writes the certificate to `$RUNNER_TEMP\cert.pfx`, takes the newest
`signtool.exe` under `Windows Kits\10\bin`, and signs each file with `/fd sha256` and DigiCert's
timestamp server (`/tr http://timestamp.digicert.com /td sha256`). A failed file stops the
script with signtool's exit code. At the end it deletes the certificate file.

The job then tries the signed installer: a silent install, a check for the file
`installed-by-setup` and `nexus.exe --help`, then a silent uninstall. The installer's script is
`packaging/windows/nexus.iss`.

## Linux

Nothing is signed. The `build` job makes the tar.gz (`scripts/package.sh`), the AppImage
(`scripts/appimage.sh`) and the deb, rpm and Arch packages (the `linux-packages` action, with
nfpm). `packaging/linux/nfpm.yaml` sets no signature.

The tools CI downloads are pinned and checked against a SHA-256: nfpm (`linux-packages`
action), linuxdeploy and the Windows mpv build (`libmpv` action). The Windows libarchive comes
from the runner's MSYS2, which the workflow does not pin.

## Checksums, attestations and `latest.json`

The `publish` job runs on a tag only. For each release file (`*.tar.gz`, `*.zip`, `*.tar`,
`*.AppImage`, `*.deb`, `*.rpm`, `*.pkg.tar.zst`, `*.exe`, `*.dmg`) it:

1. Writes `<file>.sha256` with `sha256sum`.
2. Attests the file with `actions/attest-build-provenance`. This is a signed record of the
   workflow run that built the file, not a code-signing certificate.

Then it writes `latest.json` with `scripts/update-manifest.sh`, uploads every file with
`gh release upload --clobber`, and publishes the draft with
`gh release edit --draft=false --latest`. The launcher reads the latest release, so it never
sees a release without its files.

### `latest.json`

`scripts/update-manifest.sh <folder> <tag> <owner/repo>` writes the version (the tag without
its `v`) and one entry per kind of install. Each entry has a `url` into the release and the
`sha256` from the file's `.sha256`.

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

The third-party source tars and the disk image get no entry.

`latest.json` itself is not signed.

## How the launcher checks an update

The code is `crates/nexus-core/src/launcher_update.rs`. `crates/nexus-app/src/update.rs`
applies a kept update at start.

1. The repository is `app.update_repo`, `owner/name`, default `mohamed-rekiba/nexus`
   (`DEFAULT_REPO`). An empty value turns the checks off.
2. The core reads `https://github.com/<repo>/releases/latest/download/latest.json`. `os.yml`
   publishes Nexus OS releases with `--latest=false`, so the latest release stays a Nexus one.
3. It offers the update only when the file's version is newer, as semver.
4. It takes the entry for this install's key (`package_key`). `Install::detect` decides it:
   the AppImage (`APPIMAGE` is set), a deb, rpm or Arch package (the file `packaged` beside the
   binary), the Windows installer (`installed-by-setup`), `Nexus.app`, the unpacked folder, or
   Nexus OS. With no entry for the key, it shows "Nexus `<version>` has no package for this
   system".
5. It downloads only over HTTPS from `github.com`, `objects.githubusercontent.com` and
   `release-assets.githubusercontent.com` (`HOSTS`), with at most 5 redirects
   (`MAX_REDIRECTS`). Any other host stops the download.
6. It computes the file's SHA-256 and compares it with `sha256` in `latest.json`. A mismatch
   fails the update.

How the update goes in (`Install::method`):

| Install | Method | What happens |
| --- | --- | --- |
| tar.gz folder, AppImage, `Nexus.app`, Windows zip | `swap` | Kept in `updates/` in the data folder. At the next start `apply_staged` renames the old install aside, puts the new one in, and starts it. When anything fails, the old one comes back. It tries once. |
| Windows installer | `installer` | At the next start Nexus runs the new installer with `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`, and the installer starts the new version. |
| deb, rpm, Arch | `package` | Nexus runs `pkexec` with `apt-get install -y`, `dnf install -y` or `pacman -U --noconfirm` on the file. The system asks for the administrator password. |
| Nexus OS | `system` | Nothing downloads. The new Nexus comes with the next system update ([nexus-os.md](nexus-os.md)). |

A build that is not installed, such as `cargo run`'s, never swaps an update in.

## Limitations

- **The update check proves the download, not the publisher.** `latest.json` and the packages
  come from the same release. The SHA-256 catches a damaged download, but whoever can publish a
  release can publish a matching hash. The launcher does not check the Authenticode signature,
  the Apple signature or the attestation.
- **Linux packages are not signed.** No GPG key signs the deb, rpm, Arch package or AppImage.
- **The Windows zip and the macOS zip are not signed as files.** The files inside them are.
- **Not tested for this page.** It was written from the workflow and the scripts. No build,
  signing or notarization ran for it.
