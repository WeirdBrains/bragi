//! The files in a chat, as the shells draw them (file cards with a name, type, size and a
//! preview) and save them.
//!
//! - What the user attached ([`crate::attach::Attachment`]): text files, images and audio. They
//!   are in their message, so every linked device has them.
//! - What an answer wrote, read out of its text ([`from_reply`]): fenced code blocks that name a
//!   file (```` ```rust src/main.rs ````, ```` ```python title="a.py" ````, or a file name on the
//!   line just before the fence) and Markdown images with `data:` URLs.
//!
//! A card's id is `<message id>#a<n>` (the n-th attachment) or `<message id>#r<n>` (the n-th
//! file of the answer), so it stays right when other messages are deleted.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::attach::{Attachment, Media, clean_name};
use crate::error::{CoreError, ErrorKind, Result};
use crate::history::{ChatMessage, Conversation, Role};

/// Largest file read out of an answer's `data:` URL.
pub const MAX_REPLY_FILE_BYTES: usize = 10 * 1024 * 1024;

/// What a file is, for its icon and preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Text,
    Image,
    Audio,
}

impl FileKind {
    pub fn name(self) -> &'static str {
        match self {
            FileKind::Text => "text",
            FileKind::Image => "image",
            FileKind::Audio => "audio",
        }
    }
}

/// Where a file in a chat came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOrigin {
    /// The user attached it to a message.
    Attached,
    /// An answer wrote it.
    Reply,
}

/// One file in a chat, as a card draws it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatFile {
    /// `<message id>#a<n>` or `<message id>#r<n>`; pass it to `Core::file_bytes` or `Core::save_file`.
    pub id: String,
    /// The message it belongs to.
    pub message_id: String,
    /// The name to show and to save under (no directories).
    pub name: String,
    /// The path an answer gave it (`src/main.rs`), when it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub mime: String,
    pub size: u64,
    pub kind: FileKind,
    pub origin: FileOrigin,
    /// A text file's first lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// How long a sound file plays, from its headers ([`crate::audio::duration_ms`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// How much of a text file a card previews.
pub const PREVIEW_LINES: usize = 12;
pub const PREVIEW_CHARS: usize = 1200;

/// The first [`PREVIEW_LINES`] lines (at most [`PREVIEW_CHARS`] characters) of `text`.
pub fn preview(text: &str) -> String {
    let mut out: String = text.lines().take(PREVIEW_LINES).collect::<Vec<_>>().join("\n");
    if out.chars().count() > PREVIEW_CHARS {
        out = out.chars().take(PREVIEW_CHARS).collect();
        out.push('…');
    } else if text.lines().count() > PREVIEW_LINES {
        out.push_str("\n…");
    }
    out
}

/// A size for people: `512 B`, `12 KiB`, `3.4 MiB`.
pub fn human_size(n: u64) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{} KiB", n.div_ceil(1024)),
        n => format!("{:.1} MiB", n as f64 / (1024.0 * 1024.0)),
    }
}

fn ext(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// The media type of a text file, from its extension.
pub fn text_mime(name: &str) -> &'static str {
    match ext(name).as_str() {
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "csv" => "text/csv",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "js" | "mjs" => "text/javascript",
        "ts" => "text/x-typescript",
        "rs" => "text/x-rust",
        "py" => "text/x-python",
        "dart" => "text/x-dart",
        "go" => "text/x-go",
        "c" | "h" => "text/x-c",
        "cpp" | "cc" | "hpp" => "text/x-c++",
        "java" => "text/x-java",
        "sh" | "bash" | "zsh" => "text/x-shellscript",
        _ => "text/plain",
    }
}

/// An attachment's media type, size in bytes and kind.
fn attachment_facts(a: &Attachment) -> (String, u64, FileKind) {
    match &a.media {
        Some(Media::Image { mime, data }) => (mime.clone(), decoded_len(data), FileKind::Image),
        Some(Media::Audio { format, data }) => {
            (if format == "mp3" { "audio/mpeg".into() } else { format!("audio/{format}") }, decoded_len(data), FileKind::Audio)
        }
        None => (text_mime(&a.name).into(), a.text.len() as u64, FileKind::Text),
    }
}

fn decoded_len(b64: &str) -> u64 {
    let pad = b64.bytes().rev().take_while(|b| *b == b'=').count();
    (b64.len() / 4 * 3).saturating_sub(pad) as u64
}

/// An attachment's bytes.
pub fn attachment_bytes(a: &Attachment) -> Result<Vec<u8>> {
    match &a.media {
        Some(Media::Image { data, .. } | Media::Audio { data, .. }) => {
            STANDARD.decode(data).map_err(|_| CoreError::new(ErrorKind::Storage, format!("{} is damaged (not base64)", a.name)))
        }
        None => Ok(a.text.as_bytes().to_vec()),
    }
}

