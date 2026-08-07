#!/usr/bin/env bash
#
# Send real keystrokes to the running raylib window.
#
#   scripts/gui-keys.sh [--delay MS] <key>...
#
# Keys use tmux send-keys notation, so one KEYS spec in verify-capture.sh drives
# both backends: `Space`, `Escape`, `Enter`, `Tab`, `C-n`, `M-x`, or a literal
# character. Everything else is typed verbatim.
#
# Why this exists: the raylib backend has no input seam an `init.lua` can reach.
# `ruster.cmd` queues ex commands, which covers most surfaces, but which-key,
# flash jump, multi-cursor and cmdline completion only exist *between* real
# keystrokes. macOS System Events is the only way to produce those against the
# real window, and therefore the only way to photograph them.
#
# The headless equivalent — ScriptedRenderer in crates/ruster-render/src/script.rs
# — is what the test suite uses and is far more reliable. Prefer it for
# behaviour. This script is for the cases where the pixels are the point.
#
# KNOWN LIMIT: Ctrl chords do not arrive. Measured, not assumed — sending
# `C-w v` to the GUI leaves it in VISUAL mode, so the `v` landed and the `C-w`
# did not. The raylib backend reads modifiers with `is_key_down(KEY_*_CONTROL)`
# and pairs them with whatever `get_key_pressed` returns in the same frame, and
# a synthesised keystroke does not hold the modifier across that window. So a
# `C-` capture here proves nothing either way: it cannot confirm a chord works
# and it cannot show one is broken. Test Ctrl chords in the TUI, where tmux
# `send-keys` puts real bytes through a PTY, or by hand.

set -euo pipefail

[ "$(uname)" = "Darwin" ] || { echo "gui-keys.sh is macOS-only (System Events)" >&2; exit 1; }

DELAY=250

# Accessibility permission is granted per *host application* — the terminal, or
# whatever spawned this — and there is no way to request it from a script. When
# it is missing, System Events does not error: it silently sends the keystrokes
# nowhere, and the capture comes back showing the surface that was never driven.
# A confusing screenshot is a worse failure than a refusal, so check first.
preflight() {
  if ! osascript -e 'tell application "System Events" to get name of first process' >/dev/null 2>&1; then
    cat >&2 <<'MSG'
error: this process cannot control the UI.

System Events refused. Grant Accessibility permission to whatever is running
this script (Terminal, iTerm, your editor, or the Claude Code CLI):

  System Settings → Privacy & Security → Accessibility

Then re-run. Without it, keystrokes are silently discarded and the capture
shows an undriven surface rather than failing.
MSG
    exit 1
  fi
}

focus_ruster() {
  # The binary is `ruster`; a bundled build is "ruster" too. Raise whichever is
  # running, and fail loudly if neither is — sending keys to whatever happens to
  # be frontmost is how a capture ends up with someone's browser in it.
  if ! osascript -e 'tell application "System Events"
        set procs to (name of every process whose name is "ruster")
        if procs is {} then error "no ruster process"
        set frontmost of first process whose name is "ruster" to true
      end tell' >/dev/null 2>&1; then
    echo "error: no running \`ruster\` window to send keys to" >&2
    exit 1
  fi
  sleep 0.4
}

# Escape a literal for an AppleScript string: backslash first, then quote.
#
# Without this, sending `\` produced `keystroke "\"` — an unterminated string —
# and osascript failed with a syntax error. `\` is the embedded terminal's
# escape key, so the one character this script most needed to send was the one
# it could not.
as_quote() {
  local s="$1"
  s="${s//\\/\\\\}"
  s="${s//\"/\\\"}"
  printf '%s' "$s"
}

# tmux notation -> an AppleScript statement.
send_one() {
  local k="$1"
  case "$k" in
    Space)             osascript -e 'tell application "System Events" to keystroke " "' ;;
    Enter|Return)      osascript -e 'tell application "System Events" to key code 36' ;;
    Escape)            osascript -e 'tell application "System Events" to key code 53' ;;
    Tab)               osascript -e 'tell application "System Events" to key code 48' ;;
    BSpace|BackSpace)  osascript -e 'tell application "System Events" to key code 51' ;;
    Up)                osascript -e 'tell application "System Events" to key code 126' ;;
    Down)              osascript -e 'tell application "System Events" to key code 125' ;;
    Left)              osascript -e 'tell application "System Events" to key code 123' ;;
    Right)             osascript -e 'tell application "System Events" to key code 124' ;;
    C-*)               osascript -e "tell application \"System Events\" to keystroke \"$(as_quote "${k#C-}")\" using control down" ;;
    M-*)               osascript -e "tell application \"System Events\" to keystroke \"$(as_quote "${k#M-}")\" using option down" ;;
    *)                 osascript -e "tell application \"System Events\" to keystroke \"$(as_quote "$k")\"" ;;
  esac
}

main() {
  local keys=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --delay) DELAY="$2"; shift 2 ;;
      *) keys+=("$1"); shift ;;
    esac
  done
  [ ${#keys[@]} -gt 0 ] || { echo "usage: gui-keys.sh [--delay MS] <key>..." >&2; exit 1; }

  preflight
  focus_ruster
  for k in "${keys[@]}"; do
    send_one "$k"
    sleep "$(awk "BEGIN{print $DELAY/1000}")"
  done
}

main "$@"
