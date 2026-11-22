//! Drawing [`App`] with ratatui, in the colours of the run's theme ([`crate::theme`]). Nothing
//! here changes state except the scroll clamps.
//!
//! The frame: a title bar, the view (chat with its sidebar, or code), the message box and a
//! status bar. Link, the picker, settings, status and help open as centred windows over the
//! dimmed view; notices show as toasts in the top right.

use bragi_core::code::{HunkLine, LineTag};
use bragi_core::{MessageState, ModePref, ModelStatus, PairState, Role};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, HighlightSpacing, List, ListItem, ListState, Padding, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Focus, Purpose, TOAST_MS, Tone, View};
use crate::code::Phase;
use crate::editor::Editor;
use crate::settings::{self, FIELDS, KEY_FIELD};
use crate::theme::{self, Theme};
use crate::tree::Row;

fn th() -> &'static Theme {
    theme::get()
}

/// The smallest terminal the app draws in.
pub const MIN_SIZE: (u16, u16) = (44, 12);

/// The keyboard shortcuts, as the help screen lists them.
pub const HELP: &[(&str, &str)] = &[
    ("Enter", "send the message (code mode: run the line)"),
    ("Alt-Enter, Ctrl-J", "new line in the message"),
    ("Esc", "stop the answer · close a screen · discard proposed changes"),
    ("Tab", "switch between the message box and the sidebar (folders and chats)"),
    ("↑ ↓ / j k", "move in lists and hunks; scroll the chat when the box is empty"),
    ("PgUp PgDn", "scroll the chat or the answer"),
    ("Ctrl-N", "new chat"),
    ("Ctrl-R", "continue a stopped answer"),
    ("Ctrl-K", "show or fold the models' reasoning (Thinking)"),
    ("Ctrl-L, F2", "link this terminal: show a code, open one, or add a node address"),
    ("Ctrl-O, F3", "pick where chats go (Weirdnet pool or an endpoint) and the model"),
    ("Ctrl-E, F4", "code mode: suggestions and rewrites on files"),
    ("Ctrl-T, F5", "status: this device, linked nodes and models"),
    ("Ctrl-P, F6", "settings: OpenAI-compatible endpoints and node addresses"),
    ("Ctrl-Y", "copy (link code, last answer, or the diff) to the terminal's clipboard"),
    ("F1, ?", "this help"),
    ("Ctrl-C", "stop the answer, or quit"),
    ("Ctrl-Q", "quit"),
];

/// The sidebar's keys and the chat box's commands.
pub const CHAT_HELP: &[(&str, &str)] = &[
    ("Enter · Space · ← →", "open a chat · fold or unfold a folder (sidebar)"),
    ("n · m · r · d", "new folder · move the chat · rename · delete (sidebar)"),
    ("/folder NAME", "make a folder (n in the sidebar: inside the selected one)"),
    ("/move FOLDER · /move -", "put the open chat in a folder · take it out"),
    ("/rename NAME · /nest F", "rename · move the selected folder"),
    ("/attach PATH · /detach", "attach a text, image or audio file · drop them"),
];

/// Code mode's lines and keys.
pub const CODE_HELP: &[(&str, &str)] = &[
    ("/add PATH…", "add files or folders as context (.gitignore is respected)"),
    ("/drop PATH|N", "remove one from the context"),
    ("/clear · /root DIR", "empty the context · work in another folder"),
    ("/rewrite FILE[:A-B] how", "rewrite a file or lines A to B"),
    ("text or /suggest [what]", "suggestions across the context"),
    ("/undo · /undo!", "put back the last written change (! even if edited since)"),
    ("y · n · Space", "accept · reject · toggle the hunk (review)"),
    ("a · r", "accept all · reject all"),
    ("v", "unified or side-by-side diff"),
    ("w, Enter", "write the accepted hunks (undo with /undo)"),
];

pub fn draw(f: &mut Frame, app: &mut App) {
    let t = th();
    let area = f.area();
    f.render_widget(Block::default().style(t.base()), area);
    if area.width < MIN_SIZE.0 || area.height < MIN_SIZE.1 {
        return too_small(f, area);
    }
    let modal = matches!(app.view, View::Link | View::Models | View::Settings | View::Themes | View::Status | View::Help);
    // What's under a window: the screen it was opened from.
    let base = match app.view {
        View::Link => View::Chat,
        View::Models | View::Settings | View::Themes | View::Status | View::Help if app.back == View::Code => View::Code,
        View::Models | View::Settings | View::Themes | View::Status | View::Help => View::Chat,
        v => v,
    };
    let input_h = match base {
        View::Chat => input_height(app.input.text(), area.width),
        View::Code if app.code.phase == Phase::Compose => input_height(app.input.text(), area.width),
        _ => 0,
    };
    let [header, body, input, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(input_h), Constraint::Length(1)]).areas(area);
    draw_header(f, app, header);
    match base {
        View::Code => draw_code(f, app, body),
        _ => draw_chat(f, app, body, !modal),
    }
    if input_h > 0 {
        let streaming = app.streaming();
        let (title, placeholder) = match base {
            View::Code => ("Code".to_string(), "/add PATH · /rewrite FILE:A-B how · or ask for suggestions across the context"),
            _ if streaming => (
                format!("{} Bragi is answering · Esc stops it", t.spinner(app.stream.as_ref().map_or(0, |s| s.started.elapsed().as_millis()))),
                "Type the next message…",
            ),
            _ if !app.attachments.is_empty() => (
                format!("Message · {} attachment{}", app.attachments.len(), crate::code::plural(app.attachments.len())),
                "Say what to do with the files…",
            ),
            _ => ("Message".to_string(), "Ask Bragi anything…  /attach PATH adds a file"),
        };
        let focused = !modal && (app.focus == Focus::Input || base != View::Chat);
        let keys = if base == View::Code { "Enter run" } else { "Enter send · Alt-Enter new line" };
        draw_editor(f, &app.input, input, &title, placeholder, keys, focused);
    }
    if modal {
        backdrop(f, Rect { height: body.height + input.height, ..body });
        match app.view {
            // The link window may cover the message box too: its QR code wants the rows.
            View::Link => draw_link(f, app, Rect { height: body.height + input.height, ..body }),
            View::Status => draw_status(f, app, body),
            View::Models => draw_models(f, app, body),
            View::Settings => draw_settings(f, app, body),
            View::Themes => draw_themes(f, app, body),
            _ => draw_help(f, app, body),
        }
    }
    draw_footer(f, app, footer);
    draw_toast(f, app, body);
}

fn too_small(f: &mut Frame, area: Rect) {
    let t = th();
    let lines = vec![
        Line::styled(format!("{} Bragi", t.icons.brand), t.bold_accent()),
        Line::styled("Make the terminal bigger:", t.text()),
        Line::styled(format!("{}×{} now, {}×{} at least.", area.width, area.height, MIN_SIZE.0, MIN_SIZE.1), t.muted()),
    ];
    let h = (lines.len() as u16).min(area.height);
    let y = area.y + area.height.saturating_sub(h) / 2;
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), Rect { y, height: h, ..area });
}

/// Borders in the theme's glyphs: rounded, or `+-|` without Unicode.
fn frame(block: Block<'static>) -> Block<'static> {
    if th().icons.rule == "─" {
        block.border_type(BorderType::Rounded)
    } else {
        block.border_set(symbols::border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: "|",
            vertical_right: "|",
            horizontal_top: "-",
            horizontal_bottom: "-",
        })
    }
}

/// A titled pane; the focused one has the accent border and title.
fn pane(title: &str, focused: bool) -> Block<'static> {
    let t = th();
    frame(Block::bordered())
        .border_style(t.border(focused))
        .title(Span::styled(format!(" {title} "), t.title(focused)))
        .style(Style::new().bg(t.bg).fg(t.fg))
}

/// Dims what's under a window.
fn backdrop(f: &mut Frame, area: Rect) {
    let t = th();
    let area = area.intersection(f.area());
    let buf = f.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            if t.color() {
                cell.set_fg(t.faint);
                cell.set_bg(t.shadow);
            } else {
                cell.modifier.insert(Modifier::DIM);
            }
        }
    }
}

/// A centred window of at most `w`×`h` in `area`, with a drop shadow; returns its inside.
fn window(f: &mut Frame, area: Rect, w: u16, h: u16, title: &str, right: &str) -> Rect {
    let t = th();
    let w = w.min(area.width.saturating_sub(2)).max(10);
    let h = h.min(area.height).max(3);
    let r = Rect { x: area.x + (area.width.saturating_sub(w)) / 2, y: area.y + (area.height.saturating_sub(h)) / 2, width: w, height: h };
    if t.color() {
        let shadow = Rect { x: r.x + 1, y: r.y + 1, ..r }.intersection(f.area());
        let buf = f.buffer_mut();
        for y in shadow.top()..shadow.bottom() {
            for x in shadow.left()..shadow.right() {
                buf[(x, y)].set_bg(Color::Black).set_fg(t.dim);
            }
        }
    }
    f.render_widget(Clear, r);
    let mut block = pane(title, true).style(Style::new().bg(t.surface).fg(t.fg)).padding(Padding::horizontal(1));
    if !right.is_empty() {
        block = block.title_top(Line::from(Span::styled(format!(" {right} "), t.dim())).right_aligned());
    }
    let inner = block.inner(r);
    f.render_widget(block, r);
    inner
}

fn short(id: &str) -> String {
    let s: String = id.chars().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect();
    if id.chars().count() > 8 { format!("…{s}") } else { s }
}

fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

fn line_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| width(&s.content)).sum()
}

/// `when` (ms since the epoch) the way a chat shows it: `just now`, `5 min ago`, `3 h ago`,
/// or the day.
pub fn ago(when: i64, now: i64) -> String {
    let s = (now - when).max(0) / 1000;
    match s {
        0..=44 => "just now".into(),
        45..=3599 => format!("{} min ago", (s / 60).max(1)),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => {
            // The date (UTC) from the day count.
            let days = when.div_euclid(86_400_000);
            let (y, m, d) = civil(days);
            const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            let (ny, _, _) = civil(now.div_euclid(86_400_000));
            if y == ny { format!("{} {d}", MONTHS[(m - 1) as usize]) } else { format!("{} {d}, {y}", MONTHS[(m - 1) as usize]) }
        }
    }
}

