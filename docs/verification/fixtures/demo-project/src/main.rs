//! Fixture for the surfaces that need a live language server or debugger.
//!
//! Line numbers here are load-bearing: the capture specs in
//! `scripts/verify-capture.sh` jump to a line and expect a particular symbol
//! under the cursor. Adding or removing a line above one of them moves the
//! target, and the capture comes back empty in a way that looks like the
//! feature failing rather than the fixture drifting. The two that matter:
//!
//!   line 20 — `pub fn origin`, three `w` motions from column 1 to the name,
//!             which is the hover target
//!   line 31 — the first statement of `main`, the breakpoint target

/// A point in the plane. `:hover` over this name is the hover capture.
#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub fn origin() -> Point {
        Point { x: 0, y: 0 }
    }

    // TODO: translate by a vector rather than a scalar
    pub fn shifted(self, by: i32) -> Point {
        Point { x: self.x + by, y: self.y + by }
    }
}

/// The hover target.
///
/// `p` sits deliberately in **column 1**: `:N` lands the cursor there and
/// nowhere else, so a hover capture driven by a line number alone needs a
/// symbol in that column. Every alternative — counting `w` motions, or setting
/// the cursor from Lua — depends on something that drifts when this file is
/// edited, and drifts into a capture that says "No hover info" and looks like
/// the feature is broken.
pub fn identity(p: Point) -> Point {
p
}

fn main() {
    let start = Point::origin();
    let moved = start.shifted(7);
    println!("{start:?} -> {moved:?}");
}
