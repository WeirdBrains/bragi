//! Emoji reactions on messages. They live on the message (`ChatMessage::reactions`, stored in the
//! record's extra keys, so older records and apps that don't know them stay readable), sync with
//! the rest of history, and reach the model as a short note on the reacted message in the next
//! turn's context ([`with_note`]). [`crate::Core::reaction_events`] is the hook for forwarding
//! them as feedback.

use crate::error::{CoreError, ErrorKind, Result};
use crate::history::{ChatMessage, Role};

/// The quick reaction bar, in order. The shells also offer a picker for any other emoji.
pub const QUICK_REACTIONS: [&str; 6] = ["👍", "👎", "❤️", "😂", "🎉", "🤔"];

/// Most distinct reactions on one message.
const MAX_PER_MESSAGE: usize = 12;

/// What 👍 and 👎 mean as feedback on an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feedback {
    Good,
    Bad,
}

impl Feedback {
    pub fn of(emoji: &str) -> Option<Self> {
        match emoji.trim_end_matches('\u{fe0f}') {
            "👍" => Some(Feedback::Good),
            "👎" => Some(Feedback::Bad),
            _ => None,
        }
    }
}

/// A reaction added (`on`) or removed.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactionEvent {
    pub conversation: String,
    pub message: String,
    pub role: Role,
    /// The model and node that wrote the reacted answer.
    pub model: Option<String>,
    pub node: Option<String>,
    pub emoji: String,
    pub on: bool,
    /// Set for 👍 and 👎.
    pub feedback: Option<Feedback>,
    pub at_ms: i64,
}

/// Validates one emoji: short, no whitespace or control characters, not plain letters or digits.
pub(crate) fn check(emoji: &str) -> Result<String> {
    let e = emoji.trim();
    let bad = e.is_empty() || e.chars().count() > 12 || e.chars().any(|c| c.is_whitespace() || c.is_control() || c.is_ascii_alphanumeric());
    if bad {
        return Err(CoreError::new(ErrorKind::Refused, "that is not an emoji"));
    }
    Ok(e.to_string())
}

/// Adds `emoji` or removes it. Returns whether it is now on.
pub(crate) fn toggle(list: &mut Vec<String>, emoji: &str) -> Result<bool> {
    if let Some(i) = list.iter().position(|r| r == emoji) {
        list.remove(i);
        return Ok(false);
    }
    if list.len() >= MAX_PER_MESSAGE {
        return Err(CoreError::new(ErrorKind::Refused, "too many reactions on one message"));
    }
    list.push(emoji.to_string());
    Ok(true)
}

/// `text` with the message's reactions appended as a bracketed note, for the model's context.
pub(crate) fn with_note(m: &ChatMessage, text: String) -> String {
    if m.reactions.is_empty() {
        return text;
    }
    let what = match m.role {
        Role::Assistant => "this reply",
        Role::User => "their own message",
    };
    format!("{text}\n\n[The user reacted to {what} with {}]", m.reactions.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_adds_then_removes() {
        let mut l = Vec::new();
        assert!(toggle(&mut l, "👍").unwrap());
        assert!(toggle(&mut l, "🎉").unwrap());
        assert_eq!(l, ["👍", "🎉"]);
        assert!(!toggle(&mut l, "👍").unwrap());
        assert_eq!(l, ["🎉"]);
    }

    #[test]
    fn only_emoji_are_taken() {
        assert!(check("👍").is_ok() && check("❤️").is_ok());
        assert!(check("").is_err() && check("a").is_err() && check("👍 👎").is_err() && check("x👍").is_err());
    }

    #[test]
    fn thumbs_are_feedback() {
        assert_eq!(Feedback::of("👍"), Some(Feedback::Good));
        assert_eq!(Feedback::of("👎"), Some(Feedback::Bad));
        assert_eq!(Feedback::of("❤️"), None);
    }
}
