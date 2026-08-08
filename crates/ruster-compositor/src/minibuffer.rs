//! The `:` line — a command prompt for the compositor.
//!
//! Two things go in it, told apart by the prompt: `:` runs a WM action by name,
//! and `=` evaluates Lua in the same VM the config ran in. Both end up at the
//! same place — an [`Action`] handed to
//! [`dispatch`](crate::compositor::CompositorState::dispatch) — which is the
//! same rule the keybinds and the Lua API follow, so no route into the WM can
//! do something the others cannot.
//!
//! While it is open the compositor swallows every key. That is not a
//! convenience: a prompt that let keystrokes through to the focused client
//! would type your command into a terminal at the same time.

use ruster_render::Theme;

use crate::lua::Action;

/// What a submitted line asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Submission {
    /// A WM action to dispatch.
    Action(Action),
    /// Lua source to evaluate against the live VM.
    Lua(String),
    /// Nothing — an empty line, or a word that is not an action.
    Nothing(String),
}

/// The prompt kind, which decides how the line is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// `:` — a WM action name.
    Command,
    /// `=` — Lua source.
    Lua,
}

impl Prompt {
    pub fn sigil(self) -> &'static str {
        match self {
            Prompt::Command => ":",
            Prompt::Lua => "=",
        }
    }
}

/// The open mini-buffer: which prompt, and what has been typed into it.
#[derive(Debug, Clone, PartialEq)]
pub struct MiniBuffer {
    pub prompt: Prompt,
    pub input: String,
    /// A message shown instead of the prompt — the result of the last command,
    /// or why it did nothing. Cleared as soon as the prompt opens again.
    pub message: Option<String>,
}

impl MiniBuffer {
    pub fn new(prompt: Prompt) -> Self {
        MiniBuffer {
            prompt,
            input: String::new(),
            message: None,
        }
    }

    /// A closed mini-buffer showing a message.
    pub fn message(text: impl Into<String>) -> Self {
        MiniBuffer {
            prompt: Prompt::Command,
            input: String::new(),
            message: Some(text.into()),
        }
    }

    /// Whether this is taking keyboard input, as opposed to showing a result.
    pub fn is_open(&self) -> bool {
        self.message.is_none()
    }

    pub fn push(&mut self, c: char) {
        self.input.push(c);
    }

    /// Delete the last character, reporting whether the buffer is now empty.
    ///
    /// Backspacing past the start closes the prompt, the way it does in vim —
    /// otherwise an empty prompt has to be dismissed with Escape, which is one
    /// more thing to know.
    pub fn backspace(&mut self) -> bool {
        self.input.pop();
        self.input.is_empty()
    }

    /// Read the line.
    pub fn submit(&self) -> Submission {
        let line = self.input.trim();
        if line.is_empty() {
            return Submission::Nothing(String::new());
        }
        match self.prompt {
            Prompt::Lua => Submission::Lua(line.to_string()),
            Prompt::Command => match Action::from_name(line) {
                Some(action) => Submission::Action(action),
                // Named rather than silently dropped: a typo at a prompt should
                // say so, or the prompt looks broken rather than the input.
                None => Submission::Nothing(format!("not an action: {line}")),
            },
        }
    }

    /// The text to draw: the prompt and what has been typed, or the message.
    pub fn display(&self) -> String {
        match &self.message {
            Some(msg) => msg.clone(),
            None => format!("{}{}", self.prompt.sigil(), self.input),
        }
    }

    /// How many leading characters are the prompt sigil, so it can be drawn in
    /// the accent colour. Zero for a message, which is all one colour.
    pub fn sigil_len(&self) -> usize {
        match self.message {
            Some(_) => 0,
            None => self.prompt.sigil().len(),
        }
    }

    /// The colours this line draws in.
    ///
    /// Reuses the editor's cmdline roles rather than inventing compositor-only
    /// ones: it is the same control in the same product, and a user who themed
    /// one would be surprised to find the other unchanged.
    pub fn colors(theme: &Theme) -> (ruster_render::Color, ruster_render::Color) {
        (theme.cmdline_bg, theme.cmdline_fg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruster_shell::Direction;

    #[test]
    fn a_command_line_resolves_to_the_same_action_a_keybind_would() {
        // The point of routing through `from_name`: the prompt cannot grow its
        // own vocabulary that the keymap does not have, or vice versa.
        let mut mb = MiniBuffer::new(Prompt::Command);
        for c in "focus left".chars() {
            mb.push(c);
        }
        assert_eq!(
            mb.submit(),
            Submission::Action(Action::Focus(Direction::Left))
        );
    }

    #[test]
    fn a_line_that_is_not_an_action_says_so() {
        // Silently doing nothing makes the prompt look broken rather than the
        // input, which is the wrong thing to suspect.
        let mut mb = MiniBuffer::new(Prompt::Command);
        for c in "frobnicate".chars() {
            mb.push(c);
        }
        assert_eq!(
            mb.submit(),
            Submission::Nothing("not an action: frobnicate".into())
        );
    }

    #[test]
    fn the_lua_prompt_hands_its_line_over_verbatim() {
        // Lua is not normalised the way an action name is — case and
        // punctuation are the language.
        let mut mb = MiniBuffer::new(Prompt::Lua);
        for c in r#"ruster.wm.focus("Left")"#.chars() {
            mb.push(c);
        }
        assert_eq!(
            mb.submit(),
            Submission::Lua(r#"ruster.wm.focus("Left")"#.into())
        );
    }

    #[test]
    fn an_empty_line_does_nothing_quietly() {
        // Opening the prompt and changing your mind is not an error worth a
        // message.
        let mb = MiniBuffer::new(Prompt::Command);
        assert_eq!(mb.submit(), Submission::Nothing(String::new()));
        let mut spaces = MiniBuffer::new(Prompt::Command);
        spaces.push(' ');
        assert_eq!(spaces.submit(), Submission::Nothing(String::new()));
    }

    #[test]
    fn backspacing_past_the_start_reports_that_it_closed() {
        // Vim closes the prompt when you delete back past the sigil; otherwise
        // an empty prompt needs Escape, which is one more thing to know.
        let mut mb = MiniBuffer::new(Prompt::Command);
        mb.push('q');
        mb.push('a');
        assert!(!mb.backspace(), "still has a character");
        assert!(mb.backspace(), "the last one empties it");
    }

    #[test]
    fn the_sigil_is_measured_so_it_can_be_coloured_separately() {
        // The editor tints the leading `:` in the accent, which is what
        // distinguishes "waiting for a command" from "showing you a message".
        let mut mb = MiniBuffer::new(Prompt::Command);
        mb.push('q');
        assert_eq!(mb.display(), ":q");
        assert_eq!(mb.sigil_len(), 1);

        // A message has no sigil, so none of it is accented.
        let msg = MiniBuffer::message("not an action: frobnicate");
        assert_eq!(msg.display(), "not an action: frobnicate");
        assert_eq!(msg.sigil_len(), 0);
        assert!(!msg.is_open(), "a message is not taking input");
    }
}
