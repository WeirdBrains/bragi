//! Assistant replies as Markdown, parsed once here so every shell (the GPUI desktop, the Flutter
//! app through `bragi-bridge`, the terminal UI) draws the same thing.
//!
//! [`parse`] turns CommonMark plus the GitHub extensions (tables, strikethrough, task lists) into
//! a small tree of [`Block`]s whose text is flat runs of styled [`Span`]s, ready for a rich-text
//! widget. Fenced code comes with its tokens already highlighted ([`crate::highlight`]).
//!
//! Safe by construction: raw HTML is kept as literal text (never interpreted), images become a
//! link to their address (nothing is fetched), and only `http`, `https` and `mailto` links are
//! links at all.
//!
//! [`parse_streaming`] is for an answer still arriving: it is called again on every delta and
//! never shows a half-typed marker (`**` before its word, a lone `---` that may become a heading
//! underline) and closes an open `**bold`, `~~strike` or `` `code `` so it renders styled at once.
//! An unclosed code fence is simply a code block that runs to the end, with [`Block::Code`]'s
//! `closed` false.

use std::borrow::Cow;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

use crate::highlight::{self, Token};

/// A parsed reply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub blocks: Vec<Block>,
}

/// A run of text with one style. Line breaks inside a paragraph are `\n` in `text`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    /// Inline code: draw in the monospace font.
    pub code: bool,
    /// A link (always `http`, `https` or `mailto`): open it in the system browser on click and
    /// show the address on hover.
    pub link: Option<String>,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), ..Self::default() }
    }

    fn same_style(&self, o: &Span) -> bool {
        self.bold == o.bold && self.italic == o.italic && self.strike == o.strike && self.code == o.code && self.link == o.link
    }
}

/// Column alignment in a table.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    None,
    Left,
    Center,
    Right,
}

/// One list item; `checked` is set on a task-list item (`- [ ]` / `- [x]`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph {
        spans: Vec<Span>,
    },
    /// Level 1 to 6.
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Quote {
        blocks: Vec<Block>,
    },
    /// Numbered from `start` when it is set, bulleted otherwise.
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    /// A code block: `lang` is the fence's first word, `code` the raw text (no trailing newline),
    /// `tokens` the same text split for highlighting (their texts join to `code`), and `closed`
    /// false while the closing fence hasn't arrived.
    Code {
        lang: Option<String>,
        code: String,
        tokens: Vec<Token>,
        closed: bool,
    },
    /// `head` has one entry per column; each row has the same number of cells.
    Table {
        align: Vec<Align>,
        head: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
    Rule,
}

impl Document {
    /// The document as JSON (how the Flutter app receives it).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"blocks\":[]}".into())
    }

    /// Every code block's raw text, in order (the shells' Copy buttons).
    pub fn code_blocks(&self) -> Vec<&str> {
        fn walk<'a>(blocks: &'a [Block], out: &mut Vec<&'a str>) {
            for b in blocks {
                match b {
                    Block::Code { code, .. } => out.push(code),
                    Block::Quote { blocks } => walk(blocks, out),
                    Block::List { items, .. } => items.iter().for_each(|i| walk(&i.blocks, out)),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.blocks, &mut out);
        out
    }
}

/// The options Bragi parses with: CommonMark plus GFM tables, strikethrough and task lists.
fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// Parses a finished reply.
pub fn parse(text: &str) -> Document {
    Builder::default().run(text)
}

/// Parses a reply that is still streaming in (see the module docs). Call it again with the whole
/// text on every delta; it is cheap enough for that at chat sizes.
pub fn parse_streaming(text: &str) -> Document {
    Builder::default().run(&streaming_text(text))
}

/// The links a shell may open: `http`, `https` and `mailto`. Anything else isn't a link.
pub fn safe_link(url: &str) -> Option<String> {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")).then(|| u.to_string())
}

// ---- streaming -----------------------------------------------------------------------------------

/// Whether `line` opens or closes a fence: up to three spaces, then three or more of one fence
/// character. Returns the character and the run's length.
fn fence(line: &str) -> Option<(char, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let c = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = rest.chars().take_while(|x| *x == c).count();
    (n >= 3).then_some((c, n))
}

