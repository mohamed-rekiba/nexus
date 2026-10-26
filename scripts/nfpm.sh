#!/usr/bin/env bash
# Runs nfpm, the version CI pins (.github/actions/linux-packages), downloaded once into
# target/nfpm/ and checked against its SHA-256: `scripts/nfpm.sh package …`. An nfpm on the PATH
# of that version is used as it is.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
action=$root/.github/actions/linux-packages/action.yml
version=$(sed -n 's/^ *NFPM_VERSION: //p' "$action")
sha=$(sed -n 's/^ *NFPM_SHA256: //p' "$action")
if command -v nfpm > /dev/null && nfpm --version 2>/dev/null | grep -q "$version"; then
    exec nfpm "$@"
fi
nfpm=${CARGO_TARGET_DIR:-$root/target}/nfpm/$version/nfpm
if [ ! -x "$nfpm" ]; then
    mkdir -p "$(dirname "$nfpm")"
    archive="$(dirname "$nfpm")/nfpm.tar.gz"
    curl -fsSL --retry 3 -o "$archive" \
        "https://github.com/goreleaser/nfpm/releases/download/v${version}/nfpm_${version}_Linux_x86_64.tar.gz"
    echo "$sha  $archive" | sha256sum -c - > /dev/null
    tar -xzf "$archive" -C "$(dirname "$nfpm")" nfpm
    rm "$archive"
fi
exec "$nfpm" "$@"
