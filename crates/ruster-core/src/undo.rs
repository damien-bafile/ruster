use crate::buffer::{Buffer, Change};

/// One state in the undo tree. `undo` holds the changes that move this state
/// back to its parent; `redo` holds the changes that move the parent to here.
/// Exactly one of the two is valid at a time depending on which side of the
/// current position the node sits, and traversal refreshes the other.
struct Node {
    parent: Option<usize>,
    children: Vec<usize>,
    undo: Vec<Change>,
    redo: Vec<Change>,
    /// Creation order, for chronological (`g-` / `g+`) navigation.
    seq: usize,
}

/// An undo **tree**: undoing and then editing again branches rather than
/// discarding the redo history, so no state is ever lost. `undo`/`redo` walk
/// the current branch; [`UndoStack::undo_time`] walks every state in the order
/// it was created.
pub struct UndoStack {
    nodes: Vec<Node>,
    current: usize,
    open: Vec<Change>,
    next_seq: usize,
}

impl UndoStack {
    pub fn new() -> Self {
        UndoStack {
            nodes: vec![Node {
                parent: None,
                children: Vec::new(),
                undo: Vec::new(),
                redo: Vec::new(),
                seq: 0,
            }],
            current: 0,
            open: Vec::new(),
            next_seq: 1,
        }
    }

