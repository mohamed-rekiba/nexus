#!/usr/bin/env bash
# Makes each system's icon and cover, and the Default theme's "All games" icon and cover.
#
#   Emulator plugins   plugins/emulators/<plugin>/assets/icon.png (256 px) and cover.jpg
#   Default theme      plugins/themes/default/theme/images/all-games-icon.png and -cover.jpg
#
# The icons come from the SVGs in assets/system-art, each cropped to its rounded square. The
# covers come from the folder given as the first argument, one <system>-cover.png per system
# (ps5, ps4, switch, pc, steam), and are written as JPEG, quality 85, at their own size. The
# "All games" cover comes from the picture given as the second argument, cut to 16:9 from its
# left edge, where its name is.
#
# The results are kept in the repo, so a build needs no image tools; run this again after
# changing a picture. Needs ImageMagick 7 (magick).
#
# Usage: scripts/make-system-art.sh <covers-folder> <all-games-picture>
set -euo pipefail
covers="$(realpath "${1:?usage: $0 <covers-folder> <all-games-picture>}")"
all_games="$(realpath "${2:?usage: $0 <covers-folder> <all-games-picture>}")"
cd "$(dirname "$0")/.."

art=assets/system-art
theme=plugins/themes/default/theme/images

# A square SVG as a PNG `size` pixels wide.
icon() { magick -background none -density 600 "$1" -resize "$2x$2" -strip "$3"; }

# The system's art name, then its emulator plugin's folder.
for pair in ps5:kytyps5 ps4:shadps4 switch:lichii pc:pc steam:steam; do
    system="${pair%%:*}"
    out="plugins/emulators/${pair#*:}/assets"
    mkdir -p "$out"
    icon "$art/$system.svg" 256 "$out/icon.png"
    magick "$covers/$system-cover.png" -strip -quality 85 "$out/cover.jpg"
done

mkdir -p "$theme"
icon "$art/all-games.svg" 256 "$theme/all-games-icon.png"
# 16:9 at the picture's own height, from its left edge.
height="$(magick identify -format %h "$all_games")"
magick "$all_games" -gravity west -crop "$((height * 16 / 9))x$height+0+0" +repage -strip -quality 85 \
    "$theme/all-games-cover.jpg"
