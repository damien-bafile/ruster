//! Shell state for the ruster Wayland compositor.
//!
//! [`tree`] is the container tree — how windows divide an output between them —
//! and is where Phase 1's layout lives. [`workspace`] holds nine of those trees
//! and decides which one is on screen. [`state`] is what is left of the Phase 0
//! flat model: the window records and the focus handle. [`persist`] writes all
//! nine layouts to a file and puts them back on the next boot.

pub mod persist;
pub mod state;
pub mod tree;
pub mod window;
pub mod workspace;

pub use state::ShellState;
pub use tree::{Direction, Layout, Node, NodeId, Rect, Tree};
pub use window::{ClientWindow, WindowId};
pub use workspace::{next_workspace, Workspaces, WORKSPACE_COUNT};
