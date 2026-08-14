# Phase 0 Completion: Animation System + Raylib GUI Backend

## 1. Animation System (tachyonfx)

### Purpose

Replace the bare `tokio::time::interval` clock with a proper animation system
using tachyonfx. This enables timed effects, transitions, and eventually
cursor blink — all driven through a single timer.

### Design

- Add `tachyonfx` dependency to `ruster-tui`
- Replace `interval.tick()` loop with `Timer` from tachyonfx
- Expose a `Duration` per-frame via tachyonfx's timer
- Provide a placeholder for animation effects (empty hook for now)
- No cursor blink in this phase

### Integration

```rust
// Before: bare interval
let mut interval = tokio::time::interval(Duration::from_secs_f64(1.0 / 60.0));
interval.tick().await;

// After: tachyonfx timer
use tachyonfx::Timer;
let mut timer = Timer::from_duration(Duration::from_secs_f64(1.0 / 60.0));
timer.tick();
```

The tachyonfx `Timer` gives us per-frame timing plus a foundation for
duration-based effects when we need them.

## 2. Bugfix: Cursor Disappears on Empty Lines

The cursor `for` loop in `widgets.rs` only iterates over characters in each line.
When a line is empty (`""`), the loop body never runs and the cursor cell is never
written to the buffer.

**Fix:** After the character loop, if we're on the cursor line and the cursor
column is past the end of the text, explicitly write a `' '` with cursor styling
at that position.

*Already implemented in the current codebase.*

## 3. Raylib GUI Backend

### Purpose

Native windowed GUI mode alongside the existing TUI. Raylib handles window
creation, input, and rendering on all targets (Windows, macOS, Linux, FreeBSD).

### Architecture

- New crate: `ruster-render-raylib` with `Renderer` impl
- `App` uses `Box<dyn Renderer>` to switch between TUI and GUI
- Binary flag `--gui` to select backend
- Raylib renderer draws text via `raylib::draw_text_ex()` with a monospace font

### Components

**`crates/ruster-render-raylib/`**

- `Cargo.toml` — depends on `ruster-render`, `raylib`
- `lib.rs` — `RaylibRenderer` struct implementing `Renderer`
  - Window init (800x600, "ruster" title)
  - Font loading (monospace bitmap)
  - `render_frame()` — clears screen, draws lines + cursor, presents
  - Event mapping to `crossterm::event::KeyEvent` so existing `App` works

**`crates/ruster-bin/` changes**

- Parse `--gui` flag
- Select TUI or GUI renderer at startup
- GUI path: create `RaylibRenderer`, call `App::run_gui()` (or adapt run_async)

### Platform Support

Raylib targets: Windows, macOS, Linux, FreeBSD — covers all Phase 0 targets.

### Drawing

- Text: `draw_text_ex()` with a built-in monospace font (raylib ships one, or
  load a TTF from assets/)
- Background: clear with a solid color
- Cursor: draw a filled rect at cursor position
- Each line: `draw_text_ex()` at `(x, y + line_index * font_size)`
- Character width/height derived from `MeasureText()` or fixed glyph advance

### Testing

- Renderer logic extracted for unit testing (line layout, cursor positioning)
- Integration test: App boots in GUI mode with `--gui` flag, renders a frame