/// Days since 1970-01-01 to (year, month, day), after Howard Hinnant's `civil_from_days`.
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn view_name(v: View) -> &'static str {
    match v {
        View::Chat => "Chat",
        View::Link => "Link",
        View::Code => "Code",
        View::Status => "Status",
        View::Models => "Models",
        View::Settings => "Settings",
        View::Themes => "Theme",
        View::Help => "Help",
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let t = th();
    f.render_widget(Block::default().style(Style::new().bg(t.surface)), area);
    let mut spans = vec![Span::styled(format!(" {} Bragi ", t.icons.brand), t.pill(t.bg, t.accent)), Span::raw(" ")];
    let tabs = [
        (View::Chat, "Chat", "^N"),
        (View::Code, "Code", "^E"),
        (View::Status, "Status", "^T"),
        (View::Settings, "Settings", "^P"),
        (View::Themes, "Theme", "^G"),
        (View::Link, "Link", "^L"),
    ];
    let wide = area.width >= 100;
    for (v, name, key) in tabs {
        let on = app.view == v || (app.view == View::Models && v == View::Chat);
        let st = if on { t.bold_accent().bg(t.raised) } else { t.muted().bg(t.surface) };
        spans.push(Span::styled(format!(" {name} "), st));
        if wide {
            spans.push(Span::styled(format!("{key} "), t.dim().bg(if on { t.raised } else { t.surface })));
        }
        spans.push(Span::styled(" ", Style::new().bg(t.surface)));
    }
    let left = line_width(&spans);
    f.render_widget(Paragraph::new(Line::from(spans)), area);
    // The open chat on the right.
    let title = match app.view {
        View::Chat | View::Models => app.current.as_ref().map(|c| c.title.clone()).unwrap_or_else(|| "New chat".into()),
        View::Code => app.code.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        v => view_name(v).to_string(),
    };
    let room = (area.width as usize).saturating_sub(left + 2);
    if room > 6 {
        let title = clip(&title, room);
        let r = Rect { x: area.x + area.width - width(&title) as u16 - 1, width: width(&title) as u16, ..area };
        f.render_widget(Paragraph::new(Span::styled(title, t.muted().bg(t.surface))), r);
    }
}

/// The keys a screen offers, for the status bar.
fn hints(app: &App) -> &'static [(&'static str, &'static str)] {
    match app.view {
        View::Chat if app.streaming() => &[("Esc", "stop"), ("PgUp/PgDn", "scroll"), ("F1", "help")],
        View::Chat if app.focus == Focus::Chats => {
            &[("↑↓", "move"), ("Enter", "open/fold"), ("n", "folder"), ("m", "move"), ("r", "rename"), ("d", "delete"), ("Tab", "back")]
        }
        View::Chat => &[("Enter", "send"), ("Tab", "chats"), ("^O", "target"), ("^K", "thinking"), ("F1", "help"), ("^Q", "quit")],
        View::Link => &[("s", "show a code"), ("c", "copy"), ("Enter", "open/add"), ("Esc", "back")],
        View::Code => match app.code.phase {
            Phase::Review => &[("y/n", "accept/reject"), ("↑↓", "hunks"), ("a/r", "all"), ("v", "view"), ("w", "write"), ("Esc", "discard")],
            Phase::Asking => &[("Esc", "stop")],
            Phase::Compose => &[("Enter", "run"), ("/undo", "undo"), ("Esc", "back"), ("F1", "help")],
        },
        View::Status => &[("r", "sync now"), ("Esc", "back")],
        View::Models => &[("↑↓", "move"), ("Tab", "targets/models"), ("Enter", "use"), ("a", "automatic"), ("Esc", "back")],
        View::Settings if app.settings.form.is_some() => &[("Tab", "next"), ("^O", "pool/fallback"), ("Enter", "save"), ("Esc", "cancel")],
        View::Settings => {
            &[("a", "add"), ("e", "edit"), ("Space", "on/off"), ("m", "mode"), ("t", "test"), ("J/K", "order"), ("d", "delete"), ("Esc", "back")]
        }
        View::Themes if app.themes.prompt.is_some() => &[("Enter", "ok"), ("Esc", "cancel")],
        View::Themes if app.themes.confirm.is_some() => &[("y", "yes"), ("any", "no")],
        View::Themes if app.themes.edit.as_ref().is_some_and(|e| e.hex.is_some()) => &[("#rrggbb", "live"), ("Enter", "keep"), ("Esc", "undo")],
        View::Themes if app.themes.edit.is_some() => {
            &[("↑↓", "role"), ("Enter", "colour"), ("[ ]", "palette"), ("Tab", "dark/light"), ("u", "undo"), ("s", "save"), ("Esc", "back")]
        }
        View::Themes => &[
            ("↑↓", "preview"),
            ("Enter", "use"),
            ("m", "mode"),
            ("e", "edit"),
            ("n/d", "new/copy"),
            ("x", "delete"),
            ("i/E", "import/export"),
            ("Esc", "back"),
        ],
        View::Help => &[("↑↓", "scroll"), ("Esc", "back")],
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let t = th();
    f.render_widget(Block::default().style(Style::new().bg(t.surface)), area);
    let wide = area.width as usize;
    let spin = app.stream.as_ref().map(|s| t.spinner(s.started.elapsed().as_millis()));
    let mode = match (app.view, app.focus, spin) {
        (View::Chat, _, Some(s)) | (View::Code, _, Some(s)) => format!(" {s} ANSWERING "),
        (View::Chat, Focus::Chats, _) => " CHATS ".into(),
        (View::Code, _, _) if app.code.phase == Phase::Review => " REVIEW ".into(),
        (v, _, _) => format!(" {} ", view_name(v).to_uppercase()),
    };
    let mut left = vec![Span::styled(mode, t.pill(t.bg, t.accent))];
    let (dot, color, state) = match (&app.state.pair, app.state.online) {
        (PairState::Linked { .. }, true) => (t.icons.on, t.success, "online"),
        (PairState::Linked { .. }, false) => (t.icons.half, t.warning, "connecting"),
        (PairState::Failed { .. }, _) => (t.icons.off, t.error, "link failed"),
        (PairState::Unpaired, _) => (t.icons.off, t.muted, "not linked"),
        _ => (t.icons.half, t.warning, "linking"),
    };
    let seg = Style::new().bg(t.raised);
    left.push(Span::styled(format!(" {dot}"), seg.fg(color)));
    left.push(Span::styled(format!(" {state} "), seg.fg(t.fg)));
    if wide >= 96 {
        let n = app.state.catalog.nodes.len();
        left.push(Span::styled(format!(" {n} node{} ", crate::code::plural(n)), t.muted()));
        left.push(Span::styled(t.icons.sep, t.fg(t.border)));
        left.push(Span::styled(format!(" {} ", app.model_label()), t.accent()));
    }
    if wide >= 76 {
        left.push(Span::styled(t.icons.sep, t.fg(t.border)));
        left.push(Span::styled(format!(" via {} ", app.target_label()), t.muted()));
    }
    let used = line_width(&left);
    // Key hints on the right, as many as fit.
    let mut right: Vec<Span> = Vec::new();
    let mut w = 0;
    for (k, v) in hints(app) {
        let need = width(k) + width(v) + 3;
        if used + w + need + 1 > wide {
            break;
        }
        right.push(Span::styled(format!(" {k}"), t.text().add_modifier(Modifier::BOLD)));
        right.push(Span::styled(format!(" {v} "), t.dim()));
        w += need;
    }
    f.render_widget(Paragraph::new(Line::from(left)), area);
    if w > 0 {
        let r = Rect { x: area.x + area.width - w as u16, width: w as u16, ..area };
        f.render_widget(Paragraph::new(Line::from(right)), r);
    }
}