/// Writes `bytes` to `path` through a temp file in the same folder, so a reader never sees half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let storage = |e: std::io::Error| CoreError::new(ErrorKind::Storage, format!("{}: {e}", path.display()));
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut b = [0u8; 6];
    let _ = getrandom::fill(&mut b);
    let tmp = dir.join(format!(
        ".{}.{}.part",
        clean_name(&path.file_name().unwrap_or_default().to_string_lossy()),
        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
    ));
    std::fs::write(&tmp, bytes).map_err(storage)?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        storage(e)
    })
}

/// A path in `dir` for `name` that doesn't exist yet: `name`, else `stem (2).ext`, `stem (3).ext`, ….
pub fn free_name(dir: &Path, name: &str) -> PathBuf {
    let name = clean_name(name);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut p = dir.join(&name);
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    p
}

/// A file an answer wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyFile {
    pub name: String,
    pub path: Option<String>,
    pub mime: String,
    pub kind: FileKind,
    pub bytes: Vec<u8>,
}

/// A token that reads as a file name or relative path: `main.rs`, `src/lib.rs`, `Makefile.am`.
fn file_token(t: &str) -> Option<String> {
    let quote = |c: char| matches!(c, '`' | '"' | '\'' | '*');
    let t = t.trim().trim_end_matches(':').trim_matches(quote).trim_end_matches(':').trim_matches(quote);
    let t = t.trim_start_matches("./");
    if t.is_empty() || t.len() > 160 || t.starts_with('/') || t.contains("..") || t.contains("://") {
        return None;
    }
    if !t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/')) {
        return None;
    }
    let base = t.rsplit('/').next()?;
    let (stem, ext) = base.rsplit_once('.')?;
    if stem.is_empty()
        || ext.is_empty()
        || ext.len() > 10
        || !ext.chars().all(|c| c.is_ascii_alphanumeric())
        || ext.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(t.to_string())
}

/// A name in a fence's info string: `rust src/main.rs`, `title="a.py"`, `file=a.py`, `rust:a.rs`.
fn name_in_info(info: &str) -> Option<String> {
    for tok in info.split_whitespace() {
        for key in ["title=", "file=", "filename=", "name=", "path="] {
            if let Some(v) = tok.strip_prefix(key) {
                return file_token(v);
            }
        }
        if let Some(n) = file_token(tok.rsplit(':').next().unwrap_or(tok)) {
            return Some(n);
        }
    }
    None
}

/// A name on the line before a fence: `main.rs`, `**src/main.rs**`, `` `a.py`: ``, `### a.py`,
/// `File: a.py`.
fn name_in_heading(line: &str) -> Option<String> {
    let l = line.trim().trim_start_matches('#').trim();
    let l = ["File:", "file:", "Filename:", "filename:", "Path:"].iter().find_map(|p| l.strip_prefix(p)).unwrap_or(l).trim();
    if l.split_whitespace().count() != 1 {
        return None;
    }
    file_token(l)
}

/// The files in an answer's text: named fenced blocks (the last block wins when a name repeats)
/// and Markdown images with `data:` URLs.
pub fn from_reply(text: &str) -> Vec<ReplyFile> {
    let mut out: Vec<ReplyFile> = Vec::new();
    let push = |f: ReplyFile, out: &mut Vec<ReplyFile>| match out.iter_mut().find(|o| o.path == f.path && o.name == f.name) {
        Some(o) => *o = f,
        None => out.push(f),
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    let mut last_text_line: Option<&str> = None;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim_start();
        let indent = line.len() - t.len();
        let fence_char = t.chars().next().filter(|c| *c == '`' || *c == '~');
        let run = fence_char.map_or(0, |c| t.chars().take_while(|x| *x == c).count());
        if indent <= 3 && run >= 3 {
            let c = fence_char.unwrap_or('`');
            let info = &t[run..];
            let mut body = Vec::new();
            let mut j = i + 1;
            while j < lines.len() {
                let u = lines[j].trim();
                if u.chars().take_while(|x| *x == c).count() >= run && u.chars().all(|x| x == c) {
                    break;
                }
                body.push(lines[j]);
                j += 1;
            }
            let closed = j < lines.len();
            if closed && let Some(path) = name_in_info(info).or_else(|| last_text_line.and_then(name_in_heading)) {
                let mut content = body.join("\n");
                content.push('\n');
                let name = clean_name(&path);
                let mime = text_mime(&name).to_string();
                push(ReplyFile { path: (path != name).then_some(path), name, mime, kind: FileKind::Text, bytes: content.into_bytes() }, &mut out);
            }
            last_text_line = None;
            i = j + 1;
            continue;
        }
        if !line.trim().is_empty() {
            last_text_line = Some(line);
        }
        i += 1;
    }
    let mut n = 0;
    let mut rest = text;
    while let Some(at) = rest.find("![") {
        rest = &rest[at + 2..];
        let Some(close) = rest.find("](") else { break };
        let alt = &rest[..close];
        let after = &rest[close + 2..];
        let Some(end) = after.find(')') else { break };
        let url = after[..end].trim();
        rest = &after[end..];
        let Some(meta_data) = url.strip_prefix("data:") else { continue };
        let Some((meta, data)) = meta_data.split_once(',') else { continue };
        let Some(mime) = meta.strip_suffix(";base64") else { continue };
        if !mime.starts_with("image/") || data.len() > MAX_REPLY_FILE_BYTES / 3 * 4 + 4 {
            continue;
        }
        let Ok(bytes) = STANDARD.decode(data.trim()) else { continue };
        n += 1;
        let ext = mime.trim_start_matches("image/").split(['+', ';']).next().unwrap_or("img").replace("jpeg", "jpg");
        let alt = alt.trim();
        let name = if alt.is_empty() || alt.contains(['\n', '[', ']']) {
            format!("image-{n}.{ext}")
        } else if file_token(alt).is_some() {
            clean_name(alt)
        } else {
            format!("{}.{ext}", clean_name(alt))
        };
        push(ReplyFile { name, path: None, kind: FileKind::Image, mime: mime.to_string(), bytes }, &mut out);
    }
    out
}

