#!/usr/bin/env bash
# A disk image with Nexus.app and a shortcut to Applications, to drag it into:
#   packaging/macos/make-dmg.sh <Nexus.app> <out .dmg>
# The app inside is signed (and stapled) as it is; a release signs and notarizes the image too.
set -euo pipefail
app="${1:?usage: $0 <Nexus.app> <out .dmg>}"
out="${2:?out .dmg}"
staging="$(mktemp -d)/Nexus"
mkdir -p "$staging"
ditto "$app" "$staging/Nexus.app"
ln -s /Applications "$staging/Applications"
rm -f "$out"
hdiutil create -volname Nexus -srcfolder "$staging" -fs HFS+ -format UDZO -ov "$out"
rm -rf "$(dirname "$staging")"
echo "made $out"
