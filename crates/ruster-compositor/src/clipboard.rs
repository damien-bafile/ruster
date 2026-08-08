//! The editor pane's clipboard, joined to the Wayland one.
//!
//! An editor pane runs `ruster_core`'s vim keymap, and `VimState` reaches for
//! `arboard` to talk to the system clipboard. Inside a compositor that is the
//! wrong way round: on a DRM boot there is no display server for `arboard` to
//! connect to, because this process *is* the display server. So yanking in a
//! pane put text somewhere no client could see, and pasting could not see what
//! a client had copied — two clipboards and one keyboard.
//!
//! The compositor already owns the real one. This bridges them in both
//! directions:
//!
//! - **Yank.** After a key that changes the pane's clipboard, the text is
//!   published as the seat's selection. Clients then ask for it, which arrives
//!   as [`SelectionHandler::send_selection`] and is answered from
//!   [`Clipboard::text`].
//! - **Paste.** A client taking the selection arrives as
//!   [`SelectionHandler::new_selection`]. The data is *not* fetched then and
//!   there: reading it means asking the owning client for a pipe and waiting,
//!   while a paste in a pane is synchronous and cannot wait for anything.
//!
//! So a paste uses the last selection the compositor was told about, fetched on
//! the event loop when the announcement arrived. In practice that is the same
//! text — the fetch takes microseconds and copying is followed by pasting, not
//! raced with it — but it is a cache, not a live read, and a client that changes
//! its selection contents without re-announcing would go unnoticed. The
//! alternative is blocking the compositor on a client's pipe during a keystroke,
//! which trades a rare staleness for a hang that would take the whole session
//! with it.

use std::io::Read;
use std::os::unix::io::OwnedFd;

/// The text the compositor currently holds as its selection, and who last set
/// it.
#[derive(Debug, Default)]
pub struct Clipboard {
    text: String,
    /// True while the *compositor* owns the selection, i.e. a pane yanked last.
    ///
    /// Kept because `set_data_device_selection` makes the compositor the owner,
    /// and answering `send_selection` for a selection a client owns would serve
    /// stale text over the client's own.
    owned: bool,
}

/// The mime types a pane's selection is offered as.
///
/// Both spellings: `text/plain;charset=utf-8` is what modern toolkits ask for,
/// and bare `text/plain` is what older ones send. Offering only the first means
/// a paste into some clients silently produces nothing.
pub fn mime_types() -> Vec<String> {
    vec![
        "text/plain;charset=utf-8".to_string(),
        "text/plain".to_string(),
        "UTF8_STRING".to_string(),
    ]
}

/// Whether `mime` is one this compositor will answer with plain text.
pub fn is_text_mime(mime: &str) -> bool {
    let mime = mime.trim().to_ascii_lowercase();
    mime.starts_with("text/plain") || mime == "utf8_string" || mime == "string"
}

/// The mime type to request from a client's selection, given what it offers.
///
/// Prefers an explicitly UTF-8 one, because a bare `text/plain` is only
/// *conventionally* UTF-8 and the difference shows up as mojibake rather than as
/// an error.
pub fn preferred_mime(offered: &[String]) -> Option<String> {
    let utf8 = offered
        .iter()
        .find(|m| m.to_ascii_lowercase().contains("charset=utf-8"));
    utf8.or_else(|| offered.iter().find(|m| is_text_mime(m)))
        .cloned()
}

impl Clipboard {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_owned(&self) -> bool {
        self.owned
    }

    /// Record text a pane yanked. The caller publishes it to the seat.
    pub fn set_from_pane(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.owned = true;
    }

    /// Record text read back from a client's selection.
    pub fn set_from_client(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.owned = false;
    }

    /// A client has taken the selection, so whatever the compositor was
    /// offering is no longer the selection.
    pub fn released(&mut self) {
        self.owned = false;
    }
}

