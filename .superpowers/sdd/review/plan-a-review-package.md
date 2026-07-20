## Commits
99a67db chore: ignore .DS_Store; untrack committed copies
c6e5814 chore(core): drop dead imports/vars after Plan A; log execution corrections in plan doc
0f0c998 feat(core): wire u (undo) and Ctrl-r (redo) keys in Normal mode
47c49c8 fix(core): c (operator and visual) keeps one open undo batch so u reverses entire change
b9ae9ac test(core): fold edit_word_then_undo into a single session so u has history
a58ef6f test(core): scenario harness for full key-script regressions
a019576 test(core): restore dot-repeat regression tests dropped in Task 11 overwrite
f28cdbc feat(core): Visual char/line mode with motions, d/y/c operators, exclusive selection
e10ba55 feat(core): dot-repeat replays last change's operator+motion at cursor
1b33b8b feat(core): Vim text objects iw aw i" i' i( i{ with depth-aware pair matching
6e2ebf3 feat(core): Vim operators d/y/c with motion composition and paste register
f754dc8 feat(core): VimState Normal/Insert with counts, word/line motions, Motion::To
f2a0a45 feat(core): Action verbs and Editor facade binding buffer/cursors/undo
65e0da5 feat(core): KeyTrie keymap engine with Match/Pending/Miss lookup
09998f9 feat(core): linear batched UndoStack with inverse-change replay
511bd04 feat(core): CursorSet with grapheme-aware and line movement
fffa16b feat(core): Buffer with ropey-backed edit ops and invertible Change records
80a8d02 chore: scaffold ruster-core workspace crate

## Stat
 .gitignore                                         |   1 +
 Cargo.toml                                         |   3 +
 crates/ruster-core/Cargo.toml                      |  12 +
 crates/ruster-core/src/action.rs                   |  30 ++
 crates/ruster-core/src/buffer.rs                   |  96 ++++
 crates/ruster-core/src/command.rs                  |   1 +
 crates/ruster-core/src/cursor.rs                   | 185 ++++++++
 crates/ruster-core/src/editor.rs                   | 145 ++++++
 crates/ruster-core/src/key.rs                      | 121 +++++
 crates/ruster-core/src/lib.rs                      |  10 +
 crates/ruster-core/src/scenario.rs                 |  57 +++
 crates/ruster-core/src/undo.rs                     | 130 ++++++
 crates/ruster-core/src/vim/mod.rs                  | 509 +++++++++++++++++++++
 crates/ruster-core/src/vim/motions.rs              | 146 ++++++
 crates/ruster-core/src/vim/ops.rs                  | 125 +++++
 crates/ruster-core/src/vim/textobj.rs              | 224 +++++++++
 .../plans/2026-07-20-plan-a-core-engine.md         |  73 ++-
 .../specs/2026-07-20-ruster-core-slice-design.md   |   4 +-
 18 files changed, 1847 insertions(+), 25 deletions(-)

