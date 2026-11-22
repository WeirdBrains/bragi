//! Draws a reply's Markdown ([`bragi_core::markdown`]) with GPUI. Parsing lives in bragi-core so
//! the phone app and the terminal UI show the same thing; this only maps blocks to elements.
//!
//! Links open in the system browser and show their address on hover; code blocks carry their
//! language and a Copy button. Nothing is fetched and no HTML is interpreted (the core keeps it
//! as text).

use std::ops::Range;
use std::rc::Rc;

use bragi_core::highlight::TokenKind;
use bragi_core::markdown::{Align, Block, Document, Span};
use gpui::{
    AnyElement, App, Context, Div, FontStyle, FontWeight, InteractiveText, IntoElement, Render, SharedString, StrikethroughStyle, StyledText,
    TextRun, UnderlineStyle, Window, div, font, prelude::*, px, rgb,
};

use crate::theme::Theme;

/// Called with a code block's id and text when its Copy button is pressed.
pub type OnCopy = Rc<dyn Fn(SharedString, String, &mut App)>;

/// What drawing needs besides the document.
#[derive(Clone)]
pub struct Style {
    pub theme: Theme,
    /// The monospace family found on this system.
    pub mono: SharedString,
    /// The code block whose Copy button was just pressed (it says "Copied").
    pub copied: Option<SharedString>,
    pub on_copy: OnCopy,
}

/// The monospace family to use: the first of a few common ones that is installed.
pub fn pick_mono(installed: &[String]) -> SharedString {
    const WANT: &[&str] = &[
        "Lilex",
        "JetBrains Mono",
        "JetBrainsMono Nerd Font",
        "DejaVu Sans Mono",
        "Liberation Mono",
        "Noto Sans Mono",
        "Adwaita Mono",
        "Ubuntu Mono",
        "Menlo",
        "SF Mono",
        "Consolas",
        "Cascadia Mono",
        "Courier New",
    ];
    WANT.iter()
        .find(|w| installed.iter().any(|i| i == *w))
        .map(|w| SharedString::from(*w))
        .or_else(|| installed.iter().find(|i| i.to_lowercase().contains("mono")).map(|s| SharedString::from(s.clone())))
        .unwrap_or_else(|| SharedString::from(".SystemUIFont"))
}

/// A tooltip with a link's address.
struct UrlTip(SharedString, Theme);

impl Render for UrlTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = self.1;
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(rgb(t.surface_variant))
            .border_1()
            .border_color(rgb(t.border))
            .text_size(px(12.))
            .text_color(rgb(t.text_secondary))
            .child(self.0.clone())
    }
}

fn token_color(t: &Theme, k: TokenKind) -> u32 {
    match k {
        TokenKind::Plain => t.text_primary,
        TokenKind::Keyword => 0xC792EA,
        TokenKind::Literal => 0xF78C6C,
        TokenKind::String => 0xC3E88D,
        TokenKind::Number => 0xF78C6C,
        TokenKind::Comment => 0x7A7F8C,
        TokenKind::Type => t.accent,
        TokenKind::Function => t.link,
        TokenKind::Added => 0x4CD07D,
        TokenKind::Removed => t.error,
    }
}

/// Draws `doc`. `id` must be unique in the window (the message's id).
pub fn render(doc: &Document, style: &Style, id: &str) -> Div {
    let mut out = div().flex().flex_col().gap(px(8.)).min_w(px(0.));
    for (i, b) in doc.blocks.iter().enumerate() {
        out = out.child(block(b, style, &format!("{id}-{i}"), false));
    }
    out
}