/// The cards for message `m`: its attachments, then the files its answer wrote. None while it
/// isn't saved yet (no id).
pub fn describe(m: &ChatMessage) -> Vec<ChatFile> {
    if m.id.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<ChatFile> = m
        .attachments
        .iter()
        .enumerate()
        .map(|(n, a)| {
            let (mime, size, kind) = attachment_facts(a);
            ChatFile {
                id: format!("{}#a{n}", m.id),
                message_id: m.id.clone(),
                name: a.name.clone(),
                path: None,
                mime,
                size,
                kind,
                origin: FileOrigin::Attached,
                preview: (kind == FileKind::Text).then(|| preview(&a.text)),
                duration_ms: (kind == FileKind::Audio).then(|| attachment_bytes(a).ok().and_then(|b| crate::audio::duration_ms(&b))).flatten(),
            }
        })
        .collect();
    if m.role == Role::Assistant {
        out.extend(from_reply(&m.text).into_iter().enumerate().map(|(n, f)| ChatFile {
            id: format!("{}#r{n}", m.id),
            message_id: m.id.clone(),
            preview: (f.kind == FileKind::Text).then(|| preview(&String::from_utf8_lossy(&f.bytes))),
            name: f.name,
            path: f.path,
            mime: f.mime,
            size: f.bytes.len() as u64,
            kind: f.kind,
            origin: FileOrigin::Reply,
            duration_ms: None,
        }));
    }
    out
}

/// Every file in a conversation, oldest first.
pub fn all(c: &Conversation) -> Vec<ChatFile> {
    c.messages.iter().flat_map(describe).collect()
}

/// File `id` (a [`ChatFile::id`]) of `c`, and its bytes.
pub fn bytes(c: &Conversation, id: &str) -> Result<(ChatFile, Vec<u8>)> {
    let msg_id = id.rsplit_once('#').map_or("", |(m, _)| m);
    match c.messages.iter().find(|m| m.id == msg_id && !m.id.is_empty()) {
        Some(m) => message_bytes(m, id),
        None => Err(missing(id)),
    }
}

fn missing(id: &str) -> CoreError {
    CoreError::new(ErrorKind::NotFound, format!("no file `{id}` in this chat"))
}

/// File `id` of message `m` (one of [`describe`]'s), and its bytes: what a shell that already
/// has the message (an image preview) reads without loading the conversation again.
pub fn message_bytes(m: &ChatMessage, id: &str) -> Result<(ChatFile, Vec<u8>)> {
    let card = describe(m).into_iter().find(|f| f.id == id).ok_or_else(|| missing(id))?;
    let which = id.rsplit_once('#').map_or("", |(_, w)| w);
    let n: usize = which.get(1..).unwrap_or("").parse().map_err(|_| missing(id))?;
    let bytes = match which.as_bytes().first() {
        Some(b'a') => attachment_bytes(m.attachments.get(n).ok_or_else(|| missing(id))?)?,
        Some(b'r') => from_reply(&m.text).into_iter().nth(n).ok_or_else(|| missing(id))?.bytes,
        _ => return Err(missing(id)),
    };
    Ok((card, bytes))
}

