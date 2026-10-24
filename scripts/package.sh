#!/usr/bin/env bash
# Builds a release of the launcher for this system into dist/nexus-<os>-<arch>/, with the
# README, the LICENSE and the licences of what ships with it:
#   macOS    Nexus.app: the binary, libmpv in Contents/Frameworks, the icon, extensions/ and
#            the licences in Contents/Resources, and an Info.plist. Signed ad hoc (a
#            Developer ID signature replaces it in the release). Packed as a .zip.
#   Linux    the binary, libmpv in lib/, extensions/, nexus.desktop, the icon and
#            install.sh, which installs the folder for the user. Packed as a .tar.gz, which
#            keeps the executable bit.
#   Windows  nexus.exe, libmpv-2.dll beside it, extensions/ and the icon. CI zips it.
# libmpv is for the trailer player (scripts/bundle-libmpv.sh); NEXUS_NO_LIBMPV=1 leaves it
# out. libarchive is for RAR and other archives (scripts/bundle-libarchive.sh), with its
# codecs; NEXUS_NO_LIBARCHIVE=1 leaves it out. extensions/ is for third-party plugins and themes, found next to the binary at start.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --locked -p nexus-app

case "$(uname -s)" in
    Linux) os=linux exe=nexus ;;
    Darwin) os=macos exe=nexus ;;
    MINGW* | MSYS* | CYGWIN*) os=windows exe=nexus.exe ;;
    *) echo "unknown system $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    arm64 | aarch64) arch=aarch64 ;;
    *) arch="$(uname -m)" ;;
esac
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)"
name="nexus-$os-$arch"
out="dist/$name"
rm -rf "$out"
mkdir -p "$out"
cp README.md LICENSE "$out/"

# Where each part goes on this system.
case "$os" in
    macos)
        app="$out/Nexus.app"
        bin="$app/Contents/MacOS"
        lib="$app/Contents/Frameworks"
        resources="$app/Contents/Resources"
        mkdir -p "$bin" "$lib" "$resources/extensions"
        cp assets/nexus.icns "$resources/"
        ;;
    linux)
        bin="$out" lib="$out/lib" resources="$out"
        mkdir -p "$out/extensions"
        cp assets/nexus.png "$out/nexus.png"
        cp scripts/install.sh "$out/install.sh"
        chmod +x "$out/install.sh"
        ;;
    windows)
        bin="$out" lib="$out" resources="$out"
        mkdir -p "$out/extensions"
        cp assets/nexus.ico "$out/nexus.ico"
        ;;
esac
cp "target/release/$exe" "$bin/"
licences="$resources/licenses"
mkdir -p "$licences"
# The fonts the default theme carries in the binary.
cp plugins/themes/default/theme/fonts/OFL-*.txt "$licences/"
# The Rust crates' licences (about.toml). CI must have cargo-about; a local package may go
# without, with a warning.
if command -v cargo-about > /dev/null; then
    cargo about generate --locked --workspace --fail -o "$licences/THIRD-PARTY.md" about.hbs
elif [ -n "${CI:-}" ]; then
    echo "cargo-about is missing: the package would ship without the crates' licences" >&2
    exit 1
else
    echo "warning: no cargo-about, so no THIRD-PARTY.md (cargo install --locked cargo-about)" >&2
fi

if [ "${NEXUS_NO_LIBMPV:-}" != 1 ]; then
    scripts/bundle-libmpv.sh "$os" "$lib" "$licences"
fi
if [ "${NEXUS_NO_LIBARCHIVE:-}" != 1 ]; then
    scripts/bundle-libarchive.sh "$os" "$lib" "$licences"
fi

case "$os" in
    macos)
        cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Nexus</string>
    <key>CFBundleDisplayName</key><string>Nexus</string>
    <key>CFBundleIdentifier</key><string>${NEXUS_BUNDLE_ID:-io.github.mohamed-rekiba.nexus}</string>
    <key>CFBundleExecutable</key><string>nexus</string>
    <key>CFBundleIconFile</key><string>nexus</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.games</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSAppleEventsUsageDescription</key><string>Nexus brings a game and itself to the front with System Events.</string>
</dict>
</plist>
PLIST
        # The whole bundle, ad hoc: Apple Silicon runs nothing unsigned.
        codesign --force --deep --sign - "$app"
        "$bin/$exe" --help > /dev/null
        (cd dist && ditto -c -k --keepParent "$name" "$name.zip")
        echo "packed dist/$name.zip"
        ;;
    linux)
        # The same app menu entry as the deb, rpm and Arch packages'.
        cp packaging/linux/nexus.desktop "$out/nexus.desktop"
        "$bin/$exe" --help > /dev/null
        tar -czf "dist/$name.tar.gz" -C dist "$name"
        echo "packed dist/$name.tar.gz"
        ;;
    windows)
        "$bin/$exe" --help > /dev/null
        ;;
esac
echo "packaged $out"
