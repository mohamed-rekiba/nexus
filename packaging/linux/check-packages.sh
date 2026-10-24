#!/usr/bin/env bash
# Checks the packages build-packages.sh made in a folder: every file is in each package, the
# programs are executable, the packaging marker names the format, and the dependencies are
# listed. Any mix of deb, rpm and Arch packages is fine, but there must be at least one. Needs
# dpkg-deb (deb), rpm (rpm) and tar with zstd (Arch).
#   packaging/linux/check-packages.sh dist/packages
set -euo pipefail

dir="${1:?usage: $0 <folder with the packages>}"
shopt -s nullglob
debs=("$dir"/*.deb)
rpms=("$dir"/*.rpm)
arch=("$dir"/*.pkg.tar.zst)
[ $((${#debs[@]} + ${#rpms[@]} + ${#arch[@]})) -gt 0 ] || { echo "no deb, rpm or Arch package in $dir" >&2; exit 1; }
for list in debs rpms arch; do
    declare -n files="$list"
    [ ${#files[@]} -le 1 ] || { echo "more than one $list package in $dir" >&2; exit 1; }
done

want=(
    usr/lib/nexus/nexus
    usr/lib/nexus/packaged
    usr/lib/nexus/LICENSE
    usr/lib/nexus/extensions
    usr/lib/nexus/licenses
    usr/bin/nexus
    usr/bin/nexus-session
    usr/share/nexus/session.conf
    usr/share/applications/nexus.desktop
    usr/share/wayland-sessions/nexus.desktop
    usr/share/icons/hicolor/512x512/apps/nexus.png
)
# What Nexus loads by name, which each package must depend on.
deb_deps=(libasound2 libfontconfig1 libxkbcommon0 xdotool x11-utils)
rpm_deps=(alsa-lib fontconfig libxkbcommon xdotool xprop)
arch_deps=(alsa-lib fontconfig libxkbcommon xdotool xorg-xprop)
fail=0
# Each check reads a listing of "MODE PATH" lines, PATH without its leading / or ./ and
# trailing /, which normalise() makes.
normalise() { sed -E 's#^([^ ]+) (\./|/)?#\1 #; s#/$##; s# -> .*##'; }
files() { # <package kind> <listing>
    for f in "${want[@]}"; do
        grep -Eq "^[^ ]+ $f\$" <<<"$2" || { echo "$1 is missing /$f" >&2; fail=1; }
    done
    for f in usr/lib/nexus/nexus usr/bin/nexus-session; do
        grep -Eq "^-rwxr-xr-x $f\$" <<<"$2" || { echo "$1: /$f is not 0755" >&2; fail=1; }
    done
}
marker() { # <package kind> <marker text> <expected>
    [ "$(tr -d '[:space:]' <<<"$2")" = "$3" ] || { echo "$1: /usr/lib/nexus/packaged says '$2', not $3" >&2; fail=1; }
}
checked=()
work="$(mktemp -d)"

if [ ${#debs[@]} -eq 1 ]; then
    files deb "$(dpkg-deb --contents "${debs[0]}" | awk '{print $1, $6}' | normalise)"
    info=$(dpkg-deb --info "${debs[0]}")
    for dep in "${deb_deps[@]}"; do grep -q "$dep" <<<"$info" || { echo "deb doesn't depend on $dep" >&2; fail=1; }; done
    dpkg-deb -x "${debs[0]}" "$work/deb"
    marker deb "$(cat "$work/deb/usr/lib/nexus/packaged")" deb
    checked+=("$(basename "${debs[0]}")")
fi

if [ ${#rpms[@]} -eq 1 ]; then
    files rpm "$(rpm -qp --queryformat '[%{FILEMODES:perms} %{FILENAMES}\n]' "${rpms[0]}" | normalise)"
    req=$(rpm -qp --requires "${rpms[0]}")
    for dep in "${rpm_deps[@]}"; do grep -q "$dep" <<<"$req" || { echo "rpm doesn't require $dep" >&2; fail=1; }; done
    mkdir -p "$work/rpm"
    rpm2cpio "$(realpath "${rpms[0]}")" | (cd "$work/rpm" && cpio -idm --quiet ./usr/lib/nexus/packaged)
    marker rpm "$(cat "$work/rpm/usr/lib/nexus/packaged")" rpm
    checked+=("$(basename "${rpms[0]}")")
fi

if [ ${#arch[@]} -eq 1 ]; then
    files arch "$(tar --zstd -tvf "${arch[0]}" | awk '{print $1, $6}' | normalise)"
    info=$(tar --zstd -xOf "${arch[0]}" .PKGINFO)
    for dep in "${arch_deps[@]}"; do grep -q "^depend = $dep" <<<"$info" || { echo "arch doesn't depend on $dep" >&2; fail=1; }; done
    marker arch "$(tar --zstd -xOf "${arch[0]}" usr/lib/nexus/packaged)" arch
    checked+=("$(basename "${arch[0]}")")
fi

rm -rf "$work"
[ "$fail" = 0 ] && echo "packages look right: ${checked[*]}"
exit "$fail"