/// Read a selection pipe to end, as UTF-8.
///
/// Capped, because the far end is another process: a client that offers a
/// gigabyte — or never closes the pipe — must not be able to exhaust this one's
/// memory or hold it open forever. A truncated paste is a visible annoyance; an
/// unbounded read inside the display server is not recoverable.
pub fn read_selection(fd: OwnedFd, limit: usize) -> std::io::Result<String> {
    let mut file = std::fs::File::from(fd);
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() >= limit {
            buf.truncate(limit);
            tracing::warn!(limit, "selection truncated");
            break;
        }
    }
    // Lossy rather than an error: a client offering `text/plain` that is not
    // valid UTF-8 is misbehaving, and pasting most of it beats pasting nothing
    // and saying why in a log nobody is reading.
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// How much of a client's selection is worth reading. Well past any sensible
/// paste, well short of anything that would hurt.
pub const SELECTION_LIMIT: usize = 8 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_spellings_of_plain_text_are_accepted() {
        // Modern toolkits ask for the charset form and older ones do not;
        // answering only one means a paste that silently produces nothing.
        assert!(is_text_mime("text/plain;charset=utf-8"));
        assert!(is_text_mime("text/plain"));
        assert!(is_text_mime("TEXT/PLAIN"));
        assert!(is_text_mime("UTF8_STRING"));
        assert!(!is_text_mime("image/png"));
        assert!(!is_text_mime("text/html"));
    }

    #[test]
    fn a_utf8_mime_is_preferred_over_a_bare_one() {
        // Bare `text/plain` is only conventionally UTF-8, and guessing wrong
        // shows up as mojibake rather than as an error anyone can act on.
        let offered = vec![
            "text/plain".to_string(),
            "text/plain;charset=utf-8".to_string(),
        ];
        assert_eq!(
            preferred_mime(&offered).as_deref(),
            Some("text/plain;charset=utf-8")
        );
    }

    #[test]
    fn a_selection_of_nothing_textual_is_not_requested() {
        let offered = vec!["image/png".to_string(), "text/html".to_string()];
        assert_eq!(preferred_mime(&offered), None);
    }

    #[test]
    fn ownership_follows_whoever_set_the_selection_last() {
        // `send_selection` is only ours to answer while we own it; answering for
        // a client's selection would serve our stale text over theirs.
        let mut clip = Clipboard::default();
        assert!(!clip.is_owned());
        clip.set_from_pane("yanked");
        assert!(clip.is_owned());
        assert_eq!(clip.text(), "yanked");
        clip.set_from_client("copied in firefox");
        assert!(!clip.is_owned());
        assert_eq!(clip.text(), "copied in firefox");
        clip.set_from_pane("yanked again");
        clip.released();
        assert!(!clip.is_owned(), "a client taking it releases ours");
    }

    #[test]
    fn a_selection_pipe_is_read_to_the_end() {
        let (rx, tx) = std::os::unix::net::UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            use std::io::Write;
            let mut tx = tx;
            let _ = tx.write_all(b"hello from a client");
        });
        let text = read_selection(OwnedFd::from(rx), SELECTION_LIMIT).unwrap();
        assert_eq!(text, "hello from a client");
    }

    #[test]
    fn an_oversized_selection_is_truncated_rather_than_swallowing_memory() {
        // The far end is another process. One that offers a gigabyte, or never
        // closes the pipe, must not be able to take the display server with it.
        let (rx, tx) = std::os::unix::net::UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            use std::io::Write;
            let mut tx = tx;
            // More than the limit under test, written until the reader stops.
            for _ in 0..64 {
                if tx.write_all(&[b'x'; 4096]).is_err() {
                    break;
                }
            }
        });
        let text = read_selection(OwnedFd::from(rx), 8192).unwrap();
        assert_eq!(text.len(), 8192);
    }
}

#[cfg(test)]
mod arboard_probe {
    /// How long `arboard::Clipboard::new()` takes with no display server.
    ///
    /// `VimState::new` constructs one, and a pane constructs a `VimState`, so
    /// this runs inside the compositor — on a bare VT with nothing to connect
    /// to. If it blocked, opening a pane would stall the display server, which
    /// is the kind of thing worth knowing before it happens on hardware rather
    /// than after.
    #[test]
    fn constructing_a_clipboard_without_a_display_returns_promptly() {
        let previous = std::env::var_os("WAYLAND_DISPLAY");
        // Safety: this test sets process-wide variables and restores them; no
        // other test in this crate reads them.
        unsafe {
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::remove_var("DISPLAY");
        }
        let started = std::time::Instant::now();
        let result = arboard::Clipboard::new();
        let elapsed = started.elapsed();
        if let Some(previous) = previous {
            unsafe { std::env::set_var("WAYLAND_DISPLAY", previous) };
        }
        println!(
            "arboard::Clipboard::new() -> {} in {elapsed:?}",
            if result.is_ok() { "Ok" } else { "Err" }
        );
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "arboard took {elapsed:?} with no display; opening a pane would stall the compositor"
        );
    }
}
