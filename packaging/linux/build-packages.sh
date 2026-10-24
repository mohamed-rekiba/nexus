#!/usr/bin/env bash
# Builds the deb, rpm and Arch packages from the Linux folder scripts/package.sh made, with nfpm
# (NFPM, default `nfpm` on PATH), into a folder:
#   packaging/linux/build-packages.sh dist/nexus-linux-x86_64 0.2.0 dist/packages [deb rpm archlinux]
# Run it from the repository root.
set -euo pipefail
folder="${1:?usage: $0 <release folder> <version> <output folder> [formats…]}"
version="${2:?version}"
out="${3:?output folder}"
shift 3
formats=("$@")
[ ${#formats[@]} -gt 0 ] || formats=(deb rpm archlinux)
nfpm="${NFPM:-nfpm}"
mkdir -p "$out"
marker="$(mktemp -d)/packaged"
for format in "${formats[@]}"; do
    case "$format" in
        deb | rpm) echo "$format" > "$marker" ;;
        archlinux) echo arch > "$marker" ;;
        *) echo "unknown format $format" >&2; exit 1 ;;
    esac
    FOLDER="$folder" VERSION="$version" PACKAGED="$marker" \
        "$nfpm" package -f packaging/linux/nfpm.yaml -p "$format" -t "$out"
done
ls -l "$out"
