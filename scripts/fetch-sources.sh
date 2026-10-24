#!/usr/bin/env bash
# Downloads the source of the libraries a package ships (the SOURCES lists that
# scripts/bundle-libmpv.sh and scripts/bundle-libarchive.sh write) into one archive beside it: `scripts/fetch-sources.sh <package folder>` writes
# dist/<package>-third-party-sources.tar. The release attaches it, so the source travels with
# the binaries.
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
        curl -fsSL --retry 3 -o "$work/$name-$(basename "$url")" "$url"
    done < "$list"
    cat "$list" >> "$work/SOURCES"
done
out="dist/$(basename "$package")-third-party-sources.tar"
tar -cf "$out" -C "$work" .
echo "wrote $out"
