//! Per-buffer git hunks for the sign column, and the background workers that
//! produce them.
//!
//! Four fields moved here: the hunk cache, both ends of the worker channel, and
//! the `git.signs` toggle. The channel ends in particular had no business being
//! two separate fields on `App` — they are one pipe, and a reader that can be
//! separated from its writer is an invitation to clone the wrong one.
//!
//! **What deliberately did not move**: turning hunks into a `SignsView`. That
//! picks colours, which now come from the theme, and belongs with the rest of
//! the drawing code rather than with the data.
//!
//! Refreshes are non-blocking, like the LSP and runner paths: a thread runs
//! `git diff` and writes back through an mpsc channel that the frame loop
//! drains. A repository large enough for `git diff` to take a noticeable moment
//! is exactly the one where blocking the editor would be unacceptable.

use ruster_core::document::BufferId;
use ruster_git::Hunk;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

pub struct GitGutter {
    hunks: HashMap<BufferId, Vec<Hunk>>,
    tx: Sender<(BufferId, Vec<Hunk>)>,
    rx: Receiver<(BufferId, Vec<Hunk>)>,
    /// `git.signs` — whether the gutter shows git state at all.
    enabled: bool,
}

impl Default for GitGutter {
    fn default() -> Self {
        Self::new(true)
    }
}

impl GitGutter {
    pub fn new(enabled: bool) -> Self {
        let (tx, rx) = channel();
        GitGutter {
            hunks: HashMap::new(),
            tx,
            rx,
            enabled,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Turn the gutter on or off, returning the new state.
    ///
    /// Turning it off drops the cache. Keeping stale hunks around to show again
    /// on re-enable would be worse than recomputing: the file has almost
    /// certainly changed in between, and a wrong sign column is harder to
    /// notice than an absent one.
    pub fn set_enabled(&mut self, on: bool) -> bool {
        self.enabled = on;
        if !on {
            self.hunks.clear();
        }
        on
    }

    pub fn hunks(&self, buffer: BufferId) -> &[Hunk] {
        self.hunks
            .get(&buffer)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Start a background `git diff` for `buffer`.
    ///
    /// Does nothing when the gutter is off. Silently does nothing when the file
    /// is untracked, outside a repository, or git is missing — none of which is
    /// an error worth telling anyone about, since the answer is simply "no
    /// hunks".
    pub fn request(&self, buffer: BufferId, path: PathBuf, root: PathBuf) {
        if !self.enabled {
            return;
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            if let Some(hunks) = ruster_git::diff_hunks(&root, &path) {
                let _ = tx.send((buffer, hunks));
            }
        });
    }

    /// Take whatever the workers finished since the last frame.
    ///
    /// Returns how many buffers were updated, so a caller can tell whether a
    /// redraw is warranted.
    pub fn drain(&mut self) -> usize {
        let mut n = 0;
        while let Ok((id, hunks)) = self.rx.try_recv() {
            self.hunks.insert(id, hunks);
            n += 1;
        }
        n
    }

    /// Drop the hunks for a closed buffer.
    ///
    /// This was missing before the extraction: `forget_buffer` swept the dired,
    /// syntax, LSP and terminal caches but not this one, so a long session of
    /// opening and closing files grew the map without bound. Buffer ids are not
    /// reused, so nothing showed the wrong signs — it just never shrank.
    pub fn forget(&mut self, buffer: BufferId) {
        self.hunks.remove(&buffer);
    }

    /// Seed the cache directly, standing in for a worker that finished.
    ///
    /// Test-only: production fills this through [`drain`](Self::drain) and
    /// nothing else should be able to put hunks in without them having come
    /// from a real `git diff`.
    #[cfg(test)]
    pub(crate) fn set_hunks(&mut self, buffer: BufferId, hunks: Vec<Hunk>) {
        self.hunks.insert(buffer, hunks);
    }

    /// How many buffers are cached. For tests and leak checks.
    pub fn tracked(&self) -> usize {
        self.hunks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruster_git::HunkKind;

    fn hunk(start: u32) -> Hunk {
        Hunk {
            kind: HunkKind::Added,
            start,
            count: 1,
        }
    }

    /// Feed the cache directly, standing in for a worker that finished.
    fn deliver(g: &GitGutter, buffer: BufferId, hunks: Vec<Hunk>) {
        g.tx.send((buffer, hunks)).expect("the receiver is alive");
    }

    #[test]
    fn draining_moves_worker_results_into_the_cache() {
        let mut g = GitGutter::new(true);
        let buf = BufferId(1);
        assert!(g.hunks(buf).is_empty());
        deliver(&g, buf, vec![hunk(3)]);
        assert_eq!(g.drain(), 1);
        assert_eq!(g.hunks(buf).len(), 1);
    }

    #[test]
    fn draining_an_empty_channel_reports_no_work() {
        // The frame loop calls this every frame; it must be cheap and must not
        // claim a redraw is needed when nothing arrived.
        let mut g = GitGutter::new(true);
        assert_eq!(g.drain(), 0);
    }

    #[test]
    fn a_later_result_replaces_an_earlier_one() {
        // Two refreshes can be in flight for the same buffer. The last to
        // arrive wins rather than accumulating, or the gutter would show the
        // union of every diff since the file was opened.
        let mut g = GitGutter::new(true);
        let buf = BufferId(1);
        deliver(&g, buf, vec![hunk(1), hunk(2)]);
        deliver(&g, buf, vec![hunk(5)]);
        g.drain();
        assert_eq!(
            g.hunks(buf).len(),
            1,
            "results accumulated instead of replacing"
        );
        assert_eq!(g.hunks(buf)[0].start, 5);
    }

    #[test]
    fn turning_the_gutter_off_drops_the_cache() {
        let mut g = GitGutter::new(true);
        deliver(&g, BufferId(1), vec![hunk(1)]);
        g.drain();
        assert_eq!(g.tracked(), 1);
        g.set_enabled(false);
        assert_eq!(g.tracked(), 0, "stale hunks survived being switched off");
        assert!(g.hunks(BufferId(1)).is_empty());
    }

    #[test]
    fn a_disabled_gutter_starts_no_work() {
        // `request` spawns a thread and shells out to git. With signs off that
        // is pure waste on every buffer switch.
        let g = GitGutter::new(false);
        g.request(BufferId(1), PathBuf::from("/x.rs"), PathBuf::from("/"));
        // Nothing was queued, so a drain finds nothing even after a moment.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut g = g;
        assert_eq!(g.drain(), 0);
    }

    #[test]
    fn forgetting_a_buffer_frees_its_hunks() {
        let mut g = GitGutter::new(true);
        deliver(&g, BufferId(1), vec![hunk(1)]);
        deliver(&g, BufferId(2), vec![hunk(2)]);
        g.drain();
        assert_eq!(g.tracked(), 2);
        g.forget(BufferId(1));
        assert_eq!(g.tracked(), 1, "the closed buffer's hunks leaked");
        assert_eq!(
            g.hunks(BufferId(2)).len(),
            1,
            "the wrong buffer was forgotten"
        );
    }

    #[test]
    fn an_unknown_buffer_has_no_hunks() {
        let g = GitGutter::new(true);
        assert!(g.hunks(BufferId(99)).is_empty());
    }
}
