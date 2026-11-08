#!/usr/bin/env bash
# Makes each system's icon and cover, and the Systems theme's "All games" art.
#
#   Emulator plugins   plugins/emulators/<plugin>/assets/icon.png (256 px) and cover.jpg
#   Systems theme      plugins/themes/systems/theme/images/all-games-icon.png and -cover.jpg
#
# The icons come from the SVGs in assets/system-art. The covers come from the folder given as
# the argument, one <system>-cover.png per system (ps5, ps4, switch, pc, steam), and are written
# as JPEG, quality 85, at their own size. The "All games" cover is drawn from
# assets/system-art/all-games-cover.svg with the five icons on it.
#
# The results are kept in the repo, so a build needs no image tools; run this again after
# changing a picture. Needs ImageMagick 7 (magick) and the default theme's Inter font.
#
# Usage: scripts/make-system-art.sh <covers-folder>
set -euo pipefail
covers="$(realpath "${1:?usage: $0 <covers-folder>}")"
cd "$(dirname "$0")/.."

art=assets/system-art
font=plugins/themes/default/theme/fonts/Inter-Bold.ttf
theme=plugins/themes/systems/theme/images
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# A square SVG as a PNG `size` pixels wide.
icon() { magick -background none -density 600 "$1" -resize "$2x$2" "$3"; }

# The system's art name, then its emulator plugin's folder.
for pair in ps5:kytyps5 ps4:shadps4 switch:lichii pc:pc steam:steam; do
    system="${pair%%:*}"
    out="plugins/emulators/${pair#*:}/assets"
    mkdir -p "$out"
    icon "$art/$system.svg" 256 "$out/icon.png"
    magick "$covers/$system-cover.png" -strip -quality 85 "$out/cover.jpg"
    icon "$art/$system.svg" 190 "$work/$system.png"
done

mkdir -p "$theme"
icon "$art/all-games.svg" 256 "$theme/all-games-icon.png"
icon "$art/all-games.svg" 64 "$work/mark.png"
# The five icons in two rows in the middle, and the mark and its name at the top left, where the
# systems' covers have their logos.
magick -background none "$art/all-games-cover.svg" \
    "$work/ps5.png" -geometry +506+250 -composite \
    "$work/ps4.png" -geometry +741+250 -composite \
    "$work/switch.png" -geometry +976+250 -composite \
    "$work/pc.png" -geometry +623+485 -composite \
    "$work/steam.png" -geometry +858+485 -composite \
    "$work/mark.png" -geometry +44+34 -composite \
    -font "$font" -fill white -pointsize 34 -annotate +122+62 "ALL" -annotate +122+100 "GAMES" \
    -strip -quality 85 "$theme/all-games-cover.jpg"
