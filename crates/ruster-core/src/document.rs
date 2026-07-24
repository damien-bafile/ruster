use std::path::PathBuf;

use crate::buffer::Buffer;
use crate::undo::UndoStack;

/// Handle to a [`Document`] inside a [`crate::workspace::BufferStore`].
///
/// Cheap to copy; stable for the lifetime of the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BufferId(pub u32);

/// A built-in "special" buffer that ruster renders and drives itself
/// (as opposed to plain file/scratch text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialKind {
    Ibuffer,
    Dired,
    Picker,
}

/// What a document represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    /// Backed by a file on disk.
    File,
    /// An unnamed, throwaway text buffer.
    Scratch,
    /// A ruster-managed UI buffer (buffer list, file explorer, ...).
    Special(SpecialKind),
}

const DEFAULT_INDENT: &str = "    ";

/// A document is what vim calls a "buffer": the text plus its file identity
/// and undo history. It deliberately does **not** own a cursor or scroll
/// position — those are per-window (see [`crate::windows`]).
pub struct Document {
    pub buffer: Buffer,
    pub undo: UndoStack,
    pub file_path: Option<PathBuf>,
    pub name: String,
    pub modified: bool,
    pub kind: DocKind,
    /// Buffer-local indent string (spaces), seeded from config / EditorConfig.
    pub indent: String,
}

impl Document {
    /// A document backed by `path`, initialized with `content` already read
    /// from disk. The display name is the file's final component.
    pub fn from_file(path: PathBuf, content: &str) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        Document {
            buffer: Buffer::from_str(content),
            undo: UndoStack::new(),
            file_path: Some(path),
            name,
            modified: false,
            kind: DocKind::File,
            indent: DEFAULT_INDENT.to_string(),
        }
    }

    /// An unnamed scratch document with no backing file.
    pub fn scratch(name: &str) -> Self {
        Document {
            buffer: Buffer::new(),
            undo: UndoStack::new(),
            file_path: None,
            name: name.to_string(),
            modified: false,
            kind: DocKind::Scratch,
            indent: DEFAULT_INDENT.to_string(),
        }
    }

    /// A ruster-managed UI document (ibuffer, dired, picker).
    pub fn special(kind: SpecialKind, name: &str) -> Self {
        Document {
            buffer: Buffer::new(),
            undo: UndoStack::new(),
            file_path: None,
            name: name.to_string(),
            modified: false,
            kind: DocKind::Special(kind),
            indent: DEFAULT_INDENT.to_string(),
        }
    }

    /// Set the buffer-local indent to `n` spaces.
    pub fn set_indent_width(&mut self, n: u32) {
        self.indent = " ".repeat(n as usize);
    }

    /// Ruster-managed special buffers (dired, ibuffer, picker) are non-editable
    /// — they are UI listings the app owns, so edits are gated out.
    pub fn read_only(&self) -> bool {
        matches!(self.kind, DocKind::Special(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_file_derives_name_from_path() {
        let d = Document::from_file(PathBuf::from("/tmp/foo/bar.rs"), "hi");
        assert_eq!(d.name, "bar.rs");
        assert_eq!(d.kind, DocKind::File);
        assert_eq!(d.buffer.to_string(), "hi");
        assert!(!d.modified);
        assert_eq!(d.file_path, Some(PathBuf::from("/tmp/foo/bar.rs")));
    }

    #[test]
    fn scratch_has_no_path() {
        let d = Document::scratch("[No Name]");
        assert_eq!(d.kind, DocKind::Scratch);
        assert!(d.file_path.is_none());
        assert_eq!(d.name, "[No Name]");
    }

    #[test]
    fn special_carries_kind() {
        let d = Document::special(SpecialKind::Dired, "*dired*");
        assert_eq!(d.kind, DocKind::Special(SpecialKind::Dired));
        assert!(d.file_path.is_none());
    }

    #[test]
    fn special_document_is_read_only() {
        assert!(Document::special(SpecialKind::Dired, "*dired*").read_only());
        assert!(Document::special(SpecialKind::Ibuffer, "*ibuffer*").read_only());
        assert!(!Document::from_file(PathBuf::from("a.rs"), "x").read_only());
        assert!(!Document::scratch("[No Name]").read_only());
    }

    #[test]
    fn set_indent_width_uses_spaces() {
        let mut d = Document::scratch("s");
        d.set_indent_width(2);
        assert_eq!(d.indent, "  ");
    }
}
