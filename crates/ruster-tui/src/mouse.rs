//! Mouse dispatch.
//!
//! Both frontends funnel their native mouse input through
//! [`ruster_render::mouse::MouseEvent`] and land here, so hit-testing and the
//! per-zone handlers stay free of backend conditionals. Keeping this out of
//! `app.rs` also keeps that file from absorbing the whole mouse surface.

use crossterm::event::KeyModifiers;
use ruster_render::mouse::{MouseButton, MouseEvent, MouseKind};

use crate::app::App;

/// Route one mouse event to its handler.
pub fn handle_mouse_event(app: &mut App, ev: MouseEvent) {
    if ev.kind != MouseKind::Down
        || ev.button != MouseButton::Left
        || !ev.modifiers.contains(KeyModifiers::ALT)
    {
        return;
    }
    if let Some((wid, offset)) = app.buffer_offset_at(ev.col, ev.row) {
        if let Some(win) = app.ws.borrow_mut().windows.window_mut(wid) {
            win.cursors.add_cursor(offset);
        }
    }
}