/// The notice as a toast in the top right of `area`; it goes after a few seconds (a question
/// stays until answered).
fn draw_toast(f: &mut Frame, app: &App, area: Rect) {
    let t = th();
    let Some(text) = &app.notice else { return };
    let (icon, color) = match app.notice_tone {
        Tone::Ok => (t.icons.check, t.success),
        Tone::Error => (t.icons.cross, t.error),
        Tone::Ask => ("?", t.warning),
        Tone::Info => (t.icons.bullet, t.info),
    };
    let max = ((area.width as usize) * 3 / 5).clamp(24, 72).min(area.width.saturating_sub(4) as usize);
    let body = wrap(text, max.saturating_sub(6));
    let w = (body.iter().map(|l| width(l)).max().unwrap_or(0) + 6).min(max) as u16;
    let h = (body.len().min(4) + 2) as u16;
    // Top right; in code mode bottom right, clear of the file and change being reviewed.
    let y = if app.view == View::Code { area.bottom().saturating_sub(h + 1) } else { area.y + 1 };
    let r = Rect { x: area.x + area.width.saturating_sub(w + 2), y, width: w, height: h }.intersection(area);
    f.render_widget(Clear, r);
    let fresh = app.notice_since.as_ref().map(|(_, at)| at.elapsed().as_millis()).unwrap_or(0);
    // The border fades to the pane colour in the last second.
    let fading = app.notice_tone != Tone::Ask && fresh > TOAST_MS.saturating_sub(1000);
    let border = if fading { t.fg(t.border) } else { t.fg(color) };
    let block = frame(Block::bordered()).border_style(border).style(Style::new().bg(t.raised).fg(t.fg));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let lines: Vec<Line> = body
        .into_iter()
        .take(4)
        .enumerate()
        .map(|(i, l)| {
            let lead = if i == 0 { Span::styled(format!("{icon} "), t.fg(color).add_modifier(Modifier::BOLD)) } else { Span::raw("  ") };
            Line::from(vec![lead, Span::raw(l)])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

/// Splits `text` into lines no wider than `width` columns, at spaces where it can.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(4);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let mut line = String::new();
        let mut w = 0;
        for word in raw.split_inclusive(' ') {
            let ww = UnicodeWidthStr::width(word);
            if w + ww > width && !line.is_empty() {
                out.push(std::mem::take(&mut line).trim_end().to_string());
                w = 0;
            }
            if ww > width {
                for c in word.chars() {
                    let cw = c.width().unwrap_or(0);
                    if w + cw > width {
                        out.push(std::mem::take(&mut line));
                        w = 0;
                    }
                    line.push(c);
                    w += cw;
                }
            } else {
                line.push_str(word);
                w += ww;
            }
        }
        out.push(line.trim_end().to_string());
    }
    out
}

fn input_height(text: &str, width: u16) -> u16 {
    let lines = wrap(text, width.saturating_sub(4) as usize).len().max(1);
    (lines.min(6) + 2) as u16
}

/// A text box: grows with its text (up to six lines), a placeholder when empty, the cursor
/// where the editor has it.
fn draw_editor(f: &mut Frame, editor: &Editor, area: Rect, title: &str, placeholder: &str, right: &str, focused: bool) {
    let t = th();
    let mut block = pane(title, focused).padding(Padding::horizontal(1));
    if focused && area.width as usize > width(title) + width(right) + 12 {
        block = block.title_top(Line::from(Span::styled(format!(" {right} "), t.dim())).right_aligned());
    }
    let inner = block.inner(area);
    let w = inner.width.max(1) as usize;
    if editor.text().is_empty() {
        f.render_widget(Paragraph::new(Span::styled(clip(placeholder, w), t.dim().add_modifier(Modifier::ITALIC))).block(block), area);
        if focused && inner.height > 0 {
            f.set_cursor_position((inner.x, inner.y));
        }
        return;
    }
    // Lines as the cursor sees them: wrapped hard at the width so the cursor lands right.
    let mut lines: Vec<String> = Vec::new();
    let (mut cx, mut cy) = (0usize, 0usize);
    let mut col = 0usize;
    let mut cur = String::new();
    for (i, c) in editor.text().chars().enumerate() {
        if i == editor.cursor() {
            (cx, cy) = (col, lines.len());
        }
        if c == '\n' {
            lines.push(std::mem::take(&mut cur));
            col = 0;
            continue;
        }
        let cw = c.width().unwrap_or(0);
        if col + cw > w {
            lines.push(std::mem::take(&mut cur));
            col = 0;
        }
        cur.push(c);
        col += cw;
    }
    if editor.cursor() >= editor.text().chars().count() {
        if col >= w {
            lines.push(std::mem::take(&mut cur));
            col = 0;
        }
        (cx, cy) = (col, lines.len());
    }
    lines.push(cur);
    let rows = inner.height as usize;
    let first = (cy + 1).saturating_sub(rows);
    let shown: Vec<Line> = lines.iter().skip(first).take(rows).map(|l| Line::styled(l.clone(), t.text())).collect();
    f.render_widget(Paragraph::new(shown).block(block), area);
    if focused && rows > 0 {
        f.set_cursor_position((inner.x + cx as u16, inner.y + (cy - first) as u16));
    }
}

/// Shows the last `area.height` of `lines`, `scroll` lines up from the bottom (clamped), with a
/// marker when there's more below.
fn bottom(f: &mut Frame, lines: Vec<Line<'static>>, area: Rect, scroll: &mut usize) {
    let t = th();
    let h = area.height as usize;
    let max = lines.len().saturating_sub(h);
    *scroll = (*scroll).min(max);
    let start = lines.len().saturating_sub(h + *scroll);
    let shown: Vec<Line> = lines.into_iter().skip(start).take(h).collect();
    f.render_widget(Paragraph::new(shown), area);
    if *scroll > 0 && area.height > 0 {
        let tag = format!(" {} more below · PgDn ", *scroll);
        let w = (width(&tag) as u16).min(area.width);
        let r = Rect { x: area.x + area.width - w, y: area.y + area.height - 1, width: w, height: 1 };
        f.render_widget(Paragraph::new(Span::styled(tag, t.pill(t.bg, t.muted))), r);
    }
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let t = th();
    let focused = app.focus == Focus::Chats;
    let chats = app.sessions.len();
    let block =
        pane("Chats", focused).title_top(Line::from(Span::styled(format!(" {chats} "), t.dim())).right_aligned()).padding(Padding::horizontal(0));
    let current = app.current.as_ref().map(|c| c.id.clone());
    let w = block.inner(area).width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|r| {
            let pad = "  ".repeat(r.depth());
            match r {
                Row::Folder { name, open, chats, .. } => {
                    let chev = if *open { t.icons.open } else { t.icons.closed };
                    let count = format!("{chats} ");
                    let name = clip(name, w.saturating_sub(pad.len() + 3 + count.len()));
                    let gap = w.saturating_sub(pad.len() + 2 + width(&name) + count.len());
                    ListItem::new(Line::from(vec![
                        Span::raw(pad),
                        Span::styled(format!("{chev} "), t.accent()),
                        Span::styled(name, t.text().add_modifier(Modifier::BOLD)),
                        Span::raw(" ".repeat(gap)),
                        Span::styled(count, t.dim()),
                    ]))
                }
                Row::Chat { session, .. } => {
                    let here = current.as_deref() == Some(session.id.as_str());
                    let (icon, ist) = if session.unbound {
                        (t.icons.warn, t.fg(t.error))
                    } else if here {
                        (t.icons.chat_here, t.accent())
                    } else {
                        (t.icons.chat, t.dim())
                    };
                    let title = clip(&session.title, w.saturating_sub(pad.len() + 2));
                    let tst = if here { t.text().add_modifier(Modifier::BOLD) } else { t.text() };
                    let mut lines = vec![Line::from(vec![Span::raw(pad.clone()), Span::styled(format!("{icon} "), ist), Span::styled(title, tst)])];
                    if session.target.as_ref().is_some_and(|x| *x != bragi_core::Target::Pool) || session.unbound {
                        lines.push(Line::styled(format!("{pad}  {}", clip(&session.target_label, w.saturating_sub(pad.len() + 2))), t.dim()));
                    }
                    ListItem::new(lines)
                }
            }
        })
        .collect();
    if items.is_empty() {
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = vec![Line::styled("No chats yet.", t.muted()), Line::styled("Ctrl-N starts one.", t.dim())];
        f.render_widget(Paragraph::new(msg).alignment(Alignment::Center), Rect { y: inner.y + 1, height: 2.min(inner.height), ..inner });
        return;
    }
    let sel = if focused { Some(app.chat_sel) } else { app.rows.iter().position(|r| r.chat_id().is_some() && r.chat_id() == current.as_deref()) };
    let mut state = ListState::default().with_selected(sel);
    let hl = if focused { t.selection() } else { Style::new().bg(t.raised) };
    let list = List::new(items)
        .block(block)
        .highlight_style(hl)
        .highlight_symbol(Line::styled(format!("{} ", t.icons.bar), if focused { t.accent() } else { t.fg(t.border) }))
        .highlight_spacing(HighlightSpacing::Always);
    f.render_stateful_widget(list, area, &mut state);
}

/// The Thinking section: a chevron and the title line (`Thinking… 4s` / `Thought for 4s`, the
/// words every shell uses) and, when unfolded, the reasoning drawn dim.
fn thinking(lines: &mut Vec<Line<'static>>, gutter: &Span<'static>, reasoning: &str, ms: Option<u64>, done: bool, open: bool, width: usize) {
    let t = th();
    if reasoning.trim().is_empty() {
        return;
    }
    let title = bragi_core::think::label(ms.unwrap_or(0), done);
    let fold = if open { t.icons.open } else { t.icons.closed };
    lines.push(Line::from(vec![
        gutter.clone(),
        Span::styled(format!("{fold} {title}"), t.muted().add_modifier(Modifier::ITALIC)),
        Span::styled(if open { "  Ctrl-K folds it" } else { "  Ctrl-K shows it" }, t.dim()),
    ]));
    if open {
        for l in crate::markdown::render(reasoning, width.saturating_sub(2)) {
            let mut spans = vec![gutter.clone(), Span::styled("┊ ", t.dim())];
            spans.extend(l.spans.into_iter().map(|s| Span::styled(s.content, s.style.fg(t.dim).add_modifier(Modifier::ITALIC))));
            lines.push(Line::from(spans));
        }
    }
}

/// A key cap and what it does, for the empty chat's tips.
fn tip(key: &str, what: &str) -> Line<'static> {
    let t = th();
    Line::from(vec![Span::styled(format!(" {key} "), t.pill(t.fg, t.raised)), Span::styled(format!("  {what}"), t.muted())])
}