/// The fence still open at the end of `text`, if any.
fn open_fence(text: &str) -> Option<(char, usize)> {
    let mut open: Option<(char, usize)> = None;
    for line in text.split('\n') {
        match (open, fence(line)) {
            (None, Some((c, n))) => {
                // A backtick fence's info string may not contain a backtick.
                let info = &line.trim_start()[n..];
                if !(c == '`' && info.contains('`')) {
                    open = Some((c, n));
                }
            }
            (Some((oc, on)), Some((c, n))) if c == oc && n >= on && line.trim_start()[n..].trim().is_empty() => open = None,
            _ => {}
        }
    }
    open
}

/// The text to parse for a streaming reply: drops a trailing half-typed marker and closes an
/// unfinished `**`, `~~` or `` ` `` in the last paragraph, unless the text ends inside code.
pub fn streaming_text(text: &str) -> Cow<'_, str> {
    if open_fence(text).is_some() {
        return Cow::Borrowed(text);
    }
    let mut t = text;
    // A last line of only marker characters may still become a rule, a setext heading's
    // underline, a list item or a fence: hold it back until it means something.
    let last = t.rsplit('\n').next().unwrap_or(t);
    if !last.trim().is_empty() && last.chars().all(|c| matches!(c, '-' | '=' | '*' | '_' | '`' | '~' | '+' | ' ' | '\t' | '#' | '>' | '|' | ':')) {
        t = &t[..t.len() - last.len()];
    }
    // A trailing run of emphasis characters is a marker being typed.
    let trimmed = t.trim_end_matches(['*', '_', '~', '`']);
    if trimmed.len() != t.len() && !trimmed.ends_with('\n') && !trimmed.is_empty() {
        t = trimmed;
    }
    // Close what the last paragraph left open, so `**4` already shows as a bold 4.
    let para_start = t.rfind("\n\n").map(|i| i + 2).unwrap_or(0);
    let para = &t[para_start..];
    let mut close = String::new();
    let (mut ticks, mut stars, mut tildes) = (0usize, 0usize, 0usize);
    let mut in_code = false;
    let b = para.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'`' => {
                ticks += 1;
                in_code = !in_code;
            }
            b'*' if !in_code && b.get(i + 1) == Some(&b'*') => {
                stars += 1;
                i += 1;
            }
            b'~' if !in_code && b.get(i + 1) == Some(&b'~') => {
                tildes += 1;
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    let ends_in_space = para.ends_with(char::is_whitespace);
    if ticks % 2 == 1 {
        close.push('`');
    } else if !ends_in_space {
        if tildes % 2 == 1 {
            close.push_str("~~");
        }
        if stars % 2 == 1 {
            close.push_str("**");
        }
    }
    if close.is_empty() && t.len() == text.len() { Cow::Borrowed(text) } else { Cow::Owned(format!("{t}{close}")) }
}

// ---- building the tree ---------------------------------------------------------------------------

enum Container {
    Root(Vec<Block>),
    Quote(Vec<Block>),
    List { start: Option<u64>, items: Vec<Item> },
    Item(Item),
}

#[derive(Clone, Copy, PartialEq)]
enum Target {
    /// Text straight inside a tight list item.
    Loose,
    Paragraph,
    Heading(u8),
    Cell,
}

struct Code {
    lang: Option<String>,
    text: String,
    fence: Option<(char, usize)>,
    closed: bool,
}

#[derive(Default)]
struct TableBuild {
    align: Vec<Align>,
    head: Vec<Vec<Span>>,
    rows: Vec<Vec<Vec<Span>>>,
    row: Vec<Vec<Span>>,
}

struct Builder {
    stack: Vec<Container>,
    target: Target,
    inline: Vec<Span>,
    bold: u32,
    italic: u32,
    strike: u32,
    links: Vec<Option<String>>,
    code: Option<Code>,
    table: Option<TableBuild>,
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            stack: vec![Container::Root(Vec::new())],
            target: Target::Loose,
            inline: Vec::new(),
            bold: 0,
            italic: 0,
            strike: 0,
            links: Vec::new(),
            code: None,
            table: None,
        }
    }
}

