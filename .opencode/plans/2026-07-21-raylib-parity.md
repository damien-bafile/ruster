# Plan: Raylib GUI Parity — Cursor Fix, Syntax Highlighting, Statusline, Cmdline

## Context

The raylib GUI backend has three issues:
1. **Cursor drift** — cursor position uses hardcoded `CHAR_W=12` but `draw_text_ex` renders with actual font character widths, causing misalignment as `col` increases
2. **No syntax highlighting** — TUI renders `StyledLine.highlights` for colored text; raylib draws plain white
3. **No statusline/cmdline** — TUI shows mode, file path, cursor position, and command line at bottom; raylib shows nothing

## Root Causes

**Cursor drift:** `draw_text_ex` renders the entire line at once, then cursor position is calculated as `PAD_X + col * CHAR_W`. The default font's actual character width ≠ 12px, so the cursor drifts rightward as `col` grows.

**Missing features:** The raylib renderer only handles text + cursor — it ignores `highlights`, `mode_label`, `file_path`, `cmdline`, and `message` from `EditorState`.

## Approach

### Task 1: Fix cursor drift

**File:** `crates/ruster-render-raylib/src/lib.rs`

- Replace hardcoded `CHAR_W` with measured character width from the font
- Use `font.measure_text_ex("m", FONT_SIZE, 1.0).x` to get actual char width at init time
- Store `char_w: f32` on `RaylibRenderer`
- Use `char_w` for all cursor position calculations and the smooth cursor offset
- Also use `char_w` for cursor rectangle width

**Verification:** Open GUI, type a long line — cursor should stay aligned with text.

### Task 2: Add syntax highlighting

**File:** `crates/ruster-render-raylib/src/lib.rs`

- For each `StyledLine`, iterate through `highlights` to build colored segments
- For characters NOT in any highlight range, render with default color (white/205,214,244)
- For characters IN a highlight range, render with the style's fg color
- Use `draw_text_ex` per segment (group consecutive same-style chars into one call)
- Keep a running `x_offset` tracking pixel position across the line

**Color mapping:** `ruster_render::Color::Rgb(r,g,b)` → `raylib::Color::new(r,g,b,255)`, `Color::Default` → default text color.

**Verification:** Open a `.rs` file — keywords, strings, comments should be colored.

### Task 3: Add statusline

**File:** `crates/ruster-render-raylib/src/lib.rs`

- Reserve bottom `LINE_H` pixels for statusline
- Draw dark gray background rectangle across full width
- Left: mode label (e.g. `-- NORMAL --`) in white
- Center: file path in white (truncated if needed)
- Right: cursor position `(line,col)` in white
- Buffer area height = `window_height - LINE_H` (or `- 2*LINE_H` if cmdline visible)

**Verification:** Mode label changes when switching modes, file path shows, position updates.

### Task 4: Add cmdline/message area

**File:** `crates/ruster-render-raylib/src/lib.rs`

- When `state.cmdline` or `state.message` is `Some`, reserve an additional `LINE_H` at the very bottom (below statusline)
- Draw the cmdline text (e.g. `:wq`) in white on dark background
- This shrinks the buffer area by another `LINE_H`

**Verification:** Press `:` — command line appears at bottom. Execute or press Esc — it disappears.

## Task Order

1 → 2 → 3 → 4 (cursor fix first, then features build on the corrected layout)

## Files Modified

| File | Change |
|------|--------|
| `crates/ruster-render-raylib/src/lib.rs` | All four tasks |

## Verification

1. `cargo build -p ruster-render-raylib` — no errors
2. `cargo test --all` — all existing tests pass
3. Manual: launch GUI, type text — cursor stays aligned
4. Manual: open a `.rs` file — syntax colors appear
5. Manual: mode label, file path, position shown at bottom
6. Manual: `:q` shows cmdline, Esc dismisses it
