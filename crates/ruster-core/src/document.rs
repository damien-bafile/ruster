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
    /// An embedded terminal. The buffer text is a placeholder; the live grid
    /// lives in the app's `TerminalSession` keyed by this buffer's id.
    Terminal,
    /// A read-only listing of config load/validation errors.
    ConfigErrors,
    /// A read-only results buffer for a build/test/task run.
    Build,
    /// The file-explorer sidebar (tree dired) in a persistent side column.
    Sidebar,
    /// One pane of a side-by-side diff (`:Diffview`).
    Diff,
    /// The `:Mason` external-tool listing.
    Mason,
    /// The `:help` manual.
    Help,
    /// The `:Git` status view.
    Git,
    /// The Dashboard / welcome screen — a static page that cannot be closed.
    Dashboard,
    /// A general-purpose message log (build, LSP, echo, etc.).
    Message,
    /// The aggregated problem list: diagnostics, quickfix entries and TODO
    /// markers, grouped by file.
    Trouble,
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

/// The line-ending convention a document was loaded with, so it can be
/// preserved on save. The in-memory buffer is always normalized to `\n`;
/// this only affects how bytes are written back to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// Unix `\n`.
    Lf,
    /// Windows `\r\n`.
    Crlf,
}

impl LineEnding {
    /// Detect the ending from raw file content. The first `\r\n` wins;
    /// otherwise we assume LF (including empty and lone-`\r` content).
    pub fn detect(content: &str) -> LineEnding {
        if content.contains("\r\n") {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        }
    }
}

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
    /// The on-disk line ending to preserve when saving. The buffer itself is
    /// always LF-normalized.
    pub line_ending: LineEnding,
    /// When true, `BufferStore::close()` refuses to remove this document.
    pub pinned: bool,
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
        let line_ending = LineEnding::detect(content);
        // Keep the in-memory buffer LF-only regardless of the source encoding.
        let normalized = match line_ending {
            LineEnding::Crlf => content.replace("\r\n", "\n"),
            LineEnding::Lf => content.to_string(),
        };
        Document {
            buffer: Buffer::from_str(&normalized),
            undo: UndoStack::new(),
            file_path: Some(path),
            name,
            modified: false,
            kind: DocKind::File,
            indent: DEFAULT_INDENT.to_string(),
            line_ending,
            pinned: false,
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
            line_ending: LineEnding::Lf,
            pinned: false,
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
            line_ending: LineEnding::Lf,
            pinned: false,
        }
    }

    /// Set the buffer-local indent to `n` spaces.
    pub fn set_indent_width(&mut self, n: u32) {
        self.indent = " ".repeat(n as usize);
    }

    /// The buffer's text encoded with this document's on-disk line ending,
    /// ready to write to `file_path`. The buffer is LF-internally, so this
    /// re-applies `\r\n` for CRLF documents.
    pub fn encode_content(&self) -> String {
        match self.line_ending {
            LineEnding::Lf => self.buffer.to_string(),
            LineEnding::Crlf => self.buffer.to_string().replace('\n', "\r\n"),
        }
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

    #[test]
    fn from_file_detects_lf_and_keeps_content() {
        let d = Document::from_file(PathBuf::from("a.txt"), "a\nb\n");
        assert_eq!(d.line_ending, LineEnding::Lf);
        assert_eq!(d.buffer.to_string(), "a\nb\n");
        assert_eq!(d.encode_content(), "a\nb\n");
    }

    #[test]
    fn from_file_detects_crlf_and_normalizes_to_lf_in_buffer() {
        let d = Document::from_file(PathBuf::from("a.txt"), "a\r\nb\r\n");
        assert_eq!(d.line_ending, LineEnding::Crlf);
        // In-memory buffer is always LF so editing/rendering logic stays simple.
        assert_eq!(d.buffer.to_string(), "a\nb\n");
    }

    #[test]
    fn crlf_document_round_trips_to_crlf_on_encode() {
        let d = Document::from_file(PathBuf::from("a.txt"), "a\r\nb\r\n");
        assert_eq!(d.encode_content(), "a\r\nb\r\n");
    }

    #[test]
    fn scratch_defaults_to_lf() {
        let d = Document::scratch("s");
        assert_eq!(d.line_ending, LineEnding::Lf);
    }
}
