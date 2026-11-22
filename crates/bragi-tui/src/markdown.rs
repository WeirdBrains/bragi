//! Replies drawn in the terminal from `bragi_core::markdown`'s parsed document (the same parse
//! the desktop and phone apps draw): headings, emphasis, inline code, links with their address,
//! lists and task lists, quotes, tables, rules and highlighted code blocks.

use bragi_core::highlight::{Token, TokenKind};
use bragi_core::markdown::{Align, Block, Document, Item, Span as MdSpan};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
    let mut out = Vec::new();
    blocks(&doc.blocks, width.max(8), "", &mut out);
    while out.last().is_some_and(|l: &Line| l.width() == 0) {
        out.pop();
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
            if framed {
                out.push(prefixed(prefix, vec![Span::styled(format!("┌─ {label}"), t.dim())]));
            } else {
                let tag = Span::styled(format!(" {label} "), t.pill(t.accent, t.raised));
                out.push(prefixed(prefix, fill(vec![tag], room, bg)));
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
}
