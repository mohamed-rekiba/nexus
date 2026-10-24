#!/usr/bin/env bash
# Downloads the Nexus rpm that the image installs, from a Nexus release (the latest by default),
# checks it against its .sha256, and leaves it alone in os/nexus-rpm/ for os/Containerfile:
#   os/fetch-nexus.sh [TAG]
# Needs gh, logged in or with GH_TOKEN set. GH_REPO picks the repository (default: this one).
# Nexus OS releases are never "latest", so the latest release is always a Nexus release.
# Run it from the repository root.
set -euo pipefail
tag=${1:-}
dir=os/nexus-rpm
rm -rf "$dir"
mkdir -p "$dir"
# shellcheck disable=SC2086 # no tag means the latest release
gh release download $tag --pattern 'nexus-*.x86_64.rpm' --pattern 'nexus-*.x86_64.rpm.sha256' --dir "$dir"
rpms=("$dir"/nexus-*.x86_64.rpm)
if [ "${#rpms[@]}" -ne 1 ] || [ ! -f "${rpms[0]}" ]; then
    echo "error: expected one nexus-*.x86_64.rpm in the release, found: ${rpms[*]}" >&2
    exit 1
fi
(cd "$dir" && sha256sum -c "$(basename "${rpms[0]}").sha256")
rm "$dir"/*.sha256
ls -l "$dir"
