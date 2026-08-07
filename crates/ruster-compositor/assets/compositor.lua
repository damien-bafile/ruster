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
-- Actions: "quit", "cycle workspace".
--
-- startup_clients are launched on the compositor's socket at boot. A client
-- whose binary is not installed is skipped. On a DRM boot this is the only way
-- a client appears, since there is no outer compositor to launch one by hand.
return {
  keybinds = {
    { "M-S-q", "quit" },
    { "M-t",   "cycle workspace" },
  },
  startup_clients = { "foot" },
}
