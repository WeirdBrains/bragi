//! Replies drawn in the terminal from `bragi_core::markdown`'s parsed document (the same parse
//! the desktop and phone apps draw): headings, emphasis, inline code, links with their address,
//! lists and task lists, quotes, tables, rules and highlighted code blocks.

use bragi_core::highlight::{Token, TokenKind};
use bragi_core::markdown::{Align, Block, Document, Item, Span as MdSpan};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const CODE_BG: Color = Color::Indexed(236);
const DIM: Color = Color::DarkGray;

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

fn prefixed(prefix: &str, mut spans: Vec<Span<'static>>) -> Line<'static> {
    if !prefix.is_empty() {
        spans.insert(0, Span::styled(prefix.to_string(), Style::new().fg(DIM)));
    }
    Line::from(spans)
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
            let mut st = Style::new().add_modifier(Modifier::BOLD);
            st = match level {
                1 => st.fg(Color::Cyan).add_modifier(Modifier::UNDERLINED),
                2 => st.fg(Color::Cyan),
                _ => st,
            };
            for l in wrap_spans(&styled(spans, st), room) {
                out.push(prefixed(prefix, l));
            }
        }
        Block::Quote { blocks: inner } => {
            let p = format!("{prefix}│ ");
            blocks(inner, width, &p, out);
        }
        Block::List { start, items } => {
            for (n, item) in items.iter().enumerate() {
                list_item(item, start.map(|s| s + n as u64), width, prefix, out);
            }
        }
        Block::Code { lang, tokens, code, closed } => {
            let label = lang.clone().unwrap_or_default();
            out.push(prefixed(prefix, vec![Span::styled(format!("┌─ {label}"), Style::new().fg(DIM))]));
            let toks: Vec<Token> = if tokens.is_empty() { vec![Token { text: code.clone(), kind: TokenKind::Plain }] } else { tokens.clone() };
            let mut line: Vec<Span<'static>> = Vec::new();
            let flush = |line: &mut Vec<Span<'static>>, out: &mut Vec<Line<'static>>| {
                let mut spans = vec![Span::styled("│ ", Style::new().fg(DIM))];
                spans.append(line);
                out.push(prefixed(prefix, spans));
            };
            for t in toks {
                let st = token_style(t.kind);
                let mut parts = t.text.split('\n').peekable();
                while let Some(part) = parts.next() {
                    if !part.is_empty() {
                        line.push(Span::styled(part.to_string(), st));
                    }
                    if parts.peek().is_some() {
                        flush(&mut line, out);
                    }
                }
            }
            flush(&mut line, out);
            let end = if *closed { "└─" } else { "└─ …" };
            out.push(prefixed(prefix, vec![Span::styled(end, Style::new().fg(DIM))]));
        }
        Block::Table { align, head, rows } => table(align, head, rows, room, prefix, out),
        Block::Rule => out.push(prefixed(prefix, vec![Span::styled("─".repeat(room.min(40)), Style::new().fg(DIM))])),
    }
}

fn list_item(item: &Item, number: Option<u64>, width: usize, prefix: &str, out: &mut Vec<Line<'static>>) {
    let marker = match (item.checked, number) {
        (Some(true), _) => "☑ ".to_string(),
        (Some(false), _) => "☐ ".to_string(),
        (None, Some(n)) => format!("{n}. "),
        (None, None) => "• ".to_string(),
    };
    let pad = " ".repeat(UnicodeWidthStr::width(marker.as_str()));
    let mut sub = Vec::new();
    blocks(&item.blocks, width.saturating_sub(pad.len()), "", &mut sub);
    for (i, l) in sub.into_iter().enumerate() {
        let lead = if i == 0 { Span::styled(marker.clone(), Style::new().fg(Color::Cyan)) } else { Span::raw(pad.clone()) };
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
                spans.push(Span::styled(" │ ", Style::new().fg(DIM)));
            }
            spans.push(Span::styled(c, st));
        }
        spans
    };
    out.push(prefixed(prefix, row(head.iter().enumerate().map(|(i, c)| cell(&text(c), i)).collect(), Style::new().add_modifier(Modifier::BOLD))));
    let rule: Vec<String> = w.iter().map(|&n| "─".repeat(n)).collect();
    out.push(prefixed(prefix, vec![Span::styled(rule.join("─┼─"), Style::new().fg(DIM))]));
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

fn token_style(kind: TokenKind) -> Style {
    let s = Style::new().bg(CODE_BG);
    match kind {
        TokenKind::Keyword => s.fg(Color::Magenta),
        TokenKind::Literal | TokenKind::Number => s.fg(Color::Yellow),
        TokenKind::String => s.fg(Color::Green),
        TokenKind::Comment => s.fg(DIM).add_modifier(Modifier::ITALIC),
        TokenKind::Type => s.fg(Color::Cyan),
        TokenKind::Function => s.fg(Color::Blue),
        TokenKind::Added => s.fg(Color::Green),
        TokenKind::Removed => s.fg(Color::Red),
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
            st = st.fg(Color::Yellow).bg(CODE_BG);
        }
        if s.link.is_some() {
            st = st.fg(Color::Blue).add_modifier(Modifier::UNDERLINED);
        }
        out.push((s.text.clone(), st));
        if let Some(url) = &s.link
            && url != &s.text
        {
            out.push((format!(" ({url})"), Style::new().fg(DIM)));
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
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect()
    }

    #[test]
    fn draws_the_blocks_bragi_renders() {
        let text = "# Title\n\nSome **bold** and `code` with a [link](https://example.com).\n\n- one\n- [x] done\n\n1. first\n\n> quoted\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|--:|\n| x | 10 |\n\n---";
        let out = render(text, 60);
        let p = plain(&out);
        assert_eq!(p[0], "Title");
        assert!(p.iter().any(|l| l == "Some bold and code with a link (https://example.com)."), "{p:#?}");
        assert!(p.iter().any(|l| l == "• one"));
        assert!(p.iter().any(|l| l == "☑ done"));
        assert!(p.iter().any(|l| l == "1. first"));
        assert!(p.iter().any(|l| l == "│ quoted"));
        assert!(p.iter().any(|l| l == "┌─ rust"));
        assert!(p.iter().any(|l| l == "│ fn main() {}"));
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
        assert_eq!(out.last().unwrap(), "└─ …");
    }
}
