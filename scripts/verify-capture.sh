#!/usr/bin/env bash
#
# Capture a user-visible surface in both backends.
#
#   scripts/verify-capture.sh [--tui] [--gui] <surface>...
#   scripts/verify-capture.sh all
#   scripts/verify-capture.sh --list
#
# Writes docs/verification/<surface>-tui.txt and <surface>-gui.png. With no
# backend flag it does both.
#
# Why a script rather than a test: neither artifact can be asserted on by
# machine. A PNG needs eyes for legibility, glyph fallback and theme colour, and
# the TUI text needs eyes for pane alignment. What a script can do is make the
# two halves come from one drive definition, so the backends are compared under
# identical conditions rather than under whatever each session improvised.
#
# Every surface is driven the same way: a throwaway XDG_CONFIG_HOME holding a
# generated init.lua. `ruster.cmd` queues an ex command and the queue is applied
# before the first frame, which is the whole of the GUI's control surface —
# there is no way to send it a keystroke. Surfaces that need real keys carry a
# KEYS spec: tmux send-keys drives the TUI, scripts/gui-keys.sh drives the GUI.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/docs/verification"
FIXTURE="$OUT/fixtures/demo.rs"
BIN="$ROOT/target/debug/ruster"
TMUX_BIN="${TMUX_BIN:-tmux}"

# The GUI window size is pinned so two runs of the same surface are comparable.
# The schema default (800x600) is small enough that the settings overlay and the
# debugger dock have to make hard layout choices, which is exactly where the
# defects are.
GUI_W=1200
GUI_H=800
TUI_COLS=120
TUI_ROWS=40

# ---------------------------------------------------------------------------
# Surface definitions
# ---------------------------------------------------------------------------
#
# Each sets:
#   LUA   — init.lua body: `ruster.cmd(...)` lines applied before the first frame
#   CONF  — extra config.lua groups. Prefer this over `:set` for anything the
#           capture merely needs *on*: `:set` echoes a confirmation toast, which
#           then sits in the artifact pretending to be part of the surface.
#   KEYS  — keys to send after the app settles (tmux notation); needs real input
#   OPEN  — what to open: "fixture" (default), "repo" (a dirty scratch repo),
#           "none" (bare launch, for the dashboard), or "self" (this checkout)
#   NEEDS — a live service that must be on PATH, else the surface is skipped
#   WAIT  — ms to let the surface settle before capturing (default 1200)

SURFACES=(
  dashboard editor statusline gutter sidebar dired ibuffer
  whichkey whichkey-accent cmdline flash multicursor
  git-status git-staged diffview
  trouble todos settings themes help messages mason projects
  noice-toast noice-panel noice-popup dialog
  hover debugger terminal sessions gotoline
)

NUMBERS="gutter = { number = true }"

spec() {
  LUA=""; CONF=""; KEYS=""; OPEN="fixture"; NEEDS=""; WAIT=1200
  case "$1" in
    dashboard)      OPEN="none" ;;
    editor)         CONF="$NUMBERS" ;;
    statusline)     CONF="$NUMBERS"; LUA=":16" ;;
    gutter)         OPEN="repo"; CONF="gutter = { number = true, relativenumber = true }"
                    LUA=":Gitsigns|:TodoList" ;;
    sidebar)        OPEN="self"; LUA=":sidebar" ;;
    dired)          OPEN="self"; LUA=":Dired" ;;
    ibuffer)        LUA=":ls" ;;
    whichkey)       KEYS="Space" ;;
    whichkey-accent) KEYS="Space" ;;
    cmdline)        KEYS=": e Space / t m p / Tab" ;;
    flash)          KEYS="f" ;;
    multicursor)    LUA=":16"; KEYS="C-n" ;;
    git-status)     OPEN="repo"; LUA=":Git" ;;
    git-staged)     OPEN="repo"; LUA=":GitStaged" ;;
    diffview)       OPEN="repo"; LUA=":Diffview" ;;
    trouble)        NEEDS="rust-analyzer"; OPEN="self"; LUA=":Trouble"; WAIT=6000 ;;
    todos)          LUA=":TodoList" ;;
    settings)       LUA=":settings" ;;
    themes)         LUA=":Themes" ;;
    help)           LUA=":help" ;;
    messages)       LUA=":echo first message|:echo second message|:messages" ;;
    mason)          LUA=":Mason" ;;
    projects)       LUA=":projects" ;;
    noice-toast)    LUA=":echo the mini toast renders top-right" ;;
    noice-panel)    LUA=":echo one|:echo two|:Noice" ;;
    noice-popup)    LUA=":echo popped|:Noice popup" ;;
    dialog)         LUA="@dialog" ;;
    hover)          NEEDS="rust-analyzer"; OPEN="self"; LUA=":16|:hover"; WAIT=8000 ;;
    debugger)       NEEDS="lldb-dap"; OPEN="self"; LUA=":db_toggle|:debug"; WAIT=8000 ;;
    terminal)       LUA=":term"; WAIT=2500 ;;
    sessions)       LUA=":SessionSave|:messages" ;;
    gotoline)       CONF="$NUMBERS"; LUA=":16" ;;
    *) echo "unknown surface: $1" >&2; return 1 ;;
  esac
}

