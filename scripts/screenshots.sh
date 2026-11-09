#!/usr/bin/env bash
# Takes the README and user-guide screenshots from the running app, with your real library.
#   scripts/screenshots.sh          builds Nexus, then writes docs/screenshots/*.jpg
#
# It opens a Nexus window of 1600 × 900 for about a minute, in the Default theme (the
# --theme flag doesn't save the choice), and drives it with keys. Don't touch the keyboard
# or the mouse while it runs: the keys go to the window in front. Close any other Nexus
# first. Nexus reads and writes its usual data folder, as when you start it yourself; the
# script presses no key that installs, deletes or restores anything.
#
# Linux only. It needs xdotool and ImageMagick's `import`, and runs Nexus on X11 (XWayland
# on a Wayland desktop), where xdotool can find and drive the window.
#
# The pictures:
#   home.jpg  library.jpg  detail.jpg (Game Hub)  catalog.jpg  downloads.jpg
#   settings.jpg  backup.jpg (Settings → Backup)  backup-browser.jpg
set -euo pipefail
cd "$(dirname "$0")/.."

for tool in xdotool import; do
    command -v "$tool" >/dev/null || { echo "$tool is missing" >&2; exit 1; }
done

out=docs/screenshots
mkdir -p "$out"
cargo build --release -p nexus-app
bin=target/release/nexus

# X11 even on a Wayland desktop: without WAYLAND_DISPLAY, winit opens an XWayland window.
env -u WAYLAND_DISPLAY "$bin" --windowed --theme default &
pid=$!
trap 'kill "$pid" 2>/dev/null || true' EXIT

window=$(xdotool search --sync --onlyvisible --pid "$pid" | head -n 1)
xdotool windowsize --sync "$window" 1600 900
xdotool windowactivate --sync "$window"

# The library, its art and the catalogs load.
sleep 15

# Presses each key, then waits for the screen to settle.
press() {
    for key in "$@"; do
        xdotool key --clearmodifiers "$key"
        sleep 0.4
    done
    sleep 1.5
}

shoot() {
    import -window "$window" -quality 90 "$out/$1.jpg"
    echo "$out/$1.jpg"
}

shoot home
# Enter on the first entry, All games: the Library.
press Return
shoot library
# Enter on the first game: its Game Hub. Its art and details come from the network.
press Return
sleep 3
shoot detail
press Escape Escape
# Ctrl+D opens Downloads; Shift+Tab is the screen before, the Catalog.
press ctrl+d
shoot downloads
press shift+Tab
sleep 2
shoot catalog
# Tab on through Downloads, Emulators and Themes to Settings.
press Tab Tab Tab Tab
shoot settings
# Up to the tabs, and along them to Backup.
press Up Right Right Right Right Right
shoot backup
# Down to "Restore from another folder": below Back up now, Include large items and the
# backup folder. Enter opens the browser.
press Down Down Down Down Return
shoot backup-browser
# The places, where Escape closes it.
press Left Escape

echo "done: check each picture before you commit it"
