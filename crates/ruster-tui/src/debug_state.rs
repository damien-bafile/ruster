//! The debugger surface, lifted out of `App`.
//!
//! Two fields moved here: the live DAP session and the breakpoint table. They
//! belong together because they have an invariant that was previously spread
//! across call sites — **a breakpoint change has to be pushed to a running
//! session**, and forgetting that leaves the debugger stopping at lines the
//! editor no longer shows a marker for, or worse, not stopping where it does.
//!
//! `toggle_breakpoint` now owns that. There is nowhere left to edit the table
//! without the session hearing about it.
//!
//! **What deliberately did not move**: deciding what to *do* when the debugger
//! stops. Jumping the window to a frame, updating the panel, drawing the gutter
//! — those are editor effects, and the same split `lsp_state` follows applies
//! here. This module answers "what is the debugger doing"; `App` decides what
//! the screen should look like as a result.

use ruster_dap::session::DebugSession;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct DebugState {
    session: Option<DebugSession>,
    /// Breakpoints by canonical path. A file with none is removed rather than
    /// left as an empty vector, so `is_empty()` answers "are there any
    /// breakpoints at all" without a scan.
    breakpoints: HashMap<PathBuf, Vec<u16>>,
}

/// Which way a `toggle` went, so the caller can report it without re-reading
/// the table.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Toggle {
    Added,
    Removed,
}

