#!/usr/bin/env bash
# Joins two Nexus.app bundles, one built on Apple Silicon and one on Intel, into one universal
# app: every Mach-O file found in both is joined with lipo; a file found in one only (a library
# only that build's libmpv needed) is copied as it is, and only that processor loads it. Signed
# ad hoc afterwards; a release signs it again with the Developer ID.
#   packaging/macos/universal.sh <arm64 Nexus.app> <x86_64 Nexus.app> <out Nexus.app>
set -euo pipefail
arm="${1:?usage: $0 <arm64 app> <x86_64 app> <out app>}"
intel="${2:?x86_64 app}"
out="${3:?out app}"
rm -rf "$out"
cp -R "$arm" "$out"
while IFS= read -r -d '' file; do
    rel="${file#"$out"/}"
    file -b "$file" | grep -q '^Mach-O' || continue
    if [ -f "$intel/$rel" ]; then
        lipo -create "$file" "$intel/$rel" -output "$file.universal"
        mv "$file.universal" "$file"
    else
        echo "only in the arm64 build: $rel"
    fi
done < <(find "$out" -type f -print0)
(cd "$intel" && find . -type f -print0) | while IFS= read -r -d '' rel; do
    rel="${rel#./}"
    if [ ! -e "$out/$rel" ]; then
        echo "only in the x86_64 build: $rel"
        mkdir -p "$(dirname "$out/$rel")"
        cp -p "$intel/$rel" "$out/$rel"
    fi
done
codesign --force --deep --sign - "$out"
for need in arm64 x86_64; do
    lipo -archs "$out/Contents/MacOS/nexus" | grep -qw "$need" || { echo "the app has no $need" >&2; exit 1; }
done
echo "universal: $(lipo -archs "$out/Contents/MacOS/nexus")"
