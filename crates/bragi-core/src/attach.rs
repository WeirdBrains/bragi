//! Attachments: files that ride with a prompt. Text files go inline in the user's message (a
//! fenced block per file, named); the text is kept with the message so later turns, and other
//! devices, still see what the model saw. Images (PNG, JPEG, GIF, WebP) and audio (WAV, MP3) go as
//! content parts of the message (`image_url` with a data URL, `input_audio`) and are kept with it
//! the same way; they are small ones only, since they travel inside the request. Other binaries and
//! files too big for a message are refused with a clear error; bigger files need content roots
//! (`weirdnet-net` `content`), which the infer/2 request has no field for yet (LF-12).

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use weirdnet_agent::{Content, ContentPart, Modalities};

use crate::error::{CoreError, ErrorKind, Result};

/// Largest attachment, in bytes of text.
pub const MAX_BYTES: usize = 256 * 1024;
/// Most attachments on one message.
pub const MAX_PER_MESSAGE: usize = 5;

/// Largest image attachment, in bytes of the file.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// Largest audio attachment, in bytes of the file.
pub const MAX_AUDIO_BYTES: usize = 10 * 1024 * 1024;

/// An image or a sound attached to a message, base64 as the model gets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Media {
    /// `mime` is `image/png`, `image/jpeg`, `image/gif` or `image/webp`.
    Image { mime: String, data: String },
    /// `format` is `wav` or `mp3`.
    Audio { format: String, data: String },
}

impl Media {
    /// What a model needs to read it.
    pub fn needs(&self) -> Modalities {
        match self {
            Media::Image { .. } => Modalities::VISION,
            Media::Audio { .. } => Modalities::AUDIO,
        }
    }

    fn part(&self) -> ContentPart {
        match self {
            Media::Image { mime, data } => ContentPart::image(format!("data:{mime};base64,{data}")),
            Media::Audio { format, data } => ContentPart::audio(data.clone(), format.clone()),
        }
    }

    /// What it is, for messages: `image` or `audio`.
    pub fn kind(&self) -> &'static str {
        match self {
            Media::Image { .. } => "image",
            Media::Audio { .. } => "audio",
        }
    }
}

/// A file attached to a message: text in `text`, or an image or sound in `media` (then `text` is empty).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// The file's name, without a directory.
    pub name: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<Media>,
}

