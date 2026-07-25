# Running ruster on Windows

ruster is cross-platform. This page covers the Windows-specific setup.

## Building

You need a stable Rust toolchain (MSVC target, the default on Windows) and the
Visual Studio C++ build tools — the `raylib` GUI backend compiles native code.

```powershell
# Debug build
cargo build

# Optimized binary at target\release\ruster.exe
cargo build --release
```

Prebuilt `ruster.exe` binaries are attached to tagged
[GitHub Releases](../../releases) and produced by the `Release` workflow.

## Running

```powershell
# GUI (raylib) — the default frontend
ruster path\to\file.rs

# Terminal UI (crossterm) — best in Windows Terminal
ruster --tui path\to\file.rs
```

## Runtime dependencies

- **ripgrep** — the `:Rg` live-grep command shells out to `rg`. Install it and
  make sure it is on `PATH`:

  ```powershell
  winget install BurntSushi.ripgrep.MSVC
  # or
  choco install ripgrep
  ```

  Without `rg`, `:Rg` reports that ripgrep was not found; everything else works.
  `:Files` uses an in-process file walker and needs no external tool.

- **A monospaced font (optional, improves the GUI).** The GUI looks for a
  user-installed JetBrains Mono / Nerd font first, then falls back to Windows
  system fonts (Consolas, Lucida Console), then to raylib's built-in font.
  Installing a Nerd font gives the best result.

## Embedded terminal (`:term`)

The embedded terminal uses Windows' **ConPTY** API, which requires **Windows 10
version 1809 (October 2018) or newer** (Windows 11 and Server 2019+ included).
The editor itself runs on older Windows; only `:term` needs ConPTY.

- **Default shell.** `:term` launches `%COMSPEC%` (usually `cmd.exe`) unless you
  override it. To use PowerShell, set it in `%APPDATA%\ruster\init.lua`:

  ```lua
  ruster.config.terminal_shell = "pwsh.exe"   -- or "powershell.exe"
  ```

  `terminal_shell` is the program only — no argument splitting.

- **Line endings.** Programs in the terminal emit CRLF; that is terminal-grid
  content handled by the VT parser and is unrelated to how ruster saves files
  (below).
- **Best experience.** The raylib GUI (the default frontend on Windows) is the
  smoothest host for `:term`; the `--tui` frontend runs a terminal inside your
  console, which works but is fiddlier for some Ctrl-chords.

## Line endings

Files are opened preserving their existing line endings: a CRLF file stays CRLF
on save, and an LF file stays LF. Internally the buffer is always LF, so editing
behaves identically across platforms.

## Config location

User config is loaded from `%APPDATA%\ruster\init.lua` on Windows (the
cross-platform config directory), equivalent to `~/.config/ruster/init.lua` on
Unix. See [config-reference.md](config-reference.md) and [lua-api.md](lua-api.md).
