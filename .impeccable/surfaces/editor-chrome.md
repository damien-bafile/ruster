---
version: 1
slug: "editor-chrome"
primary_target: "editor-chrome"
related_targets: []
---

# ruster Editor Chrome — Surface Brief

## 1. Job and audience

**Who:** Software developers — Neovim/Emacs veterans who spend all day in an editor. They arrive ready to work: open a file, check git status, maybe run tests. They know keyboard shortcuts, value information density, and hate UI that gets in the way.

**Visitor mode:** Operate — completing editing tasks is the goal. The chrome exists to be seen past.

## 2. Outcome and proof

**Primary task:** Open a file and start editing immediately, with clear awareness of mode, file path, git status, and LSP state.

**Success:** The welcome screen shows recent projects as destinations. Opening a file reveals a clean, dense editing surface with the statusline as a continuous systems readout. The user feels in command — every element has a purpose.

**Real evidence:** AGENTS.md and docs/ describe the full seven-phase roadmap. The product is real (10 crates), the UI is not yet built.

## 3. Selected direction

**Visual authority:** Starship Crew Terminal — industrial sci-fi crew console aesthetic. Green phosphor body text on curved CRT black, amber warnings, off-white stenciled panel labels framing each section.

**Thesis:** ruster is the command deck for your codebase — every buffer is a console readout, every action is a logged acknowledgment, and the chrome is a rugged starship interior built for long solo shifts.

**Structural thesis:** Terminal-console layout — a single "glass" dominates (the buffer), framed by labeled equipment panels (statusline, file tree, mini-buffer) with stencil-style headings. The welcome screen is the "ready room" — systems check with recent projects as navigation destinations.

**Signature interaction:** Character-by-character text display and command acknowledgment. Mode switches flash the statusline segment amber. Error states trigger a warning strip.

**Implementation consequence:** Both GUI (raylib) and TUI (ratatui) must share the same chrome language. The TUI version translates the industrial-console palette to terminal 256-color/truecolor. The GUI adds CRT-like glow effects and curved panel borders.

## 4. Scope and boundaries

**Fidelity:** Production-ready first build. Full welcome screen + basic editing view with statusline, gutter, and mini-buffer. Not a complete implementation — enough to see and feel the direction.

**Breadth:** Welcome screen, buffer view, statusline, gutter, mini-buffer command bar. Future surfaces (terminal, file explorer, settings) expand the world.

**What remains untouched:** Actual editor functionality (cursor movement, text editing, LSP) — this is chrome only.

**Anti-goals:** No skeuomorphic rivets or gratuitous scanlines. The console is functional first, atmospheric second.

## 5. States and ranges

| State | Description |
|-------|-------------|
| First run | Welcome screen — no recent files, show "Get Started" call to action |
| Normal edit | Buffer with active cursor, statusline showing mode/file/LSP/git |
| Empty buffer | Blank buffer with statusline, ready to type |
| Command | Mini-buffer open at bottom with `:` prompt |
| Warning | Amber status segment for warnings, errors, or mode changes |
| Error | Red hazard-stripe indicator for critical issues |
| Loading | Systems-check animation during startup/LSP init |

Realistic data: filenames up to ~80 chars, file paths fitting statusline width, 20+ recent projects on welcome screen.

## 6. Interaction and layout

**Topology:** Full-screen buffer dominates (80%+ of space). Statusline at bottom edge as a continuous ruled bar. Mini-buffer floats above statusline during command mode. Welcome screen is the initial state — centered panel with recent projects, recent files, and keybind reference.

**Hierarchy (top to bottom):**
1. (Buffer view / Welcome screen) — primary zone
2. Mini-buffer (contextual, only in command mode)
3. Statusline (always visible, pinned to bottom)

**Affordances:** All keyboard — no click targets in the initial build. Statusline segments are read-only indicators.

**Feedback:** Mode switch: statusline segment flashes amber briefly. Error: hazard-stripe pattern appears on relevant status segment. Command acknowledgment: character echo in mini-buffer.

## 7. Constraints and open decisions

**Platform:** Both GUI (raylib) and TUI (ratatui+crossterm). The palette must work in terminal 256-color. The GUI adds CRT glow and curved panel borders.

**Accessibility:** High contrast green-on-black as default. User-configurable themes override everything. Mode must be conveyed by text label (not just color).

**Open decisions:**
- Exact panel header font (stencil-style vs monospace caps)
- Whether statusline uses ruled-segment dividers or piped separators
- Welcome screen keybind reference — show all or minimal set
- CRT scanline/glow effect intensity in GUI mode