fn draw_chat(f: &mut Frame, app: &mut App, area: Rect, active: bool) {
    let t = th();
    // The sidebar folds away in narrow terminals.
    let (list, main) = if area.width >= 90 {
        let side = if area.width >= 140 { 34 } else { 30 };
        let [a, b] = Layout::horizontal([Constraint::Length(side), Constraint::Min(20)]).areas(area);
        (Some(a), b)
    } else {
        (None, area)
    };
    if let Some(list) = list {
        draw_sidebar(f, app, list);
    }
    let title = app.current.as_ref().map(|c| c.title.clone()).unwrap_or_else(|| "New chat".into());
    let title = clip(&title, (main.width as usize).saturating_sub(30).max(8));
    let block = pane(&title, active && app.focus == Focus::Input)
        .title_top(Line::from(Span::styled(format!(" {} · {} ", app.target_label(), app.model_label()), t.dim())).right_aligned())
        .padding(Padding::new(1, 1, 0, 0));
    let inner = block.inner(main);
    f.render_widget(block, main);
    let w = inner.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let streaming_here =
        app.stream.as_ref().filter(|s| s.purpose == Purpose::Chat && app.current.as_ref().is_some_and(|c| c.id == s.conversation_id));
    let now = bragi_core::history::now_ms();
    match &app.current {
        None => {
            let mut card = vec![
                Line::styled(format!("{}  B R A G I", t.icons.brand), t.bold_accent()),
                Line::styled("the Weirdnet in your terminal", t.dim()),
                Line::raw(""),
            ];
            if !app.linked() && app.settings.endpoints.is_empty() {
                for l in wrap("This terminal isn't linked yet: press Ctrl-L (or add an endpoint with Ctrl-P).", w.saturating_sub(4)) {
                    card.push(Line::styled(l, t.fg(t.warning)));
                }
            } else {
                for l in
                    wrap(&format!("New chat with {} · {}. Type below and press Enter.", app.target_label(), app.model_label()), w.saturating_sub(4))
                {
                    card.push(Line::styled(l, t.text()));
                }
            }
            card.push(Line::raw(""));
            card.extend([
                tip("Ctrl-O", "where chats go, and the model"),
                tip("/attach PATH", "send a file with the next message"),
                tip("Ctrl-E", "code: suggestions and rewrites"),
                tip("F1", "every key"),
            ]);
            // Centred as a block: the tips line up on the left.
            let bw = card.iter().map(|l| l.width()).max().unwrap_or(0) as u16;
            let bh = card.len() as u16;
            let r = Rect {
                x: inner.x + inner.width.saturating_sub(bw) / 2,
                y: inner.y + inner.height.saturating_sub(bh) / 2,
                width: bw.min(inner.width),
                height: bh.min(inner.height),
            };
            f.render_widget(Paragraph::new(card), r);
            return;
        }
        Some(c) => {
            let n = c.messages.len();
            for (i, m) in c.messages.iter().enumerate() {
                let live = streaming_here.filter(|_| i + 1 == n && m.role == Role::Assistant);
                let when = Span::styled(format!(" · {}", ago(m.at_ms, now)), t.dim());
                if m.role == Role::User {
                    user_bubble(&mut lines, m, when, w);
                    lines.push(Line::raw(""));
                    continue;
                }
                let model = live.and_then(|s| s.model.clone()).or_else(|| m.model.clone()).unwrap_or_default();
                let node = live.and_then(|s| s.node.clone()).or_else(|| m.node.clone()).map(|n| format!(" on {}", short(&n))).unwrap_or_default();
                let flags = match (&m.standin, m.cache_hit) {
                    (Some(s), _) => format!(" · stand-in {s}"),
                    (None, true) => " · cached".into(),
                    _ => String::new(),
                };
                let mut head = vec![Span::styled(format!("{} Bragi", t.icons.bot), t.bold_accent())];
                if !model.is_empty() {
                    head.push(Span::styled(format!(" · {model}{node}{flags}"), t.muted()));
                }
                if live.is_none() {
                    head.push(when);
                }
                lines.push(Line::from(head));
                let gutter = Span::styled(format!("{} ", t.icons.sep), t.fg(t.border));
                let body_w = w.saturating_sub(2);
                for a in &m.attachments {
                    lines.push(Line::from(vec![gutter.clone(), Span::styled(format!("{} attached {}", t.icons.attach, a.label()), t.muted())]));
                }
                let text = live.map(|s| s.text.clone()).unwrap_or_else(|| m.text.clone());
                match live {
                    Some(s) => {
                        let ms = s.thought_ms.or_else(|| s.thinking_since.map(|x| x.elapsed().as_millis() as u64));
                        thinking(&mut lines, &gutter, &s.reasoning, ms, s.thought_ms.is_some() || !text.is_empty(), app.show_thinking, body_w);
                    }
                    None => thinking(&mut lines, &gutter, &m.reasoning, m.thinking_ms, true, app.show_thinking, body_w),
                }
                let body = if live.is_some() { crate::markdown::render_streaming(&text, body_w) } else { crate::markdown::render(&text, body_w) };
                for l in body {
                    let mut spans = vec![gutter.clone()];
                    spans.extend(l.spans);
                    lines.push(Line::from(spans));
                }
                if let Some(s) = live {
                    let secs = s.started.elapsed().as_millis();
                    let what = match (s.queued, text.is_empty(), s.reasoning.is_empty()) {
                        (Some(p), _, _) => format!("Waiting for a free node (position {p})…"),
                        (None, true, false) => "thinking".into(),
                        (None, true, true) => "waiting for the first words".into(),
                        (None, false, _) => "writing".into(),
                    };
                    lines.push(Line::from(vec![
                        gutter.clone(),
                        Span::styled(format!("{} ", t.spinner(secs)), t.bold_accent()),
                        Span::styled(what, t.muted().add_modifier(Modifier::ITALIC)),
                        Span::styled(format!(" · {:.1}s", secs as f64 / 1000.0), t.dim()),
                    ]));
                } else {
                    match m.state {
                        MessageState::Stopped => {
                            lines.push(Line::from(vec![gutter.clone(), Span::styled("stopped · Ctrl-R continues", t.fg(t.warning))]))
                        }
                        MessageState::Failed => {
                            lines.push(Line::from(vec![gutter.clone(), Span::styled("failed · Ctrl-R tries to continue", t.fg(t.error))]))
                        }
                        _ => {}
                    }
                }
                lines.push(Line::raw(""));
            }
        }
    }
    while lines.last().is_some_and(|l| l.width() == 0) {
        lines.pop();
    }
    bottom(f, lines, inner, &mut app.scroll);
}

/// The user's message: a bubble on the right with the label above it.
fn user_bubble(lines: &mut Vec<Line<'static>>, m: &bragi_core::ChatMessage, when: Span<'static>, w: usize) {
    let t = th();
    let max = (w * 4 / 5).max(12).min(w);
    let mut body: Vec<(String, Style)> = Vec::new();
    for a in &m.attachments {
        body.push((format!("{} attached {}", t.icons.attach, a.label()), t.muted()));
    }
    for l in wrap(&m.text, max.saturating_sub(4)) {
        body.push((l, t.text()));
    }
    let inner = body.iter().map(|(l, _)| width(l)).max().unwrap_or(0);
    let bw = (inner + 4).min(max);
    let head = vec![Span::styled("You", t.fg(t.header).add_modifier(Modifier::BOLD)), when, Span::styled(format!(" {}", t.icons.user), t.accent())];
    let hw = line_width(&head);
    let mut spans = vec![Span::raw(" ".repeat(w.saturating_sub(hw)))];
    spans.extend(head);
    lines.push(Line::from(spans));
    let bg = Style::new().bg(t.raised);
    for (l, st) in body {
        let pad = bw.saturating_sub(width(&l) + 4);
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(w.saturating_sub(bw))),
            Span::styled("  ", bg),
            Span::styled(l, st.bg(t.raised)),
            Span::styled(" ".repeat(pad + 2), bg),
        ]));
    }
}

/// The link code as a QR code in half-block characters: low error correction and a two-module
/// margin (instead of four) keep it small enough for a 40-row terminal; screens scan it fine.
fn qr_lines(text: &str) -> Option<Vec<String>> {
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::L).ok()?;
    let s = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(false)
        // Drawn black (the text colour) on white (the background): see `qr_style`.
        .dark_color(qrcode::render::unicode::Dense1x2::Dark)
        .light_color(qrcode::render::unicode::Dense1x2::Light)
        .build();
    let rows: Vec<String> = s.lines().map(|l| format!("  {l}  ")).collect();
    let blank = " ".repeat(rows.first().map(|l| l.chars().count()).unwrap_or(0));
    Some(std::iter::once(blank.clone()).chain(rows).chain(std::iter::once(blank)).collect())
}

