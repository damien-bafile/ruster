-- ruster compositor default configuration.
--
-- keybinds: (binding, action) pairs. `M` = Mod4 (Super/Logo), `S` = Shift.
--   "M-S-q"  quit the compositor
--   "M-t"    cycle the active workspace
-- startup_clients: binaries to launch on the compositor's socket at boot.
--   Clients whose binary is not installed are skipped.
return {
  keybinds = {
    { "M-S-q", "quit" },
    { "M-t",   "cycle workspace" },
  },
  startup_clients = { "foot" },
}