fn level(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

impl Builder {
    fn run(mut self, text: &str) -> Document {
        for (ev, range) in Parser::new_ext(text, options()).into_offset_iter() {
            self.event(ev, &text[range]);
        }
        self.flush_loose();
        // Close anything a malformed stream left open (the parser always balances, but be safe).
        while self.stack.len() > 1 {
            self.pop_container();
        }
        match self.stack.pop() {
            Some(Container::Root(blocks)) => Document { blocks },
            _ => Document::default(),
        }
    }

    fn push_block(&mut self, b: Block) {
        self.flush_loose();
        self.append(b);
    }

    fn append(&mut self, b: Block) {
        match self.stack.last_mut() {
            Some(Container::Root(v) | Container::Quote(v)) => v.push(b),
            Some(Container::Item(i)) => i.blocks.push(b),
            Some(Container::List { items, .. }) => items.push(Item { checked: None, blocks: vec![b] }),
            None => self.stack.push(Container::Root(vec![b])),
        }
    }

    /// Text that sat straight in a tight list item becomes a paragraph there.
    fn flush_loose(&mut self) {
        if self.target == Target::Loose && !self.inline.is_empty() {
            let spans = finish_spans(std::mem::take(&mut self.inline));
            if !spans.is_empty() {
                self.append(Block::Paragraph { spans });
            }
        }
    }

    fn pop_container(&mut self) {
        self.flush_loose();
        let Some(c) = self.stack.pop() else { return };
        match c {
            Container::Quote(blocks) => self.append(Block::Quote { blocks }),
            Container::List { start, items } => self.append(Block::List { start, items }),
            Container::Item(item) => match self.stack.last_mut() {
                Some(Container::List { items, .. }) => items.push(item),
                _ => self.append(Block::List { start: None, items: vec![item] }),
            },
            Container::Root(blocks) => self.stack.push(Container::Root(blocks)),
        }
    }

    fn take_inline(&mut self) -> Vec<Span> {
        self.target = Target::Loose;
        finish_spans(std::mem::take(&mut self.inline))
    }

    fn text(&mut self, text: &str, code: bool) {
        if text.is_empty() {
            return;
        }
        let link = self.links.iter().rev().find_map(|l| l.clone());
        let span = Span { text: text.to_string(), bold: self.bold > 0, italic: self.italic > 0, strike: self.strike > 0, code, link };
        match self.inline.last_mut() {
            Some(last) if last.same_style(&span) => last.text.push_str(&span.text),
            _ => self.inline.push(span),
        }
    }

    fn event(&mut self, ev: Event<'_>, source: &str) {
        if let Some(code) = self.code.as_mut() {
            match ev {
                Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => {
                    code.text.push_str(&t);
                    return;
                }
                Event::End(TagEnd::CodeBlock) => {}
                _ => return,
            }
        }
        match ev {
            Event::Start(tag) => self.start(tag, source),
            Event::End(end) => self.end(end),
            Event::Text(t) => self.text(&t, false),
            Event::Code(t) => self.text(&t, true),
            // Raw HTML is shown as the text it is, never interpreted.
            Event::Html(t) | Event::InlineHtml(t) => self.text(&t, false),
            Event::InlineMath(t) | Event::DisplayMath(t) => self.text(&t, true),
            Event::FootnoteReference(t) => self.text(&format!("[{t}]"), false),
            Event::SoftBreak => self.text(" ", false),
            Event::HardBreak => self.text("\n", false),
            Event::Rule => self.push_block(Block::Rule),
            Event::TaskListMarker(done) => {
                if let Some(Container::Item(i)) = self.stack.last_mut() {
                    i.checked = Some(done);
                }
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>, source: &str) {
        match tag {
            Tag::Paragraph | Tag::HtmlBlock => {
                self.flush_loose();
                self.target = Target::Paragraph;
            }
            Tag::Heading { level: l, .. } => {
                self.flush_loose();
                self.target = Target::Heading(level(l));
            }
            Tag::BlockQuote(_) => {
                self.flush_loose();
                self.stack.push(Container::Quote(Vec::new()));
            }
            Tag::CodeBlock(kind) => {
                self.flush_loose();
                let (lang, opener) = match kind {
                    CodeBlockKind::Fenced(info) => {
                        let lang = info.split_whitespace().next().map(|s| s.trim_matches(['{', '}', '.']).to_string()).filter(|s| !s.is_empty());
                        (lang, fence(source.split('\n').next().unwrap_or("")))
                    }
                    CodeBlockKind::Indented => (None, None),
                };
                // Closed when the block's source ends with a fence like the one that opened it.
                let closed = match opener {
                    Some((c, n)) => {
                        let body = source.trim_end_matches(['\n', '\r']);
                        let mut lines = body.split('\n');
                        lines.next();
                        let last = lines.next_back();
                        last.and_then(|l| fence(l).filter(|(lc, ln)| *lc == c && *ln >= n).map(|(_, ln)| l.trim_start()[ln..].trim().is_empty()))
                            .unwrap_or(false)
                    }
                    None => true,
                };
                self.code = Some(Code { lang, text: String::new(), fence: opener, closed });
            }
            Tag::List(start) => {
                self.flush_loose();
                self.stack.push(Container::List { start, items: Vec::new() });
            }
            Tag::Item => {
                self.flush_loose();
                self.stack.push(Container::Item(Item::default()));
            }
            Tag::Table(aligns) => {
                self.flush_loose();
                let align = aligns
                    .iter()
                    .map(|a| match a {
                        Alignment::None => Align::None,
                        Alignment::Left => Align::Left,
                        Alignment::Center => Align::Center,
                        Alignment::Right => Align::Right,
                    })
                    .collect();
                self.table = Some(TableBuild { align, ..TableBuild::default() });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    t.row.clear();
                }
            }
            Tag::TableCell => {
                self.inline.clear();
                self.target = Target::Cell;
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.links.push(safe_link(&dest_url)),
            // An image is never fetched: it reads as a link to where it lives.
            Tag::Image { dest_url, .. } => {
                let link = safe_link(&dest_url);
                self.links.push(link);
                self.text("image: ", false);
            }
            _ => {}
        }
    }

    fn end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Paragraph | TagEnd::HtmlBlock => {
                let mut spans = self.take_inline();
                if let Some(last) = spans.last_mut() {
                    let t = last.text.trim_end_matches('\n').len();
                    last.text.truncate(t);
                }
                spans.retain(|s| !s.text.is_empty());
                if !spans.is_empty() {
                    self.append(Block::Paragraph { spans });
                }
            }
            TagEnd::Heading(_) => {
                let level = match self.target {
                    Target::Heading(l) => l,
                    _ => 1,
                };
                let spans = self.take_inline();
                self.append(Block::Heading { level, spans });
            }
            TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item => self.pop_container(),
            TagEnd::CodeBlock => {
                if let Some(c) = self.code.take() {
                    let code = c.text.strip_suffix('\n').unwrap_or(&c.text).to_string();
                    let tokens = highlight::highlight(c.lang.as_deref(), &code);
                    let closed = c.fence.is_none() || c.closed;
                    self.append(Block::Code { lang: c.lang, code, tokens, closed });
                }
            }
            TagEnd::TableCell => {
                let spans = self.take_inline();
                if let Some(t) = self.table.as_mut() {
                    t.row.push(spans);
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.head = std::mem::take(&mut t.row);
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    let mut row = std::mem::take(&mut t.row);
                    row.resize(t.head.len().max(row.len()), Vec::new());
                    t.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.append(Block::Table { align: t.align, head: t.head, rows: t.rows });
                }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link | TagEnd::Image => {
                self.links.pop();
            }
            _ => {}
        }
    }
}

/// Merges neighbouring runs of one style and turns bare `http(s)://` addresses into links.
fn finish_spans(spans: Vec<Span>) -> Vec<Span> {
    let mut merged: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans {
        match merged.last_mut() {
            Some(last) if last.same_style(&s) => last.text.push_str(&s.text),
            _ => merged.push(s),
        }
    }
    let mut out = Vec::with_capacity(merged.len());
    for s in merged {
        if s.code || s.link.is_some() {
            out.push(s);
        } else {
            linkify(s, &mut out);
        }
    }
    out
}

fn linkify(s: Span, out: &mut Vec<Span>) {
    let mut rest = s.text.as_str();
    loop {
        let found = ["https://", "http://"].iter().filter_map(|p| rest.find(p)).min();
        let Some(at) = found else { break };
        // Only at a word start.
        if at > 0 && rest[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric()) {
            let skip = at + 4;
            let (head, tail) = rest.split_at(skip);
            push_text(&s, head, None, out);
            rest = tail;
            continue;
        }
        let end = rest[at..].find(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == '"').map(|e| at + e).unwrap_or(rest.len());
        let mut url = &rest[at..end];
        loop {
            let t = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '*', '_']);
            let t = if t.ends_with(')') && t.matches('(').count() < t.matches(')').count() { &t[..t.len() - 1] } else { t };
            if t.len() == url.len() {
                break;
            }
            url = t;
        }
        if url.len() <= "https://".len() {
            push_text(&s, &rest[..end], None, out);
            rest = &rest[end..];
            continue;
        }
        push_text(&s, &rest[..at], None, out);
        push_text(&s, url, Some(url.to_string()), out);
        rest = &rest[at + url.len()..];
    }
    push_text(&s, rest, None, out);
}

