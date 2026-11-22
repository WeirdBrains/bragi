//! Replies drawn in the terminal from `bragi_core::markdown`'s parsed document (the same parse
//! the desktop and phone apps draw): headings, emphasis, inline code, links with their address,
//! lists and task lists, quotes, tables, rules and highlighted code blocks.

use bragi_core::highlight::{Token, TokenKind};
use bragi_core::markdown::{Align, Block, Document, Item, Span as MdSpan};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::hit::{Action, Mark};
use crate::theme::{self, Theme};

fn th() -> &'static Theme {
    theme::get()
}

/// A finished reply's lines at `width` columns.
pub fn render(text: &str, width: usize) -> Vec<Line<'static>> {
    lines(&bragi_core::markdown::parse(text), width)
}

/// A reply still streaming in: an unclosed fence or emphasis doesn't flash as raw marks.
pub fn render_streaming(text: &str, width: usize) -> Vec<Line<'static>> {
    lines(&bragi_core::markdown::parse_streaming(text), width)
}

pub fn lines(doc: &Document, width: usize) -> Vec<Line<'static>> {
    lines_marked(doc, width).0
}

thread_local! {
    /// The copy buttons of the code blocks drawn by the render in progress.
    static COPIES: std::cell::RefCell<Vec<Mark>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Like [`render`] (or [`render_streaming`]), with what a click can do in it: its links and its
/// code blocks' copy buttons. Columns are within the lines; `line` is the line's index.
pub fn render_marked(text: &str, width: usize, streaming: bool) -> (Vec<Line<'static>>, Vec<Mark>) {
    let doc = if streaming { bragi_core::markdown::parse_streaming(text) } else { bragi_core::markdown::parse(text) };
    lines_marked(&doc, width)
}

pub fn lines_marked(doc: &Document, width: usize) -> (Vec<Line<'static>>, Vec<Mark>) {
    COPIES.with(|c| c.borrow_mut().clear());
    let mut out = Vec::new();
    blocks(&doc.blocks, width.max(8), "", &mut out);
    while out.last().is_some_and(|l: &Line| l.width() == 0) {
        out.pop();
    }
    let mut marks = COPIES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    marks.retain(|m| m.line < out.len());
    marks.extend(link_marks(&out, width.max(8)));
    (out, marks)
}

/// Addresses in drawn lines (a link is drawn as its text and its address in brackets): the
/// address, and the link text before it, open it. An address wrapped at the full width goes on
/// at the start of the next line.
fn link_marks(lines: &[Line<'static>], width: usize) -> Vec<Mark> {
    let mut out = Vec::new();
    // The address a line ended inside of, to be continued on the next line.
    let mut open: Option<(String, Vec<usize>)> = None;
    for (i, line) in lines.iter().enumerate() {
        let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        if let Some((url, mut chain)) = open.take() {
            let rest: String = plain.chars().take_while(|c| !c.is_whitespace() && !")>\"".contains(*c)).collect();
            if !rest.is_empty() && !plain.starts_with(' ') {
                let full = format!("{url}{rest}");
                let w = UnicodeWidthStr::width(rest.as_str()) as u16;
                // Every part of it opens the whole address.
                for &k in &chain {
                    out[k].action = Action::Url(full.clone());
                }
                chain.push(out.len());
                out.push(Mark { line: i, col: 0, w, action: Action::Url(full.clone()) });
                if rest.len() == plain.len() && UnicodeWidthStr::width(plain.as_str()) >= width {
                    open = Some((full, chain));
                }
                continue;
            }
        }
        let mut from = 0;
        while let Some(at) = ["https://", "http://"].iter().filter_map(|p| plain[from..].find(p).map(|x| x + from)).min() {
            let tail = &plain[at..];
            let mut end = tail.find(|c: char| c.is_whitespace() || ")>\"".contains(c)).unwrap_or(tail.len());
            let reached_end = at + end == plain.len();
            while end > 0 && ".,;:!?".contains(&tail[end - 1..end]) {
                end -= 1;
            }
            let url = tail[..end].to_string();
            let col = UnicodeWidthStr::width(&plain[..at]) as u16;
            let w = UnicodeWidthStr::width(url.as_str()) as u16;
            if url.len() > "https://".len() {
                // The link's own text, drawn just before " (address)".
                if plain[..at].ends_with(" (") {
                    let mut c = 0u16;
                    for s in &line.spans {
                        let sw = UnicodeWidthStr::width(s.content.as_ref()) as u16;
                        if s.style.add_modifier.contains(Modifier::UNDERLINED) && c + sw + 2 == col {
                            out.push(Mark { line: i, col: c, w: sw, action: Action::Url(url.clone()) });
                        }
                        c += sw;
                    }
                }
                if reached_end && col as usize + w as usize >= width {
                    open = Some((url.clone(), vec![out.len()]));
                }
                out.push(Mark { line: i, col, w, action: Action::Url(url.clone()) });
            }
            from = at + end.max(1);
        }
    }
    out
}

fn blocks(bs: &[Block], width: usize, prefix: &str, out: &mut Vec<Line<'static>>) {
    for (i, b) in bs.iter().enumerate() {
        block(b, width, prefix, out);
        // A blank line between blocks, as a renderer would leave a margin.
        if i + 1 < bs.len() {
            out.push(prefixed(prefix, Vec::new()));
        }
    }
}

/// A line under `prefix` (the bars of the quotes it is in).
fn prefixed(prefix: &str, mut spans: Vec<Span<'static>>) -> Line<'static> {
    if !prefix.is_empty() {
        spans.insert(0, Span::styled(prefix.to_string(), th().fg(th().header)));
    }
    Line::from(spans)
}

/// Pads a code block's line with its background to `room` columns.
fn fill(mut spans: Vec<Span<'static>>, room: usize, bg: Style) -> Vec<Span<'static>> {
    let w: usize = spans.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref())).sum();
    if w < room {
        spans.push(Span::styled(" ".repeat(room - w), bg));
    }
    spans
}

fn block(b: &Block, width: usize, prefix: &str, out: &mut Vec<Line<'static>>) {
    let room = width.saturating_sub(UnicodeWidthStr::width(prefix)).max(4);
    match b {
        Block::Paragraph { spans } => {
            for l in wrap_spans(&styled(spans, Style::new()), room) {
                out.push(prefixed(prefix, l));
            }
        }
        Block::Heading { level, spans } => {
            let t = th();
            let st = match level {
                1 | 2 => t.bold_accent(),
                _ => t.fg(t.header).add_modifier(Modifier::BOLD),
            };
            let wrapped = wrap_spans(&styled(spans, st), room);
            let widest = wrapped.iter().map(|l| l.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref())).sum::<usize>()).max().unwrap_or(0);
            for l in wrapped {
                out.push(prefixed(prefix, l));
            }
            // A title gets an underline rule: heavy for the first level, light for the second.
            let rule = match level {
                1 if t.icons.rule == "─" => Some("━"),
                1 => Some("="),
                2 => Some(t.icons.rule),
                _ => None,
            };
            if let Some(r) = rule {
                let color = if *level == 1 { t.accent } else { t.border };
                out.push(prefixed(prefix, vec![Span::styled(r.repeat(widest.max(3)), t.fg(color))]));
            }
        }
        Block::Quote { blocks: inner } => {
            let t = th();
            let p = format!("{prefix}{} ", t.icons.quote);
            let start = out.len();
            blocks(inner, width, &p, out);
            for l in &mut out[start..] {
                for s in l.spans.iter_mut().skip(1) {
                    if s.style.fg.is_none() {
                        s.style = s.style.fg(t.muted);
                    }
                    s.style = s.style.add_modifier(Modifier::ITALIC);
                }
            }
        }
        Block::List { start, items } => {
            for (n, item) in items.iter().enumerate() {
                list_item(item, start.map(|s| s + n as u64), width, prefix, out);
            }
        }
        Block::Code { lang, tokens, code, closed } => {
            let t = th();
            let label = lang.clone().filter(|l| !l.is_empty()).unwrap_or_else(|| "code".into());
            let bg = Style::new().bg(t.code_bg);
            // Without colour there is no background to hold the block together: a frame does.
            let framed = !t.color();
            // The copy button at the right end of the header.
            let copy_label = if t.color() { " copy " } else { "[copy]" };
            let cw = UnicodeWidthStr::width(copy_label);
            let copy_at = UnicodeWidthStr::width(prefix) + room.saturating_sub(cw);
            COPIES.with(|c| {
                c.borrow_mut().push(Mark { line: out.len(), col: copy_at as u16, w: cw as u16, action: Action::Copy(code.clone()) });
            });
            if framed {
                let head = format!("┌─ {label}");
                let gap = room.saturating_sub(UnicodeWidthStr::width(head.as_str()) + cw);
                out.push(prefixed(prefix, vec![Span::styled(head, t.dim()), Span::raw(" ".repeat(gap)), Span::styled(copy_label, t.dim())]));
            } else {
                let tag = Span::styled(format!(" {label} "), t.pill(t.accent, t.raised));
                let used = UnicodeWidthStr::width(tag.content.as_ref());
                let gap = room.saturating_sub(used + cw);
                out.push(prefixed(prefix, vec![tag, Span::styled(" ".repeat(gap), bg), Span::styled(copy_label, t.pill(t.fg, t.raised))]));
            }
            let toks: Vec<Token> = if tokens.is_empty() { vec![Token { text: code.clone(), kind: TokenKind::Plain }] } else { tokens.clone() };
            let mut line: Vec<Span<'static>> = Vec::new();
            let flush = |line: &mut Vec<Span<'static>>, out: &mut Vec<Line<'static>>| {
                let lead = if framed { Span::styled("│ ", t.dim()) } else { Span::styled(" ", bg) };
                let mut spans = vec![lead];
                spans.append(line);
                out.push(prefixed(prefix, if framed { spans } else { fill(spans, room, bg) }));
            };
            for tok in toks {
                let st = token_style(tok.kind);
                let mut parts = tok.text.split('\n').peekable();
                while let Some(part) = parts.next() {
                    if !part.is_empty() {
                        line.push(Span::styled(part.replace('\t', "    "), st));
                    }
                    if parts.peek().is_some() {
                        flush(&mut line, out);
                    }
                }
            }
            flush(&mut line, out);
            let end = match (framed, *closed) {
                (true, true) => vec![Span::styled("└─", t.dim())],
                (true, false) => vec![Span::styled("└─ …", t.dim())],
                (false, true) => fill(vec![], room, bg),
                (false, false) => fill(vec![Span::styled(" …", bg.fg(t.dim))], room, bg),
            };
            out.push(prefixed(prefix, end));
        }
        Block::Table { align, head, rows } => table(align, head, rows, room, prefix, out),
        Block::Rule => out.push(prefixed(prefix, vec![Span::styled(th().icons.rule.repeat(room), th().fg(th().border))])),
    }
}

