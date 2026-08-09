-- ruster compositor default configuration.
--
-- Copy to ~/.config/ruster/compositor.lua to change it. There are two ways to
-- write a config and they can be mixed; calls are recorded first, then any
-- returned table is folded in on top.
--
-- 1. Call the API, which lets the config decide things:
--
--      ruster.wm.set_keybind("M-S-q", "quit")
--      ruster.wm.launch_client("foot")
--      ruster.wm.switch_workspace(2)
--
--      if os.getenv("RUSTER_MINIMAL") then
--        ruster.wm.launch_client("weston-terminal")
--      end
--
--    The API is live: the Lua VM stays alive for the whole session, so these
--    can be called at any time, not only while the config is read. Calls queue
--    an action that the event loop carries out on its next pass — the same
--    path a keybind takes, so nothing can drift between them.
--
--      ruster.wm.set_keybind("M-x", "screenshot")  -- takes effect at once,
--                                     -- and appears in the helper immediately
--      ruster.wm.focus("left")        -- left/right/up/down (or h/l/k/j)
--      ruster.wm.action("swap right") -- any action name from the list below
--      ruster.wm.spawn("foot")        -- launch on this compositor's socket
--      ruster.wm.quit()
--      ruster.wm.switch_workspace(2)  -- the starting workspace from a config,
--                                     -- and a plain switch at runtime
--
--    and to read the session back:
--
--      local s = ruster.wm.status()
--      -- s.workspace, s.windows, s.title, s.floating, s.layout
--
--    `focus`, `action` and `spawn` return false rather than raising when given
--    something they cannot use, so one typo does not abandon the rest of a
--    config.
--
-- 2. Return a table, which is enough when nothing needs deciding. This file
--    does that.
--
-- A binding may be a *sequence* of chords separated by spaces: "M-w h" means
-- Super+w, then h. While a sequence is half-typed the which-key overlay shows
-- what can come next; it disappears when the sequence completes, or after a
-- second of nothing, so an abandoned prefix stops swallowing keys. A single
-- chord is a sequence of length one, so ordinary binds are unchanged.
--
-- `toggle help` (M-? by default) pins that overlay open with *every* binding in
-- it — full sequences, not just what can be pressed next — so the keymap can be
-- browsed rather than only glimpsed mid-chord. A half-typed sequence still takes
-- over while it lasts. The panel wraps into as many columns as it needs, so
-- nothing is left out however long the keymap grows, and it is built from the
-- live keymap: a binding added with ruster.wm.set_keybind shows up at once.
--
-- Keybinds are (binding, action) pairs. In a binding, `M` is Mod4
-- (Super/Logo), `S` Shift, `C` Control and `A` Alt, followed by the key name:
-- "M-S-q", "M-t", "M-F9", "C-A-space". Modifiers match exactly, so "M-t" does
-- not fire while Shift is held.
--
-- Actions:
--
--   quit                     shut the compositor down
--   cycle workspace          advance to the next workspace
--   screenshot               write the screen to a PNG
--   focus <direction>        move focus to the window drawn that way
--   swap <direction>         exchange the focused window with that neighbour
--   resize <direction>       move the boundary between them
--   split horizontal|vertical   re-divide this window's container that way,
--                            which is also the axis the next window here uses
--   toggle floating          float the focused window, or re-tile it
--   spawn <command>          launch a program on this compositor's socket
--   terminal                 launch this machine's terminal (see below)
--   new pane                 open an empty editor pane as a tile
--   edit <path>              open a file in an editor pane (read-only so far)
--   toggle help              pin the shortcut helper open, or unpin it
--   command                  open the ":" prompt (an action name)
--   lua                      open the "=" prompt (Lua, in the config's VM)
--   workspace <1-9>          show a numbered workspace
--   move to workspace <1-9>  send the focused window there
--
-- <direction> is left, right, up or down (or h, l, k, j). Underscores and
-- dashes work in place of spaces, so "move_to_workspace_3" is the same action
-- as "move to workspace 3". An action that does not parse binds nothing.
--
-- `spawn` is the exception: everything after the word is taken as a command
-- line exactly as written, so its case, dashes and underscores survive
-- ("spawn foot -e htop"). It is split on whitespace and there is no quoting;
-- for anything more, spawn a shell.
--
-- `terminal` is `spawn` with the command worked out on the machine it runs on,
-- which is what a *default* keybind needs: the `terminal` setting below if there
-- is one, else $TERMINAL, else the first of foot, alacritty, kitty, wezterm,
-- ghostty, weston-terminal found on PATH. Whichever it picks is logged with the
-- reason it picked it, and finding none is logged too, so a terminal key that
-- does nothing always says why. All the candidates are Wayland-native — there is
-- no Xwayland here, so an X11-only terminal would spawn and then fail to find a
-- display.
--
-- The `terminal` setting and $TERMINAL are run exactly as written, so they may
-- carry arguments ("foot -e tmux") and are never second-guessed; if one names a
-- program that is not installed, the failed spawn says so rather than something
-- else opening in its place. Only the built-in list is searched for, since
-- nothing asked for its entries by name.
--
-- `screenshot` writes the composited output to $XDG_RUNTIME_DIR/ruster-shot-N.png
-- (or /tmp when that is unset, as on a bare VT). It exists because the
-- compositor implements no screencopy protocol, so on a real DRM boot nothing
-- outside it can see the screen.
--
-- startup_clients are launched on the compositor's socket at boot. A client
-- whose binary is not installed is skipped. On a DRM boot this is the only way
-- a client appears, since there is no outer compositor to launch one by hand.
return {
  keybinds = {
    { "M-S-q", "quit" },
    { "M-t",   "cycle workspace" },
    { "M-S-s", "screenshot" },

    -- hjkl, because the editor this compositor is named after uses them.
    { "M-h",   "focus left" },
    { "M-j",   "focus down" },
    { "M-k",   "focus up" },
    { "M-l",   "focus right" },

    { "M-S-h", "swap left" },
    { "M-S-j", "swap down" },
    { "M-S-k", "swap up" },
    { "M-S-l", "swap right" },

    { "M-C-h", "resize left" },
    { "M-C-j", "resize down" },
    { "M-C-k", "resize up" },
    { "M-C-l", "resize right" },

    -- `b` rather than i3's `h`, which is spoken for above.
    { "M-b",   "split horizontal" },
    { "M-v",   "split vertical" },
    { "M-S-space", "toggle floating" },

    -- Without this there is no way to open a window from inside the session: on
    -- DRM the only windows that exist are the startup clients. `terminal` rather
    -- than `spawn foot`, so the default keymap works on a machine that has a
    -- terminal but not that one.
    { "M-Return", "terminal" },
    { "M-S-Return",    "new pane" },
    { "M-S-slash",     "toggle help" },
    { "M-S-semicolon", "command" },
    { "M-S-equal",     "lua" },

    { "M-1",   "workspace 1" },
    { "M-2",   "workspace 2" },
    { "M-3",   "workspace 3" },
    { "M-4",   "workspace 4" },
    { "M-5",   "workspace 5" },
    { "M-6",   "workspace 6" },
    { "M-7",   "workspace 7" },
    { "M-8",   "workspace 8" },
    { "M-9",   "workspace 9" },

    { "M-S-1", "move to workspace 1" },
    { "M-S-2", "move to workspace 2" },
    { "M-S-3", "move to workspace 3" },
    { "M-S-4", "move to workspace 4" },
    { "M-S-5", "move to workspace 5" },
    { "M-S-6", "move to workspace 6" },
    { "M-S-7", "move to workspace 7" },
    { "M-S-8", "move to workspace 8" },
    { "M-S-9", "move to workspace 9" },
  },
  startup_clients = { "foot" },

  -- What the `terminal` action runs. Omit it and $TERMINAL decides; omit that
  -- too and the first installed terminal from the built-in list is used. Set it
  -- when neither guess is the terminal you want, or when yours needs arguments.
  --
  -- terminal = "foot -e tmux",

  -- Keyboard layout and repeat. Omit this table entirely and libxkbcommon uses
  -- the system default, which honours XKB_DEFAULT_LAYOUT and friends — so an
  -- unconfigured compositor matches the rest of your session rather than
  -- forcing US. A layout xkb rejects is refused with a warning and the previous
  -- keymap is kept, because on DRM a broken keymap means a black screen and no
  -- keyboard with which to fix the file that caused it.
  --
  -- keyboard = {
  --   layout = "gb",
  --   variant = "colemak",
  --   options = "ctrl:nocaps",
  --   repeat_delay = 200,   -- ms before repeating
  --   repeat_rate = 25,     -- repeats per second
  -- },
}