/// The kind of media in `bytes`, from its first bytes: `(image mime | audio format, is_image)`.
fn sniff(bytes: &[u8]) -> Option<(&'static str, bool)> {
    let riff = |kind: &[u8]| bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == kind;
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", true))
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("image/jpeg", true))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("image/gif", true))
    } else if riff(b"WEBP") {
        Some(("image/webp", true))
    } else if riff(b"WAVE") {
        Some(("wav", false))
    } else if bytes.starts_with(b"ID3") || (bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0 && bytes[1] & 0x06 != 0) {
        Some(("mp3", false))
    } else {
        None
    }
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
        if let Some((what, image)) = sniff(bytes) {
            let max = if image { MAX_IMAGE_BYTES } else { MAX_AUDIO_BYTES };
            if bytes.len() > max {
                return Err(bad(format!(
                    "{name} is {} KiB; the most a message takes is {} KiB for {}",
                    bytes.len().div_ceil(1024),
                    max / 1024,
                    if image { "an image" } else { "audio" }
                )));
            }
            let data = STANDARD.encode(bytes);
            let media = if image { Media::Image { mime: what.into(), data } } else { Media::Audio { format: what.into(), data } };
            return Ok(Self { name, text: String::new(), media: Some(media) });
        }
        if bytes.len() > MAX_BYTES {
            return Err(bad(format!("{name} is {} KiB; the most a message takes is {} KiB of text", bytes.len().div_ceil(1024), MAX_BYTES / 1024)));
        }
        if bytes.contains(&0) {
            return Err(bad(format!(
                "{name} is not a text file (other binaries can't be attached; images are PNG, JPEG, GIF or WebP, audio is WAV or MP3)"
            )));
        }
        let text = String::from_utf8(bytes.to_vec()).map_err(|_| {
            bad(format!("{name} is not UTF-8 text (other binaries can't be attached; images are PNG, JPEG, GIF or WebP, audio is WAV or MP3)"))
        })?;
        Ok(Self { name, text, media: None })
    }

    /// How a shell names it: `notes.md (3 lines)`, `shot.png (image)`, `hi.wav (audio)`.
    pub fn label(&self) -> String {
        match &self.media {
            Some(m) => format!("{} ({})", self.name, m.kind()),
            None => format!("{} ({} lines)", self.name, self.text.lines().count()),
        }
    }

    /// An attachment back from its parts (a shell hands them over as strings): `kind` is `image` or
    /// `audio`, `kind_type` the image's MIME type or the audio's format, `data` the base64. The
    /// same limits as [`from_bytes`](Self::from_bytes) apply.
    pub fn from_media(name: &str, kind: &str, kind_type: &str, data: &str) -> Result<Self> {
        let name = clean_name(name);
        let bytes = STANDARD.decode(data).map_err(|_| bad(format!("{name}: the data is not base64")))?;
        let attached = Self::from_bytes(&name, &bytes)?;
        match (&attached.media, kind) {
            (Some(Media::Image { mime, .. }), "image") if mime == kind_type => Ok(attached),
            (Some(Media::Audio { format, .. }), "audio") if format == kind_type => Ok(attached),
            _ => Err(bad(format!("{name} is not the {kind} it was said to be"))),
        }
    }

    /// Reads `path` (at most [`MAX_BYTES`]) as an attachment.
    pub fn from_path(path: &Path) -> Result<Self> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let meta = std::fs::metadata(path).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", path.display())))?;
        if !meta.is_file() {
            return Err(bad(format!("{} is not a file", path.display())));
        }
        if meta.len() as usize > MAX_AUDIO_BYTES.max(MAX_IMAGE_BYTES) {
            return Err(bad(format!("{name} is {} KiB; that is more than a message takes", meta.len().div_ceil(1024))));
        }
        let bytes = std::fs::read(path).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", path.display())))?;
        Self::from_bytes(&name, &bytes)
    }
}

/// The message the model gets: the text of [`render`], then each image or sound as a content part.
/// Plain text when nothing is media.
pub fn content(prompt: &str, attachments: &[Attachment]) -> Content {
    let text = render(prompt, attachments);
    let media: Vec<ContentPart> = attachments.iter().filter_map(|a| a.media.as_ref()).map(Media::part).collect();
    if media.is_empty() {
        return Content::Text(text);
    }
    let mut parts = Vec::with_capacity(media.len() + 1);
    if !text.is_empty() {
        parts.push(ContentPart::text(text));
    }
    parts.extend(media);
    Content::Parts(parts)
}

/// What a model needs to read `attachments`: text unless some are images or sound.
pub fn needs(attachments: &[Attachment]) -> Modalities {
    attachments.iter().filter_map(|a| a.media.as_ref()).fold(Modalities::TEXT, |n, m| n.union(m.needs()))
}

