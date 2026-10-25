#!/usr/bin/env bash
# Builds the minimal Nexus OS image (os/Containerfile) with podman as root, from the repository
# root. os.yml and `make os-image` run it:
#   os/build-image.sh [TAG]      default localhost/nexus-os:minimal
# It installs the rpm in os/nexus-rpm/: os/fetch-nexus.sh puts the latest release's there, or
# copy a local one in.
# The minimal edition builds its own base (bootc-base-imagectl needs these privileges), then is
# split into one layer per package with every timestamp zeroed: a daily rebuild then only
# changes the layers whose packages changed, and that's all a PC downloads.
# Settings, from the environment:
#   BASE    the Fedora bootc image it starts from (default quay.io/fedora/fedora-bootc:44)
#   IMAGE   where installed PCs update from (default ghcr.io/mohamed-rekiba/nexus-os)
#   REPO    the GitHub repository of Nexus's releases (default mohamed-rekiba/nexus)
set -euo pipefail
tag=${1:-localhost/nexus-os:minimal}
base=${BASE:-quay.io/fedora/fedora-bootc:44}
podman=(podman)
[ "$(id -u)" = 0 ] || podman=(sudo podman)

ls os/nexus-rpm/nexus-*.x86_64.rpm > /dev/null || {
    echo "error: no rpm in os/nexus-rpm/: run os/fetch-nexus.sh, or copy one there" >&2
    exit 1
}
"${podman[@]}" build --pull=always --cap-add=all --security-opt=label=disable --device /dev/fuse \
    --build-arg BASE_IMAGE="$base" --build-arg OS_VERSION="$(cat os/version.txt)" \
    --build-arg BUILD_DATE="$(date -u +%Y.%m.%d)" \
    --build-arg IMAGE="${IMAGE:-ghcr.io/mohamed-rekiba/nexus-os}" \
    --build-arg REPO="${REPO:-mohamed-rekiba/nexus}" \
    -f os/Containerfile -t "$tag-build" .
"${podman[@]}" run --rm --privileged -v /var/lib/containers:/var/lib/containers "$base" \
    /usr/libexec/bootc-base-imagectl rechunk "$tag-build" "$tag"
"${podman[@]}" rmi "$tag-build"
