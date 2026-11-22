//! Drawing [`App`] with ratatui. Nothing here changes state except the scroll clamp.

use bragi_core::code::{HunkLine, LineTag};
use bragi_core::{MessageState, ModelStatus, PairState, Role};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Focus, Purpose, View};
use crate::code::Phase;
use crate::settings::{self, FIELDS, KEY_FIELD};
use crate::tree::Row;

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;

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
    let area = f.area();
    let input_h = match app.view {
        View::Chat | View::Link => input_height(app, area.width),
        View::Code if app.code.phase == Phase::Compose => input_height(app, area.width),
        _ => 0,
    };
    let [header, body, input, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(input_h), Constraint::Length(1)]).areas(area);
    draw_header(f, app, header);
    match app.view {
        View::Chat => draw_chat(f, app, body),
        View::Link => draw_link(f, app, body),
        View::Code => draw_code(f, app, body),
        View::Status => draw_status(f, app, body),
        View::Models => draw_models(f, app, body),
        View::Settings => draw_settings(f, app, body),
        View::Help => draw_help(f, body),
    }
    if input_h > 0 {
        draw_input(f, app, input);
    }
    draw_footer(f, app, footer);
}

fn short(id: &str) -> String {
    let s: String = id.chars().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect();
    if id.chars().count() > 8 { format!("…{s}") } else { s }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let (dot, state) = match (&app.state.pair, app.state.online) {
        (PairState::Linked { .. }, true) => (Span::styled("●", Style::new().fg(Color::Green)), "online"),
        (PairState::Linked { .. }, false) => (Span::styled("●", Style::new().fg(Color::Yellow)), "connecting"),
        _ => (Span::styled("○", Style::new().fg(DIM)), "not linked"),
    };
    let nodes = app.state.catalog.nodes.len();
    let view = match app.view {
        View::Chat => "Chat",
        View::Link => "Link",
        View::Code => "Code",
        View::Status => "Status",
        View::Models => "Models",
        View::Settings => "Settings",
        View::Help => "Help",
    };
    let line = Line::from(vec![
        Span::styled(" Bragi ", Style::new().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(view, Style::new().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        dot,
        Span::raw(format!(" {state} · {nodes} node{} · ", if nodes == 1 { "" } else { "s" })),
        Span::styled(format!("model {}", app.model_label()), Style::new().fg(ACCENT)),
        Span::raw(format!(" · via {}", app.target_label())),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let hints = match app.view {
        View::Chat if app.streaming() => "Esc stop · PgUp/PgDn scroll · F1 help",
        View::Chat if app.focus == Focus::Chats => "↑↓ move · Enter open/fold · n folder · m move · r rename · d delete · Tab back",
        View::Chat => "Enter send · Ctrl-N new · Tab sidebar · Ctrl-O target/model · Ctrl-E code · Ctrl-P settings · F1 help · Ctrl-Q quit",
        View::Link => "s show a code · c copy · paste a code or node address + Enter · Esc back",
        View::Code => match app.code.phase {
            Phase::Review => "y accept · n reject · ↑↓ hunks · a/r all · v view · w write · Esc discard",
            Phase::Asking => "Esc stop",
            Phase::Compose => "/add PATH · /rewrite FILE:A-B how · text = suggestions · /undo · Esc back · F1 help",
        },
        View::Status => "r sync now · Esc back",
        View::Models => "↑↓ move · Tab targets/models · Enter use · a automatic model · Esc back",
        View::Settings if app.settings.form.is_some() => "Tab next field · Ctrl-O pool/fallback · Enter save · Esc cancel",
        View::Settings => "a add · e edit · Space on/off · m mode · t test · J/K order · d delete · Enter chat with it · Esc back",
        View::Help => "Esc back",
    };
    let line = match &app.notice {
        Some(n) => Line::from(vec![Span::styled(n.clone(), Style::new().fg(Color::Yellow))]),
        None => Line::from(Span::styled(hints, Style::new().fg(DIM))),
    };
    f.render_widget(Paragraph::new(line), area);
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

fn input_height(app: &App, width: u16) -> u16 {
    let text = if app.view == View::Link { app.link_input.text() } else { app.input.text() };
    let lines = wrap(text, width.saturating_sub(4) as usize).len().max(1);
    (lines.min(6) + 2) as u16
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let (editor, title) = match app.view {
        View::Link => (&app.link_input, " Paste a link code or a node address, then Enter "),
        View::Code => (&app.input, " Code: /add, /rewrite FILE:A-B how, or ask for suggestions "),
        _ if !app.attachments.is_empty() => (&app.input, " Message (with attachments) "),
        _ => (&app.input, " Message "),
    };
    let focused = app.focus == Focus::Input || app.view != View::Chat;
    let block = Block::default().borders(Borders::ALL).title(title).border_style(Style::new().fg(if focused { ACCENT } else { DIM }));
    let inner = block.inner(area);
    let width = inner.width.max(1) as usize;
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
        let w = c.width().unwrap_or(0);
        if col + w > width {
            lines.push(std::mem::take(&mut cur));
            col = 0;
        }
        cur.push(c);
        col += w;
    }
    if editor.cursor() >= editor.text().chars().count() {
        if col >= width {
            lines.push(std::mem::take(&mut cur));
            col = 0;
        }
        (cx, cy) = (col, lines.len());
    }
    lines.push(cur);
    let rows = inner.height as usize;
    let first = (cy + 1).saturating_sub(rows);
    let shown: Vec<Line> = lines.iter().skip(first).take(rows).map(|l| Line::raw(l.clone())).collect();
    f.render_widget(Paragraph::new(shown).block(block), area);
    if focused && rows > 0 {
        f.set_cursor_position((inner.x + cx as u16, inner.y + (cy - first) as u16));
    }
}

/// Shows the last `area.height` of `lines`, `scroll` lines up from the bottom (clamped).
fn bottom(f: &mut Frame, lines: Vec<Line<'static>>, area: Rect, scroll: &mut usize) {
    let h = area.height as usize;
    let max = lines.len().saturating_sub(h);
    *scroll = (*scroll).min(max);
    let start = lines.len().saturating_sub(h + *scroll);
    let shown: Vec<Line> = lines.into_iter().skip(start).take(h).collect();
    f.render_widget(Paragraph::new(shown), area);
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Chats;
    let block = Block::default().borders(Borders::RIGHT).title(" Sessions ").border_style(Style::new().fg(if focused { ACCENT } else { DIM }));
    let current = app.current.as_ref().map(|c| c.id.clone());
    let width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|r| {
            let pad = "  ".repeat(r.depth());
            match r {
                Row::Folder { name, open, chats, .. } => ListItem::new(Line::from(vec![
                    Span::raw(pad),
                    Span::styled(if *open { "▾ " } else { "▸ " }, Style::new().fg(ACCENT)),
                    Span::styled(name.clone(), Style::new().add_modifier(Modifier::BOLD)),
                    Span::styled(format!(" {chats}"), Style::new().fg(DIM)),
                ])),
                Row::Chat { session, .. } => {
                    let mark = if session.unbound { "! " } else { "  " };
                    let title = clip(&session.title, width.saturating_sub(pad.len() + 2));
                    let mut lines = vec![Line::from(vec![Span::raw(pad.clone()), Span::styled(mark, Style::new().fg(Color::Red)), Span::raw(title)])];
                    if session.target.as_ref().is_some_and(|t| *t != bragi_core::Target::Pool) || session.unbound {
                        lines.push(Line::styled(
                            format!("{pad}  {}", clip(&session.target_label, width.saturating_sub(pad.len() + 2))),
                            Style::new().fg(DIM),
                        ));
                    }
                    ListItem::new(lines)
                }
            }
        })
        .collect();
    let sel = if focused { Some(app.chat_sel) } else { app.rows.iter().position(|r| r.chat_id().is_some() && r.chat_id() == current.as_deref()) };
    let mut state = ListState::default().with_selected(sel);
    let hl = if focused { Style::new().bg(ACCENT).fg(Color::Black) } else { Style::new().add_modifier(Modifier::BOLD) };
    f.render_stateful_widget(List::new(items).block(block).highlight_style(hl), area, &mut state);
}

/// The Thinking section: a title line (`Thinking… 4s` / `Thought for 4s`, the words every shell
/// uses) and, when shown, the reasoning drawn dim with the Markdown renderer.
fn thinking(lines: &mut Vec<Line<'static>>, reasoning: &str, ms: Option<u64>, done: bool, open: bool, width: usize) {
    if reasoning.trim().is_empty() {
        return;
    }
    let title = bragi_core::think::label(ms.unwrap_or(0), done);
    let fold = if open { "▾" } else { "▸" };
    lines.push(Line::styled(format!("{fold} {title} (Ctrl-K)"), Style::new().fg(Color::Magenta)));
    if open {
        for l in crate::markdown::render(reasoning, width.saturating_sub(2)) {
            let mut spans = vec![Span::styled("┊ ", Style::new().fg(DIM))];
            spans.extend(l.spans.into_iter().map(|s| Span::styled(s.content, s.style.fg(DIM))));
            lines.push(Line::from(spans));
        }
    }
}

fn draw_chat(f: &mut Frame, app: &mut App, area: Rect) {
    let (list, main) = if area.width >= 80 {
        let [a, b] = Layout::horizontal([Constraint::Length(32), Constraint::Min(20)]).areas(area);
        (Some(a), b)
    } else {
        (None, area)
    };
    if let Some(list) = list {
        draw_sidebar(f, app, list);
    }
    let inner = Rect { x: main.x + 1, width: main.width.saturating_sub(2), ..main };
    let w = inner.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let streaming_here =
        app.stream.as_ref().filter(|s| s.purpose == Purpose::Chat && app.current.as_ref().is_some_and(|c| c.id == s.conversation_id));
    match &app.current {
        None => {
            if !app.linked() && app.settings.endpoints.is_empty() {
                lines.push(Line::styled(
                    "This terminal isn't linked yet: press Ctrl-L (or add an endpoint with Ctrl-P).",
                    Style::new().fg(Color::Yellow),
                ));
            } else {
                lines.push(Line::styled(
                    format!("New chat with {} · {}. Type below and press Enter.", app.target_label(), app.model_label()),
                    Style::new().fg(DIM),
                ));
                lines.push(Line::styled("/attach PATH sends a file with the next message. F1 lists every key.", Style::new().fg(DIM)));
            }
        }
        Some(c) => {
            let n = c.messages.len();
            for (i, m) in c.messages.iter().enumerate() {
                let live = streaming_here.filter(|_| i + 1 == n && m.role == Role::Assistant);
                let (who, style) = match m.role {
                    Role::User => ("You".to_string(), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
                    _ => {
                        let model = live.and_then(|s| s.model.clone()).or_else(|| m.model.clone()).unwrap_or_default();
                        let node =
                            live.and_then(|s| s.node.clone()).or_else(|| m.node.clone()).map(|n| format!(" on {}", short(&n))).unwrap_or_default();
                        let flags = match (&m.standin, m.cache_hit) {
                            (Some(s), _) => format!(" · stand-in {s}"),
                            (None, true) => " · cached".into(),
                            _ => String::new(),
                        };
                        (format!("Bragi · {model}{node}{flags}"), Style::new().fg(Color::Green).add_modifier(Modifier::BOLD))
                    }
                };
                lines.push(Line::styled(who, style));
                for a in &m.attachments {
                    lines.push(Line::styled(format!("attached {}", a.label()), Style::new().fg(DIM)));
                }
                let text = live.map(|s| s.text.clone()).unwrap_or_else(|| m.text.clone());
                match live {
                    Some(s) => {
                        let ms = s.thought_ms.or_else(|| s.thinking_since.map(|t| t.elapsed().as_millis() as u64));
                        thinking(&mut lines, &s.reasoning, ms, s.thought_ms.is_some() || !text.is_empty(), app.show_thinking, w);
                        if let Some(p) = s.queued {
                            lines.push(Line::styled(format!("Waiting for a free node (position {p})…"), Style::new().fg(DIM)));
                        }
                    }
                    None => thinking(&mut lines, &m.reasoning, m.thinking_ms, true, app.show_thinking, w),
                }
                if m.role == Role::User {
                    for l in wrap(&text, w) {
                        lines.push(Line::raw(l));
                    }
                } else if live.is_some() {
                    lines.extend(crate::markdown::render_streaming(&text, w));
                } else {
                    lines.extend(crate::markdown::render(&text, w));
                }
                if live.is_some() {
                    lines.push(Line::styled("▍", Style::new().fg(ACCENT)));
                } else {
                    match m.state {
                        MessageState::Stopped => lines.push(Line::styled("(stopped — Ctrl-R continues)", Style::new().fg(DIM))),
                        MessageState::Failed => lines.push(Line::styled("(failed — Ctrl-R tries to continue)", Style::new().fg(Color::Red))),
                        _ => {}
                    }
                }
                lines.push(Line::raw(""));
            }
        }
    }
    bottom(f, lines, inner, &mut app.scroll);
}

fn qr_lines(text: &str) -> Option<Vec<String>> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let s = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .dark_color(qrcode::render::unicode::Dense1x2::Light)
        .light_color(qrcode::render::unicode::Dense1x2::Dark)
        .build();
    Some(s.lines().map(str::to_string).collect())
}

fn draw_link(f: &mut Frame, app: &mut App, area: Rect) {
    let inner = Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area };
    let w = inner.width as usize;
    let mut lines: Vec<Line<'static>> =
        vec![Line::styled("Link this terminal to your Weirdnet", Style::new().add_modifier(Modifier::BOLD)), Line::raw("")];
    let push = |lines: &mut Vec<Line<'static>>, text: &str, style: Style| {
        for l in wrap(text, w) {
            lines.push(Line::styled(l, style));
        }
    };
    if let Some(check) = &app.link_check {
        push(&mut lines, &format!("Does {} show this code?", check.device), Style::new());
        lines.push(Line::styled(format!("    {}", check.sas), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)));
        push(&mut lines, "y yes, link · n no, cancel", Style::new().fg(DIM));
    } else {
        match &app.state.pair {
            PairState::Unpaired => {
                push(&mut lines, "Two ways:", Style::new());
                push(
                    &mut lines,
                    "• Press s to show a link code here, then open it on a device that is already linked (on a Ymir node: ymirctl pool add CODE; in Bragi: Link a device).",
                    Style::new(),
                );
                push(&mut lines, "• Or paste a code shown by another device below and press Enter.", Style::new());
                push(
                    &mut lines,
                    "A node address (/ip4/…/p2p/…, from ymirctl pool addresses) pasted below is dialed from now on, for nodes Bragi can't find by itself.",
                    Style::new().fg(DIM),
                );
            }
            PairState::Offering { link, expires_ms } => {
                let left = (*expires_ms - bragi_core::history::now_ms()).max(0) / 1000;
                push(
                    &mut lines,
                    &format!(
                        "Open this code on a device that's already linked (it expires in {}:{:02}). c copies it to your clipboard, even over SSH.",
                        left / 60,
                        left % 60
                    ),
                    Style::new(),
                );
                lines.push(Line::raw(""));
                push(&mut lines, link, Style::new().fg(ACCENT));
                lines.push(Line::raw(""));
                if let Some(qr) = qr_lines(link) {
                    let fits = qr.len() + lines.len() <= inner.height as usize && qr.first().map(|l| l.chars().count()).unwrap_or(0) <= w;
                    if fits {
                        lines.extend(qr.into_iter().map(Line::raw));
                    } else {
                        push(&mut lines, "(Make the terminal bigger to see the QR code.)", Style::new().fg(DIM));
                    }
                }
            }
            PairState::Confirm { sas } => {
                push(&mut lines, "Does the other device show this code?", Style::new());
                lines.push(Line::styled(format!("    {sas}"), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)));
                push(&mut lines, "y yes, link · n no, cancel", Style::new().fg(DIM));
            }
            PairState::Waiting { sas } => push(&mut lines, &format!("Confirmed {sas} here; waiting for the other device…"), Style::new()),
            PairState::Linked { user, device } => {
                push(
                    &mut lines,
                    &format!("Linked: this terminal is device {} of user {}.", short(device), short(user)),
                    Style::new().fg(Color::Green),
                );
                lines.push(Line::raw(""));
                push(&mut lines, "Paste a node address below to dial that node from now on, or a code from another device to open it.", Style::new());
                let peers = app.core.peers();
                if !peers.is_empty() {
                    lines.push(Line::raw(""));
                    lines.push(Line::styled("Node addresses dialed:", Style::new().fg(DIM)));
                    for p in peers {
                        push(&mut lines, &format!("  {p}"), Style::new().fg(DIM));
                    }
                }
            }
            PairState::Failed { message } => {
                push(&mut lines, message, Style::new().fg(Color::Red));
                push(&mut lines, "Press s for a new code, or paste one below.", Style::new());
            }
        }
    }
    // Top-anchored: the code stays where it is while the timer runs.
    lines.truncate(inner.height as usize);
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_status(f: &mut Frame, app: &mut App, area: Rect) {
    let inner = Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area };
    let s = &app.state;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let kv = |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<14}"), Style::new().fg(DIM)), Span::raw(v)]);
    lines.push(Line::styled("This device", Style::new().add_modifier(Modifier::BOLD)));
    let pair = match &s.pair {
        PairState::Linked { .. } => "linked".to_string(),
        PairState::Unpaired => "not linked (Ctrl-L)".into(),
        PairState::Failed { message } => format!("failed: {message}"),
        _ => "linking…".into(),
    };
    lines.push(kv("Link", pair));
    lines.push(kv("Online", if s.online { "yes".into() } else { "no".into() }));
    lines.push(kv("Peer id", app.core.peer_id().unwrap_or_else(|| "—".into())));
    lines.push(kv("Address", app.core.address().unwrap_or_else(|| "—".into())));
    lines.push(kv("Sends via", app.target_label()));
    lines.push(Line::raw(""));
    lines.push(Line::styled(format!("Inference nodes ({})", s.catalog.nodes.len()), Style::new().add_modifier(Modifier::BOLD)));
    if s.catalog.nodes.is_empty() {
        for l in wrap(&app.core.no_node_message(), inner.width as usize) {
            lines.push(Line::styled(l, Style::new().fg(Color::Yellow)));
        }
    }
    for n in &s.catalog.nodes {
        lines.push(Line::raw(format!("  {}  {} chat model{}, {} warm", n.id, n.models, if n.models == 1 { "" } else { "s" }, n.warm)));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(format!("Models ({})", s.catalog.models.len()), Style::new().add_modifier(Modifier::BOLD)));
    for m in &s.catalog.models {
        lines.push(model_line(m, app.model.as_deref() == Some(m.id.as_str())));
    }
    let peers = app.core.peers();
    if !peers.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled("Node addresses dialed", Style::new().add_modifier(Modifier::BOLD)));
        for p in peers {
            lines.push(Line::styled(format!("  {p}"), Style::new().fg(DIM)));
        }
    }
    lines.truncate(inner.height as usize);
    f.render_widget(Paragraph::new(lines), inner);
}