fn list_item(item: &Item, number: Option<u64>, width: usize, prefix: &str, out: &mut Vec<Line<'static>>) {
    let t = th();
    let (marker, mst) = match (item.checked, number) {
        (Some(true), _) => (format!("{} ", t.icons.task_done), t.fg(t.success)),
        (Some(false), _) => (format!("{} ", t.icons.task_todo), t.muted()),
        (None, Some(n)) => (format!("{n}. "), t.bold_accent()),
        (None, None) => (format!("{} ", t.icons.bullet), t.accent()),
    };
    let pad = " ".repeat(UnicodeWidthStr::width(marker.as_str()));
    let mut sub = Vec::new();
    blocks(&item.blocks, width.saturating_sub(pad.len()), "", &mut sub);
    for (i, l) in sub.into_iter().enumerate() {
        let lead = if i == 0 { Span::styled(marker.clone(), mst) } else { Span::raw(pad.clone()) };
        let mut spans = vec![lead];
        spans.extend(l.spans);
        out.push(prefixed(prefix, spans));
    }
}

fn table(align: &[Align], head: &[Vec<MdSpan>], rows: &[Vec<Vec<MdSpan>>], room: usize, prefix: &str, out: &mut Vec<Line<'static>>) {
    let text = |cell: &Vec<MdSpan>| cell.iter().map(|s| s.text.as_str()).collect::<String>().replace('\n', " ");
    let cols = head.len();
    let mut w: Vec<usize> = head.iter().map(|c| UnicodeWidthStr::width(text(c).as_str())).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate().take(cols) {
            w[i] = w[i].max(UnicodeWidthStr::width(text(c).as_str()));
        }
    }
    // Shrink the widest columns until the table fits.
    while w.iter().sum::<usize>() + 3 * cols.saturating_sub(1) > room && w.iter().any(|&x| x > 3) {
        if let Some(m) = w.iter_mut().max_by_key(|x| **x) {
            *m -= 1;
        }
    }
    let cell = |s: &str, i: usize| {
        let s = clip(s, w[i]);
        let gap = w[i].saturating_sub(UnicodeWidthStr::width(s.as_str()));
        match align.get(i).copied().unwrap_or_default() {
            Align::Right => format!("{}{s}", " ".repeat(gap)),
            Align::Center => format!("{}{s}{}", " ".repeat(gap / 2), " ".repeat(gap - gap / 2)),
            _ => format!("{s}{}", " ".repeat(gap)),
        }
    };
    let row = |cells: Vec<String>, st: Style| {
        let mut spans = Vec::new();
        for (i, c) in cells.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(format!(" {} ", th().icons.sep), th().fg(th().border)));
            }
            spans.push(Span::styled(c, st));
        }
        spans
    };
    out.push(prefixed(prefix, row(head.iter().enumerate().map(|(i, c)| cell(&text(c), i)).collect(), th().bold_accent())));
    let (r, cross) = if th().icons.rule == "─" { ("─", "─┼─") } else { ("-", "-+-") };
    let rule: Vec<String> = w.iter().map(|&n| r.repeat(n)).collect();
    out.push(prefixed(prefix, vec![Span::styled(rule.join(cross), th().fg(th().border))]));
    for r in rows {
        let cells = (0..cols).map(|i| cell(&r.get(i).map(text).unwrap_or_default(), i)).collect();
        out.push(prefixed(prefix, row(cells, Style::new())));
    }
}