fn draw_link(f: &mut Frame, app: &mut App, area: Rect) {
    let t = th();
    let qr = match &app.state.pair {
        PairState::Offering { link, .. } if app.link_check.is_none() => qr_lines(link),
        _ => None,
    };
    let qw = qr.as_ref().and_then(|q| q.first()).map(|l| l.chars().count()).unwrap_or(0) as u16;
    let inner = window(f, area, if qr.is_some() { 124 } else { 92 }, area.height, "Link this terminal to your Weirdnet", "Esc back");
    // The QR code takes the right side, full height, when there's room beside the words; else
    // it goes under them if it fits.
    let (col, qr_area) = match &qr {
        Some(q) if inner.width >= qw + 36 && q.len() <= inner.height as usize => {
            let [a, b] = Layout::horizontal([Constraint::Min(34), Constraint::Length(qw + 2)]).areas(inner);
            (a, Some((b, q.clone())))
        }
        _ => (inner, None),
    };
    let [top, field] = Layout::vertical([Constraint::Min(3), Constraint::Length(input_height(app.link_input.text(), col.width))]).areas(col);
    let w = top.width.saturating_sub(1) as usize;
    let mut lines: Vec<Line<'static>> = vec![Line::raw("")];
    let push = |lines: &mut Vec<Line<'static>>, text: &str, style: Style| {
        for l in wrap(text, w) {
            lines.push(Line::styled(l, style));
        }
    };
    let code = |sas: &str| Line::from(vec![Span::raw("    "), Span::styled(format!(" {sas} "), t.pill(t.bg, t.accent))]);
    if let Some(check) = &app.link_check {
        push(&mut lines, &format!("Does {} show this code?", check.device), t.text());
        lines.push(Line::raw(""));
        lines.push(code(&check.sas));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled(" y ", t.pill(t.bg, t.success)),
            Span::styled(" yes, link   ", t.muted()),
            Span::styled(" n ", t.pill(t.bg, t.error)),
            Span::styled(" no, cancel", t.muted()),
        ]));
    } else {
        match &app.state.pair {
            PairState::Unpaired => {
                push(&mut lines, "Two ways:", t.text().add_modifier(Modifier::BOLD));
                lines.push(Line::raw(""));
                let step = |lines: &mut Vec<Line<'static>>, n: &str, text: &str| {
                    for (i, l) in wrap(text, w.saturating_sub(5)).into_iter().enumerate() {
                        let lead = if i == 0 { Span::styled(format!(" {n} "), t.pill(t.bg, t.accent)) } else { Span::raw("   ") };
                        lines.push(Line::from(vec![lead, Span::raw("  "), Span::styled(l, t.text())]));
                    }
                };
                step(
                    &mut lines,
                    "1",
                    "Press s to show a link code here, then open it on a device that is already linked (on a Ymir node: ymirctl pool add CODE; in Bragi: Link a device).",
                );
                lines.push(Line::raw(""));
                step(&mut lines, "2", "Or paste a code shown by another device below and press Enter.");
                lines.push(Line::raw(""));
                push(
                    &mut lines,
                    "A node address (/ip4/…/p2p/…, from ymirctl pool addresses) pasted below is dialed from now on, for nodes Bragi can't find by itself.",
                    t.dim(),
                );
            }
            PairState::Offering { link, expires_ms } => {
                let left = (*expires_ms - bragi_core::history::now_ms()).max(0) / 1000;
                lines.push(Line::from(vec![
                    Span::styled("Open this code on a device that's already linked. ", t.text()),
                    Span::styled(format!("{} {}:{:02}", t.spinner(left as u128 * 1000 / 9), left / 60, left % 60), t.fg(t.warning)),
                ]));
                push(&mut lines, "c copies it to your clipboard, even over SSH.", t.muted());
                lines.push(Line::raw(""));
                push(&mut lines, link, t.accent());
                lines.push(Line::raw(""));
                if qr_area.is_none()
                    && let Some(qr) = qr_lines(link)
                {
                    let qw = qr.first().map(|l| l.chars().count()).unwrap_or(0);
                    if qr.len() + lines.len() <= top.height as usize && qw <= w {
                        let pad = " ".repeat((w - qw) / 2);
                        lines.extend(qr.into_iter().map(|l| Line::from(vec![Span::raw(pad.clone()), Span::styled(l, qr_style())])));
                    } else {
                        push(&mut lines, "(Make the terminal bigger to see the QR code.)", t.dim());
                    }
                } else if qr_area.is_some() {
                    push(&mut lines, "Or scan the code on the right with a phone that's linked.", t.muted());
                }
            }
            PairState::Confirm { sas } => {
                push(&mut lines, "Does the other device show this code?", t.text());
                lines.push(Line::raw(""));
                lines.push(code(sas));
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled(" y ", t.pill(t.bg, t.success)),
                    Span::styled(" yes, link   ", t.muted()),
                    Span::styled(" n ", t.pill(t.bg, t.error)),
                    Span::styled(" no, cancel", t.muted()),
                ]));
            }
            PairState::Waiting { sas } => {
                lines.push(Line::from(vec![
                    Span::styled(format!("{} ", t.spinner(bragi_core::history::now_ms() as u128)), t.accent()),
                    Span::styled(format!("Confirmed {sas} here; waiting for the other device…"), t.text()),
                ]));
            }
            PairState::Linked { user, device } => {
                lines.push(Line::from(vec![
                    Span::styled(format!("{} ", t.icons.check), t.fg(t.success).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("Linked: this terminal is device {} of user {}.", short(device), short(user)), t.fg(t.success)),
                ]));
                lines.push(Line::raw(""));
                push(&mut lines, "Paste a node address below to dial that node from now on, or a code from another device to open it.", t.text());
                let peers = app.core.peers();
                if !peers.is_empty() {
                    lines.push(Line::raw(""));
                    lines.push(Line::styled("Node addresses dialed", t.title(false)));
                    for p in peers {
                        push(&mut lines, &format!("  {} {p}", t.icons.bullet), t.dim());
                    }
                }
            }
            PairState::Failed { message } => {
                lines
                    .push(Line::from(vec![Span::styled(format!("{} ", t.icons.cross), t.fg(t.error)), Span::styled(message.clone(), t.fg(t.error))]));
                push(&mut lines, "Press s for a new code, or paste one below.", t.text());
            }
        }
    }
    // Top-anchored: the code stays where it is while the timer runs.
    lines.truncate(top.height as usize);
    f.render_widget(Paragraph::new(lines), top);
    if let Some((r, q)) = qr_area {
        let y = r.y + (r.height.saturating_sub(q.len() as u16)) / 2;
        let ql: Vec<Line> = q.into_iter().map(|l| Line::styled(l, qr_style())).collect();
        f.render_widget(Paragraph::new(ql), Rect { x: r.x + 2, y, width: r.width - 2, height: r.height.min(r.bottom() - y) });
    }
    draw_editor(f, &app.link_input, field, "Paste a link code or a node address", "weirdnet-bragi://… or /ip4/…/p2p/…", "Enter", true);
}

/// Dark on light whatever the theme, as phone cameras expect.
fn qr_style() -> Style {
    Style::new().fg(Color::Black).bg(Color::White)
}

fn section(title: &str) -> Line<'static> {
    let t = th();
    Line::styled(title.to_string(), t.fg(t.header).add_modifier(Modifier::BOLD))
}

fn draw_status(f: &mut Frame, app: &mut App, area: Rect) {
    let t = th();
    let s = &app.state;
    let mut lines: Vec<Line<'static>> = vec![Line::raw("")];
    let kv = |k: &str, v: Span<'static>| Line::from(vec![Span::styled(format!("{k:<14}"), t.muted()), v]);
    lines.push(section("This device"));
    let (pair, pc) = match &s.pair {
        PairState::Linked { .. } => ("linked".to_string(), t.success),
        PairState::Unpaired => ("not linked (Ctrl-L)".into(), t.warning),
        PairState::Failed { message } => (format!("failed: {message}"), t.error),
        _ => ("linking…".into(), t.warning),
    };
    lines.push(kv("Link", Span::styled(pair, t.fg(pc))));
    lines.push(kv("Online", if s.online { Span::styled("yes", t.fg(t.success)) } else { Span::styled("no", t.fg(t.warning)) }));
    lines.push(kv("Peer id", Span::styled(app.core.peer_id().unwrap_or_else(|| "—".into()), t.text())));
    lines.push(kv("Address", Span::styled(app.core.address().unwrap_or_else(|| "—".into()), t.text())));
    lines.push(kv("Sends via", Span::styled(app.target_label(), t.accent())));
    lines.push(Line::raw(""));
    lines.push(section(&format!("Inference nodes ({})", s.catalog.nodes.len())));
    if s.catalog.nodes.is_empty() {
        for l in wrap(&app.core.no_node_message(), 80) {
            lines.push(Line::styled(l, t.fg(t.warning)));
        }
    }
    for n in &s.catalog.nodes {
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", t.icons.on), t.fg(if n.warm > 0 { t.success } else { t.muted })),
            Span::styled(n.id.clone(), t.text()),
            Span::styled(format!("  {} chat model{}, {} warm", n.models, crate::code::plural(n.models as usize), n.warm), t.muted()),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(section(&format!("Models ({})", s.catalog.models.len())));
    for m in &s.catalog.models {
        lines.push(model_line(m, app.model.as_deref() == Some(m.id.as_str())));
    }
    let peers = app.core.peers();
    if !peers.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section("Node addresses dialed"));
        for p in peers {
            lines.push(Line::styled(format!("  {p}"), t.dim()));
        }
    }
    let h = (lines.len() + 3) as u16;
    let inner = window(f, area, 100, h, "Status", "r sync now · Esc back");
    lines.truncate(inner.height as usize);
    f.render_widget(Paragraph::new(lines), inner);
}

fn model_line(m: &bragi_core::ModelInfo, picked: bool) -> Line<'static> {
    let t = th();
    let (dot, color, label) = match m.status {
        ModelStatus::Warm => (t.icons.on, t.success, "warm"),
        ModelStatus::Busy => (t.icons.on, t.warning, "busy"),
        ModelStatus::Loading => (t.icons.half, t.warning, "loading"),
        ModelStatus::Loadable => (t.icons.off, t.muted, "loads on first use"),
    };
    let speed = m.tokens_per_sec.map(|x| format!(" · {x:.0} tok/s")).unwrap_or_default();
    let ctx = m.context.map(|c| format!(" · {}k context", c / 1024)).unwrap_or_default();
    let on = format!(" · on {} node{}", m.nodes.len(), crate::code::plural(m.nodes.len()));
    Line::from(vec![
        Span::styled(format!("  {dot} "), t.fg(color)),
        Span::styled(m.id.clone(), if picked { t.bold_accent() } else { t.text() }),
        Span::styled(format!("  {label}{speed}{ctx}{on}"), t.dim()),
    ])
}