fn model_line(m: &bragi_core::ModelInfo, picked: bool) -> Line<'static> {
    let (dot, color, label) = match m.status {
        ModelStatus::Warm => ("●", Color::Green, "warm"),
        ModelStatus::Busy => ("●", Color::Yellow, "busy"),
        ModelStatus::Loading => ("◐", Color::Yellow, "loading"),
        ModelStatus::Loadable => ("○", DIM, "loads on first use"),
    };
    let speed = m.tokens_per_sec.map(|t| format!(" · {t:.0} tok/s")).unwrap_or_default();
    let ctx = m.context.map(|c| format!(" · {}k context", c / 1024)).unwrap_or_default();
    let on = format!(" · on {} node{}", m.nodes.len(), if m.nodes.len() == 1 { "" } else { "s" });
    Line::from(vec![
        Span::styled(format!("  {dot} "), Style::new().fg(color)),
        Span::styled(m.id.clone(), if picked { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::new() }),
        Span::styled(format!("  {label}{speed}{ctx}{on}"), Style::new().fg(DIM)),
    ])
}

fn draw_models(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Where chats go (Enter) · Tab: its models · a automatic model ");
    let current = app.effective_target();
    let mut items: Vec<ListItem> = vec![];
    for (i, t) in app.targets.iter().enumerate() {
        let on = Some(&t.target) == current.as_ref() || (current.is_none() && i == 0);
        let mark = if on { "● " } else { "  " };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(mark, Style::new().fg(ACCENT)),
            Span::styled(t.label.clone(), if on { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::new() }),
        ])));
    }
    let pool = !matches!(app.targets.get(app.picker_target).map(|t| &t.target), Some(bragi_core::Target::Endpoint { .. }));
    if pool {
        for m in &app.state.catalog.models {
            items.push(ListItem::new(model_line(m, app.model.as_deref() == Some(m.id.as_str()))));
        }
    } else {
        let picked = match &current {
            Some(bragi_core::Target::Endpoint { model, .. }) => Some(model.clone()),
            _ => None,
        };
        for m in &app.endpoint_models {
            let st = if picked.as_deref() == Some(m.as_str()) { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::new() };
            items.push(ListItem::new(Line::from(vec![Span::styled("  ○ ", Style::new().fg(DIM)), Span::styled(m.clone(), st)])));
        }
    }
    let [top, rest] = Layout::vertical([Constraint::Length((app.targets.len() + 2) as u16), Constraint::Min(3)]).areas(area);
    let n_targets = app.targets.len();
    let (t_items, m_items): (Vec<_>, Vec<_>) = items.into_iter().enumerate().partition(|(i, _)| *i < n_targets);
    let t_items: Vec<ListItem> = t_items.into_iter().map(|(_, x)| x).collect();
    let m_items: Vec<ListItem> = m_items.into_iter().map(|(_, x)| x).collect();
    let (on, off) = (Style::new().bg(ACCENT).fg(Color::Black), Style::new().add_modifier(Modifier::REVERSED));
    let (thl, mhl) = if app.picker_models_pane { (off, on) } else { (on, off) };
    let mut ts = ListState::default().with_selected(Some(app.picker_target));
    f.render_stateful_widget(List::new(t_items).block(block).highlight_style(thl), top, &mut ts);
    let title = if pool { " Models on your nodes " } else { " Models on this endpoint " };
    let mblock = Block::default().borders(Borders::ALL).title(title);
    if m_items.is_empty() {
        let msg = match (pool, app.linked()) {
            (true, true) => app.core.no_node_message(),
            (true, false) => "Link this terminal first (Ctrl-L), or add an endpoint (Ctrl-P).".into(),
            (false, _) => "Asking the endpoint for its models…".into(),
        };
        f.render_widget(
            Paragraph::new(wrap(&msg, rest.width.saturating_sub(4) as usize).into_iter().map(Line::raw).collect::<Vec<_>>()).block(mblock),
            rest,
        );
        return;
    }
    let mut ms = ListState::default().with_selected(app.picker_models_pane.then_some(app.model_sel));
    f.render_stateful_widget(List::new(m_items).block(mblock).highlight_style(mhl).highlight_symbol("› "), rest, &mut ms);
}

