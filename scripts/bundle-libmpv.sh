#!/usr/bin/env bash
# Copies libmpv, the libraries it needs, and their licences into a package, for the trailer
# player: `scripts/bundle-libmpv.sh <os> <library folder> <licence folder>`. The app looks
# for libmpv there first: lib/ beside the binary, Contents/Frameworks in the macOS bundle,
# the binary's own folder on Windows.
#
# Needs, per system (the libmpv action in .github installs them):
#   macOS    Homebrew's mpv and dylibbundler
#   Linux    libmpv-dev, mpv and patchelf
#   Windows  LIBMPV_DIR: an unpacked shinchiro mpv-dev build (libmpv-2.dll)
set -euo pipefail
os="$1"
lib_dir="$2"
licences="$3/libmpv"
mkdir -p "$lib_dir" "$licences"

# mpv's licence texts, from its source at the version shipped, and where that source is.
# A version is a tag (v0.34.1): newer mpv prints it with its v, older mpv without.
fetch_licences() {
    local ref="$1"
    if [[ "$ref" =~ ^[0-9]+\.[0-9] ]]; then ref="v$ref"; fi
    for file in Copyright LICENSE.GPL LICENSE.LGPL; do
        curl -fsSL --retry 3 -o "$licences/$file" "https://raw.githubusercontent.com/mpv-player/mpv/$ref/$file"
    done
    echo "mpv https://github.com/mpv-player/mpv/archive/$ref.tar.gz" >> "$licences/SOURCES"
}

# FFmpeg's, the same way, into ffmpeg/: its tags are n<version> (n7.1.1).
fetch_ffmpeg() {
    local ref="$1"
    mkdir -p "$licences/ffmpeg"
    for file in LICENSE.md COPYING.GPLv2 COPYING.GPLv3 COPYING.LGPLv2.1 COPYING.LGPLv3; do
        curl -fsSL --retry 3 -o "$licences/ffmpeg/$file" "https://raw.githubusercontent.com/FFmpeg/FFmpeg/$ref/$file"
    done
    echo "ffmpeg https://github.com/FFmpeg/FFmpeg/archive/$ref.tar.gz" >> "$licences/SOURCES"
}
: > "$licences/SOURCES"

case "$os" in
    macos)
        cp "$(brew --prefix)/lib/libmpv.2.dylib" "$lib_dir/"
        chmod u+w "$lib_dir/libmpv.2.dylib"
        # Copies every non-system library it needs beside it, pointing each at @loader_path.
        dylibbundler -cd -of -b -x "$lib_dir/libmpv.2.dylib" -d "$lib_dir" -p @loader_path/ > /dev/null 2>&1
        # Its own name too, so nothing in the bundle names Homebrew's folder.
        install_name_tool -id @loader_path/libmpv.2.dylib "$lib_dir/libmpv.2.dylib" 2> /dev/null
        # dylibbundler can add the same search path more than once, and dyld won't load a
        # library with a duplicate LC_RPATH: keep one of each.
        for dylib in "$lib_dir"/*.dylib; do
            otool -l "$dylib" | awk '/cmd LC_RPATH/ {getline; getline; print $2}' | sort | uniq -d | while read -r dup; do
                while [ "$(otool -l "$dylib" | awk '/cmd LC_RPATH/ {getline; getline; print $2}' | grep -cx "$dup")" -gt 1 ]; do
                    install_name_tool -delete_rpath "$dup" "$dylib" 2> /dev/null
                done
            done
        done
        # Rewriting the load commands breaks each library's signature, and Apple Silicon won't
        # load a changed library that isn't signed again. Ad hoc here; a Developer ID signature
        # replaces it when the release has one.
        codesign --force --sign - "$lib_dir"/*.dylib 2> /dev/null
        version="$(mpv --version | head -n 1 | awk '{print $2}')"
        fetch_licences "$version"
        fetch_ffmpeg "n$(brew list --versions ffmpeg | awk '{print $2}' | sed 's/_[0-9]*$//')"
        source="Homebrew's mpv formula, $version (https://formulae.brew.sh/formula/mpv)"
        ;;
    linux)
        lib="$(ldconfig -p | awk '/libmpv\.so\.[0-9]/ {print $NF; exit}')"
        cp -L "$lib" "$lib_dir/"
        # Its libraries, but not the system's own: C runtime, graphics, sound and windowing.
        base='^(libc|libm|libdl|libpthread|librt|ld-linux|libstdc\+\+|libgcc_s|libX|libxcb|libGL|libEGL|libGLX|libdrm|libgbm|libwayland|libvulkan|libasound|libpulse|libudev|libdbus|libz|libexpat|libfontconfig|libfreetype)'
        ldd "$lib" | awk '/=> \// {print $3}' | while read -r dep; do
            name="$(basename "$dep")"
            if ! [[ "$name" =~ $base ]]; then
                cp -L "$dep" "$lib_dir/"
            fi
        done
        for copied in "$lib_dir"/*.so*; do
            # shellcheck disable=SC2016 # $ORIGIN is for the loader, not the shell.
            patchelf --set-rpath '$ORIGIN' "$copied"
        done
        version="$(mpv --version | head -n 1 | awk '{print $2}')"
        fetch_licences "$version"
        # Ubuntu's version, 7:4.4.2-0ubuntu0.22.04.1, is FFmpeg 4.4.2.
        avcodec="$(dpkg-query -W -f='${Version}' 'libavcodec[0-9]*' 2> /dev/null | head -n 1)"
        fetch_ffmpeg "n$(echo "$avcodec" | sed 's/^[0-9]*://; s/-.*//')"
        source="Ubuntu's mpv and libmpv packages, $version (https://packages.ubuntu.com/source/jammy/mpv)"
        ;;
    windows)
        : "${LIBMPV_DIR:?set LIBMPV_DIR to an unpacked mpv-dev build}"
        cp "$LIBMPV_DIR/libmpv-2.dll" "$lib_dir/"
        # The build's name ends in mpv's commit: -git-<hash>.
        ref="$(basename "${LIBMPV_SOURCE:-}" .7z | sed -n 's/.*-git-\([0-9a-f]*\)$/\1/p')"
        fetch_licences "${ref:-master}"
        # shinchiro builds FFmpeg from its master branch: the build's own FFmpeg commit isn't
        # in the file name, so the texts come from master.
        fetch_ffmpeg master
        source="shinchiro's mpv-winbuild-cmake build (${LIBMPV_SOURCE:-https://github.com/shinchiro/mpv-winbuild-cmake})"
        ;;
    *)
        echo "bundle-libmpv: unknown system $os" >&2
        exit 1
        ;;
esac

{
    echo "libmpv, the video player Nexus uses for trailers, ships with Nexus."
    echo
    echo "It is mpv's library, under the GNU General Public License, version 2 or later: see"
    echo "LICENSE.GPL, and Copyright for the parts under the LGPL. It is built from $source."
    echo "The libraries it uses ship beside it, each under its own licence (FFmpeg under the"
    echo "LGPL or GPL, see ffmpeg/, and others). The release has the source of mpv and FFmpeg"
    echo "at these versions (SOURCES lists them) as the third-party-sources asset; the other"
    echo "libraries' source is that of the build named above, at the same versions."
    echo
    echo "Files:"
    for file in "$lib_dir"/*mpv* "$lib_dir"/*.dylib "$lib_dir"/*.so*; do
        if [ -e "$file" ]; then echo "  $(basename "$file")"; fi
    done | sort -u
} > "$licences/NOTICE"
echo "bundled libmpv into $lib_dir"
