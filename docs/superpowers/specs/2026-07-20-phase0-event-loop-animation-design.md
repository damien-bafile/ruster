# Phase 0: Async Event Loop & Animation System

**Date:** 2026-07-20
**Status:** Spec (approved)

## Summary

Replace the current blocking `crossterm::event::read()` loop with a tokio-based async event loop and integrate `tachyonfx` for animation timing. This enables the 60fps tick-driven render loop, cursor blinking, and provides the foundation for LSP/file-watcher/timer multiplexing in later phases.

## Architecture

```
┌─────────────┐   spawn_blocking     ┌──────────────┐
│ crossterm   │──────────────────────►│ mpsc channel │
│ event::read │  AppEvent::Input(e)  │  (unbounded) │
└─────────────┘                      └──┬───────────┘
                                        │
App::run_async()         tokio::select!
                                        │
┌─────────────┐                         │
│ tokio::time │── Tick (16ms) ──────────┤
│  interval   │                         │
└─────────────┘                    ┌────▼────┐
                                   │ handle  │
                                   │ animate │
                                   │ render  │
                                   └─────────┘
```

## Components

### 1. Event Channel

A `tokio::sync::mpsc::unbounded_channel` carries `AppEvent` messages:

```rust
enum AppEvent {
    Input(crossterm::event::Event),
}
```

### 2. Blocking Reader

A `tokio::task::spawn_blocking` thread reads `crossterm::event::read()` in a loop and pushes `AppEvent::Input` onto the channel. If the channel sender is dropped (loop exiting), the thread terminates.

### 3. Main Loop (`App::async_run`)

`tokio::select!` between two branches:
- **Channel receive:** process input event, render immediately, continue
- **Timer tick (16ms / ~60fps):** tick animations, render

After any branch fires, animations tick and `self.render()` is called.

### 4. Animation System (`AnimationState`)

```rust
struct AnimationState {
    last_frame: std::time::Instant,
    cursor_timer: tachyonfx::EffectTimer,
    cursor_visible: bool,
}
```

- Each frame: compute `delta = now - last_frame`, advance `cursor_timer.process(delta)`
- When `cursor_timer.done()`, toggle cursor visibility and reset the timer
- Passed through `EditorState` to `BufferWidget` which skips cursor highlight when invisible

### 5. Runtime

`App::run_async()` creates a `tokio::runtime::Builder::new_current_thread()` runtime (enabled `time` feature) internally. The binary calls `run_async()` instead of `run()`.

## File Changes

### `ruster-render/src/lib.rs`
- Add `cursor_visible: bool` field to `EditorState` (default `true`)

### `ruster-tui/Cargo.toml`
- Add `tokio = { version = "1", features = ["rt", "time"] }`
- Add `tachyonfx = "0.2"`

### `ruster-tui/src/app.rs`
- Add `AppEvent` enum
- Add `AnimationState` struct with `EffectTimer`
- Add `App::run_async()` → creates tokio rt, enters terminal raw mode, calls `async_run()`
- Add `App::async_run()` → the `select!` loop
- Add `App::tick_animations()` → advances `EffectTimer`, toggles cursor
- Move event handling logic into a helper used by both `run()` and `async_run()`
- `run()` (sync) kept for test compatibility

### `ruster-tui/src/widgets.rs`
- `BufferWidget` respects `cursor_visible` flag in `render()`

### `ruster-bin/src/main.rs`
- `app.run_async()` instead of `app.run()`

## Error Handling & Cleanup

- `run_async()` wraps terminal setup/teardown identically to `run()` (raw mode + alt screen)
- Channel close triggers loop exit; reader thread terminates
- Panic safety: `run_async()` catches panics and restores terminal state

## Testing Strategy

- Sync `run()` method preserved — existing parse-cmdline tests continue to work
- New test: `AnimationState` cursor toggle over time (simulate ticks, verify flip)
- Existing 84 tests must still pass
