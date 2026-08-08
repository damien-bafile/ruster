use crate::window::{ClientWindow, WindowId};

/// The window records — title and last known size — in map order, plus the one
/// focus handle. Where a window sits, and which workspace it is on, belong to
/// [`Tree`](crate::Tree) and [`Workspaces`](crate::Workspaces); this is what is
/// left once they have taken them.
pub struct ShellState {
    windows: Vec<ClientWindow>,
    next_id: u32,
    pub focus: Option<WindowId>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellState {
    pub fn new() -> Self {
        ShellState {
            windows: Vec::new(),
            next_id: 0,
            focus: None,
        }
    }

    pub fn add_window(&mut self, title: String, width: i32, height: i32) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id += 1;
        self.windows.push(ClientWindow {
            id,
            title,
            width,
            height,
        });
        id
    }

    pub fn remove_window(&mut self, id: WindowId) {
        if let Some(pos) = self.windows.iter().position(|w| w.id == id) {
            self.windows.remove(pos);
        }
        if self.focus == Some(id) {
            self.focus = self.windows.last().map(|w| w.id);
        }
    }

    pub fn set_focus(&mut self, id: WindowId) {
        if self.windows.iter().any(|w| w.id == id) {
            self.focus = Some(id);
        }
    }

    pub fn focused(&self) -> Option<&ClientWindow> {
        self.focus
            .and_then(|id| self.windows.iter().find(|w| w.id == id))
    }

    pub fn window(&mut self, id: WindowId) -> Option<&mut ClientWindow> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn windows(&self) -> impl Iterator<Item = &ClientWindow> {
        self.windows.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_window_assigns_incrementing_ids() {
        let mut s = ShellState::new();
        let a = s.add_window("foot".into(), 800, 600);
        let b = s.add_window("firefox".into(), 1024, 768);
        assert_eq!(a, WindowId(0));
        assert_eq!(b, WindowId(1));
        assert_eq!(s.windows().count(), 2);
    }

    #[test]
    fn focus_tracks_most_recent() {
        let mut s = ShellState::new();
        let a = s.add_window("a".into(), 100, 100);
        let b = s.add_window("b".into(), 100, 100);
        s.set_focus(b);
        assert_eq!(s.focused().unwrap().id, b);
        s.set_focus(a);
        assert_eq!(s.focused().unwrap().id, a);
    }

    #[test]
    fn remove_window_clears_focus_if_needed() {
        let mut s = ShellState::new();
        let a = s.add_window("a".into(), 100, 100);
        s.set_focus(a);
        s.remove_window(a);
        assert_eq!(s.focused(), None);
        assert_eq!(s.windows().count(), 0);
    }

    #[test]
    fn remove_focused_window_falls_back_to_last_remaining() {
        // Removing the focused window refocuses the shell onto the most
        // recently added remaining window (the semantics the compositor's
        // unmap path mirrors when the focused toplevel hides itself).
        let mut s = ShellState::new();
        let a = s.add_window("a".into(), 100, 100);
        let _b = s.add_window("b".into(), 100, 100);
        let c = s.add_window("c".into(), 100, 100);
        s.set_focus(a);
        s.remove_window(a);
        assert_eq!(s.focused().unwrap().id, c);
    }

    #[test]
    fn window_title_and_size_mutators() {
        let mut s = ShellState::new();
        let a = s.add_window("old".into(), 10, 10);
        s.window(a).unwrap().set_title("new".into());
        s.window(a).unwrap().set_size(1920, 1080);
        let w = s.window(a).unwrap();
        assert_eq!(w.title, "new");
        assert_eq!((w.width, w.height), (1920, 1080));
    }
}