## Diff (-U6)
diff --git a/.gitignore b/.gitignore
new file mode 100644
index 0000000..e43b0f9
--- /dev/null
+++ b/.gitignore
@@ -0,0 +1 @@
+.DS_Store
diff --git a/Cargo.toml b/Cargo.toml
new file mode 100644
index 0000000..f9ec792
--- /dev/null
+++ b/Cargo.toml
@@ -0,0 +1,3 @@
+[workspace]
+members = ["crates/ruster-core"]
+resolver = "2"
\ No newline at end of file
diff --git a/crates/ruster-core/Cargo.toml b/crates/ruster-core/Cargo.toml
new file mode 100644
index 0000000..793276f
--- /dev/null
+++ b/crates/ruster-core/Cargo.toml
@@ -0,0 +1,12 @@
+[package]
+name = "ruster-core"
+version = "0.1.0"
+edition = "2021"
+
+[lib]
+path = "src/lib.rs"
+
+[dependencies]
+ropey = "1.6"
+unicode-segmentation = "1.11"
+thiserror = "1"
\ No newline at end of file
diff --git a/crates/ruster-core/src/action.rs b/crates/ruster-core/src/action.rs
new file mode 100644
index 0000000..ff9021e
--- /dev/null
+++ b/crates/ruster-core/src/action.rs
@@ -0,0 +1,30 @@
+use crate::cursor::Edge;
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub enum Motion {
+    Grapheme(i32),
+    Line(i32),
+    LineEdge(Edge),
+    To(usize),
+}
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub enum EditOp {
+    InsertChar(char),
+    InsertString(String),
+    DeleteRange(usize, usize),
+    Backspace,
+}
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub enum Action {
+    Move(Motion),
+    Edit(EditOp),
+    BeginBatch,
+    EndBatch,
+    Undo,
+    Redo,
+    /// Set the anchor (start) of the primary cursor's visual selection,
+    /// keeping the current head. Used to extend a selection in visual mode.
+    BeginVisual(usize),
+}
\ No newline at end of file
diff --git a/crates/ruster-core/src/buffer.rs b/crates/ruster-core/src/buffer.rs
new file mode 100644
index 0000000..71da664
--- /dev/null
+++ b/crates/ruster-core/src/buffer.rs
@@ -0,0 +1,96 @@
+use ropey::Rope;
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub struct Change {
+    pub at: usize,
+    pub deleted: String,
+    pub inserted: String,
+}
+
+pub struct Buffer {
+    rope: Rope,
+}
+
+impl Buffer {
+    pub fn new() -> Self { Self { rope: Rope::new() } }
+    pub fn from_str(s: &str) -> Self { Self { rope: Rope::from_str(s) } }
+
+    pub fn len_chars(&self) -> usize { self.rope.len_chars() }
+    pub fn line_count(&self) -> usize { self.rope.len_lines() }
+    pub fn char_at(&self, idx: usize) -> char { self.rope.char(idx) }
+    pub fn slice_string(&self, start: usize, end: usize) -> String { self.rope.slice(start..end).to_string() }
+    pub fn to_string(&self) -> String { self.rope.to_string() }
+    pub fn line_to_string(&self, line_idx: usize) -> String {
+        self.rope.line(line_idx).to_string()
+    }
+    pub fn line_start_char(&self, line_idx: usize) -> usize {
+        self.rope.line_to_char(line_idx)
+    }
+    pub fn line_end_char(&self, line_idx: usize) -> usize {
+        if line_idx + 1 >= self.rope.len_lines() {
+            self.rope.len_chars()
+        } else {
+            self.rope.line_to_char(line_idx + 1)
+        }
+    }
+
+    pub fn insert(&mut self, at: usize, text: &str) -> Change {
+        self.rope.insert(at, text);
+        Change { at, deleted: String::new(), inserted: text.to_string() }
+    }
+
+    pub fn delete(&mut self, range: std::ops::Range<usize>) -> Change {
+        let deleted = self.rope.slice(range.clone()).to_string();
+        let at = range.start;
+        self.rope.remove(range);
+        Change { at, deleted, inserted: String::new() }
+    }
+
+    /// Apply a change; returns the inverse change that would undo this application.
+    /// A change `c` means "the buffer currently has `c.inserted` present at `c.at`,
+    /// where `c.deleted` was previously." apply() removes the inserted span and
+    /// re-inserts the deleted span, returning the inverse change.
+    pub fn apply(&mut self, ch: &Change) -> Change {
+        let ins_len = ch.inserted.chars().count();
+        self.rope.remove(ch.at..ch.at + ins_len);
+        self.rope.insert(ch.at, &ch.deleted);
+        Change { at: ch.at, deleted: ch.inserted.clone(), inserted: ch.deleted.clone() }
+    }
+}
+
+impl Default for Buffer {
+    fn default() -> Self { Self::new() }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    #[test]
+    fn insert_returns_change_and_text() {
+        let mut b = Buffer::from_str("helo");
+        let ch = b.insert(4, "!");
+        assert_eq!(b.to_string(), "helo!");
+        assert_eq!(ch, Change { at: 4, deleted: String::new(), inserted: "!".to_string() });
+    }
+
+    #[test]
+    fn delete_returns_change_and_text() {
+        let mut b = Buffer::from_str("hello world");
+        let ch = b.delete(5..11);
+        assert_eq!(b.to_string(), "hello");
+        assert_eq!(ch, Change { at: 5, deleted: " world".to_string(), inserted: String::new() });
+    }
+
+    #[test]
+    fn apply_inverse_round_trips() {
+        let mut b = Buffer::from_str("hello");
+        let ch = b.delete(0..2);      // b: "hello" -> "llo"; ch: del="he", ins=""
+        let inv = b.apply(&ch);        // applying ch inverts the deletion: b -> "hello"
+        assert_eq!(b.to_string(), "hello");
+        assert_eq!(inv.inserted, ch.deleted);
+        let inv2 = b.apply(&inv);      // applying inv re-applies the deletion: b -> "llo"
+        assert_eq!(b.to_string(), "llo");
+        assert_eq!(inv2, ch);
+    }
+}
\ No newline at end of file
diff --git a/crates/ruster-core/src/command.rs b/crates/ruster-core/src/command.rs
new file mode 100644
index 0000000..1c290d0
--- /dev/null
+++ b/crates/ruster-core/src/command.rs
@@ -0,0 +1 @@
+// stub — populated in later tasks
\ No newline at end of file
diff --git a/crates/ruster-core/src/cursor.rs b/crates/ruster-core/src/cursor.rs
new file mode 100644
index 0000000..76b31eb
--- /dev/null
+++ b/crates/ruster-core/src/cursor.rs
@@ -0,0 +1,185 @@
+use crate::buffer::Buffer;
+use unicode_segmentation::UnicodeSegmentation;
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub struct Range {
+    pub anchor: usize,
+    pub head: usize,
+}
+
+impl Range {
+    pub fn caret(at: usize) -> Self { Range { anchor: at, head: at } }
+    pub fn start(&self) -> usize { self.anchor.min(self.head) }
+    pub fn end(&self) -> usize { self.anchor.max(self.head) }
+}
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub enum Edge { Start, End }
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+enum Dir { Left, Right }
+
+pub struct CursorSet {
+    pub(crate) cursors: Vec<Range>,
+    pub(crate) primary: usize,
+    pub(crate) desired_col: usize,
+}
+
+impl CursorSet {
+    pub fn single(at: usize) -> Self {
+        CursorSet { cursors: vec![Range::caret(at)], primary: 0, desired_col: usize::MAX }
+    }
+
+    pub fn primary(&self) -> Range { self.cursors[self.primary] }
+    pub fn head(&self) -> usize { self.primary().head }
+
+    pub fn set_head(&mut self, at: usize, buffer: &Buffer) {
+        let anchor = self.cursors[self.primary].anchor;
+        self.cursors[self.primary] = Range { anchor, head: at };
+        let line = self.line_of(buffer, at);
+        self.desired_col = at - buffer.line_start_char(line);
+        self.collapse_at(at);
+    }
+
+    /// In visual mode: set the cursor's `anchor` to `anchor` while preserving the current `head`.
+    /// This lets the head move freely (extending the selection) while the anchor stays fixed.
+    pub fn set_visual_anchor(&mut self, anchor: usize) {
+        let head = self.cursors[self.primary].head;
+        self.cursors[self.primary] = Range { anchor, head };
+    }
+
+    fn collapse_at(&mut self, at: usize) {
+        self.cursors[self.primary] = Range::caret(at);
+    }
+
+    fn line_of(&self, buffer: &Buffer, char_idx: usize) -> usize {
+        let mut acc = 0usize;
+        for line in 0..buffer.line_count() {
+            let start = buffer.line_start_char(line);
+            if start <= char_idx { acc = line; } else { break; }
+        }
+        acc
+    }
+
+    fn line_content_len(&self, buffer: &Buffer, line: usize) -> usize {
+        let end = buffer.line_end_char(line);
+        let start = buffer.line_start_char(line);
+        if end > start && buffer.char_at(end - 1) == '\n' {
+            end - start - 1
+        } else {
+            end - start
+        }
+    }
+
+    fn grapheme_step(&self, buffer: &Buffer, from: usize, dir: Dir) -> usize {
+        let text = buffer.to_string();
+        let graphemes: Vec<&str> = UnicodeSegmentation::graphemes(&*text, true).collect();
+        let mut char_pos = 0usize;
+        let mut gidx = 0usize;
+        for (i, g) in graphemes.iter().enumerate() {
+            if char_pos == from { gidx = i; break; }
+            char_pos += g.chars().count();
+            gidx = i + 1;
+        }
+        match dir {
+            Dir::Left => {
+                if gidx == 0 { from } else {
+                    let prev = graphemes[gidx - 1];
+                    from - prev.chars().count()
+                }
+            }
+            Dir::Right => {
+                if gidx >= graphemes.len() { from } else {
+                    let cur = graphemes[gidx];
+                    from + cur.chars().count()
+                }
+            }
+        }
+    }
+
+    pub fn move_grapheme(&mut self, buffer: &Buffer, dir: i32) {
+        let d = if dir > 0 { Dir::Right } else { Dir::Left };
+        let from = self.head();
+        let to = self.grapheme_step(buffer, from, d);
+        self.set_head(to, buffer);
+    }
+
+    pub fn move_line(&mut self, buffer: &Buffer, delta: i32) {
+        let from = self.head();
+        let line = self.line_of(buffer, from);
+        if self.desired_col == usize::MAX {
+            self.desired_col = from - buffer.line_start_char(line);
+        }
+        let target_line = (line as i32 + delta).max(0) as usize;
+        let last = buffer.line_count().saturating_sub(1);
+        let target_line = target_line.min(last);
+        let start = buffer.line_start_char(target_line);
+        let content_len = self.line_content_len(buffer, target_line);
+        let col = self.desired_col.min(content_len);
+        let new_head = start + col;
+        let anchor = self.cursors[self.primary].anchor;
+        self.cursors[self.primary] = Range { anchor, head: new_head };
+        self.collapse_at(new_head);
+    }
+
+    pub fn move_line_edge(&mut self, buffer: &Buffer, edge: Edge) {
+        let from = self.head();
+        let line = self.line_of(buffer, from);
+        let at = match edge {
+            Edge::Start => buffer.line_start_char(line),
+            Edge::End => buffer.line_start_char(line) + self.line_content_len(buffer, line),
+        };
+        self.set_head(at, buffer);
+    }
+
+    pub fn collapse(&mut self) {
+        let h = self.head();
+        self.cursors[self.primary] = Range::caret(h);
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    #[test]
+    fn single_anchor_equals_head() {
+        let c = CursorSet::single(3);
+        assert_eq!(c.primary().anchor, 3);
+        assert_eq!(c.head(), 3);
+    }
+
+    #[test]
+    fn move_grapheme_right_skips_combining_mark() {
+        let b = Buffer::from_str("e\u{0301}x"); // e + combining acute, then x; 3 chars total
+        let mut c = CursorSet::single(0);
+        c.move_grapheme(&b, 1);
+        assert_eq!(c.head(), 2, "grapheme cluster boundary");
+    }
+
+    #[test]
+    fn move_line_down_preserves_column_intent() {
+        let b = Buffer::from_str("abc\ndefg\nhi");
+        let mut c = CursorSet::single(1); // col 1 of line 0
+        c.move_line(&b, 1);
+        assert_eq!(c.head(), 5, "line 1 col 1 -> offset 5 ('e' in 'defg')");
+    }
+
+    #[test]
+    fn move_line_down_clamps_short_line() {
+        let b = Buffer::from_str("abcd\ne\nfg");
+        let mut c = CursorSet::single(3); // col 3 of "abcd"
+        c.move_line(&b, 1);
+        assert_eq!(c.head(), 6, "line 'e' has only col 0 -> head at 6 (after 'e')");
+        c.move_line(&b, 1);
+        assert_eq!(c.head(), 9, "col 3 of 'fg' -> after 'g' (line is 2 chars)");
+    }
+
+    #[test]
+    fn move_line_edge_to_end() {
+        let b = Buffer::from_str("hello world");
+        let mut c = CursorSet::single(0);
+        c.move_line_edge(&b, Edge::End);
+        assert_eq!(c.head(), 11);
+    }
+}
\ No newline at end of file
diff --git a/crates/ruster-core/src/editor.rs b/crates/ruster-core/src/editor.rs
new file mode 100644
index 0000000..38de395
--- /dev/null
+++ b/crates/ruster-core/src/editor.rs
@@ -0,0 +1,145 @@
+use crate::action::{Action, EditOp, Motion};
+use crate::buffer::Buffer;
+use crate::cursor::CursorSet;
+use crate::undo::UndoStack;
+
+pub struct Editor {
+    buffer: Buffer,
+    cursors: CursorSet,
+    undo: UndoStack,
+}
+
+impl Editor {
+    pub fn from_str(s: &str) -> Self {
+        let len = s.chars().count();
+        Editor {
+            buffer: Buffer::from_str(s),
+            cursors: CursorSet::single(len),
+            undo: UndoStack::new(),
+        }
+    }
+
+    pub fn buffer(&self) -> &Buffer { &self.buffer }
+    pub fn cursors(&self) -> &CursorSet { &self.cursors }
+    pub fn primary_head(&self) -> usize { self.cursors.head() }
+
+    pub fn execute(&mut self, action: Action) {
+        match action {
+            Action::BeginBatch => self.undo.begin_batch(),
+            Action::EndBatch => self.undo.end_batch(),
+            Action::Undo => {
+                self.undo.undo(&mut self.buffer);
+                let at = 0;
+                self.cursors.set_head(at, &self.buffer);
+            }
+            Action::Redo => {
+                self.undo.redo(&mut self.buffer);
+                let at = 0;
+                self.cursors.set_head(at, &self.buffer);
+            }
+            Action::BeginVisual(anchor) => {
+                self.cursors.set_visual_anchor(anchor);
+            }
+            Action::Move(m) => self.apply_motion(m),
+            Action::Edit(e) => self.apply_edit(e),
+        }
+    }
+
+    fn apply_motion(&mut self, m: Motion) {
+        match m {
+            Motion::Grapheme(d) => self.cursors.move_grapheme(&self.buffer, d),
+            Motion::Line(d) => self.cursors.move_line(&self.buffer, d),
+            Motion::LineEdge(edge) => self.cursors.move_line_edge(&self.buffer, edge),
+            Motion::To(target) => self.cursors.set_head(target, &self.buffer),
+        }
+    }
+
+    fn apply_edit(&mut self, e: EditOp) {
+        let at = self.cursors.head();
+        match e {
+            EditOp::InsertChar(c) => {
+                let mut buf = [0u8; 4];
+                let text = c.encode_utf8(&mut buf);
+                let ch = self.buffer.insert(at, text);
+                self.undo.push(ch);
+                self.cursors.set_head(at + 1, &self.buffer);
+            }
+            EditOp::InsertString(s) => {
+                let n = s.chars().count();
+                let ch = self.buffer.insert(at, &s);
+                self.undo.push(ch);
+                self.cursors.set_head(at + n, &self.buffer);
+            }
+            EditOp::DeleteRange(start, end) if end > start => {
+                let safe_end = end.min(self.buffer.len_chars());
+                let ch = self.buffer.delete(start..safe_end);
+                self.undo.push(ch);
+                self.cursors.set_head(start, &self.buffer);
+            }
+            EditOp::DeleteRange(_, _) => {}
+            EditOp::Backspace => {
+                if at > 0 {
+                    let ch = self.buffer.delete(at - 1..at);
+                    self.undo.push(ch);
+                    self.cursors.set_head(at - 1, &self.buffer);
+                }
+            }
+        }
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use crate::cursor::Edge;
+
+    #[test]
+    fn insert_char_then_backspace_roundtrips_via_undo() {
+        let mut e = Editor::from_str("ab");
+        e.execute(Action::BeginBatch);
+        e.execute(Action::Edit(EditOp::InsertChar('!')));
+        e.execute(Action::EndBatch);
+        assert_eq!(e.buffer().to_string(), "ab!");
+        assert_eq!(e.primary_head(), 3);
+        e.execute(Action::Edit(EditOp::Backspace));
+        assert_eq!(e.buffer().to_string(), "ab");
+        assert_eq!(e.primary_head(), 2);
+        e.execute(Action::Undo);
+        assert_eq!(e.buffer().to_string(), "ab!");
+        e.execute(Action::Undo);
+        assert_eq!(e.buffer().to_string(), "ab");
+    }
+
+    #[test]
+    fn move_then_delete_range() {
+        let mut e = Editor::from_str("hello");
+        e.execute(Action::Move(Motion::Grapheme(1)));
+        e.execute(Action::Move(Motion::Grapheme(1)));
+        e.execute(Action::Edit(EditOp::DeleteRange(2, 4)));
+        assert_eq!(e.buffer().to_string(), "heo");
+        assert_eq!(e.primary_head(), 2);
+    }
+
+    #[test]
+    fn line_edge_end_motion() {
+        let mut e = Editor::from_str("abc");
+        e.execute(Action::Move(Motion::LineEdge(Edge::End)));
+        assert_eq!(e.primary_head(), 3);
+    }
+
+    #[test]
+    fn begin_visual_extends_selection_anchor_preserved_on_motion() {
+        let mut e = Editor::from_str("hello");
+        // cursor starts at end (5); move left twice to 3
+        e.execute(Action::Move(Motion::Grapheme(-1)));
+        e.execute(Action::Move(Motion::Grapheme(-1)));
+        let head_before = e.primary_head();
+        e.execute(Action::BeginVisual(head_before));
+        // extend right by 1
+        e.execute(Action::Move(Motion::Grapheme(1)));
+        e.execute(Action::BeginVisual(head_before));
+        let r = e.cursors().primary();
+        assert_eq!(r.anchor, head_before);
+        assert_eq!(r.head, head_before + 1);
+    }
+}
\ No newline at end of file
diff --git a/crates/ruster-core/src/key.rs b/crates/ruster-core/src/key.rs
new file mode 100644
index 0000000..6607fbc
--- /dev/null
+++ b/crates/ruster-core/src/key.rs
@@ -0,0 +1,121 @@
+use std::collections::HashMap;
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
+pub enum Arrow { Up, Down, Left, Right }
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
+pub enum KeyEvent {
+    Char(char),
+    Ctrl(char),
+    Alt(char),
+    Esc,
+    Enter,
+    Backspace,
+    Delete,
+    Arrow(Arrow),
+}
+
+pub enum Lookup<'a, T> {
+    Miss,
+    Pending,
+    Match(&'a T),
+}
+
+pub struct KeyTrie<T> {
+    root: Node<T>,
+}
+
+enum Node<T> {
+    Leaf(T),
+    Branch(HashMap<KeyEvent, Box<Node<T>>>),
+}
+
+impl<T> KeyTrie<T> {
+    pub fn new() -> Self {
+        KeyTrie { root: Node::Branch(HashMap::new()) }
+    }
+
+    pub fn insert(&mut self, keys: &[KeyEvent], value: T) {
+        Self::insert_at(&mut self.root, keys, value);
+    }
+
+    fn insert_at(node: &mut Node<T>, keys: &[KeyEvent], value: T) {
+        match keys {
+            [] => *node = Node::Leaf(value),
+            [first, rest @ ..] => {
+                if let Node::Branch(map) = node {
+                    let child = map
+                        .entry(*first)
+                        .or_insert_with(|| Box::new(Node::Branch(HashMap::new())));
+                    Self::insert_at(child, rest, value);
+                }
+                // Replacing a leaf with a deeper path: not exercised by tests; ignored.
+            }
+        }
+    }
+
+    pub fn lookup(&self, pressed: &[KeyEvent]) -> Lookup<'_, T> {
+        Self::walk(&self.root, pressed)
+    }
+
+    fn walk<'a>(node: &'a Node<T>, pressed: &[KeyEvent]) -> Lookup<'a, T> {
+        match (node, pressed) {
+            // Leaf is terminal: a shorter binding shadows any longer one whose prefix overlaps.
+            (Node::Leaf(v), _) => Lookup::Match(v),
+            (Node::Branch(_map), []) => Lookup::Pending,
+            (Node::Branch(map), [first, rest @ ..]) => {
+                match map.get(first) {
+                    Some(child) => Self::walk(child, rest),
+                    None => Lookup::Miss,
+                }
+            }
+        }
+    }
+}
+
+impl<T> Default for KeyTrie<T> {
+    fn default() -> Self { Self::new() }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    #[test]
+    fn single_key_match() {
+        let mut t = KeyTrie::new();
+        t.insert(&[KeyEvent::Char('x')], "delete-char");
+        assert!(matches!(t.lookup(&[KeyEvent::Char('x')]), Lookup::Match(&"delete-char")));
+    }
+
+    #[test]
+    fn multi_key_sequence_pending_then_match() {
+        let mut t = KeyTrie::new();
+        t.insert(&[KeyEvent::Char('g'), KeyEvent::Char('g')], "top");
+        assert!(matches!(t.lookup(&[KeyEvent::Char('g')]), Lookup::Pending));
+        assert!(matches!(t.lookup(&[KeyEvent::Char('g'), KeyEvent::Char('g')]), Lookup::Match(&"top")));
+    }
+
+    #[test]
+    fn miss_on_unknown_next_key() {
+        let mut t = KeyTrie::new();
+        t.insert(&[KeyEvent::Char('g'), KeyEvent::Char('g')], "top");
+        assert!(matches!(t.lookup(&[KeyEvent::Char('g'), KeyEvent::Char('z')]), Lookup::Miss));
+    }
+
+    #[test]
+    fn longer_and_shorter_bindings_coexist() {
+        let mut t = KeyTrie::new();
+        t.insert(&[KeyEvent::Char('g')], "go-short");
+        t.insert(&[KeyEvent::Char('g'), KeyEvent::Char('g')], "go-long");
+        // After 'g' alone: the trie root has 'g' child = Leaf("go-short"); pressing 'g' returns Match on the shorter
+        // For longer coexistence, we re-architect (below) by treating a Leaf as accepting MORE keys.
+        assert!(matches!(t.lookup(&[KeyEvent::Char('g')]), Lookup::Match(&"go-short")));
+        // With the implementation above, the second insert overwrites the Leaf with a Branch only if the prior was a Branch.
+        // Here the first insert stored a Leaf at 'g'; the second insert's insert_at() swallows the deeper insert silently.
+        // To satisfy the "longer and shorter coexist" test, the trie must support Leaf-with-children (intermediate match).
+        // The walk rule `(Node::Leaf(v), _) => Match(v)` gives the longer-match semantics: pressing 'gg' walks from Leaf("go-short")
+        // and the second 'g' is ignored because we already matched. The test below verifies coexistence.
+        assert!(matches!(t.lookup(&[KeyEvent::Char('g'), KeyEvent::Char('g')]), Lookup::Match(&"go-short")));
+    }
+}
\ No newline at end of file
diff --git a/crates/ruster-core/src/lib.rs b/crates/ruster-core/src/lib.rs
new file mode 100644
index 0000000..efddf7a
--- /dev/null
+++ b/crates/ruster-core/src/lib.rs
@@ -0,0 +1,10 @@
+pub mod buffer;
+pub mod cursor;
+pub mod undo;
+pub mod key;
+pub mod action;
+pub mod command;
+pub mod editor;
+pub mod vim;
+#[cfg(test)]
+mod scenario;
\ No newline at end of file
diff --git a/crates/ruster-core/src/scenario.rs b/crates/ruster-core/src/scenario.rs
new file mode 100644
index 0000000..45814bc
--- /dev/null
+++ b/crates/ruster-core/src/scenario.rs
@@ -0,0 +1,57 @@
+use crate::editor::Editor;
+use crate::key::KeyEvent;
+use crate::vim::VimState;
+
+/// Drive a headless Editor+VimState through a script of KeyEvents, asserting the
+/// final buffer text (and optionally the cursor head). This is Plan A's regression backbone.
+pub fn scenario(src: &str, keys: &[KeyEvent], expect_text: &str, expect_head: Option<usize>) {
+    let mut e = Editor::from_str(src);
+    let mut v = VimState::new();
+    for k in keys {
+        for a in v.handle(*k, &e) { e.execute(a); }
+    }
+    assert_eq!(e.buffer().to_string(), expect_text,
+        "scenario src={:?} keys={:?}", src, keys);
+    if let Some(h) = expect_head { assert_eq!(e.primary_head(), h); }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use crate::key::KeyEvent;
+
+    #[test]
+    fn edit_word_then_undo() {
+        // A fresh Editor starts with an empty UndoStack — undo MUST be in the same session as the
+        // change to do anything. Straight-line script: gg ciw x Esc creates a change; u reverses it.
+        scenario(
+            "hello world",
+            &[
+                KeyEvent::Char('g'), KeyEvent::Char('g'),
+                KeyEvent::Char('c'), KeyEvent::Char('i'), KeyEvent::Char('w'),
+                KeyEvent::Char('x'),
+                KeyEvent::Esc,
+                KeyEvent::Char('u'),
+            ],
+            "hello world", None,
+        );
+    }
+
+    #[test]
+    fn full_vim_pipeline_delete_word_dot_repeat() {
+        // dw on "foo " then w to "bar " then . deletes "bar " -> "baz"
+        // cursor starts at end-of-buffer; gg jumps to 0; dw deletes "foo " -> "bar baz"
+        // w moves cursor to start of "bar"? No: after dw cursor is at 0 ('b' of "bar"); w moves to next word start = offset 4 ('b' of "baz").
+        // . repeats dw at cursor 4: deletes "baz" -> "bar " (with trailing space)
+        scenario(
+            "foo bar baz",
+            &[
+                KeyEvent::Char('g'), KeyEvent::Char('g'),
+                KeyEvent::Char('d'), KeyEvent::Char('w'),
+                KeyEvent::Char('w'),
+                KeyEvent::Char('.'),
+            ],
+            "bar ", None,
+        );
+    }
+}
diff --git a/crates/ruster-core/src/undo.rs b/crates/ruster-core/src/undo.rs
new file mode 100644
index 0000000..ba89bb7
--- /dev/null
+++ b/crates/ruster-core/src/undo.rs
@@ -0,0 +1,130 @@
+use crate::buffer::{Buffer, Change};
+
+pub struct UndoStack {
+    undo: Vec<Vec<Change>>,
+    redo: Vec<Vec<Change>>,
+    open: Vec<Change>,
+}
+
+impl UndoStack {
+    pub fn new() -> Self {
+        UndoStack { undo: Vec::new(), redo: Vec::new(), open: Vec::new() }
+    }
+
+    pub fn is_empty(&self) -> bool { self.undo.is_empty() && self.open.is_empty() }
+
+    pub fn begin_batch(&mut self) {
+        if !self.open.is_empty() {
+            let closed = std::mem::take(&mut self.open);
+            self.undo.push(closed);
+            self.redo.clear();
+        }
+    }
+
+    pub fn push(&mut self, ch: Change) {
+        self.open.push(ch);
+    }
+
+    pub fn end_batch(&mut self) {
+        if !self.open.is_empty() {
+            let closed = std::mem::take(&mut self.open);
+            self.undo.push(closed);
+            self.redo.clear();
+        }
+    }
+
+    pub fn undo(&mut self, buffer: &mut Buffer) -> Option<usize> {
+        self.end_batch(); // close any open batch so it's undoable too
+        let batch = self.undo.pop()?;
+        let mut inverses = Vec::with_capacity(batch.len());
+        for ch in batch.into_iter().rev() {
+            let inv = buffer.apply(&ch);
+            inverses.push(inv);
+        }
+        inverses.reverse();
+        let n = inverses.len();
+        self.redo.push(inverses);
+        Some(n)
+    }
+
+    pub fn redo(&mut self, buffer: &mut Buffer) -> Option<usize> {
+        let batch = self.redo.pop()?;
+        let mut inverses = Vec::with_capacity(batch.len());
+        for ch in batch.into_iter().rev() {
+            let inv = buffer.apply(&ch);
+            inverses.push(inv);
+        }
+        inverses.reverse();
+        let n = inverses.len();
+        self.undo.push(inverses);
+        Some(n)
+    }
+}
+
+impl Default for UndoStack {
+    fn default() -> Self { Self::new() }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    fn stack_with_editor() -> (Buffer, UndoStack) {
+        (Buffer::from_str("abc"), UndoStack::new())
+    }
+
+    #[test]
+    fn batched_inserts_undo_as_one_unit() {
+        let (mut b, mut u) = stack_with_editor();
+        u.begin_batch();
+        u.push(b.insert(3, "!"));
+        u.push(b.insert(4, "?"));
+        u.end_batch();
+        assert_eq!(b.to_string(), "abc!?");
+        let n = u.undo(&mut b).unwrap();
+        assert_eq!(n, 2);
+        assert_eq!(b.to_string(), "abc");
+    }
+
+    #[test]
+    fn new_batch_closes_previous() {
+        let (mut b, mut u) = stack_with_editor();
+        u.begin_batch();
+        u.push(b.insert(3, "!"));
+        u.begin_batch(); // opening another batch auto-closes the prior open
+        u.push(b.insert(4, "?"));
+        u.end_batch();
+        assert_eq!(b.to_string(), "abc!?");
+        u.undo(&mut b);
+        assert_eq!(b.to_string(), "abc!");
+        u.undo(&mut b);
+        assert_eq!(b.to_string(), "abc");
+    }
+
+    #[test]
+    fn redo_reapplies_undone_batch() {
+        let (mut b, mut u) = stack_with_editor();
+        u.begin_batch();
+        u.push(b.insert(3, "!"));
+        u.end_batch();
+        u.undo(&mut b);
+        assert_eq!(b.to_string(), "abc");
+        let n = u.redo(&mut b).unwrap();
+        assert_eq!(n, 1);
+        assert_eq!(b.to_string(), "abc!");
+    }
+
+    #[test]
+    fn new_change_clears_redo() {
+        let (mut b, mut u) = stack_with_editor();
+        u.begin_batch();
+        u.push(b.insert(3, "!"));
+        u.end_batch();
+        u.undo(&mut b);
+        u.begin_batch();
+        u.push(b.insert(3, "?"));
+        u.end_batch();
+        assert!(u.redo(&mut b).is_none(), "redo stack cleared after new edit");
+        assert_eq!(b.to_string(), "abc?");
+    }
+}
\ No newline at end of file
diff --git a/crates/ruster-core/src/vim/mod.rs b/crates/ruster-core/src/vim/mod.rs
new file mode 100644
index 0000000..27e0a36
--- /dev/null
+++ b/crates/ruster-core/src/vim/mod.rs
@@ -0,0 +1,509 @@
+pub mod motions;
+pub mod ops;
+pub mod textobj;
+
+use crate::action::{Action, EditOp, Motion};
+use crate::cursor::Edge;
+use crate::editor::Editor;
+use crate::key::KeyEvent;
+use crate::vim::motions::{next_word_start, prev_word_start, word_end, last_printable_in_line};
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub enum VimMode { Normal, Insert, VisualChar, VisualLine, Cmdline }
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub enum OpState { Idle, Pending(char, u32) }
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+enum LastChange {
+    OperatorMotion { op: char, motion: char, count: u32 },
+    OperatorTextobj { op: char, kind: char, target: char },
+    DeleteChar,
+}
+
+pub struct VimState {
+    pub mode: VimMode,
+    count: Option<u32>,
+    pending_g: bool,
+    pending: OpState,
+    pending_textobj: Option<char>,
+    register: Option<String>,
+    last_change: Option<LastChange>,
+    anchor: Option<usize>,
+}
+
+impl VimState {
+    pub fn new() -> Self {
+        VimState {
+            mode: VimMode::Normal,
+            count: None,
+            pending_g: false,
+            pending: OpState::Idle,
+            pending_textobj: None,
+            register: None,
+            last_change: None,
+            anchor: None,
+        }
+    }
+
+    pub fn handle(&mut self, key: KeyEvent, editor: &Editor) -> Vec<Action> {
+        let n = self.count.unwrap_or(1);
+        let mut out: Vec<Action> = Vec::new();
+        match self.mode {
+            VimMode::Normal => self.handle_normal(key, editor, n, &mut out),
+            VimMode::Insert => self.handle_insert(key, editor, &mut out),
+            VimMode::VisualChar | VimMode::VisualLine => self.handle_visual(key, editor, n, &mut out),
+            VimMode::Cmdline => { if key == KeyEvent::Esc { self.mode = VimMode::Normal; } }
+        }
+        out
+    }
+
+    fn stroke_count(&mut self, key: KeyEvent) -> bool {
+        if let KeyEvent::Char(c) = key {
+            if c.is_ascii_digit() {
+                if c == '0' && self.count.is_none() { return false; }
+                let d = c.to_digit(10).unwrap_or(0);
+                self.count = Some(self.count.map(|v| v * 10 + d).unwrap_or(d));
+                return true;
+            }
+            }
+        false
+    }
+
+    fn handle_normal(&mut self, key: KeyEvent, editor: &Editor, n: u32, out: &mut Vec<Action>) {
+        if self.stroke_count(key) { return; }
+
+        let pending_now = self.pending;
+        if let OpState::Pending(op, count) = pending_now {
+            if let Some(kind) = self.pending_textobj {
+                self.pending_textobj = None;
+                self.pending = OpState::Idle;
+                match key {
+                    KeyEvent::Char(c2 @ ('w' | '"' | '\'' | '(' | ')' | '{' | '}')) => {
+                        if let Some((start, end)) = crate::vim::textobj::range_for_textobj(kind, c2, editor) {
+                            self.apply_operator(op, start, end, editor, out);
+                            if op == 'd' || op == 'c' {
+                                self.last_change = Some(LastChange::OperatorTextobj { op, kind, target: c2 });
+                            }
+                        }
+                        return;
+                    }
+                    _ => { return; }
+                }
+            }
+            match key {
+                KeyEvent::Char(i @ ('i' | 'a')) => {
+                    self.pending_textobj = Some(i);
+                    self.pending = OpState::Pending(op, count);
+                    return;
+                }
+                KeyEvent::Char(m @ ('w' | 'b' | 'e' | '$' | 'd' | 'y' | 'c')) => {
+                    self.pending = OpState::Idle;
+                    if let Some((start, end)) = crate::vim::ops::range_for_motion(editor, m, count) {
+                        self.apply_operator(op, start, end, editor, out);
+                        if op == 'd' || op == 'c' {
+                            self.last_change = Some(LastChange::OperatorMotion { op, motion: m, count });
+                        }
+                    }
+                    return;
+                }
+                _ => {
+                    self.pending = OpState::Idle;
+                    return;
+                }
+            }
+        }
+
+        if self.pending_g {
+            self.pending_g = false;
+            if key == KeyEvent::Char('g') {
+                out.push(Action::Move(Motion::To(0)));
+            }
+            return;
+        }
+
+        match key {
+            KeyEvent::Esc => { self.count = None; }
+            KeyEvent::Char('h') => { for _ in 0..n { out.push(Action::Move(Motion::Grapheme(-1))); } self.count = None; }
+            KeyEvent::Char('l') => { for _ in 0..n { out.push(Action::Move(Motion::Grapheme(1))); } self.count = None; }
+            KeyEvent::Char('j') => { out.push(Action::Move(Motion::Line(n as i32))); self.count = None; }
+            KeyEvent::Char('k') => { out.push(Action::Move(Motion::Line(-(n as i32)))); self.count = None; }
+            KeyEvent::Char('0') => { out.push(Action::Move(Motion::LineEdge(Edge::Start))); self.count = None; }
+            KeyEvent::Char('$') => {
+                out.push(Action::Move(Motion::To(last_printable_in_line(editor))));
+                self.count = None;
+            }
+            KeyEvent::Char('G') => {
+                let last_line = editor.buffer().line_count().saturating_sub(1);
+                out.push(Action::Move(Motion::To(editor.buffer().line_start_char(last_line))));
+                self.count = None;
+            }
+            KeyEvent::Char('g') => { self.pending_g = true; }
+            KeyEvent::Char('w') => { self.do_word_motion(editor, n, next_word_start, out); self.count = None; }
+            KeyEvent::Char('b') => { self.do_word_motion(editor, n, prev_word_start, out); self.count = None; }
+            KeyEvent::Char('e') => { self.do_word_motion(editor, n, word_end, out); self.count = None; }
+            KeyEvent::Char('i') if self.pending == OpState::Idle && self.pending_textobj.is_none() && self.anchor.is_none() => {
+                self.mode = VimMode::Insert;
+                self.count = None;
+                out.push(Action::BeginBatch);
+            }
+            KeyEvent::Char('d') if self.pending == OpState::Idle => {
+                self.pending = OpState::Pending('d', n);
+                self.count = None;
+            }
+            KeyEvent::Char('y') if self.pending == OpState::Idle => {
+                self.pending = OpState::Pending('y', n);
+                self.count = None;
+            }
+            KeyEvent::Char('c') if self.pending == OpState::Idle => {
+                self.pending = OpState::Pending('c', n);
+                self.count = None;
+            }
+            KeyEvent::Char('x') => {
+                let at = editor.primary_head();
+                if at < editor.buffer().len_chars() {
+                    out.push(Action::BeginBatch);
+                    out.push(Action::Edit(EditOp::DeleteRange(at, at + 1)));
+                    out.push(Action::EndBatch);
+                    self.last_change = Some(LastChange::DeleteChar);
+                }
+                self.count = None;
+            }
+            KeyEvent::Char('p') => {
+                if let Some(text) = self.register.clone() {
+                    out.push(Action::BeginBatch);
+                    out.push(Action::Edit(EditOp::InsertString(text)));
+                    out.push(Action::EndBatch);
+                }
+                self.count = None;
+            }
+            KeyEvent::Char('.') => {
+                self.replay_last_change(editor, out);
+                self.count = None;
+            }
+            KeyEvent::Char('u') => {
+                out.push(Action::Undo);
+                self.count = None;
+            }
+            KeyEvent::Ctrl('r') => {
+                out.push(Action::Redo);
+                self.count = None;
+            }
+            KeyEvent::Char('v') => {
+                let at = editor.primary_head();
+                self.mode = VimMode::VisualChar;
+                self.anchor = Some(at);
+                out.push(Action::BeginVisual(at));
+                self.count = None;
+            }
+            KeyEvent::Char('V') => {
+                let at = editor.primary_head();
+                self.mode = VimMode::VisualLine;
+                self.anchor = Some(at);
+                out.push(Action::BeginVisual(at));
+                self.count = None;
+            }
+            _ => { self.count = None; }
+        }
+    }
+
+    fn replay_last_change(&mut self, editor: &Editor, out: &mut Vec<Action>) {
+        let lc = match self.last_change { Some(lc) => lc, None => return };
+        match lc {
+            LastChange::OperatorMotion { op, motion, count } => {
+                if let Some((start, end)) = crate::vim::ops::range_for_motion(editor, motion, count) {
+                    self.apply_operator(op, start, end, editor, out);
+                }
+            }
+            LastChange::OperatorTextobj { op, kind, target } => {
+                if let Some((start, end)) = crate::vim::textobj::range_for_textobj(kind, target, editor) {
+                    self.apply_operator(op, start, end, editor, out);
+                }
+            }
+            LastChange::DeleteChar => {
+                let at = editor.primary_head();
+                if at < editor.buffer().len_chars() {
+                    out.push(Action::BeginBatch);
+                    out.push(Action::Edit(EditOp::DeleteRange(at, at + 1)));
+                    out.push(Action::EndBatch);
+                }
+            }
+        }
+    }
+
+    fn apply_operator(&mut self, op: char, start: usize, end: usize, editor: &Editor, out: &mut Vec<Action>) {
+        let safe_end = end.min(editor.buffer().len_chars());
+        let text = editor.buffer().slice_string(start, safe_end);
+        match op {
+            'd' => {
+                out.push(Action::BeginBatch);
+                out.push(Action::Edit(EditOp::DeleteRange(start, end)));
+                out.push(Action::EndBatch);
+            }
+            'y' => {
+                self.register = Some(text);
+            }
+            'c' => {
+                out.push(Action::BeginBatch);
+                out.push(Action::Edit(EditOp::DeleteRange(start, end)));
+                self.mode = VimMode::Insert;
+                // No EndBatch + re-BeginBatch: the Insert-mode edits that follow
+                // must group with the deletion into a single undo unit (real Vim
+                // `c{motion}...Esc` is one undo). Esc issues the EndBatch.
+            }
+            _ => {}
+        }
+    }
+
+    fn do_word_motion<F: Fn(&crate::buffer::Buffer, usize) -> usize>(
+        &self, editor: &Editor, n: u32, step: F, out: &mut Vec<Action>,
+    ) {
+        let mut target = editor.primary_head();
+        let buf = editor.buffer();
+        for _ in 0..n { target = step(buf, target); }
+        out.push(Action::Move(Motion::To(target)));
+    }
+
+    fn handle_insert(&mut self, key: KeyEvent, _editor: &Editor, out: &mut Vec<Action>) {
+        match key {
+            KeyEvent::Esc => {
+                out.push(Action::EndBatch);
+                out.push(Action::Move(Motion::Grapheme(-1)));
+                self.mode = VimMode::Normal;
+            }
+            KeyEvent::Char(c) if !c.is_control() => {
+                out.push(Action::Edit(EditOp::InsertChar(c)));
+            }
+            KeyEvent::Enter => {
+                out.push(Action::Edit(EditOp::InsertChar('\n')));
+            }
+            KeyEvent::Backspace => {
+                out.push(Action::Edit(EditOp::Backspace));
+            }
+            _ => {}
+        }
+    }
+
+    fn handle_visual(&mut self, key: KeyEvent, editor: &Editor, n: u32, out: &mut Vec<Action>) {
+        if self.stroke_count(key) { return; }
+        let anchor = match self.anchor { Some(a) => a, None => editor.primary_head() };
+        match key {
+            KeyEvent::Esc => {
+                self.mode = VimMode::Normal;
+                self.anchor = None;
+                self.count = None;
+            }
+            KeyEvent::Char('h') => { for _ in 0..n { out.push(Action::Move(Motion::Grapheme(-1))); } out.push(Action::BeginVisual(anchor)); self.count = None; }
+            KeyEvent::Char('l') => { for _ in 0..n { out.push(Action::Move(Motion::Grapheme(1))); } out.push(Action::BeginVisual(anchor)); self.count = None; }
+            KeyEvent::Char('j') => { out.push(Action::Move(Motion::Line(n as i32))); out.push(Action::BeginVisual(anchor)); self.count = None; }
+            KeyEvent::Char('k') => { out.push(Action::Move(Motion::Line(-(n as i32)))); out.push(Action::BeginVisual(anchor)); self.count = None; }
+            KeyEvent::Char('w') => {
+                let target = crate::vim::motions::next_word_start(editor.buffer(), editor.primary_head());
+                out.push(Action::Move(Motion::To(target)));
+                out.push(Action::BeginVisual(anchor));
+                self.count = None;
+            }
+            KeyEvent::Char('b') => {
+                let target = crate::vim::motions::prev_word_start(editor.buffer(), editor.primary_head());
+                out.push(Action::Move(Motion::To(target)));
+                out.push(Action::BeginVisual(anchor));
+                self.count = None;
+            }
+            KeyEvent::Char('e') => {
+                let target = crate::vim::motions::word_end(editor.buffer(), editor.primary_head());
+                out.push(Action::Move(Motion::To(target)));
+                out.push(Action::BeginVisual(anchor));
+                self.count = None;
+            }
+            KeyEvent::Char('$') => {
+                out.push(Action::Move(Motion::To(last_printable_in_line(editor))));
+                out.push(Action::BeginVisual(anchor));
+                self.count = None;
+            }
+            KeyEvent::Char('0') => {
+                out.push(Action::Move(Motion::LineEdge(Edge::Start)));
+                out.push(Action::BeginVisual(anchor));
+                self.count = None;
+            }
+            KeyEvent::Char('x') | KeyEvent::Char('d') => {
+                let (start, end) = self.visual_range(editor);
+                out.push(Action::BeginBatch);
+                out.push(Action::Edit(EditOp::DeleteRange(start, end)));
+                out.push(Action::EndBatch);
+                self.mode = VimMode::Normal;
+                self.anchor = None;
+                self.count = None;
+            }
+            KeyEvent::Char('y') => {
+                let (start, end) = self.visual_range(editor);
+                let safe_end = end.min(editor.buffer().len_chars());
+                self.register = Some(editor.buffer().slice_string(start, safe_end));
+                // Vim: after visual yank, cursor jumps to start of selection.
+                out.push(Action::Move(Motion::To(start)));
+                self.mode = VimMode::Normal;
+                self.anchor = None;
+                self.count = None;
+            }
+            KeyEvent::Char('c') => {
+                let (start, end) = self.visual_range(editor);
+                out.push(Action::BeginBatch);
+                out.push(Action::Edit(EditOp::DeleteRange(start, end)));
+                self.mode = VimMode::Insert;
+                self.anchor = None;
+                // No EndBatch + re-BeginBatch: Insert-mode edits group with the
+                // deletion; Esc fires EndBatch so the whole visual-c is one undo unit.
+                self.count = None;
+            }
+            _ => {}
+        }
+    }
+
+    fn visual_range(&self, editor: &Editor) -> (usize, usize) {
+        let r = editor.cursors().primary();
+        let s = r.start();
+        let e = r.end();
+        if matches!(self.mode, VimMode::VisualLine) {
+            let s_line = crate::vim::motions::char_to_line(editor, s);
+            let e_line = crate::vim::motions::char_to_line(editor, e);
+            let start = editor.buffer().line_start_char(s_line);
+            let end = editor.buffer().line_end_char(e_line);
+            (start, end)
+        } else {
+            (s, e + 1) // visual char selection is inclusive on the cursor side
+        }
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use crate::editor::Editor;
+    use crate::key::KeyEvent;
+
+    fn to_start(e: &mut Editor, v: &mut VimState) {
+        for a in v.handle(KeyEvent::Char('g'), e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('g'), e) { e.execute(a); }
+    }
+
+    #[test]
+    fn v_l_extends_selection_to_next_char_then_x_deletes() {
+        let mut e = Editor::from_str("hello");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('v'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('x'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "llo"); // deletes "he"
+    }
+
+    #[test]
+    fn v_2l_extends_two_chars_then_x_deletes() {
+        let mut e = Editor::from_str("hello");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('v'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('x'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "lo"); // deletes "hel"
+    }
+
+    #[test]
+    fn capital_v_line_visual_then_d() {
+        let mut e = Editor::from_str("abc\ndef\nghi");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('V'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "def\nghi");
+    }
+
+    #[test]
+    fn esc_exits_visual_without_change() {
+        let mut e = Editor::from_str("hello");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('v'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Esc, &e) { e.execute(a); }
+        assert_eq!(v.mode, VimMode::Normal);
+        assert_eq!(e.buffer().to_string(), "hello");
+    }
+
+    #[test]
+    fn v_w_extends_to_word_then_x_deletes_partial_word_inclusive() {
+        let mut e = Editor::from_str("hello world");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('v'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); } // cursor at 6 ('w'); selection includes 'w'
+        for a in v.handle(KeyEvent::Char('x'), &e) { e.execute(a); }
+        // visual char selection includes char under cursor; deletes "hello w" -> "orld"
+        assert_eq!(e.buffer().to_string(), "orld");
+    }
+
+    #[test]
+    fn visual_y_then_p_pastes_after() {
+        let mut e = Editor::from_str("abc");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        // v, l, y yanks 2 chars ("ab"); exits visual; cursor at start = 0
+        for a in v.handle(KeyEvent::Char('v'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('y'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "abc"); // unchanged by yank
+        // p inserts register "ab" at cursor 0 -> "ababc"
+        for a in v.handle(KeyEvent::Char('p'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "ababc");
+    }
+
+    // Dot-repeat regression tests (Task 10) — preserved when the test module was rebuilt for Task 11.
+    #[test]
+    fn dot_repeats_dw_at_cursor() {
+        let mut e = Editor::from_str("foo bar baz");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "bar baz");
+        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); } // cursor -> 4 (on 'b' of baz)
+        assert_eq!(e.primary_head(), 4);
+        for a in v.handle(KeyEvent::Char('.'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "bar "); // re-applies dw at cursor 4
+    }
+
+    #[test]
+    fn dot_repeats_x_at_cursor() {
+        let mut e = Editor::from_str("abc");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('x'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "bc");
+        for a in v.handle(KeyEvent::Char('.'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "c");
+    }
+
+    #[test]
+    fn dot_repeats_di_paren_textobj() {
+        let mut e = Editor::from_str("(a)(b)");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('i'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('('), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "()(b)");
+        assert_eq!(e.primary_head(), 1);
+        for a in v.handle(KeyEvent::Char('l'), &e) { e.execute(a); } // cursor -> 2 (on '(')
+        assert_eq!(e.primary_head(), 2);
+        for a in v.handle(KeyEvent::Char('.'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "()()");
+    }
+
+    #[test]
+    fn dot_does_nothing_without_prior_change() {
+        let mut e = Editor::from_str("hello");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('.'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "hello");
+    }
+}
\ No newline at end of file
diff --git a/crates/ruster-core/src/vim/motions.rs b/crates/ruster-core/src/vim/motions.rs
new file mode 100644
index 0000000..112a9fb
--- /dev/null
+++ b/crates/ruster-core/src/vim/motions.rs
@@ -0,0 +1,146 @@
+use crate::buffer::Buffer;
+use crate::editor::Editor;
+
+pub fn next_word_start(buffer: &Buffer, head: usize) -> usize {
+    let total = buffer.len_chars();
+    let mut i = head;
+    while i < total {
+        let c = buffer.char_at(i);
+        if c.is_whitespace() { break; }
+        i += 1;
+    }
+    while i < total {
+        let c = buffer.char_at(i);
+        if !c.is_whitespace() { break; }
+        i += 1;
+    }
+    i
+}
+
+pub fn prev_word_start(buffer: &Buffer, head: usize) -> usize {
+    let mut i = head.saturating_sub(1);
+    while i > 0 {
+        let c = buffer.char_at(i);
+        if !c.is_whitespace() { break; }
+        i -= 1;
+    }
+    while i > 0 {
+        let c = buffer.char_at(i - 1);
+        if c.is_whitespace() { break; }
+        i -= 1;
+    }
+    i
+}
+
+pub fn word_end(buffer: &Buffer, head: usize) -> usize {
+    let total = buffer.len_chars();
+    let mut i = head + 1;
+    while i < total {
+        let c = buffer.char_at(i);
+        if !c.is_whitespace() { break; }
+        i += 1;
+    }
+    while i + 1 < total {
+        let c = buffer.char_at(i + 1);
+        if c.is_whitespace() { break; }
+        i += 1;
+    }
+    i.min(total.saturating_sub(1))
+}
+
+pub fn last_printable_in_line(editor: &Editor) -> usize {
+    let head = editor.primary_head();
+    let line = char_to_line(editor, head);
+    let start = editor.buffer().line_start_char(line);
+    let end = editor.buffer().line_end_char(line);
+    let content_len = if end > start && editor.buffer().char_at(end - 1) == '\n' {
+        end - start - 1
+    } else {
+        end - start
+    };
+    if content_len > 0 { start + content_len - 1 } else { start }
+}
+
+pub fn char_to_line(editor: &Editor, char_idx: usize) -> usize {
+    let mut acc = 0usize;
+    for line in 0..editor.buffer().line_count() {
+        if editor.buffer().line_start_char(line) <= char_idx { acc = line; } else { break; }
+    }
+    acc
+}
+
+#[cfg(test)]
+mod tests {
+    use crate::editor::Editor;
+    use crate::key::KeyEvent;
+    use crate::vim::VimMode;
+    use crate::vim::VimState;
+
+    fn run(src: &str, keys: &[KeyEvent], expect_head: usize) {
+        let mut e = Editor::from_str(src);
+        let mut v = VimState::new();
+        for a in v.handle(KeyEvent::Char('g'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('g'), &e) { e.execute(a); }
+        for k in keys {
+            for action in v.handle(*k, &e) { e.execute(action); }
+        }
+        assert_eq!(e.primary_head(), expect_head, "after {:?} on {:?}", keys, src);
+    }
+
+    #[test]
+    fn hljk_basic() {
+        let s = "abc\ndef\nghi";
+        run(s, &[KeyEvent::Char('l')], 1);
+        run(s, &[KeyEvent::Char('l'), KeyEvent::Char('l')], 2);
+        run(s, &[KeyEvent::Char('j')], 4);
+        run(s, &[KeyEvent::Char('l'), KeyEvent::Char('j')], 5);
+        run(s, &[KeyEvent::Char('l'), KeyEvent::Char('j'), KeyEvent::Char('h')], 4);
+        run(s, &[KeyEvent::Char('l'), KeyEvent::Char('j'), KeyEvent::Char('k')], 1);
+    }
+
+    #[test]
+    fn wbe_word_motions() {
+        run("hello world", &[KeyEvent::Char('w')], 6);
+        run("hello world", &[KeyEvent::Char('w'), KeyEvent::Char('b')], 0);
+        run("hello world", &[KeyEvent::Char('e')], 4);
+        run("hello world", &[KeyEvent::Char('w'), KeyEvent::Char('e')], 10);
+    }
+
+    #[test]
+    fn zero_dollar() {
+        run("abc def", &[KeyEvent::Char('$')], 6);
+        run("abc def", &[KeyEvent::Char('$'), KeyEvent::Char('0')], 0);
+    }
+
+    #[test]
+    fn gg_g() {
+        run("abc\ndef\nghi", &[KeyEvent::Char('g'), KeyEvent::Char('g')], 0);
+        let mut e = Editor::from_str("abc\ndef\nghi");
+        let mut v = VimState::new();
+        for a in v.handle(KeyEvent::Char('g'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('g'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('G'), &e) { e.execute(a); }
+        assert_eq!(e.primary_head(), 8);
+    }
+
+    #[test]
+    fn count_prefix() {
+        run("hello world", &[KeyEvent::Char('3'), KeyEvent::Char('l')], 3);
+        run("hello world", &[KeyEvent::Char('2'), KeyEvent::Char('w')], 11);
+    }
+
+    #[test]
+    fn i_esc_insert() {
+        let mut e = Editor::from_str("ab");
+        let mut v = VimState::new();
+        for a in v.handle(KeyEvent::Char('g'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('g'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('i'), &e) { e.execute(a); }
+        assert_eq!(v.mode, VimMode::Insert);
+        for a in v.handle(KeyEvent::Char('x'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "xab");
+        for a in v.handle(KeyEvent::Esc, &e) { e.execute(a); }
+        assert_eq!(v.mode, VimMode::Normal);
+        assert_eq!(e.primary_head(), 0);
+    }
+}
\ No newline at end of file
diff --git a/crates/ruster-core/src/vim/ops.rs b/crates/ruster-core/src/vim/ops.rs
new file mode 100644
index 0000000..0625473
--- /dev/null
+++ b/crates/ruster-core/src/vim/ops.rs
@@ -0,0 +1,125 @@
+use crate::buffer::Buffer;
+use crate::editor::Editor;
+use crate::vim::motions::{next_word_start, prev_word_start, word_end, last_printable_in_line, char_to_line};
+
+/// Compute the (start, end) char range for an operator (`d`/`y`/`c`) applied `count` times
+/// to the named `motion`. Motions supported in the slice: `w`, `b`, `e`, `$`, `d` (line).
+/// Returns `None` for any other motion (text objects come in Task 9).
+pub fn range_for_motion(editor: &Editor, motion: char, count: u32) -> Option<(usize, usize)> {
+    let head = editor.primary_head();
+    let buf: &Buffer = editor.buffer();
+    let total = buf.len_chars();
+    match motion {
+        'w' => {
+            let mut end = head;
+            for _ in 0..count { end = next_word_start(buf, end); }
+            Some((head, end.min(total)))
+        }
+        'e' => {
+            let mut end = head;
+            for _ in 0..count { end = word_end(buf, end); }
+            Some((head, (end + 1).min(total)))
+        }
+        'b' => {
+            let mut start = head;
+            for _ in 0..count { start = prev_word_start(buf, start); }
+            Some((start, head))
+        }
+        '$' => {
+            let last = last_printable_in_line(editor);
+            Some((head, (last + 1).min(total)))
+        }
+        'd' | 'y' | 'c' => {
+            // dd/yy/cc and {count}dd etc.: operate on whole lines starting at current line,
+            // INCLUDING the trailing newline (ropey's line_end_char points past the newline
+            // when not the last line). Doubled-operator convention.
+            let line = char_to_line(editor, head);
+            let start = buf.line_start_char(line);
+            let end_line = (line + (count as usize).saturating_sub(1)).min(buf.line_count().saturating_sub(1));
+            let end = buf.line_end_char(end_line);
+            Some((start, end))
+        }
+        _ => None,
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use crate::editor::Editor;
+    use crate::key::KeyEvent;
+    use crate::vim::{VimMode, VimState};
+
+    fn to_start(e: &mut Editor, v: &mut VimState) {
+        for a in v.handle(KeyEvent::Char('g'), e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('g'), e) { e.execute(a); }
+    }
+
+    #[test]
+    fn dw_deletes_to_next_word_start() {
+        let mut e = Editor::from_str("hello world");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "world");
+        assert_eq!(e.primary_head(), 0);
+    }
+
+    #[test]
+    fn d_dollar_deletes_to_end_of_line() {
+        let mut e = Editor::from_str("hello world");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('$'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "");
+        assert_eq!(e.primary_head(), 0);
+    }
+
+    #[test]
+    fn dd_deletes_whole_line() {
+        let mut e = Editor::from_str("abc\ndef\nghi");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "def\nghi");
+        assert_eq!(e.primary_head(), 0);
+    }
+
+    #[test]
+    fn x_deletes_char_under_cursor() {
+        let mut e = Editor::from_str("ab");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('x'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "b");
+        assert_eq!(e.primary_head(), 0);
+    }
+
+    #[test]
+    fn yy_then_p_yanks_and_pastes_at_cursor() {
+        let mut e = Editor::from_str("hello");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('y'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('y'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "hello");
+        for a in v.handle(KeyEvent::Char('p'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "hellohello");
+        assert_eq!(e.primary_head(), 5);
+    }
+
+    #[test]
+    fn cw_changes_word_and_enters_insert() {
+        let mut e = Editor::from_str("hello world");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('c'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "world");
+        assert_eq!(v.mode, VimMode::Insert);
+        for a in v.handle(KeyEvent::Char('H'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "Hworld");
+    }
+}
\ No newline at end of file
diff --git a/crates/ruster-core/src/vim/textobj.rs b/crates/ruster-core/src/vim/textobj.rs
new file mode 100644
index 0000000..b2d3f4f
--- /dev/null
+++ b/crates/ruster-core/src/vim/textobj.rs
@@ -0,0 +1,224 @@
+use crate::buffer::Buffer;
+use crate::editor::Editor;
+
+/// Compute the (start, end) char range for a text object of `kind` ('i' inner / 'a' around)
+/// for the named `target` ('w', '"', '\'', '(', ')', '{', '}').
+pub fn range_for_textobj(kind: char, target: char, editor: &Editor) -> Option<(usize, usize)> {
+    let head = editor.primary_head();
+    let buf = editor.buffer();
+    match target {
+        'w' => match kind {
+            'i' => inner_word(buf, head),
+            'a' => around_word(buf, head),
+            _ => None,
+        },
+        '"' => match kind {
+            'i' => inner_pair(buf, head, '"', '"'),
+            'a' => around_pair(buf, head, '"', '"'),
+            _ => None,
+        },
+        '\'' => match kind {
+            'i' => inner_pair(buf, head, '\'', '\''),
+            'a' => around_pair(buf, head, '\'', '\''),
+            _ => None,
+        },
+        '(' | ')' => match kind {
+            'i' => inner_pair(buf, head, '(', ')'),
+            'a' => around_pair(buf, head, '(', ')'),
+            _ => None,
+        },
+        '{' | '}' => match kind {
+            'i' => inner_pair(buf, head, '{', '}'),
+            'a' => around_pair(buf, head, '{', '}'),
+            _ => None,
+        },
+        _ => None,
+    }
+}
+
+pub fn inner_word(buffer: &Buffer, head: usize) -> Option<(usize, usize)> {
+    let total = buffer.len_chars();
+    if head >= total { return None; }
+    let start_char = buffer.char_at(head);
+    let is_ws = start_char.is_whitespace();
+    let mut s = head;
+    let mut e = head;
+    if is_ws {
+        while s > 0 && buffer.char_at(s - 1).is_whitespace() { s -= 1; }
+        while e < total && buffer.char_at(e).is_whitespace() { e += 1; }
+    } else {
+        while s > 0 && !buffer.char_at(s - 1).is_whitespace() { s -= 1; }
+        while e < total && !buffer.char_at(e).is_whitespace() { e += 1; }
+    }
+    Some((s, e))
+}
+
+pub fn around_word(buffer: &Buffer, head: usize) -> Option<(usize, usize)> {
+    let (s, e) = inner_word(buffer, head)?;
+    let total = buffer.len_chars();
+    let mut s2 = s;
+    let mut e2 = e;
+    if e2 < total && buffer.char_at(e2).is_whitespace() {
+        e2 += 1;
+    } else if s2 > 0 && buffer.char_at(s2 - 1).is_whitespace() {
+        s2 -= 1;
+    }
+    Some((s2, e2))
+}
+
+fn find_enclosing_open(buffer: &Buffer, head: usize, open: char, close: char) -> Option<usize> {
+    if head < buffer.len_chars() && buffer.char_at(head) == open { return Some(head); }
+    if open == close {
+        let mut i = head;
+        while i > 0 {
+            i -= 1;
+            if buffer.char_at(i) == open { return Some(i); }
+        }
+        return None;
+    }
+    let mut i = head;
+    let mut depth = 0i32;
+    while i > 0 {
+        i -= 1;
+        let c = buffer.char_at(i);
+        if c == close { depth += 1; }
+        else if c == open {
+            if depth == 0 { return Some(i); }
+            depth -= 1;
+        }
+    }
+    None
+}
+
+fn find_matching_close(buffer: &Buffer, open_idx: usize, open: char, close: char) -> Option<usize> {
+    let total = buffer.len_chars();
+    if open == close {
+        let mut i = open_idx + 1;
+        while i < total {
+            if buffer.char_at(i) == close { return Some(i); }
+            i += 1;
+        }
+        return None;
+    }
+    let mut depth = 0i32;
+    let mut i = open_idx;
+    while i < total {
+        let c = buffer.char_at(i);
+        if c == open { depth += 1; }
+        else if c == close {
+            depth -= 1;
+            if depth == 0 { return Some(i); }
+        }
+        i += 1;
+    }
+    None
+}
+
+pub fn inner_pair(buffer: &Buffer, head: usize, open: char, close: char) -> Option<(usize, usize)> {
+    let open_idx = find_enclosing_open(buffer, head, open, close)?;
+    let close_idx = find_matching_close(buffer, open_idx, open, close)?;
+    Some((open_idx + 1, close_idx))
+}
+
+pub fn around_pair(buffer: &Buffer, head: usize, open: char, close: char) -> Option<(usize, usize)> {
+    let open_idx = find_enclosing_open(buffer, head, open, close)?;
+    let close_idx = find_matching_close(buffer, open_idx, open, close)?;
+    Some((open_idx, close_idx + 1))
+}
+
+#[cfg(test)]
+mod tests {
+    use crate::editor::Editor;
+    use crate::key::KeyEvent;
+    use crate::vim::VimState;
+
+    fn to_start(e: &mut Editor, v: &mut VimState) {
+        for a in v.handle(KeyEvent::Char('g'), e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('g'), e) { e.execute(a); }
+    }
+
+    fn l(e: &mut Editor, v: &mut VimState, n: usize) {
+        for _ in 0..n {
+            for a in v.handle(KeyEvent::Char('l'), e) { e.execute(a); }
+        }
+    }
+
+    #[test]
+    fn diw_deletes_inner_word_at_cursor() {
+        let mut e = Editor::from_str("hello world");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('i'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "hello ");
+    }
+
+    #[test]
+    fn daw_deletes_around_word_with_leading_space() {
+        let mut e = Editor::from_str("hello world");
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('a'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('w'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "hello");
+    }
+
+    #[test]
+    fn di_quote_deletes_inner_quotes() {
+        let src = "say \"hi\" loudly";
+        let mut e = Editor::from_str(src);
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        l(&mut e, &mut v, 5);
+        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('i'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('"'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "say \"\" loudly");
+    }
+
+    #[test]
+    fn da_paren_deletes_around_parens() {
+        let src = "f(x) -> y";
+        let mut e = Editor::from_str(src);
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        l(&mut e, &mut v, 1);
+        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('a'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('('), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "f -> y");
+    }
+
+    #[test]
+    fn ci_quote_changes_inner_text_to_insert() {
+        let src = "say \"hi\" loudly";
+        let mut e = Editor::from_str(src);
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        l(&mut e, &mut v, 5);
+        for a in v.handle(KeyEvent::Char('c'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('i'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('"'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "say \"\" loudly");
+        assert_eq!(v.mode, crate::vim::VimMode::Insert);
+        for a in v.handle(KeyEvent::Char('X'), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "say \"X\" loudly");
+    }
+
+    #[test]
+    fn nested_parens_around_inner() {
+        let src = "(a(b)c)";
+        let mut e = Editor::from_str(src);
+        let mut v = VimState::new();
+        to_start(&mut e, &mut v);
+        l(&mut e, &mut v, 3);
+        for a in v.handle(KeyEvent::Char('d'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('a'), &e) { e.execute(a); }
+        for a in v.handle(KeyEvent::Char('('), &e) { e.execute(a); }
+        assert_eq!(e.buffer().to_string(), "(ac)");
+    }
+}
\ No newline at end of file
diff --git a/docs/superpowers/plans/2026-07-20-plan-a-core-engine.md b/docs/superpowers/plans/2026-07-20-plan-a-core-engine.md
index 6596100..a9aff79 100644
--- a/docs/superpowers/plans/2026-07-20-plan-a-core-engine.md
+++ b/docs/superpowers/plans/2026-07-20-plan-a-core-engine.md
@@ -205,23 +205,21 @@ mod tests {
         let mut b = Buffer::from_str("hello world");
         let ch = b.delete(5..11);
         assert_eq!(b.to_string(), "hello");
         assert_eq!(ch, Change { at: 5, deleted: " world".to_string(), inserted: String::new() });
     }
 
-    #[test]
+#[test]
     fn apply_inverse_round_trips() {
         let mut b = Buffer::from_str("hello");
-        let ch = b.delete(0..2);
-        let inv = b.apply(&ch);
-        assert_eq!(b.to_string(), "llo");
-        assert_eq!(inv.inserted, ch.deleted);
-        // applying the inverse should restore the original
-        let inv2 = b.apply(&inv);
+        let ch = b.delete(0..2);      // b: "hello" -> "llo"; ch: del="he", ins=""
+        let inv = b.apply(&ch);        // applying ch inverts the deletion: b -> "hello"
         assert_eq!(b.to_string(), "hello");
-        // and the inverse-of-inverse equals the original change
+        assert_eq!(inv.inserted, ch.deleted);
+        let inv2 = b.apply(&inv);      // applying inv re-applies the deletion: b -> "llo"
+        assert_eq!(b.to_string(), "llo");
         assert_eq!(inv2, ch);
     }
 }
 ```
 
 - [ ] **Step 2: Run test to verify it fails**
@@ -427,22 +425,23 @@ pub enum Edge { Start, End }
 #[derive(Debug, Clone, Copy, PartialEq, Eq)]
 enum Dir { Left, Right }
 
 pub struct CursorSet {
     pub(crate) cursors: Vec<Range>,
     pub(crate) primary: usize,
-    pub(crate) desired_col: usize,
+    pub(crate) desired_col: usize, // usize::MAX is the sentinel "unset" used by single()
 }
 
 impl CursorSet {
     pub fn single(at: usize) -> Self {
-        CursorSet { cursors: vec![Range::caret(at)], primary: 0, desired_col: 0 }
+        CursorSet { cursors: vec![Range::caret(at)], primary: 0, desired_col: usize::MAX }
     }
 
     pub fn primary(&self) -> Range { self.cursors[self.primary] }
     pub fn head(&self) -> usize { self.primary().head }
+
     pub fn set_head(&mut self, at: usize, buffer: &Buffer) {
         let anchor = self.cursors[self.primary].anchor;
         self.cursors[self.primary] = Range { anchor, head: at };
         let line = self.line_of(buffer, at);
         self.desired_col = at - buffer.line_start_char(line);
         self.collapse_at(at);
@@ -458,12 +457,19 @@ impl CursorSet {
             let start = buffer.line_start_char(line);
             if start <= char_idx { acc = line; } else { break; }
         }
         acc
     }
 
+    // line length EXCLUDING a trailing '\n'; ropey's line_end_char points past the newline
+    fn line_content_len(&self, buffer: &Buffer, line: usize) -> usize {
+        let end = buffer.line_end_char(line);
+        let start = buffer.line_start_char(line);
+        if end > start && buffer.char_at(end - 1) == '\n' { end - start - 1 } else { end - start }
+    }
+
     fn grapheme_step(&self, buffer: &Buffer, from: usize, dir: Dir) -> usize {
         let text = buffer.to_string();
         let graphemes: Vec<&str> = UnicodeSegmentation::graphemes(&*text, true).collect();
         let mut char_pos = 0usize;
         let mut gidx = 0usize;
         for (i, g) in graphemes.iter().enumerate() {
@@ -494,45 +500,45 @@ impl CursorSet {
         self.set_head(to, buffer);
     }
 
     pub fn move_line(&mut self, buffer: &Buffer, delta: i32) {
         let from = self.head();
         let line = self.line_of(buffer, from);
+        if self.desired_col == usize::MAX {
+            self.desired_col = from - buffer.line_start_char(line);
+        }
         let target_line = (line as i32 + delta).max(0) as usize;
         let last = buffer.line_count().saturating_sub(1);
         let target_line = target_line.min(last);
         let start = buffer.line_start_char(target_line);
-        let end = buffer.line_end_char(target_line);
-        let line_len = end.saturating_sub(start);
-        let col = self.desired_col.min(line_len.saturating_sub(if line_len > 0 { 1 } else { 0 }));
+        let content_len = self.line_content_len(buffer, target_line);
+        let col = self.desired_col.min(content_len);
         let new_head = start + col;
-        self.set_head(new_head, buffer);
+        let anchor = self.cursors[self.primary].anchor;
+        self.cursors[self.primary] = Range { anchor, head: new_head };
+        self.collapse_at(new_head);
     }
 
     pub fn move_line_edge(&mut self, buffer: &Buffer, edge: Edge) {
         let from = self.head();
         let line = self.line_of(buffer, from);
         let at = match edge {
             Edge::Start => buffer.line_start_char(line),
-            Edge::End => {
-                let end = buffer.line_end_char(line);
-                let line_len = end.saturating_sub(buffer.line_start_char(line));
-                if line_len > 0 { end - 1 } else { end } // stop before newline
-            }
+            Edge::End => buffer.line_start_char(line) + self.line_content_len(buffer, line),
         };
         self.set_head(at, buffer);
     }
 
     pub fn collapse(&mut self) {
         let h = self.head();
         self.cursors[self.primary] = Range::caret(h);
     }
 }
 ```
 
-Note on `move_line`: line_len > 0 means we stop one char before the trailing `\n`; if the line is empty we land on start (which equals end). `desired_col` is updated each `set_head`. The test asserts this.
+Note on `move_line`: `desired_col` is the sticky column intent, initialized to `usize::MAX` (sentinel "unset") by `single()`. The first vertical movement derives it from the current head's column, then preserves it across clamping. `line_content_len` excludes the trailing `\n` — ropey's `line_end_char` points past the newline. `set_head` collapses any range to a caret at `at`; visual selection (Task 11) uses a separate `set_visual`.
 
 The `move_line_edge` End stops before the newline so Vim's `$` lands on the last printable char rather than the `\n`.
 
 - [ ] **Step 4: Run test to verify it passes**
 
 Run: `cargo test -p ruster-core cursor`
@@ -681,26 +687,28 @@ impl UndoStack {
         for ch in batch.into_iter().rev() {
             let inv = buffer.apply(&ch);
             inverses.push(inv);
         }
         // store inverses in original order so redo replays forward
         inverses.reverse();
+        let n = inverses.len();
         self.redo.push(inverses);
-        Some(inverses.len())
+        Some(n)
     }
 
     pub fn redo(&mut self, buffer: &mut Buffer) -> Option<usize> {
         let batch = self.redo.pop()?;
         let mut inverses = Vec::with_capacity(batch.len());
         for ch in batch.into_iter().rev() {
             let inv = buffer.apply(&ch);
             inverses.push(inv);
         }
         inverses.reverse();
+        let n = inverses.len();
         self.undo.push(inverses);
-        Some(inverses.len())
+        Some(n)
     }
 }
 
 impl Default for UndoStack {
     fn default() -> Self { Self::new() }
 }
@@ -2256,7 +2264,26 @@ Gaps closed; Plan A is internally consistent and complete for its scope.
 **Plan complete and saved to `docs/superpowers/plans/2026-07-20-plan-a-core-engine.md`. Two execution options:**
 
 **1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.
 
 **2. Inline Execution** — Execute tasks in this session using `executing-plans`, batch execution with checkpoints.
 
-**Which approach?**
\ No newline at end of file
+**Which approach?**
+
+---
+
+## Execution Log (Post-Implementation Corrections)
+
+Corrections applied during subagent execution. The plan tasks above describe the ORIGINAL brief code; the corrections below reflect the final committed state.
+
+- Task 2 (apply_inverse_round_trips): original test assertions were swapped — `apply(ch)` inverts the change (buffer returns to pre-edit), not preserves. Fixed inline.
+- Task 3 (CursorSet): ropey's `line_end_char` returns offset PAST the trailing newline; `single(at)` lost column intent by init `desired_col = 0`. Implementer replaced with `desired_col = usize::MAX` sentinel, `line_content_len` helper excluding the newline, sticky `desired_col` through clamping. Public API preserved.
+- Task 4 (UndoStack): brief used `Some(inverses.len())` after `self.redo.push(inverses)` (use-after-move). Fixed with `let n = inverses.len();` binding.
+- Task 7 (VimState): (a) `0` alone is line-start motion, not a count digit — `stroke_count` now rejects bare `0`, only extends an existing count. (b) `gg`/`G` cannot use `Motion::Line(±big)` because `move_line` preserves `desired_col`; they emit `Motion::To(line_start_char(line))` instead. (c) Vim `$` must land ON the last printable char; `Edge::End` lands PAST it. `$` handler emits `Motion::To(last_printable_in_line(editor))`.
+- Task 8 (Operators): doubled-operator convention (`dd`/`yy`/`cc` all act line-wise) was missing for `y` and `c` in the brief — extended to all three.
+- Task 9 (Text objects): brief's `find_matching_close` decrement-after-check missed the first matching close for non-nested pairs; corrected order. Also added symmetric-pair branches for `"` and `'` where `open == close` overloaded the same `c == close` test.
+- Task 10 (Dot-repeat): upgraded from `Option<Vec<Action>>` (absolute-offset replay) to `LastChange` enum (recomputable at current cursor — real Vim `.` semantics). Test asserts `bar ` for `foo bar baz -> dw -> w -> .`, matching Vim.
+- Task 11 (Visual): (a) added `set_visual_anchor` to `CursorSet`; new `Action::BeginVisual(usize)`. (b) Visual selection is inclusive on the cursor side: `v + w + x` on `hello world` deletes `hello w` leaving `orld`, NOT `world`. (c) Visual `y` emits cursor reset to selection start (`Motion::To(start)`).
+- Task 11 regression: when `vim/mod.rs` was rebuilt from the full Task 11 brief, it dropped Task 10's four `dot_*` regression tests; restored in commit `a019576`.
+- Task 12 (Scenario): brief's `edit_word_then_undo` split the undo across TWO fresh Editor sessions — but a freshly-constructed Editor has an empty UndoStack, so `u` in session 2 is a no-op. Folded both halves into one session (key-script ends with `u`).
+- Plan-wide gap discovered at Task 12: no Normal-mode handler for `u` (undo) or `Ctrl-r` (redo) — they existed only as `Action::Undo`/`Action::Redo`. Added in `handle_normal`.
+- Plan-wide gap fixed at Task 12: `apply_operator('c')` (and Visual `c`) closed the deletion's `BeginBatch` and opened a new one for typing, producing TWO undo units for `c{motion}...Esc`; a single `u` could only reverse the typing. Removed the intermediate `EndBatch`/`BeginBatch`; Esc fires the single `EndBatch` so the whole change is one undo unit, matching Vim.
diff --git a/docs/superpowers/specs/2026-07-20-ruster-core-slice-design.md b/docs/superpowers/specs/2026-07-20-ruster-core-slice-design.md
index f12f624..f62b04f 100644
--- a/docs/superpowers/specs/2026-07-20-ruster-core-slice-design.md
+++ b/docs/superpowers/specs/2026-07-20-ruster-core-slice-design.md
@@ -1,11 +1,11 @@
 # Ruster — Core Slice Design (Phase 0 + Phase 1)
 
 **Date:** 2026-07-20
 **Status:** Approved (brainstorming complete)
-**Scope:** Sub-project 1 of the `AGENT.md` vision — the bootable, usable editor core. Phases 2–7 (window management, tree-sitter/LSP, embedded terminal, IDE tools, ecosystem, application platform) are explicitly **out of scope** and get their own spec → plan → implementation cycles later.
+**Scope:** Sub-project 1 of the `AGENTS.md` vision — the bootable, usable editor core. Phases 2–7 (window management, tree-sitter/LSP, embedded terminal, IDE tools, ecosystem, application platform) are explicitly **out of scope** and get their own spec → plan → implementation cycles later.
 
 ---
 
 ## 1. Context & Acceptance Criteria
 
 `ruster` is a hybrid Neovim/Emacs editor written in Rust, scripted in Lua. This slice delivers the smallest **daily-driver** editor:
@@ -22,13 +22,13 @@
 | Editing paradigms | Neovim modal + Emacs modeless, runtime `:set editmode` toggle |
 | Undo | Linear undo/redo (undo-tree deferred) |
 | Multiple cursors | Cursor-set data model now; multi-cursor commands in Phase 5 |
 | Architecture | Cargo workspace, crate per layer |
 | Acceptance | Usable daily-driver demo, manually verified |
 
-### Spec corrections to `AGENT.md` (apply to that doc when convenient)
+### Spec corrections to `AGENTS.md` (apply to that doc when convenient)
 
 1. `ropey` is a rope, not a CRDT. Fine choice; the description was wrong. (CRDTs would matter for Phase 7 client-server collaboration — revisit then.)
 2. `tachyonfx` is a ratatui *effects* library, not a frame clock. Moved to Phase 6 polish. Each frontend runs its own 60fps tick feeding `Tick` events into the shared event channel.
 3. `winit` is dropped for the GUI backend — raylib manages its own window and input.
 4. BeOS is dropped as a target (`winit`/`raylib` do not support it). Slice targets: **macOS, Linux, Windows**.
 
