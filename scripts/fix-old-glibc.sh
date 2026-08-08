#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Installs a Proton build that runs on an old glibc, and points the launcher at
# it. For distros whose glibc is older than the one recent Proton needs, where
# the game dies with "libc.so.6: version `GLIBC_2.38' not found".
#
#   wget -qO- https://raw.githubusercontent.com/N7z/vortex-launcher/master/scripts/fix-old-glibc.sh | sh
#
# The whole script is a function so a truncated download cannot half-run.

set -eu

PROTON_TAG=GE-Proton9-27
PROTON_NEEDS=2.38   # what the newest GE-Proton wants; older builds need only 2.25
DATA="${XDG_DATA_HOME:-$HOME/.local/share}/vortex-launcher"
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/vortex-launcher/config.json"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# "2.35" and "2.9" compared as numbers, not as strings, so 2.9 stays below 2.35
older_than() {
    awk -v a="$1" -v b="$2" 'BEGIN {
        split(a, x, "."); split(b, y, ".")
        exit !(x[1] < y[1] || (x[1] == y[1] && x[2] < y[2]))
    }'
}

host_glibc() {
    # awk exits 0 even when getconf wrote nothing, so test the text, not $?
    version=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')
    [ -n "$version" ] || version=$(ldd --version 2>/dev/null | awk 'NR == 1 {print $NF}')
    printf '%s' "$version"
}

main() {
    for tool in awk tar python3; do
        command -v "$tool" >/dev/null 2>&1 || die "$tool is needed and not installed"
    done
    command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1 ||
        die "curl or wget is needed and neither is installed"

    glibc=$(host_glibc)
    [ -n "$glibc" ] || die "cannot tell which glibc this system has"
    say "system glibc is $glibc"

    if ! older_than "$glibc" "$PROTON_NEEDS"; then
        say "that is new enough for current Proton, nothing to fix"
        exit 0
    fi

    if pgrep -x vortex-launcher >/dev/null 2>&1; then
        die "close the launcher first, it rewrites the config when it exits"
    fi

    dir="$DATA/proton/$PROTON_TAG"
    if [ -x "$dir/proton" ]; then
        say "$PROTON_TAG is already installed"
    else
        url="https://github.com/GloriousEggroll/proton-ge-custom/releases/download/$PROTON_TAG/$PROTON_TAG.tar.gz"
        mkdir -p "$DATA/proton"
        say "downloading $PROTON_TAG, about 700 MB"
        if command -v curl >/dev/null 2>&1; then
            curl -fL# "$url" | tar xzf - -C "$DATA/proton"
        else
            wget -O- "$url" | tar xzf - -C "$DATA/proton"
        fi
        [ -x "$dir/proton" ] || die "the archive did not contain $PROTON_TAG/proton"
    fi

    # written into the config rather than left to autodetection: the launcher
    # keeps whichever Proton it already stored, and picks the highest-sorting
    # build when it does look, so neither would reliably land on this one
    mkdir -p "$(dirname "$CONFIG")"
    CONFIG="$CONFIG" PROTON_TAG="$PROTON_TAG" PROTON_DIR="$dir" python3 - <<'PY'
import json, os, pathlib

path = pathlib.Path(os.environ["CONFIG"])
try:
    config = json.loads(path.read_text())
    if not isinstance(config, dict):
        raise ValueError("config is not an object")
except FileNotFoundError:
    config = {}
except ValueError as err:
    raise SystemExit(f"error: {path} is not valid JSON ({err}), move it aside and rerun")

config["proton"] = {
    "name": os.environ["PROTON_TAG"],
    "dir": os.environ["PROTON_DIR"],
    "source": "managed",
}
path.write_text(json.dumps(config, indent=2) + "\n")
PY

    say ""
    say "done, the launcher now uses $PROTON_TAG"
    say "if the game still fails, send the output of: cat $DATA/logs/game.log"
}

main "$@"
