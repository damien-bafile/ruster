#!/usr/bin/env bash
#
# Send real mouse events to the running raylib window.
#
#   scripts/gui-mouse.sh <command>...
#
#     click <x> <y> [button] [count]   press and release
#     down|up <x> <y> [button]         press / release
#     move <x> <y>                     move the pointer
#     drag <x> <y>                     move with the button held
#     wheel <x> <y> <notches>          scroll; positive is up
#     sleep <ms>
#
# Coordinates are **pixels relative to the window's content area**, so a surface
# spec is written once and does not move when the window does. They are chosen
# by eye against a reference capture; the alternative — cell coordinates — would
# need the font's glyph advance, which only the renderer knows.
#
# Why this exists: the mouse surface only exists between real pointer events.
# `ruster.cmd` queues ex commands and gui-keys.sh sends keystrokes; neither can
# produce a click. scripts/inject-input.py can, but goes through /dev/uinput and
# is Linux-only. This is the macOS equivalent, built from scripts/gui-mouse.c
# against CoreGraphics, which ships with the OS.
#
# The headless equivalent — ScriptedRenderer's simulate_mouse_* helpers — is
# what the test suite uses and is far more reliable. Prefer it for behaviour.
# This is for the cases where the pixels are the point.

set -euo pipefail

[ "$(uname)" = "Darwin" ] || { echo "gui-mouse.sh is macOS-only (CoreGraphics)" >&2; exit 1; }

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/gui-mouse"

# Built on demand: it is a 200-line C file against a system framework, so a
# stale binary is a worse failure mode than a one-second rebuild.
if [ ! -x "$BIN" ] || [ "$ROOT/scripts/gui-mouse.c" -nt "$BIN" ]; then
  mkdir -p "$ROOT/target"
  xcrun clang -O2 -o "$BIN" "$ROOT/scripts/gui-mouse.c" -framework ApplicationServices
fi

# The window's content origin in screen points.
#
# System Events reports the window frame; the content area starts below the
# title bar. Asking for the title bar's own height rather than assuming one
# keeps this right on a window that has none.
origin() {
  osascript <<'APPLESCRIPT' 2>/dev/null
tell application "System Events"
  set procs to (every process whose name is "ruster")
  if procs is {} then return "none"
  set p to item 1 of procs
  if (count of windows of p) is 0 then return "none"
  set w to window 1 of p
  set {wx, wy} to position of w
  set {ww, wh} to size of w
  -- The content area is the window minus its title bar. AXWindow reports the
  -- frame including chrome, so the difference is what the editor draws into.
  return (wx as text) & " " & (wy as text) & " " & (ww as text) & " " & (wh as text)
end tell
APPLESCRIPT
}

# Poll rather than ask once: the window takes a variable moment to exist and
# become AX-visible (font loading, the first frame), and a single early ask
# fails in a way that reads as "the editor is not running".
geom=""
for _ in $(seq 1 40); do
  geom="$(origin || true)"
  [ -n "$geom" ] && [ "$geom" != "none" ] && break
  sleep 0.25
done
if [ -z "$geom" ] || [ "$geom" = "none" ]; then
  echo "error: no running \`ruster\` window to send mouse events to" >&2
  exit 1
fi
read -r WIN_X WIN_Y WIN_W WIN_H <<<"$geom"

# The window has to be frontmost or the clicks land on whatever is. Raising it
# is also what makes the screenshot show the editor rather than this terminal.
osascript -e 'tell application "System Events" to set frontmost of first process whose name is "ruster" to true' >/dev/null 2>&1 || true
sleep 0.4

# raylib windows carry a standard title bar; the content starts under it.
TITLEBAR="${RUSTER_TITLEBAR_PX:-32}"

args=()
while [ $# -gt 0 ]; do
  cmd="$1"; shift
  case "$cmd" in
    sleep)
      args+=(sleep "$1"); shift ;;
    mods)
      args+=(mods)
      while [ $# -gt 0 ] && [[ "$1" =~ ^(alt|option|ctrl|control|shift|cmd|command|none)$ ]]; do
        args+=("$1"); shift
      done ;;
    move|drag|click|down|up)
      x="$1"; y="$2"; shift 2
      args+=("$cmd" "$((WIN_X + x))" "$((WIN_Y + TITLEBAR + y))")
      # Optional button, then (for click) an optional repeat count.
      if [ $# -gt 0 ] && [[ "$1" =~ ^(left|right|middle)$ ]]; then
        args+=("$1"); shift
      fi
      if [ "$cmd" = "click" ] && [ $# -gt 0 ] && [[ "$1" =~ ^[1-9]$ ]]; then
        args+=("$1"); shift
      fi
      ;;
    wheel)
      x="$1"; y="$2"; n="$3"; shift 3
      args+=(wheel "$((WIN_X + x))" "$((WIN_Y + TITLEBAR + y))" "$n") ;;
    *)
      echo "gui-mouse.sh: unknown command $cmd" >&2; exit 2 ;;
  esac
done

exec "$BIN" "${args[@]}"
