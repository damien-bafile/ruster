/// Handle to a compositor-managed client window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u32);

/// One mapped client window. Phase 0 stores no layout tree — windows are
/// ordered by map time and the focused one renders fullscreen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientWindow {
    pub id: WindowId,
    pub title: String,
    pub width: i32,
    pub height: i32,
}

impl ClientWindow {
    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }
    pub fn set_size(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }
}
