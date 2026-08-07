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
--    ruster.wm.focus(...) exists but does nothing in Phase 0 — there is no way
--    to name a window until the layout tree lands.
--
-- 2. Return a table, which is enough when nothing needs deciding. This file
--    does that.
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

    -- Without a spawn bind there is no way to open a window from inside the
    -- session: on DRM the only windows that exist are the startup clients.
    { "M-Return", "spawn foot" },

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
}