# The dialog is the one surface no ex command reaches, so it gets a literal.
DIALOG_LUA='ruster.ui.dialog{
  title = "Install language server?",
  fields = {
    { label = "Runs",    kind = "text",   value = "cargo install rust-analyzer" },
    { label = "Dry run", kind = "toggle", value = "off" },
    { label = "Install", kind = "button" },
    { label = "Cancel",  kind = "button" },
  },
  on_submit = function() end,
}'

# ---------------------------------------------------------------------------
# Preconditions
# ---------------------------------------------------------------------------

die() { echo "error: $*" >&2; exit 1; }

require_binary() {
  [ -x "$BIN" ] || die "$BIN not built — run \`cargo build\` first"
}

# macOS refuses to create a window for a locked session: GLFW enumerates zero
# monitors and raylib panics with "Attempting to create window failed!", which
# says nothing about the real cause. Catch it here and say the real thing.
require_unlocked_screen() {
  if [ "$(uname)" != "Darwin" ]; then return 0; fi
  if ioreg -n Root -d1 -a 2>/dev/null | grep -q -A1 CGSSessionScreenIsLocked; then
    if ioreg -n Root -d1 -a 2>/dev/null | grep -A1 CGSSessionScreenIsLocked | grep -q "<true/>"; then
      die "the screen is locked — unlock it and re-run; the GUI cannot open a window otherwise"
    fi
  fi
}

# ---------------------------------------------------------------------------
# Fixture workspaces
# ---------------------------------------------------------------------------

# A throwaway git repo holding the fixture with one committed version, one
# staged change and one unstaged change — enough for signs, `:Git`, `:GitStaged`
# and `:Diffview` to have something to show.
#
# Built in a temp dir rather than as a checked-in dirty file: a fixture that has
# to stay modified would show up in this repo's `git status` forever.
make_scratch_repo() {
  local dir="$1"
  mkdir -p "$dir"
  git -C "$dir" init -q
  git -C "$dir" config user.email verify@example.com
  git -C "$dir" config user.name "Verification Capture"
  cp "$FIXTURE" "$dir/demo.rs"
  printf 'name = "demo"\n' > "$dir/Cargo.toml"
  git -C "$dir" add -A
  git -C "$dir" commit -qm "the committed version"
  # Staged: a new function. Unstaged: an edit on top of it.
  printf '\npub fn staged_addition() -> i32 {\n    41\n}\n' >> "$dir/demo.rs"
  git -C "$dir" add demo.rs
  printf '\npub fn unstaged_addition() -> i32 {\n    42\n}\n' >> "$dir/demo.rs"
}

# Resolve OPEN into (workdir, target) for the run.
resolve_target() {
  local surface="$1" scratch="$2"
  case "$OPEN" in
    none)    WORKDIR="$ROOT"; TARGET="" ;;
    self)    WORKDIR="$ROOT"; TARGET="$ROOT/crates/ruster-render/src/script.rs" ;;
    repo)    make_scratch_repo "$scratch/repo"; WORKDIR="$scratch/repo"; TARGET="$scratch/repo/demo.rs" ;;
    fixture) mkdir -p "$scratch/work"; cp "$FIXTURE" "$scratch/work/demo.rs"
             WORKDIR="$scratch/work"; TARGET="$scratch/work/demo.rs" ;;
    *) die "unknown OPEN kind: $OPEN" ;;
  esac
}

# Write config.lua (pinning the window size) and init.lua (the drive) into a
# throwaway XDG_CONFIG_HOME. `extra` is appended to init.lua — the GUI half uses
# it for the deferred screenshot and quit.
write_config() {
  local cfg="$1" extra="${2:-}"
  mkdir -p "$cfg/ruster"
  cat > "$cfg/ruster/config.lua" <<EOF
ruster.config = {
  gui = { window_width = $GUI_W, window_height = $GUI_H },
  -- The which-key panel is gated behind this delay; a capture cannot wait it
  -- out reliably, and the delay is not what the capture is checking.
  whichkey = { timeoutlen = 0 },
  session = { autosave = false, autoload = false },
  $CONF
}
EOF
  : > "$cfg/ruster/init.lua"
  if [ "$LUA" = "@dialog" ]; then
    printf '%s\n' "$DIALOG_LUA" >> "$cfg/ruster/init.lua"
  elif [ -n "$LUA" ]; then
    # LUA is a `|`-separated list of ex commands.
    local IFS='|'
    for cmd in $LUA; do
      printf 'ruster.cmd(%s)\n' "\"$cmd\"" >> "$cfg/ruster/init.lua"
    done
  fi
  [ -n "$extra" ] && printf '%s\n' "$extra" >> "$cfg/ruster/init.lua"
  return 0
}

# ---------------------------------------------------------------------------
# The TUI half
# ---------------------------------------------------------------------------

