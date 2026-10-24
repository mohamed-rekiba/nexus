#!/usr/bin/env bash
# Installs Nexus for this user from the unpacked Linux package this script is in. No sudo.
#
#   ./install.sh              install, or put a newer package in place of the installed one
#   ./install.sh --uninstall  remove Nexus (your settings, games and downloads stay)
#
# Where it goes (PREFIX is ~/.local unless you set it):
#   $PREFIX/lib/nexus         the package folder, whole: the launcher's own updates replace
#                             it, and it loads its libraries from lib/ in it
#   $PREFIX/bin/nexus         a link to the launcher in that folder
#   the app menu entry and the icon, in $XDG_DATA_HOME (~/.local/share)
#
# The settings are in ~/.local/share/nexus, which is not the package folder, so an update or
# --uninstall never touches them.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
prefix="${PREFIX:-$HOME/.local}"
folder="$prefix/lib/nexus"
bin="$prefix/bin"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
apps="$share/applications"
icons="$share/icons/hicolor"
autostart="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"

say() { printf '%s\n' "$*"; }
die() {
    printf 'install.sh: %s\n' "$*" >&2
    exit 1
}

# A path for the Exec line of a desktop entry: in double quotes, with ", `, $ and \ escaped,
# and each \ doubled again, as the Desktop Entry specification says.
exec_path() {
    printf '"%s"' "$(printf '%s' "$1" | sed -e 's/[\\"`$]/\\&/g' -e 's/\\/\\\\/g')"
}

refresh_menu() {
    if command -v update-desktop-database > /dev/null; then
        update-desktop-database "$apps" 2> /dev/null || true
    fi
    if command -v gtk-update-icon-cache > /dev/null; then
        gtk-update-icon-cache -q -f -t "$icons" 2> /dev/null || true
    fi
}

uninstall() {
    rm -f "$bin/nexus" "$apps/nexus.desktop" "$icons/512x512/apps/nexus.png" "$autostart/nexus.desktop"
    rm -rf "$folder" "$folder.old" "$folder.update"
    refresh_menu
    say "Nexus is removed. Your settings are still in ${XDG_DATA_HOME:-$HOME/.local/share}/nexus."
}

case "${1:-}" in
    "") ;;
    --uninstall)
        uninstall
        exit 0
        ;;
    -h | --help)
        sed -n '2,15p' "$0"
        exit 0
        ;;
    *) die "unknown option $1 (see --help)" ;;
esac

[ "$(uname -s)" = Linux ] || die "this script installs the Linux package; it runs on Linux only"
[ -x "$here/nexus" ] && [ -d "$here/extensions" ] || die "run it from the unpacked package: $here has no nexus and extensions/"
"$here/nexus" --help > /dev/null || die "$here/nexus does not run on this system"

# The package folder goes in whole. A copy goes in next to the old one first, so a failed copy
# leaves the old install as it was.
if [ "$here" != "$folder" ]; then
    mkdir -p "$(dirname "$folder")"
    rm -rf "$folder.new"
    cp -R "$here" "$folder.new"
    rm -rf "$folder"
    mv "$folder.new" "$folder"
fi
mkdir -p "$bin" "$apps" "$icons/512x512/apps"
ln -sfn "$folder/nexus" "$bin/nexus"
cp "$folder/nexus.png" "$icons/512x512/apps/nexus.png"
cat > "$apps/nexus.desktop" << DESKTOP
[Desktop Entry]
Type=Application
Name=Nexus
GenericName=Game Launcher
Comment=Games from every emulator, in one place
Exec=$(exec_path "$folder/nexus")
Icon=nexus
Terminal=false
Categories=Game;Emulator;
Keywords=games;launcher;emulator;playstation;ps4;ps5;shadps4;kyty;
StartupWMClass=nexus
DESKTOP
refresh_menu

say "Nexus is installed in $folder."
say "Start it from the app menu, or run: nexus"
case ":$PATH:" in
    *":$bin:"*) ;;
    *) say "$bin is not in your PATH: run $bin/nexus, or add the folder to PATH." ;;
esac