fn draw_models(f: &mut Frame, app: &mut App, area: Rect) {
    let t = th();
    let current = app.effective_target();
    let pool = !matches!(app.targets.get(app.picker_target).map(|x| &x.target), Some(bragi_core::Target::Endpoint { .. }));
    let picked_model = match &current {
        Some(bragi_core::Target::Endpoint { model, .. }) => Some(model.clone()),
        _ => None,
    };
    let t_items: Vec<ListItem> = app
        .targets
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let on = Some(&x.target) == current.as_ref() || (current.is_none() && i == 0);
            let mark = if on { t.icons.on } else { t.icons.off };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{mark} "), if on { t.accent() } else { t.dim() }),
                Span::styled(x.label.clone(), if on { t.bold_accent() } else { t.text() }),
            ]))
        })
        .collect();
    let m_items: Vec<ListItem> = if pool {
        app.state.catalog.models.iter().map(|m| ListItem::new(model_line(m, app.model.as_deref() == Some(m.id.as_str())))).collect()
    } else {
        app.endpoint_models
            .iter()
            .map(|m| {
                let on = picked_model.as_deref() == Some(m.as_str());
                ListItem::new(Line::from(vec![
                    Span::styled(format!("  {} ", if on { t.icons.on } else { t.icons.off }), if on { t.accent() } else { t.dim() }),
                    Span::styled(m.clone(), if on { t.bold_accent() } else { t.text() }),
                ]))
            })
            .collect()
    };
    let rows = (app.targets.len() + m_items.len().max(2) + 7) as u16;
    let inner = window(f, area, 100, rows + 2, "Where chats go, and the model", "Tab switch · Enter use · a automatic");
    let [top, rest] = Layout::vertical([Constraint::Length((app.targets.len() + 2) as u16), Constraint::Min(3)]).areas(Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(1),
        ..inner
    });
    let on = t.selection();
    let off = Style::new().bg(t.raised);
    let (thl, mhl) = if app.picker_models_pane { (off, on) } else { (on, off) };
    let bar = |active: bool| Line::styled(format!("{} ", t.icons.bar), if active { t.accent() } else { t.fg(t.border) });
    let mut ts = ListState::default().with_selected(Some(app.picker_target));
    let tblock = pane("Send to", !app.picker_models_pane).style(Style::new().bg(t.surface));
    f.render_stateful_widget(
        List::new(t_items)
            .block(tblock)
            .highlight_style(thl)
            .highlight_symbol(bar(!app.picker_models_pane))
            .highlight_spacing(HighlightSpacing::Always),
        top,
        &mut ts,
    );
    let title = if pool { "Models on your nodes" } else { "Models on this endpoint" };
    let mblock = pane(title, app.picker_models_pane).style(Style::new().bg(t.surface));
    if m_items.is_empty() {
        let msg = match (pool, app.linked()) {
            (true, true) => app.core.no_node_message(),
            (true, false) => "Link this terminal first (Ctrl-L), or add an endpoint (Ctrl-P).".into(),
            (false, _) => format!("{} Asking the endpoint for its models…", t.spinner(bragi_core::history::now_ms() as u128)),
        };
        let lines: Vec<Line> = wrap(&msg, rest.width.saturating_sub(4) as usize).into_iter().map(|l| Line::styled(l, t.muted())).collect();
        f.render_widget(Paragraph::new(lines).block(mblock.padding(Padding::horizontal(1))), rest);
        return;
    }
    let mut ms = ListState::default().with_selected(app.picker_models_pane.then_some(app.model_sel));
    f.render_stateful_widget(
        List::new(m_items)
            .block(mblock)
            .highlight_style(mhl)
            .highlight_symbol(bar(app.picker_models_pane))
            .highlight_spacing(HighlightSpacing::Always),
        rest,
        &mut ms,
    );
}

/// Whether a form field holds something usable: `Some(problem)` when it doesn't.
fn field_problem(i: usize, text: &str) -> Option<&'static str> {
    let s = text.trim();
    match i {
        0 if s.is_empty() => Some("required"),
        1 if s.is_empty() => Some("required"),
        1 if !(s.starts_with("http://") || s.starts_with("https://")) => Some("starts with http:// or https://"),
        _ => None,
    }
}

fn draw_settings(f: &mut Frame, app: &mut App, area: Rect) {
    let t = th();
    let st = &app.settings;
    let right = if st.form.is_some() { "Tab next · Enter save · Esc cancel" } else { "a add · e edit · t test · d delete · Esc back" };
    // Laid out at the window's width first, then the window is made as tall as its lines.
    let ww = 108.min(area.width.saturating_sub(2)).saturating_sub(4);
    let w = ww as usize;
    let mut lines: Vec<Line<'static>> = vec![Line::raw("")];
    if st.endpoints.is_empty() {
        for l in wrap("No endpoints. a adds one: any server with an OpenAI-compatible /v1 API (llama.cpp, vLLM, Ollama, a hosted one).", w) {
            lines.push(Line::styled(l, t.muted()));
        }
    }
    for (i, e) in st.endpoints.iter().enumerate() {
        let sel = i == st.sel && st.form.is_none();
        let base = if sel { t.selection() } else { Style::new() };
        let lead = if sel { Span::styled(format!("{} ", t.icons.bar), t.accent()) } else { Span::raw("  ") };
        let dot = if e.enabled {
            Span::styled(format!("{} ", t.icons.on), base.fg(t.success))
        } else {
            Span::styled(format!("{} ", t.icons.off), base.fg(t.dim))
        };
        let mode = match e.mode {
            bragi_core::EndpointMode::Alongside => "next to the pool",
            bragi_core::EndpointMode::Fallback => "fallback",
        };
        let model = e.model.clone().map(|m| format!(" · {m}")).unwrap_or_default();
        let key = if e.has_key { " · key set" } else { "" };
        let mut spans = vec![
            lead,
            dot,
            Span::styled(format!("{}  ", e.name), base.add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}{model} · {mode}{key}", e.base_url), if sel { base } else { t.muted() }),
        ];
        let used = line_width(&spans);
        if sel && used < w {
            spans.push(Span::styled(" ".repeat(w - used), base));
        }
        lines.push(Line::from(spans));
        if e.insecure {
            lines.push(Line::styled("      plain http to another machine: the key and chats can be read on the way", t.fg(t.warning)));
        }
        if let Some(r) = st.tests.get(&e.id) {
            let ok = r.starts_with("OK");
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(format!("{} test: ", if ok { t.icons.check } else { t.icons.cross }), t.fg(if ok { t.success } else { t.error })),
                Span::styled(r.clone(), t.muted()),
            ]));
        }
    }
    let mut cursor = None;
    if let Some(form) = &st.form {
        lines.push(Line::raw(""));
        let title = if form.editing.is_some() { "Edit endpoint" } else { "Add an endpoint" };
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", t.icons.closed), t.accent()),
            Span::styled(title, t.fg(t.header).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::raw(""));
        const LABEL: usize = 20;
        let fw = w.saturating_sub(LABEL + 2 + 34).clamp(16, 56);
        for (i, name) in FIELDS.iter().enumerate() {
            let raw = form.fields[i].text();
            let shown = if i == KEY_FIELD { settings::mask(raw) } else { raw.to_string() };
            let active = i == form.field;
            let problem = field_problem(i, raw);
            // Labels line up on the right; the active one is the accent.
            let lst = if active { t.bold_accent() } else { t.muted() };
            let mut spans = vec![
                Span::styled(format!("{name:>LABEL$}"), lst),
                Span::styled(if active { format!(" {}", t.icons.bar) } else { "  ".into() }, t.accent()),
            ];
            let box_st = if active { Style::new().bg(t.raised).fg(t.fg) } else { Style::new().bg(t.surface).fg(t.fg) };
            let value = clip(&shown, fw.saturating_sub(2));
            let pad = fw.saturating_sub(width(&value) + 1);
            spans.push(Span::styled(
                format!(" {value}{}", " ".repeat(pad)),
                box_st.add_modifier(if active { Modifier::empty() } else { Modifier::UNDERLINED }),
            ));
            let hint = if i == KEY_FIELD && form.editing.is_some() && !form.key_touched { Some(("(unchanged)", t.dim())) } else { None };
            match (hint, problem, raw.trim().is_empty()) {
                (Some(h), _, _) => spans.push(Span::styled(format!("  {}", h.0), h.1)),
                (None, Some(p), true) if !active => spans.push(Span::styled(format!("  {p}"), t.dim())),
                (None, Some(p), false) => spans.push(Span::styled(format!("  {} {p}", t.icons.cross), t.fg(t.error))),
                (None, None, false) => spans.push(Span::styled(format!("  {}", t.icons.check), t.fg(t.success))),
                _ => {}
            }
            if active {
                let col = if i == KEY_FIELD {
                    form.fields[i].cursor()
                } else {
                    UnicodeWidthStr::width(&raw[..raw.char_indices().nth(form.fields[i].cursor()).map_or(raw.len(), |(b, _)| b)])
                };
                cursor = Some(((LABEL + 3 + col.min(fw.saturating_sub(2))) as u16, lines.len() as u16));
            }
            lines.push(Line::from(spans));
            lines.push(Line::raw(""));
        }
        let mode = if form.alongside {
            "offered next to the Weirdnet pool (a chat can be sent to it)"
        } else {
            "only when the pool can't answer (fallback)"
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{:>LABEL$}", "Use"), t.muted()),
            Span::raw("   "),
            Span::styled(mode, t.text()),
            Span::styled("  Ctrl-O switches", t.dim()),
        ]));
    }
    if !st.links.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section("Weirdnet node addresses (add one with Ctrl-L)"));
        for l in &st.links {
            let (dot, c) = if l.online { (t.icons.on, t.success) } else { (t.icons.off, t.dim) };
            lines.push(Line::from(vec![Span::styled(format!("  {dot} "), t.fg(c)), Span::styled(l.address.clone(), t.dim())]));
        }
    }
    lines.push(Line::raw(""));
    let inner = window(f, area, 108, (lines.len() + 2).max(14) as u16, "Settings · OpenAI-compatible endpoints", right);
    lines.truncate(inner.height as usize);
    f.render_widget(Paragraph::new(lines), inner);
    if let Some((x, y)) = cursor
        && y < inner.height
    {
        f.set_cursor_position((inner.x + x, inner.y + y));
    }
}

fn draw_help(f: &mut Frame, app: &mut App, area: Rect) {
    let t = th();
    let row = |k: &str, v: &str| Line::from(vec![Span::styled(format!("  {k:<26}"), t.accent()), Span::styled(v.to_string(), t.text())]);
    let mut lines = vec![section("Keyboard")];
    lines.extend(HELP.iter().map(|(k, v)| row(k, v)));
    lines.push(Line::raw(""));
    lines.push(section("Sidebar and chat box"));
    lines.extend(CHAT_HELP.iter().map(|(k, v)| row(k, v)));
    lines.push(Line::raw(""));
    lines.push(section("Code mode (Ctrl-E)"));
    lines.extend(CODE_HELP.iter().map(|(k, v)| row(k, v)));
    let inner = window(f, area, 112, (lines.len() + 2) as u16, "Help", "↑↓ scroll · Esc back");
    let h = inner.height as usize;
    let max = lines.len().saturating_sub(h);
    app.help_scroll = app.help_scroll.min(max);
    let more = max - app.help_scroll;
    let shown: Vec<Line> = lines.into_iter().skip(app.help_scroll).take(h).collect();
    f.render_widget(Paragraph::new(shown), inner);
    if more > 0 {
        let tag = format!(" {more} more · ↓ ");
        let w = width(&tag) as u16;
        f.render_widget(
            Paragraph::new(Span::styled(tag, t.pill(t.bg, t.muted))),
            Rect { x: inner.right().saturating_sub(w), y: inner.bottom() - 1, width: w, height: 1 },
        );
    }
}

