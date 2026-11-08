#!/usr/bin/env bash
# Downloads the source of the libraries a package ships (the SOURCES lists that
# scripts/bundle-libmpv.sh and scripts/bundle-libarchive.sh write) into one archive beside it: `scripts/fetch-sources.sh <package folder>` writes
# dist/<package>-third-party-sources.tar. The release attaches it, so the source travels with
# the binaries.
#
# With NEXUS_SOURCES_CACHE set to a folder, each archive is kept there and taken from there
# the next time: CI caches that folder, so a release downloads only what changed.
set -euo pipefail
package="$1"
lists="$(find "$package" -path '*licenses/*/SOURCES')"
if [ -z "$lists" ]; then
    echo "no SOURCES in $package: nothing to fetch" >&2
    exit 1
fi
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
for list in $lists; do
    while read -r name url; do
        [ -n "$name" ] || continue
        file="$name-$(basename "$url")"
        # A branch (master) moves, so only a pinned version comes from the cache.
        if [ -n "${NEXUS_SOURCES_CACHE:-}" ] && [[ "$url" != *master* ]]; then
            if [ ! -s "$NEXUS_SOURCES_CACHE/$file" ]; then
                "$(dirname "$0")/download.sh" "$url" "$NEXUS_SOURCES_CACHE/$file.part"
                mv "$NEXUS_SOURCES_CACHE/$file.part" "$NEXUS_SOURCES_CACHE/$file"
            fi
            cp "$NEXUS_SOURCES_CACHE/$file" "$work/$file"
        else
            "$(dirname "$0")/download.sh" "$url" "$work/$file"
        fi
    done < "$list"
    cat "$list" >> "$work/SOURCES"
done
out="dist/$(basename "$package")-third-party-sources.tar"
tar -cf "$out" -C "$work" .
echo "wrote $out"
