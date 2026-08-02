use ropey::Rope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub at: usize,
    pub deleted: String,
    pub inserted: String,
}

pub struct Buffer {
    rope: Rope,
    /// Bumped on every mutation, so a consumer can tell "unchanged since I last
    /// looked" without comparing contents.
    ///
    /// Exists because syntax highlighting re-parsed the whole buffer *every
    /// frame* — 107 ms on a 10k-line file, against a 16.7 ms budget — with no
    /// way to know nothing had changed. Comparing text would be the same O(n)
    /// walk the reparse was doing.
    revision: u64,
}

impl std::fmt::Display for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.rope)
    }
}

impl Buffer {
    pub fn new() -> Self { Self { rope: Rope::new(), revision: 0 } }
    // An infallible constructor, not the fallible `FromStr` trait.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self { Self { rope: Rope::from_str(s), revision: 0 } }

    /// A counter that changes whenever the text does. Equal revisions mean
    /// equal contents; unequal ones mean it *may* have changed.
    pub fn revision(&self) -> u64 { self.revision }

    pub fn len_chars(&self) -> usize { self.rope.len_chars() }
    pub fn line_count(&self) -> usize { self.rope.len_lines() }
    pub fn char_at(&self, idx: usize) -> char { self.rope.char(idx) }
    pub fn slice_string(&self, start: usize, end: usize) -> String { self.rope.slice(start..end).to_string() }
    pub fn line_to_string(&self, line_idx: usize) -> String {
        self.rope.line(line_idx).to_string()
    }
    pub fn line_start_char(&self, line_idx: usize) -> usize {
        self.rope.line_to_char(line_idx)
    }
    pub fn line_end_char(&self, line_idx: usize) -> usize {
        if line_idx + 1 >= self.rope.len_lines() {
            self.rope.len_chars()
        } else {
            self.rope.line_to_char(line_idx + 1)
        }
    }

    pub fn char_to_line(&self, char_idx: usize) -> usize {
        self.rope.char_to_line(char_idx)
    }

    /// Length of a line in chars, excluding its trailing newline. This is the
    /// number of columns a cursor can occupy on the line, so it is what column
    /// clamping and end-of-line motions want.
    pub fn line_content_len(&self, line_idx: usize) -> usize {
        let start = self.line_start_char(line_idx);
        let end = self.line_end_char(line_idx);
        if end > start && self.char_at(end - 1) == '\n' {
            end - start - 1
        } else {
            end - start
        }
    }

    pub fn insert(&mut self, at: usize, text: &str) -> Change {
        self.rope.insert(at, text);
        self.revision += 1;
        Change { at, deleted: String::new(), inserted: text.to_string() }
    }

    pub fn delete(&mut self, range: std::ops::Range<usize>) -> Change {
        let deleted = self.rope.slice(range.clone()).to_string();
        let at = range.start;
        self.rope.remove(range);
        self.revision += 1;
        Change { at, deleted, inserted: String::new() }
    }

    /// Apply a change; returns the inverse change that would undo this application.
    /// A change `c` means "the buffer currently has `c.inserted` present at `c.at`,
    /// where `c.deleted` was previously." apply() removes the inserted span and
    /// re-inserts the deleted span, returning the inverse change.
    pub fn apply(&mut self, ch: &Change) -> Change {
        let ins_len = ch.inserted.chars().count();
        self.rope.remove(ch.at..ch.at + ins_len);
        self.rope.insert(ch.at, &ch.deleted);
        Change { at: ch.at, deleted: ch.inserted.clone(), inserted: ch.deleted.clone() }
    }
}

impl Default for Buffer {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_returns_change_and_text() {
        let mut b = Buffer::from_str("helo");
        let ch = b.insert(4, "!");
        assert_eq!(b.to_string(), "helo!");
        assert_eq!(ch, Change { at: 4, deleted: String::new(), inserted: "!".to_string() });
    }

    #[test]
    fn delete_returns_change_and_text() {
        let mut b = Buffer::from_str("hello world");
        let ch = b.delete(5..11);
        assert_eq!(b.to_string(), "hello");
        assert_eq!(ch, Change { at: 5, deleted: " world".to_string(), inserted: String::new() });
    }

    #[test]
    fn apply_inverse_round_trips() {
        let mut b = Buffer::from_str("hello");
        let ch = b.delete(0..2);      // b: "hello" -> "llo"; ch: del="he", ins=""
        let inv = b.apply(&ch);        // applying ch inverts the deletion: b -> "hello"
        assert_eq!(b.to_string(), "hello");
        assert_eq!(inv.inserted, ch.deleted);
        let inv2 = b.apply(&inv);      // applying inv re-applies the deletion: b -> "llo"
        assert_eq!(b.to_string(), "llo");
        assert_eq!(inv2, ch);
    }

#[cfg(test)]
mod revision_tests {
    use super::*;

    #[test]
    fn the_revision_changes_only_when_the_text_does() {
        let mut b = Buffer::from_str("hello");
        let start = b.revision();

        // Reads do not count as changes.
        let _ = b.to_string();
        let _ = b.line_count();
        assert_eq!(b.revision(), start, "reading is not a change");

        b.insert(5, " world");
        let after_insert = b.revision();
        assert_ne!(after_insert, start, "an insert is a change");

        b.delete(0..1);
        assert_ne!(b.revision(), after_insert, "a delete is a change");
    }

    /// Two edits must not collide onto one revision, or the second is missed.
    #[test]
    fn every_edit_gets_its_own_revision() {
        let mut b = Buffer::new();
        let mut seen = vec![b.revision()];
        for i in 0..5 {
            b.insert(i, "x");
            seen.push(b.revision());
        }
        let mut sorted = seen.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), seen.len(), "revisions repeat: {seen:?}");
    }
}
}