fn block(b: &Block, s: &Style, id: &str, muted: bool) -> AnyElement {
    let t = s.theme;
    match b {
        Block::Paragraph { spans } => rich(spans, s, id, muted, false).into_any_element(),
        Block::Heading { level, spans } => {
            let size = match level {
                1 => 22.,
                2 => 19.,
                3 => 17.,
                _ => 15.,
            };
            div().text_size(px(size)).line_height(px(size * 1.35)).mt(px(4.)).child(rich(spans, s, id, muted, true)).into_any_element()
        }
        Block::Quote { blocks } => {
            let mut inner = div().flex().flex_col().gap(px(6.)).flex_1().min_w(px(0.));
            for (i, b) in blocks.iter().enumerate() {
                inner = inner.child(block(b, s, &format!("{id}-{i}"), true));
            }
            div().flex().flex_row().gap(px(10.)).child(div().w(px(3.)).flex_none().rounded(px(2.)).bg(rgb(t.border))).child(inner).into_any_element()
        }
        Block::List { start, items } => {
            let mut list = div().flex().flex_col().gap(px(4.));
            for (n, item) in items.iter().enumerate() {
                let marker: AnyElement = match (item.checked, start) {
                    (Some(done), _) => div()
                        .mt(px(4.))
                        .w(px(13.))
                        .h(px(13.))
                        .rounded(px(3.))
                        .border_1()
                        .border_color(rgb(if done { t.accent } else { t.text_secondary }))
                        .when(done, |d| d.bg(rgb(t.accent)))
                        .into_any_element(),
                    (None, Some(first)) => div().text_color(rgb(t.text_secondary)).child(format!("{}.", first + n as u64)).into_any_element(),
                    (None, None) => div().text_color(rgb(t.text_secondary)).child("•").into_any_element(),
                };
                let mut body = div().flex().flex_col().gap(px(4.)).flex_1().min_w(px(0.));
                for (i, b) in item.blocks.iter().enumerate() {
                    body = body.child(block(b, s, &format!("{id}-{n}-{i}"), muted));
                }
                list = list.child(div().flex().flex_row().gap(px(8.)).child(div().flex_none().min_w(px(18.)).child(marker)).child(body));
            }
            list.into_any_element()
        }
        Block::Code { lang, code, tokens, .. } => {
            let block_id = SharedString::from(format!("{id}-code"));
            let copied = s.copied.as_ref() == Some(&block_id);
            let on_copy = s.on_copy.clone();
            let (bid, text) = (block_id.clone(), code.clone());
            let header = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px(px(10.))
                .py(px(4.))
                .border_b_1()
                .border_color(rgb(t.border))
                .text_size(px(11.))
                .text_color(rgb(t.text_secondary))
                .child(lang.clone().unwrap_or_else(|| "text".into()))
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-copy")))
                        .px(px(8.))
                        .py(px(2.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .hover(|h| h.bg(rgb(t.surface_variant)).text_color(rgb(t.text_primary)))
                        .on_click(move |_, _, cx| on_copy(bid.clone(), text.clone(), cx))
                        .child(if copied { "Copied" } else { "Copy" }),
                );
            let mut runs = Vec::with_capacity(tokens.len());
            let mut text = String::with_capacity(code.len());
            for tok in tokens {
                text.push_str(&tok.text);
                runs.push(TextRun {
                    len: tok.text.len(),
                    font: font(s.mono.clone()),
                    color: rgb(token_color(&t, tok.kind)).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                });
            }
            if text.is_empty() {
                text.push(' ');
                runs.push(TextRun {
                    len: 1,
                    font: font(s.mono.clone()),
                    color: rgb(t.text_primary).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                });
            }
            div()
                .flex()
                .flex_col()
                .rounded(px(8.))
                .bg(rgb(t.background))
                .border_1()
                .border_color(rgb(t.border))
                .child(header)
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-scroll")))
                        .overflow_x_scroll()
                        .px(px(10.))
                        .py(px(8.))
                        .text_size(px(13.))
                        .line_height(px(19.))
                        .child(div().whitespace_nowrap().child(StyledText::new(text).with_runs(runs))),
                )
                .into_any_element()
        }
        Block::Table { align, head, rows } => {
            let cell = |spans: &[Span], col: usize, cid: String, bold: bool| {
                let a = align.get(col).copied().unwrap_or_default();
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .px(px(8.))
                    .py(px(5.))
                    .border_r_1()
                    .border_color(rgb(t.border))
                    .when(a == Align::Center, |d| d.text_center())
                    .when(a == Align::Right, |d| d.text_right())
                    .child(rich(spans, s, &cid, muted, bold))
            };
            let mut table = div().flex().flex_col().rounded(px(6.)).border_1().border_color(rgb(t.border)).overflow_hidden();
            let mut hr = div().flex().flex_row().bg(rgb(t.surface_variant));
            for (c, h) in head.iter().enumerate() {
                hr = hr.child(cell(h, c, format!("{id}-h{c}"), true));
            }
            table = table.child(hr);
            for (r, row) in rows.iter().enumerate() {
                let mut tr = div().flex().flex_row().border_t_1().border_color(rgb(t.border));
                for (c, spans) in row.iter().enumerate() {
                    tr = tr.child(cell(spans, c, format!("{id}-r{r}c{c}"), false));
                }
                table = table.child(tr);
            }
            table.into_any_element()
        }
        Block::Rule => div().h(px(1.)).my(px(4.)).bg(rgb(t.border)).into_any_element(),
    }
}

/// One paragraph's runs, clickable where they link.
fn rich(spans: &[Span], s: &Style, id: &str, muted: bool, bold: bool) -> AnyElement {
    let t = s.theme;
    let mut text = String::new();
    let mut runs = Vec::with_capacity(spans.len());
    let mut links: Vec<(Range<usize>, String)> = Vec::new();
    for sp in spans {
        let start = text.len();
        text.push_str(&sp.text);
        let mut f = font(if sp.code { s.mono.clone() } else { SharedString::from(".SystemUIFont") });
        if sp.bold || bold {
            f.weight = FontWeight::BOLD;
        }
        if sp.italic {
            f.style = FontStyle::Italic;
        }
        let color = if sp.link.is_some() {
            t.link
        } else if muted {
            t.text_secondary
        } else {
            t.text_primary
        };
        runs.push(TextRun {
            len: sp.text.len(),
            font: f,
            color: rgb(color).into(),
            background_color: sp.code.then(|| rgb(t.surface_variant).into()),
            underline: sp.link.as_ref().map(|_| UnderlineStyle { thickness: px(1.), color: Some(rgb(t.link).into()), wavy: false }),
            strikethrough: sp.strike.then(|| StrikethroughStyle { thickness: px(1.), color: Some(rgb(color).into()) }),
        });
        if let Some(url) = &sp.link {
            links.push((start..text.len(), url.clone()));
        }
    }
    if text.is_empty() {
        return div().into_any_element();
    }
    let styled = StyledText::new(text).with_runs(runs);
    if links.is_empty() {
        return styled.into_any_element();
    }
    let ranges: Vec<Range<usize>> = links.iter().map(|(r, _)| r.clone()).collect();
    let urls: Rc<Vec<(Range<usize>, String)>> = Rc::new(links);
    let (click, tip) = (urls.clone(), urls);
    InteractiveText::new(SharedString::from(format!("{id}-text")), styled)
        .on_click(ranges, move |i, _, cx| {
            if let Some((_, url)) = click.get(i) {
                cx.open_url(url);
            }
        })
        .tooltip(move |ix, _, cx| {
            tip.iter().find(|(r, _)| r.contains(&ix)).map(|(_, url)| cx.new(|_| UrlTip(SharedString::from(url.clone()), t)).into())
        })
        .into_any_element()
}
