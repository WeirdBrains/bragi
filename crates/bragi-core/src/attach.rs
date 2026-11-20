//! Attachments: text files that ride with a prompt. B5 sends them inline in the user's message
//! (a fenced block per file, named); the text is kept with the message so later turns, and other
//! devices, still see what the model saw. Images, other binaries and files too big for a message
//! are refused with a clear error: they need content roots (`weirdnet-net` `content`), which the
//! infer/2 request has no field for yet.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, ErrorKind, Result};

/// Largest attachment, in bytes of text.
pub const MAX_BYTES: usize = 256 * 1024;
/// Most attachments on one message.
pub const MAX_PER_MESSAGE: usize = 5;

/// A text file attached to a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// The file's name, without a directory.
    pub name: String,
    pub text: String,
}

fn bad(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, msg)
}

/// A file name that is safe to show and to put in a prompt: no directories, no control characters.
fn clean_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let s: String = base.chars().filter(|c| !c.is_control()).take(80).collect();
    if s.trim().is_empty() { "attachment".into() } else { s }
}

impl Attachment {
    pub fn from_bytes(name: &str, bytes: &[u8]) -> Result<Self> {
        let name = clean_name(name);
        if bytes.len() > MAX_BYTES {
            return Err(bad(format!("{name} is {} KiB; the most a message takes is {} KiB of text", bytes.len().div_ceil(1024), MAX_BYTES / 1024)));
        }
        if bytes.contains(&0) {
            return Err(bad(format!("{name} is not a text file (images and other binaries can't be attached yet)")));
        }
        let text = String::from_utf8(bytes.to_vec())
            .map_err(|_| bad(format!("{name} is not UTF-8 text (images and other binaries can't be attached yet)")))?;
        Ok(Self { name, text })
    }

    /// Reads `path` (at most [`MAX_BYTES`]) as an attachment.
    pub fn from_path(path: &Path) -> Result<Self> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let meta = std::fs::metadata(path).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", path.display())))?;
        if !meta.is_file() {
            return Err(bad(format!("{} is not a file", path.display())));
        }
        if meta.len() as usize > MAX_BYTES {
            return Err(bad(format!("{name} is {} KiB; the most a message takes is {} KiB of text", meta.len().div_ceil(1024), MAX_BYTES / 1024)));
        }
        let bytes = std::fs::read(path).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", path.display())))?;
        Self::from_bytes(&name, &bytes)
    }
}

/// The message the model gets: each attachment as a named, fenced block, then the prompt.
pub fn render(prompt: &str, attachments: &[Attachment]) -> String {
    if attachments.is_empty() {
        return prompt.to_string();
    }
    let mut out = String::new();
    for a in attachments {
        // A fence longer than any run of backticks in the file, so the file can't close it.
        let longest = a.text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        let fence = "`".repeat(longest.max(2) + 1);
        out.push_str(&format!("[attached file: {}]\n{fence}\n{}{}{fence}\n\n", a.name, a.text, if a.text.ends_with('\n') { "" } else { "\n" }));
    }
    out.push_str(prompt);
    out
}

/// Checks a list of attachments before a send.
pub fn check(attachments: &[Attachment]) -> Result<()> {
    if attachments.len() > MAX_PER_MESSAGE {
        return Err(bad(format!("at most {MAX_PER_MESSAGE} files per message")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_taken_and_binaries_and_big_files_are_refused() {
        let a = Attachment::from_bytes("/home/me/notes.md", b"# hi\n").unwrap();
        assert_eq!(a.name, "notes.md");
        assert!(Attachment::from_bytes("x.png", &[0x89, b'P', 0, 1]).is_err());
        assert!(Attachment::from_bytes("x.bin", &[0xff, 0xfe, 0xfd]).is_err());
        let big = vec![b'a'; MAX_BYTES + 1];
        assert!(Attachment::from_bytes("big.txt", &big).unwrap_err().message.contains("KiB"));
        assert_eq!(Attachment::from_bytes("\n\u{7}", b"x").unwrap().name, "attachment");
    }

    #[test]
    fn render_fences_the_file_so_it_cannot_close_the_fence() {
        let a = Attachment { name: "a.md".into(), text: "```rust\nfn main() {}\n```\n".into() };
        let r = render("what is this?", std::slice::from_ref(&a));
        assert!(r.starts_with("[attached file: a.md]\n````\n```rust"), "{r}");
        assert!(r.ends_with("````\n\nwhat is this?"), "{r}");
        assert_eq!(render("hi", &[]), "hi");
    }
}
