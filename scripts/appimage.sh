#!/usr/bin/env bash
# Makes dist/Nexus-<arch>.AppImage from the Linux package scripts/package.sh made: one file
# that runs on most distributions. linuxdeploy (LINUXDEPLOY, the AppImage of a pinned
# release) bundles libmpv and the libraries it needs, deciding itself which the system has.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${LINUXDEPLOY:?set LINUXDEPLOY to the linuxdeploy AppImage}"
package="$(find dist -maxdepth 1 -type d -name 'nexus-linux-*' | head -n 1)"
[ -n "$package" ] || { echo "no Linux package in dist/: run scripts/package.sh first" >&2; exit 1; }
arch="${package##*-}"

appdir="$(mktemp -d)/Nexus.AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/licenses/nexus" "$appdir/usr/share/doc/nexus"
cp "$package/nexus" "$appdir/usr/bin/"
mkdir -p "$appdir/usr/bin/extensions"
cp -R "$package/licenses/." "$appdir/usr/share/licenses/nexus/"
cp "$package/LICENSE" "$appdir/usr/share/licenses/nexus/"
cp "$package/README.md" "$appdir/usr/share/doc/nexus/"
libmpv="$(ldconfig -p | awk '/libmpv\.so\.[0-9]/ {print $NF; exit}')"

# Runners have no FUSE: run linuxdeploy from its extracted files.
export APPIMAGE_EXTRACT_AND_RUN=1
OUTPUT="dist/Nexus-$arch.AppImage" "$LINUXDEPLOY" \
    --appdir "$appdir" \
    --executable "$appdir/usr/bin/nexus" \
    --library "$libmpv" \
    --desktop-file "$package/nexus.desktop" \
    --icon-file "$package/nexus.png" \
    --output appimage
echo "made dist/Nexus-$arch.AppImage"
