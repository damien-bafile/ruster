#!/usr/bin/env bash
#
# Send real mouse events to a ruster running in a tmux pane.
#
#   scripts/tui-mouse.sh <session> <command>...
#
#     click <col> <row> [button] [count]   press and release
#     down|up <col> <row> [button]         press / release
#     move <col> <row>                     motion with no button held
#     drag <col> <row>                     motion with the left button held
#     wheel <col> <row> <notches>          scroll; positive is up
#     sleep <ms>
#
# Coordinates are 0-based cells, matching what the editor's hit-test sees.
#
# Why this exists: `tmux send-keys` sends keys, and there is no mouse
# equivalent. But a terminal mouse event *is* just an escape sequence on the
# pane's input, so writing the SGR encoding directly is exactly what a real
# mouse would have produced — the editor cannot tell the difference, because
# there is nothing to tell apart.
#
# SGR (1006) encoding, which is what crossterm enables and parses:
#
#   ESC [ < Cb ; Ccol ; Crow M     press
#   ESC [ < Cb ; Ccol ; Crow m     release
#
# Cb is the button (0 left, 1 middle, 2 right) plus 4/8/16 for shift/alt/ctrl,
# plus 32 for motion, and 64/65 for wheel up/down. Columns and rows are
# 1-based on the wire, hence the +1 below.

set -euo pipefail

TMUX_BIN="${TMUX_BIN:-tmux}"

SESSION="${1:?usage: tui-mouse.sh <session> <command>...}"
shift

# Write one SGR sequence into the pane.
send() {
  local cb="$1" col="$2" row="$3" final="$4"
  # -l sends the string literally; without it tmux would interpret the escape.
  "$TMUX_BIN" send-keys -t "$SESSION" -l \
    "$(printf '\033[<%d;%d;%d%s' "$cb" "$((col + 1))" "$((row + 1))" "$final")"
  # Let the editor's event loop pick it up. Without a pause a burst can be read
  # as one batch, and a scripted double-click becomes an unpredictable one.
  sleep 0.05
}

button_code() {
  case "${1:-left}" in
    left) echo 0 ;;
    middle) echo 1 ;;
    right) echo 2 ;;
    *) echo "tui-mouse.sh: unknown button $1" >&2; exit 2 ;;
  esac
}

while [ $# -gt 0 ]; do
  cmd="$1"; shift
  case "$cmd" in
    sleep)
      sleep "$(awk "BEGIN{print $1/1000}")"; shift ;;

    move)
      col="$1"; row="$2"; shift 2
      # 35 = 32 (motion) + 3 (no button held).
      send 35 "$col" "$row" M ;;

    drag)
      col="$1"; row="$2"; shift 2
      # 32 = motion with the left button down.
      send 32 "$col" "$row" M ;;

    wheel)
      col="$1"; row="$2"; n="$3"; shift 3
      code=64; [ "$n" -lt 0 ] && code=65
      for _ in $(seq 1 "${n#-}"); do send "$code" "$col" "$row" M; done ;;

    down|up|click)
      col="$1"; row="$2"; shift 2
      btn="left"
      if [ $# -gt 0 ] && [[ "$1" =~ ^(left|right|middle)$ ]]; then btn="$1"; shift; fi
      count=1
      if [ "$cmd" = "click" ] && [ $# -gt 0 ] && [[ "$1" =~ ^[1-9]$ ]]; then count="$1"; shift; fi
      cb="$(button_code "$btn")"

      case "$cmd" in
        down) send "$cb" "$col" "$row" M ;;
        up)   send "$cb" "$col" "$row" m ;;
        click)
          # A terminal has no notion of a click streak: it reports N presses and
          # the editor's own timer decides they are a double. The 50ms pause in
          # `send` is well inside the 400ms default window.
          for _ in $(seq 1 "$count"); do
            send "$cb" "$col" "$row" M
            send "$cb" "$col" "$row" m
          done ;;
      esac ;;

    *)
      echo "tui-mouse.sh: unknown command $cmd" >&2; exit 2 ;;
  esac
done
