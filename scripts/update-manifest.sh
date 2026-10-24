#!/usr/bin/env bash
# Writes the release's latest.json, which the launcher reads to update itself:
# `scripts/update-manifest.sh <folder of release files> <tag> <owner/repo>`. Each package's
# SHA-256 comes from its .sha256 file. Keys, one per kind of install (the core's package_key):
#   linux-<arch>            nexus-linux-<arch>.tar.gz
#   linux-<arch>-appimage   Nexus-<arch>.AppImage
#   linux-x86_64-deb        nexus_<version>-1_amd64.deb
#   linux-x86_64-rpm        nexus-<version>-1.x86_64.rpm
#   linux-x86_64-arch       nexus-<version>-1-x86_64.pkg.tar.zst
#   macos-aarch64, macos-x86_64   nexus-macos-universal.zip (one app for both)
#   windows-<arch>          nexus-windows-<arch>.zip
#   windows-<arch>-setup    nexus-windows-<arch>-setup.exe
set -euo pipefail
dir="$1"
tag="$2"
repo="$3"
version="${tag#v}"
entry() { # <key> <file> <sha256>
    [ "$first" = 1 ] || printf ','
    first=0
    printf '\n    "%s": { "url": "https://github.com/%s/releases/download/%s/%s", "sha256": "%s" }' "$1" "$repo" "$tag" "$2" "$3"
}
{
    printf '{\n  "version": "%s",\n  "assets": {' "$version"
    first=1
    for sum in "$dir"/*.sha256; do
        file="$(basename "${sum%.sha256}")"
        sha="$(awk '{print $1}' "$sum")"
        case "$file" in
            nexus-*-third-party-sources.tar) continue ;;
            nexus-macos-universal.zip) entry macos-aarch64 "$file" "$sha"; entry macos-x86_64 "$file" "$sha" ;;
            nexus-*-setup.exe) key="${file#nexus-}"; entry "${key%.exe}" "$file" "$sha" ;;
            nexus-*.tar.gz) key="${file#nexus-}"; entry "${key%.tar.gz}" "$file" "$sha" ;;
            nexus-*.zip) key="${file#nexus-}"; entry "${key%.zip}" "$file" "$sha" ;;
            Nexus-*.AppImage) key="linux-${file#Nexus-}"; entry "${key%.AppImage}-appimage" "$file" "$sha" ;;
            nexus_*_amd64.deb) entry linux-x86_64-deb "$file" "$sha" ;;
            nexus-*.x86_64.rpm) entry linux-x86_64-rpm "$file" "$sha" ;;
            nexus-*-x86_64.pkg.tar.zst) entry linux-x86_64-arch "$file" "$sha" ;;
            *) continue ;;
        esac
    done
    printf '\n  }\n}\n'
}