fn draw_settings(f: &mut Frame, app: &mut App, area: Rect) {
    let st = &app.settings;
    let block = Block::default().borders(Borders::ALL).title(" OpenAI-compatible endpoints ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines: Vec<Line<'static>> = Vec::new();
    if st.endpoints.is_empty() {
        lines.push(Line::styled(
            "No endpoints. a adds one: any server with an OpenAI-compatible /v1 API (llama.cpp, vLLM, Ollama, a hosted one).",
            Style::new().fg(DIM),
        ));
    }
    for (i, e) in st.endpoints.iter().enumerate() {
        let sel = i == st.sel && st.form.is_none();
        let base = if sel { Style::new().bg(ACCENT).fg(Color::Black) } else { Style::new() };
        let dot =
            if e.enabled { Span::styled("● ", base.fg(if sel { Color::Black } else { Color::Green })) } else { Span::styled("○ ", base.fg(DIM)) };
        let mode = match e.mode {
            bragi_core::EndpointMode::Alongside => "next to the pool",
            bragi_core::EndpointMode::Fallback => "fallback",
        };
        let model = e.model.clone().map(|m| format!(" · {m}")).unwrap_or_default();
        let key = if e.has_key { " · key set" } else { "" };
        lines.push(Line::from(vec![
            dot,
            Span::styled(format!("{}  ", e.name), base.add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}{model} · {mode}{key}", e.base_url), base),
        ]));
        if e.insecure {
            lines.push(Line::styled("    plain http to another machine: the key and chats can be read on the way", Style::new().fg(Color::Yellow)));
        }
        if let Some(t) = st.tests.get(&e.id) {
            lines.push(Line::styled(format!("    test: {t}"), Style::new().fg(DIM)));
        }
    }
    if let Some(form) = &st.form {
        lines.push(Line::raw(""));
        let title = if form.editing.is_some() { "Edit endpoint" } else { "Add an endpoint" };
        lines.push(Line::styled(title, Style::new().add_modifier(Modifier::BOLD)));
        let mut cursor = None;
        for (i, name) in FIELDS.iter().enumerate() {
            let raw = form.fields[i].text();
            let shown = if i == KEY_FIELD { settings::mask(raw) } else { raw.to_string() };
            let hint = if i == KEY_FIELD && form.editing.is_some() && !form.key_touched { "(unchanged)" } else { "" };
            let active = i == form.field;
            let label = Span::styled(format!("  {name:<20}"), Style::new().fg(if active { ACCENT } else { DIM }));
            if active {
                let col = if i == KEY_FIELD {
                    form.fields[i].cursor()
                } else {
                    UnicodeWidthStr::width(&raw[..raw.char_indices().nth(form.fields[i].cursor()).map_or(raw.len(), |(b, _)| b)])
                };
                cursor = Some((inner.x + 22 + col as u16, inner.y + lines.len() as u16));
            }
            lines.push(Line::from(vec![label, Span::raw(shown), Span::styled(hint, Style::new().fg(DIM))]));
        }
        let mode = if form.alongside {
            "offered next to the Weirdnet pool (a chat can be sent to it)"
        } else {
            "only when the pool can't answer (fallback)"
        };
        lines.push(Line::from(vec![Span::styled(format!("  {:<20}", "Use (Ctrl-O)"), Style::new().fg(DIM)), Span::raw(mode)]));
        if let Some(c) = cursor
            && c.1 < inner.y + inner.height
        {
            f.set_cursor_position(c);
        }
    }
    if !st.links.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled("Weirdnet node addresses (add one with Ctrl-L)", Style::new().add_modifier(Modifier::BOLD)));
        for l in &st.links {
            let (dot, c) = if l.online { ("●", Color::Green) } else { ("○", DIM) };
            lines
                .push(Line::from(vec![Span::styled(format!("  {dot} "), Style::new().fg(c)), Span::styled(l.address.clone(), Style::new().fg(DIM))]));
        }
    }
    lines.truncate(inner.height as usize);
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_help(f: &mut Frame, area: Rect) {
    f.render_widget(Clear, area);
    let mut lines = vec![Line::styled("Keyboard", Style::new().add_modifier(Modifier::BOLD))];
    let row = |k: &str, v: &str| Line::from(vec![Span::styled(format!("  {k:<26}"), Style::new().fg(ACCENT)), Span::raw(v.to_string())]);
    lines.extend(HELP.iter().map(|(k, v)| row(k, v)));
    lines.push(Line::raw(""));
    lines.push(Line::styled("Code mode (Ctrl-E)", Style::new().add_modifier(Modifier::BOLD)));
    lines.extend(CODE_HELP.iter().map(|(k, v)| row(k, v)));
    let mut right = vec![Line::styled("Sidebar and chat box", Style::new().add_modifier(Modifier::BOLD))];
    right.extend(CHAT_HELP.iter().map(|(k, v)| row(k, v)));
    let area = Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area };
    if area.width >= 150 {
        let [l, r] = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);
        f.render_widget(Paragraph::new(lines), l);
        f.render_widget(Paragraph::new(right), r);
    } else {
        lines.push(Line::raw(""));
        lines.extend(right);
        f.render_widget(Paragraph::new(lines), area);
    }
}