/// Saves `bytes` named `name` at `dest`: a file path (what a save dialog returned) is replaced; a
/// folder gets the name, with ` (2)`, ` (3)`, … added rather than replacing anything. Returns
/// where it went.
pub fn save(name: &str, bytes: &[u8], dest: &Path) -> Result<PathBuf> {
    let path = if dest.is_dir() { free_name(dest, name) } else { dest.to_path_buf() };
    write_atomic(&path, bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::MessageState;

    fn msg(role: Role, text: &str) -> ChatMessage {
        ChatMessage {
            id: String::new(),
            role,
            text: text.into(),
            reasoning: String::new(),
            thinking_ms: None,
            state: MessageState::Complete,
            model: None,
            node: None,
            cache_hit: false,
            standin: None,
            resumes: 0,
            attachments: Vec::new(),
            tools: Vec::new(),
            agent: false,
            reactions: Vec::new(),
            at_ms: 0,
        }
    }

    #[test]
    fn named_fenced_blocks_are_files() {
        let text = "Here you go.\n\n**src/main.rs**\n```rust\nfn main() {}\n```\n\n```python title=\"tool.py\"\nprint(1)\n```\n\n```rust\nlet unnamed = 1;\n```\n\n`notes.md`:\n~~~\n# n\n~~~\n";
        let f = from_reply(text);
        let names: Vec<(&str, Option<&str>)> = f.iter().map(|f| (f.name.as_str(), f.path.as_deref())).collect();
        assert_eq!(names, vec![("main.rs", Some("src/main.rs")), ("tool.py", None), ("notes.md", None)]);
        assert_eq!(f[0].bytes, b"fn main() {}\n");
        assert_eq!((f[0].mime.as_str(), f[2].mime.as_str()), ("text/x-rust", "text/markdown"));
        // Prose before a fence is not a name; an unclosed fence is not a file.
        assert!(from_reply("Try this, e.g.\n```\nx\n```").is_empty());
        assert!(from_reply("a.rs\n```\nx").is_empty());
        assert!(from_reply("version 1.2\n```\nx\n```").is_empty());
    }

    #[test]
    fn a_repeated_name_keeps_the_last_version() {
        let f = from_reply("```a.py\nv1\n```\nthen\n```a.py\nv2\n```\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].bytes, b"v2\n");
    }

    #[test]
    fn data_url_images_are_files() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        let b64 = STANDARD.encode(png);
        let f = from_reply(&format!("A chart: ![sales chart](data:image/png;base64,{b64}) and ![](https://example.com/x.png)"));
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].name.as_str(), f[0].kind, f[0].bytes.as_slice()), ("sales chart.png", FileKind::Image, png.as_slice()));
    }

    #[test]
    fn saving_never_replaces_in_a_folder_and_replaces_a_picked_path() {
        let tmp = tempfile::tempdir().unwrap();
        let a = save("a.txt", b"1", tmp.path()).unwrap();
        let b = save("a.txt", b"2", tmp.path()).unwrap();
        assert_eq!((a.file_name().unwrap(), b.file_name().unwrap()), ("a.txt".as_ref(), "a (2).txt".as_ref()));
        let c = save("x", b"3", &a).unwrap();
        assert_eq!((c, std::fs::read(&a).unwrap()), (a.clone(), b"3".to_vec()));
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 2, "no temp file left");
    }

    #[test]
    fn cards_cover_attachments_and_answers_by_message_id() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0";
        let user = ChatMessage {
            id: "u1".into(),
            attachments: vec![Attachment::from_bytes("n.md", b"# a\nb\n").unwrap(), Attachment::from_bytes("p.png", png).unwrap()],
            ..msg(Role::User, "look")
        };
        let answer = ChatMessage { id: "a1".into(), ..msg(Role::Assistant, "```a.py\nprint(1)\n```") };
        let c = Conversation { id: "c".into(), title: "t".into(), created_ms: 0, updated_ms: 0, messages: vec![user, answer] };
        let files = all(&c);
        let got: Vec<(&str, &str, FileKind, u64)> = files.iter().map(|f| (f.id.as_str(), f.mime.as_str(), f.kind, f.size)).collect();
        assert_eq!(
            got,
            vec![
                ("u1#a0", "text/markdown", FileKind::Text, 6),
                ("u1#a1", "image/png", FileKind::Image, png.len() as u64),
                ("a1#r0", "text/x-python", FileKind::Text, 9)
            ]
        );
        assert_eq!(files[0].preview.as_deref(), Some("# a\nb"));
        assert_eq!(bytes(&c, "u1#a1").unwrap().1, png.to_vec());
        assert_eq!(bytes(&c, "a1#r0").unwrap().1, b"print(1)\n".to_vec());
        assert_eq!(bytes(&c, "a1#r1").unwrap_err().kind, ErrorKind::NotFound);
        assert_eq!(bytes(&c, "nope").unwrap_err().kind, ErrorKind::NotFound);
    }
}