fn push_text(style: &Span, text: &str, link: Option<String>, out: &mut Vec<Span>) {
    if text.is_empty() {
        return;
    }
    let span = Span { text: text.to_string(), link, ..style.clone() };
    match out.last_mut() {
        Some(last) if last.same_style(&span) => last.text.push_str(text),
        _ => out.push(span),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(doc: &Document) -> &[Span] {
        match &doc.blocks[0] {
            Block::Paragraph { spans } => spans,
            b => panic!("not a paragraph: {b:?}"),
        }
    }

    #[test]
    fn bold_four_from_the_report() {
        // Michael's qwen3.8-27b-nvfp4 reply, which showed its asterisks.
        let doc = parse("2 + 2 equals **4**.");
        assert_eq!(para(&doc), &[Span::plain("2 + 2 equals "), Span { text: "4".into(), bold: true, ..Span::default() }, Span::plain(".")]);
        assert!(!doc.to_json().contains("**"));
    }

    #[test]
    fn streaming_never_shows_half_markers() {
        let full = "2 + 2 equals **4**.";
        for n in 0..=full.len() {
            let doc = parse_streaming(&full[..n]);
            let shown: String = doc
                .blocks
                .iter()
                .flat_map(|b| match b {
                    Block::Paragraph { spans } => spans.clone(),
                    _ => vec![],
                })
                .map(|s| s.text)
                .collect();
            assert!(!shown.contains('*'), "{:?} showed {shown:?}", &full[..n]);
        }
        // `**4` already renders as a bold 4.
        let doc = parse_streaming("2 + 2 equals **4");
        assert!(para(&doc).iter().any(|s| s.bold && s.text == "4"), "{doc:?}");
    }

    #[test]
    fn half_open_fence_is_code_to_the_end() {
        let doc = parse_streaming("Here:\n\n```rust\nfn main() {\n    let x = \"**hi**\";");
        match &doc.blocks[1] {
            Block::Code { lang, code, closed, tokens } => {
                assert_eq!(lang.as_deref(), Some("rust"));
                assert!(code.ends_with("\"**hi**\";"));
                assert!(!closed);
                assert_eq!(tokens.iter().map(|t| t.text.as_str()).collect::<String>(), *code);
            }
            b => panic!("{b:?}"),
        }
        let doc = parse("```rust\nfn main() {}\n```\n");
        assert!(matches!(&doc.blocks[0], Block::Code { closed: true, .. }), "{doc:?}");
    }

    #[test]
    fn a_trailing_dash_line_is_held_back() {
        // "Title\n-" would briefly be a setext heading.
        let doc = parse_streaming("Title\n-");
        assert_eq!(doc.blocks, vec![Block::Paragraph { spans: vec![Span::plain("Title")] }]);
        let doc = parse_streaming("Title\n---\n");
        assert!(matches!(doc.blocks[0], Block::Heading { level: 2, .. }));
    }

    #[test]
    fn gfm_blocks() {
        let md = "# Head\n\n- one\n- **two**\n  - nested\n\n1. a\n2. b\n\n- [x] done\n- [ ] todo\n\n> quoted *it*\n\n| a | b |\n|:--|--:|\n| 1 | `x` |\n\n---\n\n~~gone~~ and ***both***";
        let doc = parse(md);
        let kinds: Vec<&str> = doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::Heading { .. } => "h",
                Block::List { start: None, .. } => "ul",
                Block::List { start: Some(_), .. } => "ol",
                Block::Quote { .. } => "quote",
                Block::Table { .. } => "table",
                Block::Rule => "hr",
                Block::Paragraph { .. } => "p",
                Block::Code { .. } => "code",
            })
            .collect();
        assert_eq!(kinds, ["h", "ul", "ol", "ul", "quote", "table", "hr", "p"]);
        let Block::List { items, .. } = &doc.blocks[1] else { unreachable!() };
        assert_eq!(items[1].blocks[0], Block::Paragraph { spans: vec![Span { text: "two".into(), bold: true, ..Span::default() }] });
        assert!(matches!(items[1].blocks[1], Block::List { .. }), "nested list: {:?}", items[1]);
        let Block::List { items, .. } = &doc.blocks[3] else { unreachable!() };
        assert_eq!((items[0].checked, items[1].checked), (Some(true), Some(false)));
        let Block::Table { align, head, rows } = &doc.blocks[5] else { unreachable!() };
        assert_eq!(align, &[Align::Left, Align::Right]);
        assert_eq!(head.len(), 2);
        assert_eq!(rows[0][1], vec![Span { text: "x".into(), code: true, ..Span::default() }]);
        let Block::Paragraph { spans } = &doc.blocks[7] else { unreachable!() };
        assert!(spans[0].strike && spans[0].text == "gone");
        assert!(spans[2].bold && spans[2].italic);
    }

    #[test]
    fn links_are_safe_and_nothing_is_fetched() {
        let doc = parse("[site](https://example.com) [bad](javascript:alert(1)) ![pic](https://example.com/a.png) see https://weirdnet.org/x.");
        let spans = para(&doc);
        let links: Vec<(&str, Option<&str>)> = spans.iter().filter(|s| s.link.is_some()).map(|s| (s.text.as_str(), s.link.as_deref())).collect();
        assert_eq!(
            links,
            [
                ("site", Some("https://example.com")),
                ("image: pic", Some("https://example.com/a.png")),
                ("https://weirdnet.org/x", Some("https://weirdnet.org/x"))
            ]
        );
        assert!(spans.iter().any(|s| s.text.contains("bad") && s.link.is_none()));
    }

    #[test]
    fn raw_html_stays_text() {
        let doc = parse("<script>alert(1)</script>\n\nhi <b>there</b>");
        let all = doc.to_json();
        assert!(all.contains("<script>alert(1)</script>"), "{all}");
        assert!(all.contains("<b>"));
    }

    #[test]
    fn json_shape() {
        let doc = parse("**a**\n\n```py\nx = 1\n```");
        let v: serde_json::Value = serde_json::from_str(&doc.to_json()).unwrap();
        assert_eq!(v["blocks"][0]["type"], "paragraph");
        assert_eq!(v["blocks"][0]["spans"][0]["bold"], true);
        assert_eq!(v["blocks"][1]["type"], "code");
        assert_eq!(v["blocks"][1]["lang"], "py");
        assert_eq!(v["blocks"][1]["tokens"][0]["kind"], "plain");
        assert_eq!(serde_json::from_str::<Document>(&doc.to_json()).unwrap(), doc);
    }

    #[test]
    fn every_prefix_parses() {
        let md = "# T\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```js\nconst a = `x`; // **c**\n```\n\n- [ ] x\n> q\n\n1) z ~~s~~ [l](http://a.b)";
        for n in 0..=md.len() {
            if md.is_char_boundary(n) {
                let _ = parse_streaming(&md[..n]);
            }
        }
    }
}
