#!/usr/bin/env bash
#
# Boot ruster on DRM from a free VT, for the compositor hardware verification.
#
#   Ctrl+Alt+F3, log in if you are not already, then:
#     bash scripts/drm-test.sh
#
# It has to be launched from a shell *on the VT you want it on*. logind only
# grants DRM master to the session that owns the seat, so starting it from a
# terminal inside your graphical session fails with "failed to initialize
# libseat session" — correctly, and without touching your display.
#
# Escape hatches, in order of preference:
#   Ctrl+Alt+F2   switch back to your Wayland session (compositor keeps running)
#   Super+Shift+q quit — unconditional, whatever the config says
#   ask Claude to kill it — works regardless of what the compositor is doing
#
# Test Ctrl+Alt+F2 FIRST, while it is running. It is the only escape hatch that
# has never been exercised, and testing it after quitting proves nothing.
#
# Screenshots: Super+Shift+s writes $XDG_RUNTIME_DIR/ruster-shot-N.png, or
# /tmp/ruster-shot-N.png when XDG_RUNTIME_DIR is unset, as it often is on a bare
# VT login. The compositor implements no screencopy protocol, so this is the
# only way to see what happened.
#
# Worth driving once on hardware, now that they are bound (see
# assets/compositor.lua for the full set):
#   Super+h/j/k/l          focus
#   Super+Shift+h/j/k/l    swap
#   Super+Ctrl+h/j/k/l     resize
#   Super+1..9             workspace
#   Super+Shift+1..9       move window to workspace
#   Super+Shift+space      toggle floating
# The compositor logs the resulting geometry for each, so the log is evidence
# on its own even without a screenshot.
set -u

REPO=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
LOG=/tmp/ruster-drm.log

cd "$REPO" || { echo "cannot cd to $REPO"; exit 1; }

echo "building..."
if ! cargo build -p ruster-compositor --features ruster-compositor/udev; then
    echo "BUILD FAILED — not launching."
    exit 1
fi

BIN="$REPO/target/debug/ruster-compositor"
[ -x "$BIN" ] || { echo "missing binary: $BIN"; exit 1; }

echo
echo "launching on DRM; log -> $LOG"
echo "  Ctrl+Alt+F2    back to your session (this keeps running) — try this first"
echo "  Super+Shift+s  screenshot"
echo "  Super+Shift+q  quit"
echo
# A marker to date this run by. Screenshots live in a directory that keeps them
# between sessions, so listing them all would show yesterday's nested captures
# next to today's — and reading a stale PNG as proof the DRM capture worked is
# exactly the mistake this harness exists to prevent.
MARKER=$(mktemp)
trap 'rm -f "$MARKER"' EXIT

RUST_LOG=ruster_compositor=debug,smithay=info "$BIN" --drm >"$LOG" 2>&1
status=$?

# The VT is readable again by the time we get here.
echo
echo "ruster-compositor exited with status $status"
echo "screenshots from this run:"
shots=$(find "${XDG_RUNTIME_DIR:-/tmp}" -maxdepth 1 -name 'ruster-shot-*.png' \
    -newer "$MARKER" -printf '  %p\n' 2>/dev/null | sort)
echo "${shots:-  (none)}"
echo
# `grep -c` exits non-zero on zero matches, so a plain `|| echo 0` prints the
# count *and* the fallback. Count without letting grep's status leak.
switches=$(grep -c "pausing session\|resuming session" "$LOG" 2>/dev/null || true)
echo "VT switches seen (the escape hatch): ${switches:-0}"
echo
echo "last lines of $LOG:"
tail -n 20 "$LOG"
