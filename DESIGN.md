# ruster — Design System

<!-- impeccable:design-schema 1 -->

## Visual Authority

Starship Crew Terminal — industrial sci-fi crew console aesthetic. The editor is a rugged starship command deck for your codebase: green phosphor body text on CRT black, amber warnings for mode changes and errors, off-white stenciled panel labels framing each section.

**THESIS:** ruster is the command deck for your codebase — every buffer is a console readout, every action is a logged acknowledgment, and the chrome is a rugged starship interior built for long solo shifts.

**SOURCE:** Industrial sci-fi starship consoles, avionics panels, crew terminals (Alien, The Expanse, 2001: A Space Odyssey).

## Palette

### Core (roles)

| Token | Hex | Use |
|-------|-----|-----|
| `bg` | `#0a0e0a` | CRT black — main background |
| `fg` | `#33ff66` | Green phosphor — body text |
| `gutter` | `#1a6633` | Dim green — line numbers |
| `gutter_bg` | `#0a0e0a` | Same as bg |
| `selection` | `#0d331a` | Dark green — selection highlight |
| `selection_fg` | `#33ff66` | Same as fg |
| `cursor` | `#66ff99` | Bright green — block cursor |
| `cursor_fg` | `#0a0e0a` | Inverse on cursor block |
| `divider` | `#111a11` | Panel borders, window dividers |
| `statusline_fg` | `#33ff66` | Green phosphor — statusline text |
| `accent` | `#ff8800` | Amber — warnings, mode indicators |
| `accent_fg` | `#0a0e0a` | Text on amber bars |

### Named palette

| Name | Hex | Use |
|------|-----|-----|
| `crt_black` | `#0a0e0a` | Background |
| `phosphor_green` | `#33ff66` | Primary text |
| `phosphor_dim` | `#1a6633` | Diminished text (line numbers, comments) |
| `phosphor_bright` | `#66ff99` | Emphasized text (cursor, active) |
| `amber` | `#ff8800` | Warnings, attention |
| `amber_dim` | `#664400` | Diminished warning |
| `panel_offwhite` | `#ccbbaa` | Stencil panel labels (metadata) |
| `panel_gray` | `#222a22` | Inactive panel backgrounds |
| `stencil` | `#88aa88` | Section header labels |
| `hazard_red` | `#ff3333` | Errors, critical state |

## Typography

### Code / primary

- **Face:** `Iosevka` or `JetBrains Mono` (user-configured via `gui.font`)
- **Weight:** Regular (400) for body, Bold (700) for emphasis, mode labels
- **Size:** 20px default (user-configurable)

### Display / stencil labels

- **Face:** Same monospace, uppercase with extra letter-spacing (tracked)
- **Weight:** Bold
- **Style:** All-caps with `▌▐` bracket framing for section headers

### Statusline

- **Face:** Same monospace
- **Weight:** Regular
- **Style:** Mode labels in uppercase with amber accent background.
  Format: ` ▌MODE▐  file/path   ▐LSP● OK▐  ▐git:main▐  50%  12,34 `

## Composition

### Layout hierarchy (top to bottom)

1. **Buffer / Welcome screen** — primary zone, ~80%+ of vertical space
2. **Which-key panel** — slides up from bottom, overlays buffer
3. **Mini-buffer (cmdline)** — single row above statusline, only in command mode
4. **Statusline** — continuous ruled bar at bottom, always visible

### Panel framing

- Each window section is framed with `divider` color rules
- Active window's statusline has a full-width background in `bg` with `statusline_fg` text
- Inactive windows' statusline dims to `panel_gray` background
- Window dividers between side-by-side windows use `divider` color

### Statusline segments

The statusline is a systems readout bar with ruled segments:

```
 ▌NORMAL▐  src/main.rs            ▐LSP●▐ ▐git:feat/starship▐  45%  23,8
```

- **Left:** Mode indicator in amber-on-black (`accent`/`bg`) or amber-on-amber
  - Active: ` ▌NORMAL▐ ` with amber background
  - Terminal: ` ▌TERMINAL▐ ` with green background
- **Center:** File path, Lua statusline sections
- **Right:** LSP status, git branch, cursor position

### Welcome screen ("Ready Room")

When no file is open, display a centered panel:

```
╔══════════════════════════════════════╗
║         RUSTER  v0.1.0              ║
║         ─── READY ROOM ───          ║
║                                      ║
║   ▌RECENT PROJECTS▐                 ║
║     ~/dev/ruster                    ║
║     ~/dev/other                     ║
║                                      ║
║   ▌QUICK ACTIONS▐                   ║
║     :e path/to/file    Open File    ║
║     :FuzzySearch       Find Files   ║
║     :term              Terminal     ║
║                                      ║
║   ▌SYSTEM▐                          ║
║     LSP ● Ready                     ║
║     Mode: Neovim                    ║
║                                      ║
║   ▌KEYBINDS▐                        ║
║     Ctrl+P    Fuzzy Finder          ║
║     Ctrl+S    Save                  ║
║                                      ║
╚══════════════════════════════════════╝
```

## Interaction patterns

### Mode transitions

- Mode switch (Normal → Insert): Statusline amber segment flashes briefly
- Error state: Statusline amber segment switches to `hazard_red` with hazard-stripe pattern
- Command entry: Mini-buffer opens with `:` prompt in `phosphor_green`

### Statusline update animation

- Character-by-character cascade when statusline content changes (TUI: instant; GUI: optional animation)

## States

| State | Visual |
|-------|--------|
| First run / no file | Welcome screen ("Ready Room") |
| Normal editing | Buffer + statusline with `NORMAL` mode indicator |
| Insert mode | Statusline amber segment shows `INSERT` |
| Visual mode | Statusline amber segment shows `VISUAL` |
| Terminal | Statusline green segment shows `TERMINAL` |
| Command mode | Mini-buffer opens with `:` prompt, statusline shows `CMDLINE` |
| Warning | Amber accent in relevant statusline segment |
| Error | Red (`hazard_red`) accent, hazard-stripe visual |
| Loading | Pulsing amber indicator |

## Adaptations

### TUI (ratatui + crossterm)

- Palette maps directly to terminal 256-color / truecolor
- No CRT glow effects — rely on color alone
- Statusline is a flat bar with segment dividers
- Panel borders use box-drawing characters (`│`, `─`, `╔`, `╗`, `╚`, `╝`)

### GUI (raylib)

- CRT glow effect: subtle green halo behind text via semi-transparent circle drawing
- Curved panel borders (rounded rectangles with `divider` stroke)
- Scanline overlay: optional subtle horizontal lines (1px every 3px, 5% opacity)
- Welcome screen uses the same layout but with smooth type rendering and glow

## Anti-patterns

- No skeuomorphic rivets, bolts, or metallic textures
- No gratuitous scanlines — functional before atmospheric
- No comic sans or non-monospace display fonts in the chrome
- No colors outside the defined palette without theme override