fn hunk_lines(lines: &[HunkLine], width: usize) -> Vec<Line<'static>> {
    lines
        .iter()
        .map(|l| {
            let (sign, style) = match l.tag {
                LineTag::Context => (' ', Style::new()),
                LineTag::Removed => ('-', Style::new().fg(Color::Red)),
                LineTag::Added => ('+', Style::new().fg(Color::Green)),
            };
            let no = match l.tag {
                LineTag::Added => l.new_no,
                _ => l.old_no,
            };
            let no = no.map(|n| format!("{n:>5} ")).unwrap_or_else(|| "      ".into());
            let text: String = clip(&l.text, width.saturating_sub(8));
            Line::from(vec![Span::styled(no, Style::new().fg(DIM)), Span::styled(format!("{sign} {text}"), style)])
        })
        .collect()
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
    let (side, main) = if area.width >= 90 {
        let [a, b] = Layout::horizontal([Constraint::Length(34), Constraint::Min(30)]).areas(area);
        (Some(a), b)
    } else {
        (None, area)
    };
    if let Some(side) = side {
        let mut lines = vec![
            Line::styled("Folder", Style::new().fg(DIM)),
            Line::raw(clip(&app.code.root.display().to_string(), side.width.saturating_sub(2) as usize)),
            Line::raw(""),
            Line::styled(
                format!("Context ({} files, {} KiB)", app.code.gathered.files.len(), app.code.gathered.total_bytes().div_ceil(1024)),
                Style::new().fg(DIM),
            ),
        ];
        for (i, p) in app.code.context.iter().enumerate() {
            lines.push(Line::raw(clip(&format!("{}. {}", i + 1, p.display()), side.width.saturating_sub(2) as usize)));
        }
        if app.code.context.is_empty() {
            lines.push(Line::styled("none yet: /add PATH", Style::new().fg(DIM)));
        }
        for file in app.code.gathered.files.iter().take(side.height.saturating_sub(lines.len() as u16 + 3) as usize) {
            lines.push(Line::styled(
                clip(&format!("  {} ({} lines)", file.path, file.text.lines().count()), side.width.saturating_sub(2) as usize),
                Style::new().fg(DIM),
            ));
        }
        if !app.code.gathered.skipped.is_empty() {
            lines.push(Line::styled(format!("{} left out", app.code.gathered.skipped.len()), Style::new().fg(DIM)));
        }
        if !app.code.diffs.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::styled("Proposed", Style::new().fg(DIM)));
            for (i, d) in app.code.diffs.iter().enumerate() {
                let (a, r) = d.stats();
                let mark = if i == app.code.file { "›" } else { " " };
                lines.push(Line::raw(clip(&format!("{mark} {} +{a} -{r}", d.path), side.width.saturating_sub(2) as usize)));
            }
        }
        f.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::RIGHT).border_style(Style::new().fg(DIM))), side);
    }
    let inner = Rect { x: main.x + 1, width: main.width.saturating_sub(2), ..main };
    let w = inner.width as usize;
    match app.code.phase {
        Phase::Compose => {
            let mut lines: Vec<Line<'static>> =
                vec![Line::styled("Code suggestions and rewrites", Style::new().add_modifier(Modifier::BOLD)), Line::raw("")];
            if !app.code.explanation.is_empty() {
                lines.push(Line::styled("Last answer:", Style::new().fg(DIM)));
                lines.extend(wrap(&app.code.explanation, w).into_iter().map(Line::raw));
                lines.push(Line::raw(""));
            }
            for (k, v) in CODE_HELP.iter().take(6) {
                lines.push(Line::from(vec![
                    Span::styled(format!("{k:<26}"), Style::new().fg(ACCENT)),
                    Span::styled(v.to_string(), Style::new().fg(DIM)),
                ]));
            }
            f.render_widget(Paragraph::new(lines), inner);
        }
        Phase::Asking => {
            let title = app.code.request.as_ref().map(bragi_core::code::prompt::title).unwrap_or_default();
            let mut lines: Vec<Line<'static>> = vec![Line::styled(title, Style::new().add_modifier(Modifier::BOLD))];
            if let Some(s) = &app.stream {
                if let Some(p) = s.queued {
                    lines.push(Line::styled(format!("Waiting for a free node (position {p})…"), Style::new().fg(DIM)));
                }
                lines.extend(wrap(&s.text, w).into_iter().map(Line::raw));
                lines.push(Line::styled("▍", Style::new().fg(ACCENT)));
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
                Span::styled(format!("{}{} ", d.path, if d.created { " (new)" } else { "" }), Style::new().add_modifier(Modifier::BOLD)),
                Span::styled(format!("+{a} -{r}  "), Style::new().fg(DIM)),
                Span::raw(format!("change {index}/{total}  ")),
                if ok {
                    Span::styled("✓ accepted", Style::new().fg(Color::Green))
                } else {
                    Span::styled("✗ rejected", Style::new().fg(Color::Red))
                },
                Span::styled(format!("  ({acc}/{all} accepted)"), Style::new().fg(DIM)),
            ])];
            if !app.code.explanation.is_empty() {
                for l in wrap(&app.code.explanation, w).into_iter().take(4) {
                    lines.push(Line::styled(l, Style::new().fg(DIM)));
                }
            }
            let h = &d.hunks[hi];
            lines.push(Line::styled(h.header(), Style::new().fg(Color::Magenta)));
            if app.code.side_by_side && w >= 40 {
                let half = (w - 1) / 2;
                for (left, right) in h.side_by_side() {
                    let cell = |c: &Option<(usize, String)>, color: Color, changed: bool| match c {
                        Some((n, t)) => Span::styled(
                            format!("{:<half$}", clip(&format!("{n:>5} {t}"), half)),
                            if changed { Style::new().fg(color) } else { Style::new() },
                        ),
                        None => Span::raw(" ".repeat(half)),
                    };
                    let changed = left.as_ref().map(|l| &l.1) != right.as_ref().map(|r| &r.1);
                    lines.push(Line::from(vec![
                        cell(&left, Color::Red, changed),
                        Span::styled("│", Style::new().fg(DIM)),
                        cell(&right, Color::Green, changed),
                    ]));
                }
            } else {
                lines.extend(hunk_lines(&h.lines, w));
            }
            f.render_widget(Paragraph::new(lines.into_iter().take(inner.height as usize).collect::<Vec<_>>()), inner);
        }
    }
}
