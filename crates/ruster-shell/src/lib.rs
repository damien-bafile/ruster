//! Shell state for the ruster Wayland compositor.
//!
//! [`tree`] is the container tree — how windows divide an output between them —
//! and is where Phase 1's layout lives. [`state`] still carries the Phase 0 flat
//! window list, focus and workspace counter; the tasks that follow move focus
//! and workspaces onto the tree and it shrinks to the window records.

pub mod state;
pub mod tree;
pub mod window;

pub use state::{next_workspace, ShellState, WORKSPACE_COUNT};
pub use tree::{Layout, Node, NodeId, Rect, Tree};
pub use window::{ClientWindow, WindowId};