/// The gutter of a diff line: old and new line numbers.
fn gutter(old: Option<usize>, new: Option<usize>) -> Span<'static> {
    let t = th();
    let n = |x: Option<usize>| x.map(|n| format!("{n:>4}")).unwrap_or_else(|| "    ".into());
    Span::styled(format!("{} {} ", n(old), n(new)), Style::new().fg(t.dim).bg(t.surface))
}

/// The colours of a diff line: sign, text and background.
fn diff_style(tag: LineTag) -> (char, Style, Style) {
    let t = th();
    match tag {
        LineTag::Context => (' ', t.muted(), t.text()),
        LineTag::Removed => ('-', t.fg(t.remove).bg(t.remove_bg).add_modifier(Modifier::BOLD), t.text().bg(t.remove_bg)),
        LineTag::Added => ('+', t.fg(t.add).bg(t.add_bg).add_modifier(Modifier::BOLD), t.text().bg(t.add_bg)),
    }
}

fn hunk_lines(lines: &[HunkLine], width: usize) -> Vec<Line<'static>> {
    lines
        .iter()
        .map(|l| {
            let (sign, sst, tst) = diff_style(l.tag);
            let room = width.saturating_sub(12);
            let text = clip(&l.text, room);
            let pad = room.saturating_sub(width_tabs(&text));
            Line::from(vec![
                gutter(l.old_no, l.new_no),
                Span::styled(format!("{sign} "), sst),
                Span::styled(text, tst),
                Span::styled(" ".repeat(pad), tst),
            ])
        })
        .collect()
}

fn width_tabs(s: &str) -> usize {
    width(s)
}

/// A hunk's `@@ -a,b +c,d @@` line as a header strip.
fn hunk_header(text: String, w: usize) -> Line<'static> {
    let t = th();
    let pad = w.saturating_sub(width(&text) + 2);
    Line::from(vec![Span::styled(format!(" {text} "), t.pill(t.info, t.raised)), Span::styled(" ".repeat(pad), Style::new().bg(t.surface))])
}

fn clip(s: &str, width: usize) -> String {
    let mut w = 0;
    let mut out = String::new();
    for c in s.chars() {
        let cw = if c == '\t' { 4 } else { c.width().unwrap_or(0) };
        if w + cw > width {
            out.push('…');
            break;
        }
        if c == '\t' {
            out.push_str("    ");
        } else {
            out.push(c);
        }
        w += cw;
    }
    out
}

fn draw_code(f: &mut Frame, app: &mut App, area: Rect) {
    let t = th();
    let (side, main) = if area.width >= 96 {
        let [a, b] = Layout::horizontal([Constraint::Length(34), Constraint::Min(30)]).areas(area);
        (Some(a), b)
    } else {
        (None, area)
    };
    if let Some(side) = side {
        let block = pane("Context", false).padding(Padding::horizontal(1));
        let inner = block.inner(side);
        let sw = inner.width as usize;
        let mut lines = vec![
            Line::styled("Folder", t.muted()),
            Line::styled(clip(&app.code.root.display().to_string(), sw), t.text()),
            Line::raw(""),
            Line::from(vec![
                Span::styled("Context ", t.muted()),
                Span::styled(format!("({} files, {} KiB)", app.code.gathered.files.len(), app.code.gathered.total_bytes().div_ceil(1024)), t.dim()),
            ]),
        ];
        for (i, p) in app.code.context.iter().enumerate() {
            lines.push(Line::from(vec![
                Span::styled(format!("{}. ", i + 1), t.accent()),
                Span::styled(clip(&p.display().to_string(), sw.saturating_sub(3)), t.text()),
            ]));
        }
        if app.code.context.is_empty() {
            lines.push(Line::styled("none yet: /add PATH", t.dim()));
        }
        for file in app.code.gathered.files.iter().take(inner.height.saturating_sub(lines.len() as u16 + 3) as usize) {
            lines.push(Line::styled(clip(&format!("  {} ({} lines)", file.path, file.text.lines().count()), sw), t.dim()));
        }
        if !app.code.gathered.skipped.is_empty() {
            lines.push(Line::styled(format!("{} left out", app.code.gathered.skipped.len()), t.dim()));
        }
        if !app.code.diffs.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::styled("Proposed", t.muted()));
            for (i, d) in app.code.diffs.iter().enumerate() {
                let (a, r) = d.stats();
                let on = i == app.code.file;
                let name = clip(&d.path, sw.saturating_sub(12));
                lines.push(Line::from(vec![
                    Span::styled(if on { format!("{} ", t.icons.bar) } else { "  ".into() }, t.accent()),
                    Span::styled(name, if on { t.text().add_modifier(Modifier::BOLD) } else { t.text() }),
                    Span::styled(format!(" +{a}"), t.fg(t.add)),
                    Span::styled(format!(" -{r}"), t.fg(t.remove)),
                ]));
            }
        }
        f.render_widget(Paragraph::new(lines).block(block), side);
    }
    let title = match app.code.phase {
        Phase::Compose => "Code suggestions and rewrites".to_string(),
        Phase::Asking => app.code.request.as_ref().map(bragi_core::code::prompt::title).unwrap_or_else(|| "Asking".into()),
        Phase::Review => "Review the proposed change".to_string(),
    };
    let block = pane(&clip(&title, (main.width as usize).saturating_sub(6)), app.code.phase != Phase::Compose).padding(Padding::horizontal(1));
    let inner = block.inner(main);
    f.render_widget(block, main);
    let w = inner.width as usize;
    match app.code.phase {
        Phase::Compose => {
            let mut lines: Vec<Line<'static>> = vec![Line::raw("")];
            if !app.code.explanation.is_empty() {
                lines.push(Line::styled("Last answer", t.fg(t.header).add_modifier(Modifier::BOLD)));
                lines.extend(wrap(&app.code.explanation, w).into_iter().map(|l| Line::styled(l, t.text())));
                lines.push(Line::raw(""));
            }
            for (k, v) in CODE_HELP.iter().take(6) {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {k} "), t.pill(t.fg, t.raised)),
                    Span::raw(" ".repeat(26usize.saturating_sub(width(k) + 2))),
                    Span::styled(v.to_string(), t.muted()),
                ]));
                lines.push(Line::raw(""));
            }
            f.render_widget(Paragraph::new(lines), inner);
        }
        Phase::Asking => {
            let mut lines: Vec<Line<'static>> = Vec::new();
            if let Some(s) = &app.stream {
                let secs = s.started.elapsed().as_millis();
                let what = match s.queued {
                    Some(p) => format!("Waiting for a free node (position {p})…"),
                    None if s.text.is_empty() => "asking".into(),
                    None => "writing the change".into(),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{} ", t.spinner(secs)), t.bold_accent()),
                    Span::styled(what, t.muted().add_modifier(Modifier::ITALIC)),
                    Span::styled(format!(" · {:.1}s", secs as f64 / 1000.0), t.dim()),
                ]));
                lines.push(Line::raw(""));
                lines.extend(crate::markdown::render_streaming(&s.text, w));
            }
            bottom(f, lines, inner, &mut app.scroll);
        }
        Phase::Review => {
            let Some((d, hi)) = app.code.current() else { return };
            let total: usize = app.code.diffs.iter().map(|d| d.hunks.len()).sum();
            let index = app.code.diffs[..app.code.file].iter().map(|d| d.hunks.len()).sum::<usize>() + hi + 1;
            let ok = app.code.accepted.get(app.code.file).and_then(|a| a.get(hi)).copied().unwrap_or(false);
            let (acc, all) = app.code.counts();
            let (a, r) = d.stats();
            let mut lines: Vec<Line<'static>> = vec![Line::from(vec![
                Span::styled(format!("{}{}", d.path, if d.created { " (new)" } else { "" }), t.text().add_modifier(Modifier::BOLD)),
                Span::styled(format!("  +{a}"), t.fg(t.add)),
                Span::styled(format!(" -{r}"), t.fg(t.remove)),
                Span::styled(format!("   change {index}/{total}   "), t.muted()),
                if ok {
                    Span::styled(format!(" {} accepted ", t.icons.check), t.pill(t.bg, t.success))
                } else {
                    Span::styled(format!(" {} rejected ", t.icons.cross), t.pill(t.bg, t.error))
                },
                Span::styled(format!("  {acc}/{all} accepted"), t.dim()),
            ])];
            lines.push(Line::raw(""));
            if !app.code.explanation.is_empty() {
                for l in wrap(&app.code.explanation, w).into_iter().take(4) {
                    lines.push(Line::styled(l, t.muted().add_modifier(Modifier::ITALIC)));
                }
                lines.push(Line::raw(""));
            }
            let h = &d.hunks[hi];
            lines.push(hunk_header(h.header(), w));
            if app.code.side_by_side && w >= 40 {
                let half = (w - 1) / 2;
                for (left, right) in h.side_by_side() {
                    let changed = left.as_ref().map(|l| &l.1) != right.as_ref().map(|r| &r.1);
                    let cell = |c: &Option<(usize, String)>, tag: LineTag| -> Vec<Span<'static>> {
                        let (sign, sst, tst) = if changed { diff_style(tag) } else { diff_style(LineTag::Context) };
                        match c {
                            Some((n, text)) => {
                                let room = half.saturating_sub(7);
                                let text = clip(text, room);
                                let pad = room.saturating_sub(width(&text));
                                vec![
                                    Span::styled(format!("{n:>4} "), Style::new().fg(t.dim).bg(t.surface)),
                                    Span::styled(format!("{} ", if changed { sign } else { ' ' }), sst),
                                    Span::styled(text, tst),
                                    Span::styled(" ".repeat(pad), tst),
                                ]
                            }
                            None => vec![Span::styled(" ".repeat(half), Style::new().bg(t.surface))],
                        }
                    };
                    let mut spans = cell(&left, LineTag::Removed);
                    spans.push(Span::styled(t.icons.sep, t.fg(t.border)));
                    spans.extend(cell(&right, LineTag::Added));
                    lines.push(Line::from(spans));
                }
            } else {
                lines.extend(hunk_lines(&h.lines, w));
            }
            f.render_widget(Paragraph::new(lines.into_iter().take(inner.height as usize).collect::<Vec<_>>()), inner);
        }
    }
}

