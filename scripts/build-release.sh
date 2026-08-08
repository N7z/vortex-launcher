#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Builds both binaries against an old glibc, so one release runs everywhere
# instead of only on distros as new as the machine that built it.
#
# Needs cargo-zigbuild and zig:
#   cargo install cargo-zigbuild
#   pacman -S zig        (or: pip install ziglang)

set -eu

TARGET=x86_64-unknown-linux-gnu.2.31
OUT=target/x86_64-unknown-linux-gnu/release

command -v cargo >/dev/null 2>&1 || { echo "error: cargo is not installed" >&2; exit 1; }
cargo zigbuild --help >/dev/null 2>&1 ||
    { echo "error: cargo-zigbuild is missing, run: cargo install cargo-zigbuild" >&2; exit 1; }
command -v zig >/dev/null 2>&1 ||
    { echo "error: zig is not on PATH" >&2; exit 1; }

rustup target add x86_64-unknown-linux-gnu >/dev/null 2>&1 || true
cargo zigbuild --release --target "$TARGET"

# the point of the whole exercise, so check it rather than trust it
fail=0
for binary in vortex-launcher vortex-launcher-cli; do
    path="$OUT/$binary"
    [ -f "$path" ] || { echo "error: $path was not built" >&2; exit 1; }
    needs=$(objdump -T "$path" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)
    printf '%-22s %8s bytes  needs %s\n' "$binary" "$(stat -c%s "$path")" "${needs:-unknown}"
    case "$needs" in
        GLIBC_2.3[01]|GLIBC_2.[12][0-9]|GLIBC_2.[0-9]) ;;
        *) echo "  ^ higher than $TARGET allows" >&2; fail=1 ;;
    esac
done
[ "$fail" -eq 0 ] || { echo "error: a binary wants a newer glibc than intended" >&2; exit 1; }

echo
echo "built in $OUT"