fn clip(s: &str, width: usize) -> String {
    if UnicodeWidthStr::width(s) <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > width {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// Syntax colours from the theme's roles.
pub fn token_style(kind: TokenKind) -> Style {
    let t = th();
    let s = Style::new().bg(t.code_bg).fg(t.fg);
    match kind {
        TokenKind::Keyword => s.fg(t.accent).add_modifier(Modifier::BOLD),
        TokenKind::Literal | TokenKind::Number => s.fg(t.warning),
        TokenKind::String => s.fg(t.success),
        TokenKind::Comment => s.fg(t.dim).add_modifier(Modifier::ITALIC),
        TokenKind::Type => s.fg(t.info),
        TokenKind::Function => s.fg(t.header),
        TokenKind::Added => s.fg(t.add),
        TokenKind::Removed => s.fg(t.remove),
        TokenKind::Plain => s,
    }
}

/// Spans as (text, style), a link followed by its address so it can be read and copied.
fn styled(spans: &[MdSpan], base: Style) -> Vec<(String, Style)> {
    let mut out = Vec::new();
    for s in spans {
        let mut st = base;
        if s.bold {
            st = st.add_modifier(Modifier::BOLD);
        }
        if s.italic {
            st = st.add_modifier(Modifier::ITALIC);
        }
        if s.strike {
            st = st.add_modifier(Modifier::CROSSED_OUT);
        }
        if s.code {
            st = st.fg(th().header).bg(th().raised);
        }
        if s.link.is_some() {
            st = st.fg(th().link).add_modifier(Modifier::UNDERLINED);
        }
        out.push((s.text.clone(), st));
        if let Some(url) = &s.link
            && url != &s.text
        {
            out.push((format!(" ({url})"), th().dim()));
        }
    }
    out
}

/// Word-wraps styled runs to `width` columns; `\n` inside a run breaks the line.
pub fn wrap_spans(runs: &[(String, Style)], width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(4);
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut w = 0usize;
    let push = |lines: &mut Vec<Vec<Span<'static>>>, text: &str, st: Style| {
        let line = lines.last_mut().expect("one line");
        match line.last_mut() {
            Some(last) if last.style == st => last.content.to_mut().push_str(text),
            _ => line.push(Span::styled(text.to_string(), st)),
        }
    };
    for (text, st) in runs {
        for (n, part) in text.split('\n').enumerate() {
            if n > 0 {
                lines.push(Vec::new());
                w = 0;
            }
            for word in part.split_inclusive(' ') {
                let ww = UnicodeWidthStr::width(word);
                if w + ww > width && w > 0 {
                    if let Some(last) = lines.last_mut().and_then(|l| l.last_mut()) {
                        let t = last.content.trim_end().to_string();
                        last.content = t.into();
                    }
                    lines.push(Vec::new());
                    w = 0;
                    if word.trim().is_empty() {
                        continue;
                    }
                }
                if ww > width {
                    for c in word.chars() {
                        let cw = c.width().unwrap_or(0);
                        if w + cw > width {
                            lines.push(Vec::new());
                            w = 0;
                        }
                        push(&mut lines, &c.to_string(), *st);
                        w += cw;
                    }
                } else {
                    push(&mut lines, word, *st);
                    w += ww;
                }
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim_end().to_string()).collect()
    }

    #[test]
    fn draws_the_blocks_bragi_renders() {
        let text = "# Title\n\nSome **bold** and `code` with a [link](https://example.com).\n\n- one\n- [x] done\n\n1. first\n\n> quoted\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|--:|\n| x | 10 |\n\n---";
        let out = render(text, 60);
        let p = plain(&out);
        assert_eq!(p[0], "Title");
        assert_eq!(p[1], "━━━━━", "a first-level title is underlined");
        assert!(p.iter().any(|l| l == "Some bold and code with a link (https://example.com)."), "{p:#?}");
        assert!(p.iter().any(|l| l == "• one"));
        assert!(p.iter().any(|l| l == "☑ done"));
        assert!(p.iter().any(|l| l == "1. first"));
        assert!(p.iter().any(|l| l == "▎ quoted"));
        assert!(p.iter().any(|l| l == " rust"), "the language tag heads the block");
        assert!(p.iter().any(|l| l == " fn main() {}"));
        // The block's lines are filled to the width with the code background.
        let code = out.iter().find(|l| l.spans.iter().any(|s| s.content == "fn")).unwrap();
        assert_eq!(code.width(), 60);
        assert!(code.spans.iter().all(|s| s.style.bg == Some(theme::get().code_bg)));
        assert!(p.iter().any(|l| l == "a │  b"));
        assert!(p.iter().any(|l| l == "x │ 10"));
        let bold = out.iter().flat_map(|l| l.spans.iter()).find(|s| s.content == "bold").unwrap();
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn wraps_at_the_width_and_streams_an_open_fence() {
        let out = plain(&render("aaa bbb ccc ddd", 8));
        assert_eq!(out, vec!["aaa bbb", "ccc ddd"]);
        let out = plain(&render_streaming("```py\nprint(1)", 40));
        assert_eq!(out.last().unwrap(), " …");
    }

    #[test]
    fn marks_links_and_copy_buttons() {
        let text = "See [the docs](https://example.com/docs) or https://rust-lang.org.\n\n```rust\nfn main() {}\n```\n";
        let (lines, marks) = render_marked(text, 80, false);
        let plain: Vec<String> = lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect();
        let urls: Vec<(usize, u16, u16, &str)> =
            marks.iter().filter_map(|m| if let Action::Url(u) = &m.action { Some((m.line, m.col, m.w, u.as_str())) } else { None }).collect();
        // The link text and its address both open the address, and a bare one loses its full stop.
        let docs = urls.iter().filter(|u| u.3 == "https://example.com/docs").count();
        assert_eq!(docs, 2, "{urls:?} in {plain:?}");
        let text_mark = urls.iter().find(|u| u.3 == "https://example.com/docs").unwrap();
        assert_eq!(&plain[0][text_mark.1 as usize..(text_mark.1 + text_mark.2) as usize], "the docs");
        let bare = urls.iter().find(|u| u.3 == "https://rust-lang.org").expect("the bare address");
        assert_eq!(&plain[0][bare.1 as usize..(bare.1 + bare.2) as usize], "https://rust-lang.org");
        // The code block's copy button sits at the right end of its header line.
        let copy = marks.iter().find(|m| matches!(&m.action, Action::Copy(c) if c.contains("fn main"))).expect("a copy button");
        assert!(plain[copy.line].contains("rust") && plain[copy.line].trim_end().ends_with(if th().color() { "copy" } else { "[copy]" }));
        assert_eq!(plain[copy.line].chars().count(), 80);
    }

    #[test]
    fn a_long_address_wrapped_at_the_width_is_one_link() {
        let url = format!("https://example.com/{}", "a".repeat(40));
        let (lines, marks) = render_marked(&url, 20, false);
        assert!(lines.len() >= 3);
        let parts: Vec<&Mark> = marks.iter().filter(|m| m.action == Action::Url(url.clone())).collect();
        assert_eq!(parts.len(), lines.len(), "each line of it opens it: {marks:?}");
    }
}