impl DebugState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a debug session is running — the guard on every step command.
    pub fn is_running(&self) -> bool {
        self.session.is_some()
    }

    pub fn session(&self) -> Option<&DebugSession> {
        self.session.as_ref()
    }

    pub fn session_mut(&mut self) -> Option<&mut DebugSession> {
        self.session.as_mut()
    }

    /// Attach a freshly-started session and push the breakpoints already set.
    ///
    /// Pushing on attach is the point: breakpoints are usually placed *before*
    /// the debugger starts, and a session that came up knowing nothing about
    /// them would run straight past every one.
    /// Then `configurationDone`, which is what actually lets the program run —
    /// the adapter holds the `launch` reply until it arrives. Breakpoints go
    /// first so they are in place before the target starts.
    pub fn start(&mut self, session: DebugSession) {
        self.session = Some(session);
        self.push_breakpoints();
        if let Some(session) = &mut self.session {
            session.send_configuration_done().ok();
        }
    }

    /// End the session and forget its breakpoints.
    ///
    /// Returns the session so the caller can shut it down — dropping it here
    /// would hide the process teardown inside a state setter.
    pub fn stop(&mut self) -> Option<DebugSession> {
        self.breakpoints.clear();
        self.session.take()
    }

    pub fn any_breakpoints(&self) -> bool {
        !self.breakpoints.is_empty()
    }

    /// Breakpoint lines for one file, empty if none.
    pub fn breakpoints_in(&self, path: &Path) -> &[u16] {
        self.breakpoints
            .get(path)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Add or remove a breakpoint, and tell a running session about it.
    ///
    /// The push is not optional and not the caller's job. A breakpoint the
    /// editor draws but the debugger has not been told about is worse than no
    /// breakpoint: it looks like the debugger is broken.
    pub fn toggle_breakpoint(&mut self, path: &Path, line: u16) -> Toggle {
        let lines = self.breakpoints.entry(path.to_path_buf()).or_default();
        let outcome = match lines.iter().position(|&l| l == line) {
            Some(pos) => {
                lines.remove(pos);
                if lines.is_empty() {
                    self.breakpoints.remove(path);
                }
                Toggle::Removed
            }
            None => {
                lines.push(line);
                lines.sort_unstable();
                Toggle::Added
            }
        };
        self.push_breakpoints();
        outcome
    }

    /// Send the whole breakpoint table to a running session.
    ///
    /// All of them, not a delta: DAP's `setBreakpoints` replaces the set for a
    /// file, so sending only the changed file would clear every other one.
    fn push_breakpoints(&mut self) {
        let all = self.breakpoint_payload();
        if let Some(session) = &mut self.session {
            session.set_breakpoints_all(all).ok();
        }
    }

    /// Exactly what `push_breakpoints` sends.
    ///
    /// Split out because the send itself needs a live adapter subprocess and so
    /// cannot be unit-tested, whereas *what would be sent* can be.
    pub fn breakpoint_payload(&self) -> Vec<(PathBuf, Vec<u16>)> {
        self.breakpoints
            .iter()
            .map(|(p, ls)| (p.clone(), ls.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn toggling_adds_then_removes() {
        let mut d = DebugState::new();
        let p = path("/src/main.rs");
        assert_eq!(d.toggle_breakpoint(&p, 10), Toggle::Added);
        assert_eq!(d.breakpoints_in(&p), &[10]);
        assert_eq!(d.toggle_breakpoint(&p, 10), Toggle::Removed);
        assert!(d.breakpoints_in(&p).is_empty());
    }

    #[test]
    fn breakpoints_stay_sorted() {
        // The gutter draws them in order and a binary search reads them; an
        // unsorted vector shows markers against the wrong lines.
        let mut d = DebugState::new();
        let p = path("/src/main.rs");
        for line in [30, 10, 20] {
            d.toggle_breakpoint(&p, line);
        }
        assert_eq!(d.breakpoints_in(&p), &[10, 20, 30]);
    }

    #[test]
    fn a_file_with_no_breakpoints_left_is_removed_not_emptied() {
        // `any_breakpoints` is the per-frame test for whether to build a sign
        // column at all. An empty vector left behind makes it answer yes
        // forever after the last breakpoint is cleared.
        let mut d = DebugState::new();
        let p = path("/src/main.rs");
        d.toggle_breakpoint(&p, 1);
        assert!(d.any_breakpoints());
        d.toggle_breakpoint(&p, 1);
        assert!(!d.any_breakpoints(), "an empty entry was left behind");
    }

    #[test]
    fn breakpoints_are_per_file() {
        let mut d = DebugState::new();
        let (a, b) = (path("/src/a.rs"), path("/src/b.rs"));
        d.toggle_breakpoint(&a, 5);
        d.toggle_breakpoint(&b, 7);
        assert_eq!(d.breakpoints_in(&a), &[5]);
        assert_eq!(d.breakpoints_in(&b), &[7]);
        d.toggle_breakpoint(&a, 5);
        assert!(d.breakpoints_in(&a).is_empty());
        assert_eq!(
            d.breakpoints_in(&b),
            &[7],
            "clearing one file cleared another"
        );
    }

    #[test]
    fn an_unknown_file_reports_no_breakpoints() {
        let d = DebugState::new();
        assert!(d.breakpoints_in(&path("/nowhere.rs")).is_empty());
    }

    #[test]
    fn stopping_clears_the_breakpoints_and_the_session() {
        let mut d = DebugState::new();
        d.toggle_breakpoint(&path("/src/main.rs"), 1);
        assert!(d.any_breakpoints());
        assert!(d.stop().is_none(), "there was no session to hand back");
        assert!(!d.any_breakpoints(), "breakpoints outlived the session");
        assert!(!d.is_running());
    }

    #[test]
    fn the_payload_carries_every_file_not_a_delta() {
        // DAP's `setBreakpoints` *replaces* the set for a file. Sending only the
        // file that just changed would clear every breakpoint in all the others.
        let mut d = DebugState::new();
        d.toggle_breakpoint(&path("/src/a.rs"), 1);
        d.toggle_breakpoint(&path("/src/b.rs"), 2);
        d.toggle_breakpoint(&path("/src/a.rs"), 3);

        let mut payload = d.breakpoint_payload();
        payload.sort();
        assert_eq!(
            payload,
            vec![
                (path("/src/a.rs"), vec![1, 3]),
                (path("/src/b.rs"), vec![2])
            ],
            "the payload must describe the whole table"
        );
    }

    /// The bug this module was written to make unrepresentable.
    ///
    /// Breakpoints are placed *before* the debugger starts — that is the normal
    /// order — but `toggle_breakpoint` can only push to a session that already
    /// exists. So `start` has to push, or a fresh session knows about none of
    /// them and runs straight past every one.
    ///
    /// A scrape rather than a behavioural test because the send needs a live
    /// adapter subprocess: `DebugSession` owns a `DapClient` that spawns one,
    /// so there is nothing to assert against in-process. This at least fails
    /// the build if the call is removed.
    #[test]
    fn starting_a_session_pushes_the_breakpoints_already_set() {
        const SRC: &str = include_str!("debug_state.rs");
        let start = SRC.find("pub fn start(").expect("start exists");
        let end = SRC[start..].find("\n    }").expect("start closes") + start;
        assert!(
            SRC[start..end].contains("push_breakpoints()"),
            "DebugState::start no longer pushes the breakpoint table. Breakpoints \
             set before `:DebugStart` — the normal order — would silently never \
             reach the adapter."
        );
    }

    #[test]
    fn toggling_without_a_session_is_fine() {
        // Breakpoints are normally placed before the debugger is started; the
        // push has to be a no-op rather than a precondition.
        let mut d = DebugState::new();
        assert!(!d.is_running());
        d.toggle_breakpoint(&path("/src/main.rs"), 3);
        assert_eq!(d.breakpoints_in(&path("/src/main.rs")), &[3]);
    }
}
