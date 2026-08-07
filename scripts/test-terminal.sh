#!/usr/bin/env bash
#
# Functional test of the embedded terminal, driven through a real PTY.
#
#   scripts/test-terminal.sh
#
# Why this is a shell script and not `#[cfg(test)]`: the unit tests construct a
# `KeyEvent` and hand it to `handle_key`, which proves the handler and nothing
# about whether the key can arrive. That gap is not hypothetical — two tests
# asserted `Ctrl-\` left Terminal-Insert while neither backend could produce the
# event, so the terminal was a one-way door with a green suite. tmux `send-keys`
# puts real bytes through a real PTY, which is the only way to check that.
#
# It also caught the reverse: `:echo hi` typed in Terminal-Normal re-focused the
# shell on the `i` of "hi" and sent the rest of the line to zsh.
#
# Needs tmux and a debug build (`cargo build`).
set -uo pipefail
T="${TMUX_BIN:-tmux}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
S="$(mktemp -d "${TMPDIR:-/tmp}/ruster-termtest-XXXXXX")"
trap 'rm -rf "$S"' EXIT
PASS=0; FAIL=0

command -v "$T" >/dev/null || { echo "tmux not found; set TMUX_BIN" >&2; exit 1; }
[ -x "$ROOT/target/debug/ruster" ] || { echo "run \`cargo build\` first" >&2; exit 1; }

# Fresh config dir per case; $1 = extra config.lua body, $2 = init.lua body
setup() {
  CFG="$S/tt-$RANDOM"; mkdir -p "$CFG/ruster"
  cat > "$CFG/ruster/config.lua" <<EOF
ruster.config = { session = { autosave = false, autoload = false }, $1 }
EOF
  printf '%s\n' "$2" > "$CFG/ruster/init.lua"
}

start() { # $1 = session name
  $T -f /dev/null new-session -d -s "$1" -x 100 -y 20 \
     "cd $ROOT && XDG_CONFIG_HOME='$CFG' ./target/debug/ruster --tui docs/verification/fixtures/demo.rs"
  sleep 2.5
}

pane() { $T capture-pane -p -t "$1"; }
mode() { pane "$1" | tail -1 | grep -o '\-\- [A-Z]* --' | head -1; }
stop() { $T kill-session -t "$1" 2>/dev/null; rm -rf "$CFG"; }

check() { # name expected actual
  if [ "$2" = "$3" ]; then printf '  PASS  %s\n' "$1"; PASS=$((PASS+1))
  else printf '  FAIL  %s\n        expected %-22s got %s\n' "$1" "[$2]" "[$3]"; FAIL=$((FAIL+1)); fi
}
contains() { # name needle haystack
  if printf '%s' "$3" | grep -q -- "$2"; then printf '  PASS  %s\n' "$1"; PASS=$((PASS+1))
  else printf '  FAIL  %s (no %s)\n' "$1" "$2"; FAIL=$((FAIL+1)); fi
}
absent() {
  if printf '%s' "$3" | grep -q -- "$2"; then printf '  FAIL  %s (unexpectedly found %s)\n' "$1" "$2"; FAIL=$((FAIL+1))
  else printf '  PASS  %s\n' "$1"; PASS=$((PASS+1)); fi
}

echo "=== 1. mode round trip (default <C-\\>) ==="
setup '' 'ruster.cmd(":term")'; start t1
check "starts in Terminal-Insert" "-- TERMINAL --" "$(mode t1)"
$T send-keys -t t1 'C-\'; sleep 0.8
check "Ctrl-\\ -> Terminal-Normal" "-- NORMAL --" "$(mode t1)"
$T send-keys -t t1 'i'; sleep 0.8
check "i -> Terminal-Insert" "-- TERMINAL --" "$(mode t1)"
$T send-keys -t t1 'C-\'; sleep 0.6; $T send-keys -t t1 'A'; sleep 0.6
check "A -> Terminal-Insert" "-- TERMINAL --" "$(mode t1)"
stop t1

echo "=== 2. terminal.escape = <Esc> (vterm/evil style) ==="
setup 'terminal = { escape = "<Esc>" }' 'ruster.cmd(":term")'; start t2
$T send-keys -t t2 Escape; sleep 0.8
check "Esc -> Terminal-Normal" "-- NORMAL --" "$(mode t2)"
$T send-keys -t t2 'i'; sleep 0.6
check "i -> Terminal-Insert" "-- TERMINAL --" "$(mode t2)"
stop t2

echo "=== 3. keys reach the shell ==="
setup '' 'ruster.cmd(":term")'; start t3
$T send-keys -t t3 "echo alpha-beta" Enter; sleep 1.5
contains "typed command runs" "alpha-beta" "$(pane t3)"
$T send-keys -t t3 "sleep 30" Enter; sleep 1
$T send-keys -t t3 'C-c'; sleep 1
$T send-keys -t t3 "echo after-interrupt" Enter; sleep 1.5
contains "Ctrl-C interrupts the child" "after-interrupt" "$(pane t3)"
$T send-keys -t t3 "echo hist-marker" Enter; sleep 1
$T send-keys -t t3 Up; sleep 0.8
contains "Up recalls shell history" "hist-marker" "$(pane t3)"
stop t3

echo "=== 4. Terminal-Normal is a working editor buffer ==="
setup '' 'ruster.cmd(":term")'; start t4
$T send-keys -t t4 "echo yankable-line" Enter; sleep 1.5
$T send-keys -t t4 'C-\'; sleep 0.8
$T send-keys -t t4 'gg'; sleep 0.5
check "gg works in Terminal-Normal" "-- NORMAL --" "$(mode t4)"
contains "cursor at line 1" "1,1" "$(pane t4 | tail -1)"
$T send-keys -t t4 'G'; sleep 0.5
absent "G moved off line 1" " 1,1" "$(pane t4 | tail -1)"
$T send-keys -t t4 ':' 'echo cmdline-worked' Enter; sleep 1.2
contains ": command runs from Terminal-Normal" "cmdline-worked" "$(pane t4)"
absent  ": text does not leak to the shell" "command not found" "$(pane t4)"
$T send-keys -t t4 Escape; sleep 0.6
$T send-keys -t t4 'C-w' 'v'; sleep 1
contains "Ctrl-w v splits from Terminal-Normal" "terminal" "$(pane t4)"
stop t4

echo "=== 5. scrollback reachability ==="
setup '' 'ruster.cmd(":term")'; start t5
$T send-keys -t t5 "seq 1 300" Enter; sleep 2.5
contains "recent output visible" "300" "$(pane t5)"
$T send-keys -t t5 'C-\'; sleep 0.8; $T send-keys -t t5 'gg'; sleep 0.6
absent "KNOWN GAP: scrollback unreachable (PASS here = still unreachable)" "^1$" "$(pane t5)"
stop t5

echo "=== 6. terminal.default_mode = normal ==="
setup 'terminal = { default_mode = "normal" }' 'ruster.cmd(":term")'; start t6
check "starts in Terminal-Normal" "-- NORMAL --" "$(mode t6)"
stop t6

echo "=== 7. shell exit ==="
setup '' 'ruster.cmd(":term")'; start t7
$T send-keys -t t7 "exit" Enter; sleep 2
contains "editor survives the shell exiting" "terminal" "$(pane t7)"
stop t7

echo
echo "passed: $PASS   failed: $FAIL"
[ "$FAIL" -eq 0 ]
