#!/usr/bin/env sh
# Build the euterpe_insn QEMU plugin: tools/qemu/build-plugin.sh <out.so>
#
# Needs a C compiler and glib headers (Debian/Ubuntu: libglib2.0-dev,
# macOS: brew install glib pkg-config). The plugin API header must match the
# installed QEMU, so it is downloaded once for that version and cached next
# to the output.
set -eu

out="$1"
here="$(cd "$(dirname "$0")" && pwd)"
dir="$(dirname "$out")"
mkdir -p "$dir"

version="$(qemu-system-arm --version | sed -n 's/.*version \([0-9][0-9.]*\).*/\1/p' | head -n 1)"
[ -n "$version" ] || { echo "build-plugin: can't read qemu-system-arm's version" >&2; exit 1; }
header_dir="$dir/qemu-$version/include"
if [ ! -f "$header_dir/qemu-plugin.h" ]; then
    mkdir -p "$header_dir"
    curl -sSfL -o "$header_dir/qemu-plugin.h" \
        "https://raw.githubusercontent.com/qemu/qemu/v$version/include/qemu/qemu-plugin.h"
fi

cflags="$(pkg-config --cflags glib-2.0 2>/dev/null || true)"
case "$(uname -s)" in
    Darwin) shared="-bundle -undefined dynamic_lookup" ;;
    *) shared="-shared -fPIC" ;;
esac
# shellcheck disable=SC2086
${CC:-cc} $shared -O2 -Wall -I"$header_dir" $cflags -o "$out" "$here/euterpe_insn.c"
