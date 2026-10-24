#!/usr/bin/env bash
# Copies libarchive, the libraries it needs, and their licences into a package, for archives
# the Rust readers don't read (RAR, tar.xz, tar.bz2, tar.zst, ISO, cab):
# `scripts/bundle-libarchive.sh <os> <library folder> <licence folder>`. The app looks for it
# there first: lib/ beside the binary, Contents/Frameworks in the macOS bundle, the binary's
# own folder on Windows.
#
# macOS and Linux build the pinned release below from source, with only its compression
# codecs: no XML, ACL, crypto or iconv, so nothing under the LGPL comes with it. Windows
# copies MSYS2's build (LIBARCHIVE_DIR, which the libarchive action in .github fills).
#
# Needs, per system:
#   macOS    cmake, and Homebrew's zstd, lz4 and dylibbundler
#   Linux    cmake, patchelf, and liblzma-dev, libzstd-dev, libbz2-dev, liblz4-dev, zlib1g-dev
#   Windows  LIBARCHIVE_DIR: the DLLs and a licenses/ folder
set -euo pipefail
os="$1"
lib_dir="$2"
licences="$3/libarchive"
mkdir -p "$lib_dir" "$licences"

# Change both together: a release at least two weeks old, and its SHA-256 as GitHub lists it.
VERSION=3.8.9
SHA256=888c934f9d95648ecb9163dc8e23ab80a476ecb81a8f1154704a227b5b676dde

build() {
    local work
    work="$(mktemp -d)"
    local archive="$work/libarchive-$VERSION.tar.xz"
    curl -fsSL --retry 3 -o "$archive" \
        "https://github.com/libarchive/libarchive/releases/download/v$VERSION/libarchive-$VERSION.tar.xz"
    echo "$SHA256  $archive" | shasum -a 256 -c - > /dev/null
    mkdir "$work/src"
    tar -xJf "$archive" -C "$work/src" --strip-components 1
    cmake -S "$work/src" -B "$work/build" -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=ON \
        -DENABLE_TEST=OFF -DENABLE_TAR=OFF -DENABLE_CPIO=OFF -DENABLE_CAT=OFF -DENABLE_UNZIP=OFF \
        -DENABLE_LIBXML2=OFF -DENABLE_EXPAT=OFF -DENABLE_OPENSSL=OFF -DENABLE_NETTLE=OFF \
        -DENABLE_MBEDTLS=OFF -DENABLE_LIBB2=OFF -DENABLE_ACL=OFF -DENABLE_XATTR=OFF \
        -DENABLE_ICONV=OFF -DENABLE_CNG=OFF -DENABLE_LZO=OFF -DENABLE_PCREPOSIX=OFF \
        -DENABLE_PCRE2POSIX=OFF -DENABLE_LIBGCC=OFF > "$work/cmake.log" 2>&1
    cmake --build "$work/build" -j 4 > "$work/build.log" 2>&1
    built="$work/build/libarchive"
    cp "$work/src/COPYING" "$licences/COPYING"
}

case "$os" in
    macos)
        build
        cp "$built/libarchive.13.dylib" "$lib_dir/"
        chmod u+w "$lib_dir/libarchive.13.dylib"
        install_name_tool -id @loader_path/libarchive.13.dylib "$lib_dir/libarchive.13.dylib"
        # Homebrew's zstd and lz4 beside it; the system's zlib, bzip2 and xz stay the system's.
        dylibbundler -cd -of -b -x "$lib_dir/libarchive.13.dylib" -d "$lib_dir" -p @loader_path/ > /dev/null 2>&1
        # dyld won't load a library with a duplicate LC_RPATH, and changed libraries need
        # signing again (see bundle-libmpv.sh).
        for dylib in "$lib_dir"/*.dylib; do
            otool -l "$dylib" | awk '/cmd LC_RPATH/ {getline; getline; print $2}' | sort | uniq -d | while read -r dup; do
                while [ "$(otool -l "$dylib" | awk '/cmd LC_RPATH/ {getline; getline; print $2}' | grep -cx "$dup")" -gt 1 ]; do
                    install_name_tool -delete_rpath "$dup" "$dylib" 2> /dev/null
                done
            done
        done
        codesign --force --sign - "$lib_dir"/*.dylib 2> /dev/null
        for dep in zstd lz4; do
            cp "$(brew --prefix "$dep")/LICENSE" "$licences/LICENSE.$dep"
        done
        source="libarchive $VERSION, built from its release with zlib, bzip2, xz (the system's), zstd and lz4 (Homebrew's)"
        ;;
    linux)
        build
        cp -L "$built/libarchive.so.13" "$lib_dir/"
        # Its codecs, but not the C runtime or zlib, which every system has.
        base='^(libc|libm|libdl|libpthread|librt|ld-linux|libgcc_s|libz)\.'
        ldd "$lib_dir/libarchive.so.13" | awk '/=> \// {print $3}' | while read -r dep; do
            if ! [[ "$(basename "$dep")" =~ $base ]]; then
                cp -L "$dep" "$lib_dir/"
            fi
        done
        for copied in "$lib_dir"/libarchive.so.13 "$lib_dir"/liblzma* "$lib_dir"/libzstd* "$lib_dir"/libbz2* "$lib_dir"/liblz4*; do
            # shellcheck disable=SC2016 # $ORIGIN is for the loader, not the shell.
            [ -e "$copied" ] && patchelf --set-rpath '$ORIGIN' "$copied"
        done
        for dep in liblzma5 libzstd1 libbz2-1.0 liblz4-1; do
            cp "/usr/share/doc/$dep/copyright" "$licences/copyright.$dep"
        done
        source="libarchive $VERSION, built from its release with Ubuntu's xz, zstd, bzip2 and lz4 libraries"
        ;;
    windows)
        : "${LIBARCHIVE_DIR:?set LIBARCHIVE_DIR to the libarchive DLLs from MSYS2}"
        cp "$LIBARCHIVE_DIR"/*.dll "$lib_dir/"
        cp -R "$LIBARCHIVE_DIR/licenses/." "$licences/"
        packages="${LIBARCHIVE_PACKAGES:-see this folder}"
        source="the mingw-w64-ucrt-x86_64-libarchive package from MSYS2 and the packages it needs ($packages)"
        ;;
    *)
        echo "bundle-libarchive: unknown system $os" >&2
        exit 1
        ;;
esac

# Where the source of what ships is: the action lists MSYS2's source packages on Windows.
if [ "$os" != windows ]; then
    echo "libarchive https://github.com/libarchive/libarchive/releases/download/v$VERSION/libarchive-$VERSION.tar.xz" > "$licences/SOURCES"
fi
{
    echo "libarchive, which Nexus uses for RAR and other archives, ships with Nexus."
    echo
    echo "It is under the BSD 2-clause licence: see COPYING. It is $source."
    echo "The libraries it uses ship beside it, each under its own licence (zstd and lz4 BSD,"
    echo "xz 0BSD, bzip2 its own BSD-style licence, on Windows also others: see this folder)."
    echo
    echo "Files:"
    if [ "$os" = windows ]; then
        for file in "$LIBARCHIVE_DIR"/*.dll; do echo "  $(basename "$file")"; done
    else
        for file in "$lib_dir"/*archive* "$lib_dir"/*lzma* "$lib_dir"/*zstd* "$lib_dir"/*lz4* "$lib_dir"/*bz2*; do
            if [ -e "$file" ]; then echo "  $(basename "$file")"; fi
        done
    fi | sort -u
} > "$licences/NOTICE"
echo "bundled libarchive into $lib_dir"