// ---- the Theme screen ----

/// A role's colour as the terminal gets it now.
fn swatch_color(hex: &str) -> Color {
    match weirdnet_themes::parse_hex(hex) {
        Some(c) => weirdnet_themes::to_color(c, th().depth),
        None => Color::Reset,
    }
}

fn draw_themes(f: &mut Frame, app: &mut App, area: Rect) {
    let t = th();
    let st = &app.themes;
    let right = if st.edit.is_some() { "Editing" } else { "Pick a theme: moving previews it" };
    let inner = window(f, area, 104, 29, "Theme", right);
    if inner.height < 8 || inner.width < 40 {
        return;
    }
    let [main, foot] = Layout::vertical([Constraint::Min(4), Constraint::Length(2)]).areas(inner);
    let left_w = 34.min(main.width / 2);
    let [left, _gap, right] = Layout::horizontal([Constraint::Length(left_w), Constraint::Length(2), Constraint::Min(10)]).areas(main);
    let mut cursor: Option<(u16, u16)> = None;

    if let Some(e) = &st.edit {
        draw_theme_roles(f, e, left);
        draw_theme_side(f, app, right, true);
    } else {
        let h = left.height.saturating_sub(2) as usize;
        let first = st.sel.saturating_sub(h.saturating_sub(1));
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (i, r) in st.rows.iter().enumerate().skip(first).take(h) {
            let sel = i == st.sel;
            let base = if sel { t.selection() } else { Style::new() };
            let lead = if sel { Span::styled(format!("{} ", t.icons.bar), t.accent()) } else { Span::raw("  ") };
            let on = if r.name == st.saved.theme { t.icons.on } else { t.icons.off };
            let tag = match (r.builtin, r.user_file, &r.error) {
                (_, _, Some(_)) => " broken",
                (true, true, _) => " edited",
                (true, false, _) => "",
                (false, _, _) => " yours",
            };
            let mut spans = vec![
                lead,
                Span::styled(format!("{on} "), base.fg(if r.name == st.saved.theme { t.success } else { t.dim })),
                Span::styled(r.display_name.clone(), base.add_modifier(Modifier::BOLD)),
                Span::styled(
                    tag.to_string(),
                    if sel {
                        base
                    } else if r.error.is_some() {
                        t.fg(t.error)
                    } else {
                        t.dim()
                    },
                ),
            ];
            let used = line_width(&spans);
            if sel && used < left.width as usize {
                spans.push(Span::styled(" ".repeat(left.width as usize - used), base));
            }
            lines.push(Line::from(spans));
        }
        f.render_widget(Paragraph::new(lines), Rect { height: left.height.saturating_sub(2), ..left });
        let mode = |m: ModePref, name: &str| {
            if st.mode == m { Span::styled(format!(" {name} "), t.pill(t.bg, t.accent)) } else { Span::styled(format!(" {name} "), t.muted()) }
        };
        let mode_line = Line::from(vec![
            Span::styled("Mode ", t.dim()),
            mode(ModePref::Dark, "dark"),
            mode(ModePref::Light, "light"),
            mode(ModePref::System, "system"),
        ]);
        f.render_widget(Paragraph::new(mode_line), Rect { y: left.y + left.height.saturating_sub(1), height: 1, ..left });
        draw_theme_side(f, app, right, false);
    }

    // The footer: a prompt, a question or what the last action said.
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some((ask, ed)) = &st.prompt {
        lines.push(Line::from(vec![Span::styled(format!("{}: ", ask.label()), t.bold_accent()), Span::styled(ed.text().to_string(), t.text())]));
        let x = foot.x
            + (width(ask.label()) + 2 + ed.text()[..ed.text().char_indices().nth(ed.cursor()).map_or(ed.text().len(), |(i, _)| i)].chars().count())
                as u16;
        cursor = Some((x.min(foot.x + foot.width.saturating_sub(1)), foot.y));
    }
    if let Some(m) = &st.msg {
        lines.push(Line::styled(clip(m, foot.width as usize), if st.confirm.is_some() { t.fg(t.warning) } else { t.muted() }));
    }
    f.render_widget(Paragraph::new(lines), foot);
    if let Some((x, y)) = cursor {
        f.set_cursor_position((x, y));
    }
}

/// The role list of the editor: a swatch, the role, its colour.
fn draw_theme_roles(f: &mut Frame, e: &crate::app::EditState, area: Rect) {
    let t = th();
    let pal = bragi_core::Appearance::palette_of(&e.file, e.variant);
    let roles = bragi_core::appearance::roles();
    let h = area.height.saturating_sub(1) as usize;
    let first = e.sel.saturating_sub(h.saturating_sub(1));
    let head = Line::from(vec![
        Span::styled(format!("{} ", e.name), t.bold_accent()),
        Span::styled(format!("· {}{}", e.variant.name(), if e.dirty() { " · unsaved" } else { "" }), t.muted()),
    ]);
    f.render_widget(Paragraph::new(head), Rect { height: 1, ..area });
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, r) in roles.iter().enumerate().skip(first).take(h) {
        let sel = i == e.sel;
        let base = if sel { t.selection() } else { Style::new() };
        let hex = pal.get(r).unwrap_or("?").to_string();
        let derived = e.stored(r).is_none();
        let hexbox = match (&e.hex, sel) {
            (Some(h), true) => h.text().to_string(),
            _ => hex.clone(),
        };
        let bad = sel && e.hex.as_ref().is_some_and(|h| crate::app::parse_hex(h.text()).is_none());
        let mut spans = vec![
            if sel { Span::styled(format!("{} ", t.icons.bar), t.accent()) } else { Span::raw("  ") },
            Span::styled(if t.color() { "██" } else { "[]" }, Style::new().fg(swatch_color(&hex))),
            Span::styled(format!(" {r:<13}"), base),
            Span::styled(format!("{hexbox:<8}"), if bad { base.fg(t.error) } else { base }),
            Span::styled(if derived { " auto" } else { "" }.to_string(), if sel { base } else { t.dim() }),
        ];
        let used = line_width(&spans);
        if sel && used < area.width as usize {
            spans.push(Span::styled(" ".repeat(area.width as usize - used), base));
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area });
}

/// A sample of the screen in the colours being previewed (the window already is), the role
/// under the cursor in editing, and the contrast notes.
fn draw_theme_side(f: &mut Frame, app: &App, area: Rect, editing: bool) {
    let t = th();
    let st = &app.themes;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let name = match &st.edit {
        Some(e) => e.name.clone(),
        None => st.selected().map(|r| r.display_name.clone()).unwrap_or_default(),
    };
    lines.push(Line::from(vec![
        Span::styled(format!(" {} Bragi ", t.icons.brand), t.pill(t.bg, t.accent)),
        Span::raw(" "),
        Span::styled(format!(" {name} "), t.bold_accent().bg(t.raised)),
        Span::styled(format!(" {} ", t.variant.name()), t.muted().bg(t.surface)),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![Span::styled(format!("{} ", t.icons.user), t.bold_accent()), Span::styled("How do I undo a rebase?", t.text())]));
    lines.push(Line::from(vec![
        Span::styled(format!("{} ", t.icons.bot), t.fg(t.header)),
        Span::styled("Use ", t.text()),
        Span::styled("git reflog", Style::new().fg(t.fg).bg(t.code_bg)),
        Span::styled(", then ", t.text()),
        Span::styled("reset", Style::new().fg(t.link).add_modifier(Modifier::UNDERLINED)),
        Span::styled(".", t.text()),
    ]));
    lines.push(Line::styled("  secondary text · a hint", t.muted()));
    lines.push(Line::styled("  timestamps and placeholders", t.dim()));
    lines.push(Line::from(vec![Span::styled("  ", Style::new()), Span::styled(" selected row ", t.selection())]));
    lines.push(Line::raw(""));
    lines.push(Line::styled("  fn main() {", Style::new().fg(t.fg).bg(t.code_bg)));
    lines.push(Line::styled(format!("  {} let ok = true;", "+"), Style::new().fg(t.add).bg(t.add_bg)));
    lines.push(Line::styled(format!("  {} let ok = false;", "-"), Style::new().fg(t.remove).bg(t.remove_bg)));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(format!("{} ok  ", t.icons.check), t.fg(t.success)),
        Span::styled(format!("{} warn  ", t.icons.warn), t.fg(t.warning)),
        Span::styled(format!("{} error  ", t.icons.cross), t.fg(t.error)),
        Span::styled(format!("{} info", t.icons.bullet), t.fg(t.info)),
    ]));
    if editing && let Some(e) = &st.edit {
        lines.push(Line::raw(""));
        let role = e.role();
        lines.push(Line::from(vec![
            Span::styled(role.to_string(), t.bold_accent()),
            Span::styled(format!("  {}", bragi_core::appearance::role_label(role)), t.muted()),
        ]));
        for w in weirdnet_themes::contrast_warnings(e.file.variant(e.variant)).into_iter().take(2) {
            lines.push(Line::styled(clip(&format!("{} {w}", t.icons.warn), area.width as usize), t.fg(t.warning)));
        }
    }
    f.render_widget(Paragraph::new(lines), area);
}
