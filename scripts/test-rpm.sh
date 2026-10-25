#!/usr/bin/env bash
# A quick rpm of this checkout, to try a change on this Linux PC (the Nexus session, say):
#   scripts/test-rpm.sh            then: sudo dnf install dist/test-rpm/nexus-*.rpm
# It builds the launcher, and takes the rest of the package (libmpv, libarchive, the licences,
# extensions/) from the latest release's Linux folder, so it needs none of package.sh's tools.
# Its version is this one with ^local.<UTC time> (0.2.6^local.202610241720): rpm puts it above
# the release it's built on and below the next one, so it installs over the first and the next
# release replaces it. The binary links against this system's libraries: for this PC only.
# Needs cargo and gh, logged in or with GH_TOKEN set. nfpm comes from scripts/nfpm.sh (NFPM picks
# another). TAG picks the release (default the latest).
set -euo pipefail
cd "$(dirname "$0")/.."

out=dist/test-rpm
target=${CARGO_TARGET_DIR:-target}
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)^local.$(date -u +%Y%m%d%H%M)"

cargo build --release --locked -p nexus-app

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# shellcheck disable=SC2086 # no tag means the latest release
gh release download ${TAG:-} --pattern nexus-linux-x86_64.tar.gz --dir "$work"
tar -xzf "$work/nexus-linux-x86_64.tar.gz" -C "$work"
cp "$target/release/nexus" "$work/nexus-linux-x86_64/nexus"

rm -rf "$out"
NFPM=${NFPM:-scripts/nfpm.sh} packaging/linux/build-packages.sh "$work/nexus-linux-x86_64" "$version" "$out" rpm
echo "sudo dnf install $out/nexus-$version-1.x86_64.rpm"