capture_tui() {
  local surface="$1" scratch="$2" dest="$OUT/$surface-tui.txt"
  local cfg="$scratch/cfg-tui" sess="ruster-verify-$$-$surface"

  resolve_target "$surface" "$scratch"
  write_config "$cfg"

  # -f /dev/null: the user's tmux.conf must not decide what the artifact looks
  # like. Fixed -x/-y for the same reason.
  "$TMUX_BIN" -f /dev/null new-session -d -s "$sess" -x "$TUI_COLS" -y "$TUI_ROWS" \
    "cd '$WORKDIR' && XDG_CONFIG_HOME='$cfg' '$BIN' --tui $TARGET"
  sleep "$(awk "BEGIN{print $WAIT/1000}")"

  if [ -n "$KEYS" ]; then
    # shellcheck disable=SC2086 — KEYS is deliberately word-split into tmux args.
    "$TMUX_BIN" send-keys -t "$sess" $KEYS
    sleep 1
  fi

  "$TMUX_BIN" capture-pane -p -t "$sess" > "$dest"
  "$TMUX_BIN" kill-session -t "$sess" 2>/dev/null || true

  if [ ! -s "$dest" ]; then
    echo "  tui: FAILED (empty capture)" >&2
    return 1
  fi
  echo "  tui: $dest"
}

# ---------------------------------------------------------------------------
# The GUI half
# ---------------------------------------------------------------------------

capture_gui() {
  local surface="$1" scratch="$2" dest="$OUT/$surface-gui.png"
  local cfg="$scratch/cfg-gui"

  require_unlocked_screen
  resolve_target "$surface" "$scratch"

  # The shot fires on a timer so LSP and DAP round-trips have landed, then a
  # forced quit ends the run. `:q!` sets should_quit unconditionally, where a
  # plain `:q` would only close a window when a surface opened a second one —
  # which is why the old recipe relied on `timeout` and could not tell a clean
  # finish from a hang.
  local quit_at=$((WAIT + 900))
  local extra
  extra="$(cat <<EOF
ruster.defer($WAIT, function() ruster.cmd(":screenshot $dest") end)
ruster.defer($quit_at, function() ruster.cmd(":q!") end)
EOF
)"
  write_config "$cfg" "$extra"

  rm -f "$dest"
  local budget=$(( (quit_at / 1000) + 10 ))
  local rc=0
  if [ -n "$KEYS" ]; then
    ( cd "$WORKDIR" && XDG_CONFIG_HOME="$cfg" timeout "$budget" "$BIN" $TARGET >/dev/null 2>&1 ) &
    local pid=$!
    "$ROOT/scripts/gui-keys.sh" $KEYS || rc=$?
    wait $pid || rc=$?
  else
    ( cd "$WORKDIR" && XDG_CONFIG_HOME="$cfg" timeout "$budget" "$BIN" $TARGET >/dev/null 2>&1 ) || rc=$?
  fi

  if [ ! -s "$dest" ]; then
    echo "  gui: FAILED (no image written; exit $rc)" >&2
    return 1
  fi
  if [ "$rc" -eq 124 ]; then
    echo "  gui: $dest (WARNING: the deferred quit never fired; timeout ended the run)"
  else
    echo "  gui: $dest"
  fi
}

# ---------------------------------------------------------------------------
# Driver
# ---------------------------------------------------------------------------

capture() {
  local surface="$1" do_tui="$2" do_gui="$3"
  spec "$surface"

  if [ -n "$NEEDS" ] && ! command -v "$NEEDS" >/dev/null 2>&1; then
    echo "$surface: skipped — needs $NEEDS on PATH"
    return 0
  fi

  echo "$surface:"
  local scratch
  scratch="$(mktemp -d "${TMPDIR:-/tmp}/ruster-verify-XXXXXX")"
  # shellcheck disable=SC2064 — expand scratch now, not at trap time.
  trap "rm -rf '$scratch'" RETURN

  local failed=0
  if [ "$do_tui" = 1 ]; then capture_tui "$surface" "$scratch" || failed=1; fi
  if [ "$do_gui" = 1 ]; then capture_gui "$surface" "$scratch" || failed=1; fi
  return $failed
}

main() {
  local do_tui=0 do_gui=0 args=()
  for a in "$@"; do
    case "$a" in
      --tui)  do_tui=1 ;;
      --gui)  do_gui=1 ;;
      --list) printf '%s\n' "${SURFACES[@]}"; return 0 ;;
      -h|--help) sed -n '3,10p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; return 0 ;;
      all)    args+=("${SURFACES[@]}") ;;
      *)      args+=("$a") ;;
    esac
  done
  [ ${#args[@]} -gt 0 ] || die "nothing to capture; try --list, a surface name, or 'all'"
  if [ "$do_tui" = 0 ] && [ "$do_gui" = 0 ]; then do_tui=1; do_gui=1; fi

  require_binary
  mkdir -p "$OUT"

  local failures=()
  for s in "${args[@]}"; do
    capture "$s" "$do_tui" "$do_gui" || failures+=("$s")
  done

  if [ ${#failures[@]} -gt 0 ]; then
    echo
    echo "incomplete: ${failures[*]}" >&2
    return 1
  fi
}

main "$@"