/// The message the model gets: each text attachment as a named, fenced block, then the prompt.
/// Images and sound are named in a line (`[attached image: name]`) and sent as parts by [`content`].
pub fn render(prompt: &str, attachments: &[Attachment]) -> String {
    if attachments.is_empty() {
        return prompt.to_string();
    }
    let mut out = String::new();
    for a in attachments {
        if let Some(m) = &a.media {
            out.push_str(&format!("[attached {}: {}]\n", m.kind(), a.name));
            continue;
        }
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
        assert!(Attachment::from_bytes("x.png", &[0x89, b'P', 0, 1]).is_err(), "not a whole PNG header");
        assert!(Attachment::from_bytes("x.bin", &[0x80, 0x81, 0x82]).is_err());
        let big = vec![b'a'; MAX_BYTES + 1];
        assert!(Attachment::from_bytes("big.txt", &big).unwrap_err().message.contains("KiB"));
        assert_eq!(Attachment::from_bytes("\n\u{7}", b"x").unwrap().name, "attachment");
    }

    // A 1x1 PNG, and the first bytes of a WAV, an MP3 and a JPEG.
    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R', 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15,
        0xc4, 0x89,
    ];
    fn wav() -> Vec<u8> {
        [b"RIFF".as_slice(), &[36, 0, 0, 0], b"WAVEfmt ", &[0; 16]].concat()
    }

    #[test]
    fn images_and_sound_become_content_parts_and_text_stays_text() {
        let img = Attachment::from_bytes("dir/shot.png", PNG).unwrap();
        assert_eq!((img.name.as_str(), img.text.as_str()), ("shot.png", ""));
        assert!(matches!(&img.media, Some(Media::Image { mime, .. }) if mime == "image/png"));
        let snd = Attachment::from_bytes("hi.wav", &wav()).unwrap();
        assert!(matches!(&snd.media, Some(Media::Audio { format, .. }) if format == "wav"));
        let mp3 = Attachment::from_bytes("a.mp3", b"ID3\x03\0\0\0\0\0\0").unwrap();
        assert!(matches!(&mp3.media, Some(Media::Audio { format, .. }) if format == "mp3"));
        assert!(matches!(Attachment::from_bytes("p.jpg", &[0xff, 0xd8, 0xff, 0xe0, 0]).unwrap().media, Some(Media::Image { .. })));

        let file = Attachment::from_bytes("n.txt", b"hello\n").unwrap();
        assert_eq!(needs(std::slice::from_ref(&file)), Modalities::TEXT);
        assert_eq!(content("hi", std::slice::from_ref(&file)), Content::Text(render("hi", std::slice::from_ref(&file))));

        let all = [file, img.clone(), snd];
        assert_eq!(needs(&all), Modalities::new(true, true));
        let Content::Parts(parts) = content("what is this?", &all) else { panic!("parts") };
        assert_eq!(parts.len(), 3, "text, image, audio");
        let ContentPart::Text { text } = &parts[0] else { panic!("text first") };
        assert!(
            text.contains("[attached file: n.txt]") && text.contains("[attached image: shot.png]") && text.contains("[attached audio: hi.wav]"),
            "{text}"
        );
        assert!(text.ends_with("what is this?"));
        let ContentPart::ImageUrl { image_url } = &parts[1] else { panic!("image") };
        assert!(image_url.url.starts_with("data:image/png;base64,iVBORw0KGgo"), "{}", image_url.url);
        assert!(matches!(&parts[2], ContentPart::InputAudio { input_audio } if input_audio.format == "wav" && !input_audio.data.is_empty()));
        assert!(Content::Parts(parts).check().is_ok());
        // Only media, no words.
        let Content::Parts(only) = content("", &[img]) else { panic!("parts") };
        assert_eq!(only.len(), 2);
    }

    #[test]
    fn big_media_is_refused() {
        let mut big = PNG.to_vec();
        big.resize(MAX_IMAGE_BYTES + 1, 0);
        assert!(Attachment::from_bytes("big.png", &big).unwrap_err().message.contains("image"));
        let mut big = wav();
        big.resize(MAX_AUDIO_BYTES + 1, 0);
        assert!(Attachment::from_bytes("big.wav", &big).unwrap_err().message.contains("audio"));
    }

    #[test]
    fn render_fences_the_file_so_it_cannot_close_the_fence() {
        let a = Attachment { name: "a.md".into(), text: "```rust\nfn main() {}\n```\n".into(), media: None };
        let r = render("what is this?", std::slice::from_ref(&a));
        assert!(r.starts_with("[attached file: a.md]\n````\n```rust"), "{r}");
        assert!(r.ends_with("````\n\nwhat is this?"), "{r}");
        assert_eq!(render("hi", &[]), "hi");
    }
}