    /// True when nothing has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.nodes.len() == 1 && self.open.is_empty()
    }

    /// Number of recorded states (excluding the root).
    pub fn len(&self) -> usize {
        self.nodes.len() - 1
    }

    pub fn begin_batch(&mut self) {
        self.commit_open();
    }

    pub fn push(&mut self, ch: Change) {
        self.open.push(ch);
    }

    pub fn end_batch(&mut self) {
        self.commit_open();
    }

    /// Close the open batch into a new child of the current state.
    fn commit_open(&mut self) {
        if self.open.is_empty() {
            return;
        }
        let undo = std::mem::take(&mut self.open);
        let seq = self.next_seq;
        self.next_seq += 1;
        let idx = self.nodes.len();
        self.nodes.push(Node {
            parent: Some(self.current),
            children: Vec::new(),
            undo,
            redo: Vec::new(),
            seq,
        });
        self.nodes[self.current].children.push(idx);
        self.current = idx;
    }

    /// Step from `node` up to its parent, applying that node's undo changes.
    /// Returns the position the edit touched.
    fn step_up(&mut self, buffer: &mut Buffer, node: usize) -> Option<(usize, usize)> {
        let changes = self.nodes[node].undo.clone();
        if changes.is_empty() {
            return None;
        }
        let mut inverses = Vec::with_capacity(changes.len());
        for ch in changes.iter().rev() {
            inverses.push(buffer.apply(ch));
        }
        inverses.reverse();
        let at = inverses[0].at;
        let n = inverses.len();
        self.nodes[node].redo = inverses;
        self.current = self.nodes[node].parent.expect("non-root has a parent");
        Some((n, at))
    }

    /// Step from the current state down into `child`, applying its redo changes.
    fn step_down(&mut self, buffer: &mut Buffer, child: usize) -> Option<(usize, usize)> {
        let changes = self.nodes[child].redo.clone();
        if changes.is_empty() {
            return None;
        }
        let mut inverses = Vec::with_capacity(changes.len());
        for ch in changes.iter().rev() {
            inverses.push(buffer.apply(ch));
        }
        inverses.reverse();
        let at = inverses[0].at;
        let n = inverses.len();
        self.nodes[child].undo = inverses;
        self.current = child;
        Some((n, at))
    }

    /// Undo the change that produced the current state.
    pub fn undo(&mut self, buffer: &mut Buffer) -> Option<(usize, usize)> {
        self.commit_open(); // an open batch is undoable too
        if self.current == 0 {
            return None;
        }
        let node = self.current;
        self.step_up(buffer, node)
    }

    /// Redo along the most recently created branch.
    pub fn redo(&mut self, buffer: &mut Buffer) -> Option<(usize, usize)> {
        let child = *self.nodes[self.current].children.last()?;
        self.step_down(buffer, child)
    }

    /// Move to the state created just before (`forward == false`) or just after
    /// (`forward == true`) the current one, in creation order — `g-` / `g+`.
    /// This reaches states on other branches that plain redo cannot.
    pub fn undo_time(&mut self, buffer: &mut Buffer, forward: bool) -> Option<(usize, usize)> {
        self.commit_open();
        let cur_seq = self.nodes[self.current].seq;
        // The nearest sequence number in the requested direction.
        let target = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                if forward {
                    n.seq > cur_seq
                } else {
                    n.seq < cur_seq
                }
            })
            .min_by_key(|(_, n)| {
                if forward {
                    n.seq - cur_seq
                } else {
                    cur_seq - n.seq
                }
            })
            .map(|(i, _)| i)?;
        self.travel_to(buffer, target)
    }

    /// Walk from the current state to `target` via their common ancestor.
    fn travel_to(&mut self, buffer: &mut Buffer, target: usize) -> Option<(usize, usize)> {
        let up_path = self.path_to_root(self.current);
        let down_path = self.path_to_root(target);
        // Deepest node present in both paths.
        let ancestor = up_path
            .iter()
            .find(|n| down_path.contains(n))
            .copied()
            .unwrap_or(0);

        let mut last = None;
        while self.current != ancestor {
            let node = self.current;
            last = self.step_up(buffer, node).or(last);
        }
        // Descend from the ancestor to the target.
        let mut descent: Vec<usize> = down_path
            .into_iter()
            .take_while(|&n| n != ancestor)
            .collect();
        descent.reverse();
        for node in descent {
            last = self.step_down(buffer, node).or(last);
        }
        last
    }

    /// The chain of nodes from `node` up to (and including) the root.
    fn path_to_root(&self, node: usize) -> Vec<usize> {
        let mut path = vec![node];
        let mut cur = node;
        while let Some(parent) = self.nodes[cur].parent {
            path.push(parent);
            cur = parent;
        }
        path
    }
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack_with_editor() -> (Buffer, UndoStack) {
        (Buffer::from_str("abc"), UndoStack::new())
    }

    #[test]
    fn batched_inserts_undo_as_one_unit() {
        let (mut b, mut u) = stack_with_editor();
        u.begin_batch();
        u.push(b.insert(3, "!"));
        u.push(b.insert(4, "?"));
        u.end_batch();
        assert_eq!(b.to_string(), "abc!?");
        let (n, at) = u.undo(&mut b).unwrap();
        assert_eq!(n, 2);
        assert_eq!(at, 3);
        assert_eq!(b.to_string(), "abc");
    }

    #[test]
    fn new_batch_closes_previous() {
        let (mut b, mut u) = stack_with_editor();
        u.begin_batch();
        u.push(b.insert(3, "!"));
        u.begin_batch(); // opening another batch auto-closes the prior open
        u.push(b.insert(4, "?"));
        u.end_batch();
        assert_eq!(b.to_string(), "abc!?");
        u.undo(&mut b);
        assert_eq!(b.to_string(), "abc!");
        u.undo(&mut b);
        assert_eq!(b.to_string(), "abc");
    }

    #[test]
    fn redo_reapplies_undone_batch() {
        let (mut b, mut u) = stack_with_editor();
        u.begin_batch();
        u.push(b.insert(3, "!"));
        u.end_batch();
        u.undo(&mut b);
        assert_eq!(b.to_string(), "abc");
        let (n, at) = u.redo(&mut b).unwrap();
        assert_eq!(n, 1);
        assert_eq!(at, 3);
        assert_eq!(b.to_string(), "abc!");
    }

    #[test]
    fn undo_at_root_returns_none() {
        let (mut b, mut u) = stack_with_editor();
        assert!(u.undo(&mut b).is_none());
        assert!(u.redo(&mut b).is_none());
    }

    #[test]
    fn branching_keeps_the_old_state_reachable() {
        // Type "!", undo it, then type "?" — a linear stack would discard "!".
        let (mut b, mut u) = stack_with_editor();
        u.begin_batch();
        u.push(b.insert(3, "!"));
        u.end_batch();
        u.undo(&mut b);
        assert_eq!(b.to_string(), "abc");

        u.begin_batch();
        u.push(b.insert(3, "?"));
        u.end_batch();
        assert_eq!(b.to_string(), "abc?");
        // Both edits are still recorded as separate states.
        assert_eq!(u.len(), 2);

        // Walking back through time reaches the abandoned "!" branch, which a
        // plain undo could never return to.
        u.undo_time(&mut b, false); // the state before "abc?" is "abc!"
        assert_eq!(b.to_string(), "abc!");
        u.undo_time(&mut b, false); // and before that, the original
        assert_eq!(b.to_string(), "abc");
    }

    #[test]
    fn undo_time_forward_returns_to_the_newer_branch() {
        let (mut b, mut u) = stack_with_editor();
        u.begin_batch();
        u.push(b.insert(3, "!"));
        u.end_batch();
        u.undo(&mut b);
        u.begin_batch();
        u.push(b.insert(3, "?"));
        u.end_batch();
        assert_eq!(b.to_string(), "abc?");

        u.undo_time(&mut b, false);
        assert_eq!(b.to_string(), "abc!", "walked back onto the older branch");
        u.undo_time(&mut b, true);
        assert_eq!(b.to_string(), "abc?", "and forward again to the newer one");
        assert!(
            u.undo_time(&mut b, true).is_none(),
            "nothing newer than the latest state"
        );
    }
}
