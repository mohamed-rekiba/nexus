# Signing the release

`.github/workflows/release.yml` signs the macOS and Windows packages when their secrets are set.
Without them it writes a warning in the run and goes on. The secrets go in the GitHub repo under
Settings, Secrets and variables, Actions. How a release runs is in
[development.md](development.md#releases).

## macOS

`scripts/package.sh` signs `Nexus.app` ad hoc: Apple Silicon runs nothing unsigned. The
release's "Sign and notarize (macOS)" step then replaces that signature with a Developer ID one
when `MACOS_CERT_P12` is set:

| Secret | What it holds |
|---|---|
| `MACOS_CERT_P12` | The "Developer ID Application" certificate and its private key, as a `.p12`, in base64 |
| `MACOS_CERT_PASSWORD` | The `.p12`'s password |
| `APPLE_ID` | The Apple account that submits for notarization |
| `APPLE_TEAM_ID` | That account's team id |
| `APPLE_APP_PASSWORD` | An app-specific password for that account |

With `MACOS_CERT_P12` set, the step:

1. imports the certificate into a temporary keychain;
2. signs each `.dylib` in `Contents/Frameworks` (libmpv, libarchive and the libraries they need),
   then the bundle, with `codesign --timestamp --options runtime` (the hardened runtime);
3. zips the folder, sends the zip to `xcrun notarytool submit --wait`, staples the ticket to
   `Nexus.app` with `xcrun stapler staple`, and zips it again.

The step checks only `MACOS_CERT_P12`. Set all five together: with the certificate but without
the Apple account, notarization fails and the release fails with it.

The certificate needs a paid Apple Developer Program membership. It can be added at any time:
set the secrets, and the next release signs. The workflow doesn't change.

**Without the secrets** the app stays signed ad hoc and isn't notarized. It runs, but
Gatekeeper blocks the first start of a downloaded copy. The player gets past it once:

- macOS 15 and later: open `Nexus.app`, close the warning, then in System Settings, Privacy &
  Security, choose Open Anyway and confirm.
- macOS 12 to 14: Control-click `Nexus.app`, choose Open, and confirm.
- Any version: `xattr -dr com.apple.quarantine /path/to/Nexus.app`.

## Windows

The release's "Sign (Windows)" step signs every `.exe` and `.dll` in `dist/nexus-windows-*/`
(`nexus.exe`, `libmpv-2.dll`, and libarchive's MSYS2 DLLs) when `WINDOWS_CERT_PFX` is set:

| Secret | What it holds |
|---|---|
| `WINDOWS_CERT_PFX` | The code-signing certificate and its private key, as a `.pfx`, in base64 |
| `WINDOWS_CERT_PASSWORD` | The `.pfx`'s password |

It runs the Windows SDK's `signtool sign /fd sha256` with a timestamp from
`http://timestamp.digicert.com`, before the folder is zipped and its checksum written. A file that
fails to sign fails the release.

**Without the secrets** nothing is signed, and SmartScreen warns on the first start: the player
chooses More info, then Run anyway. A signature lets SmartScreen build a reputation for the
publisher over time; it doesn't stop the warning at once.

**A `.pfx` fits only an older certificate, or a test one.** Since June 2023 the certificate
authorities must keep the private key of a new code-signing certificate on a hardware token or in
a cloud HSM, so it usually can't be exported as a `.pfx`. A cloud signing service is the usual
route now. Each needs its own workflow step in place of the `signtool /f` one:

- **SignPath Foundation**: free signing for open-source projects that it accepts.
- **Microsoft's Artifact Signing** (formerly Trusted Signing): a monthly fee and an identity
  check. Which countries and individuals it accepts changes; check before choosing it.
- **DigiCert KeyLocker** or **SSL.com eSigner**: paid certificates with cloud-held keys.

**A self-signed certificate is for testing only.** It shows that the step works, but Windows
doesn't trust it, so SmartScreen still warns. To make one, in PowerShell:

```powershell
$cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject "CN=Nexus Test" -CertStoreLocation Cert:\CurrentUser\My
$pw = Read-Host -AsSecureString "Password"
Export-PfxCertificate -Cert $cert -FilePath nexus-test.pfx -Password $pw
[Convert]::ToBase64String([IO.File]::ReadAllBytes("nexus-test.pfx")) | Set-Clipboard
```

Then set `WINDOWS_CERT_PFX` to the base64 text on the clipboard, and `WINDOWS_CERT_PASSWORD` to
the password you typed.

## Linux

Nothing is signed. Each file in the release has a `.sha256` file beside it and signed build
provenance: `gh attestation verify <file> --repo <owner>/<repo>` checks where it was built.

## The launcher's own update

The update checks no signature. It reads `latest.json` from the newest published release,
downloads only from GitHub's release hosts, and refuses a package whose SHA-256 isn't the one in
`latest.json`. A signed and notarized `Nexus.app` stays signed after the update, because the
update swaps in the whole bundle from the release. See
[development.md](development.md#the-launchers-own-update).
