//! What the mouse does: clicks, double clicks, the wheel and hover, looked up in the hit map
//! the last draw built ([`crate::hit`]). A click is mostly a key pressed for you, so the mouse
//! and the keyboard share one code path.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::{App, Focus, Phase, Row, View};
use crate::hit::{Action, Wheel};

/// Two clicks on the same thing within this are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Lines the wheel scrolls the transcript per notch.
const WHEEL_LINES: usize = 3;

/// The address may be opened by the system opener: web pages and mail only.
pub fn openable(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    (u.starts_with("https://") || u.starts_with("http://") || u.starts_with("mailto:")) && !url.chars().any(char::is_control)
}

impl App {
    pub fn on_mouse(&mut self, m: MouseEvent) -> bool {
        if !self.mouse {
            return false;
        }
        match m.kind {
            MouseEventKind::Moved | MouseEventKind::Drag(_) => {
                let was = self.hits.at(self.hover.map_or(u16::MAX, |h| h.0), self.hover.map_or(u16::MAX, |h| h.1)).map(|h| h.rect);
                self.hover = Some((m.column, m.row));
                let now = self.hits.at(m.column, m.row).map(|h| h.rect);
                was != now
            }
            MouseEventKind::Down(MouseButton::Left) => self.click(m.column, m.row),
            MouseEventKind::ScrollUp => self.wheel(m.column, m.row, true),
            MouseEventKind::ScrollDown => self.wheel(m.column, m.row, false),
            _ => false,
        }
    }

    fn click(&mut self, x: u16, y: u16) -> bool {
        self.hover = Some((x, y));
        let Some(hit) = self.hits.at(x, y).cloned() else { return false };
        let double = self.last_click.take().is_some_and(|(at, a)| a == hit.action && at.elapsed() < DOUBLE_CLICK);
        if !double {
            self.last_click = Some((Instant::now(), hit.action.clone()));
        }
        self.activate(hit.action, double);
        true
    }

    /// Does what a click on `action` does.
    pub fn activate(&mut self, action: Action, double: bool) {
        // A pending "y yes" question is answered by its buttons or the keys; any other click
        // keeps things as they are, like any other key does.
        if !matches!(action, Action::Key(..) | Action::Noop) && (self.confirm_delete.is_some() || self.settings.confirm_delete.is_some()) {
            self.confirm_delete = None;
            self.settings.confirm_delete = None;
            self.say("Kept.");
        }
        match action {
            Action::Key(code, mods) => self.on_key(KeyEvent::new(code, mods)),
            Action::Goto(v) => self.goto(v),
            Action::Url(u) => {
                if openable(&u) {
                    self.say(format!("Opening {u}"));
                    self.open_urls.push(u);
                } else {
                    self.fail("That address isn't one Bragi opens (web pages and mail only).");
                }
            }
            Action::Copy(text) => {
                if text.is_empty() {
                    self.say("Nothing to copy here.");
                } else {
                    self.clipboard.push(text);
                    self.say("Copied to the terminal's clipboard (OSC 52).");
                }
            }
            Action::Sidebar(i) => self.click_sidebar(i, double),
            Action::PickerTarget(i) if i < self.targets.len() => {
                self.picker_target = i;
                self.picker_models_pane = false;
                self.model_sel = 0;
                self.load_models();
                if double {
                    self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            Action::PickerModel(i) if i < self.picker_models().len() => {
                self.picker_models_pane = true;
                self.model_sel = i;
                if double {
                    self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            Action::SettingsRow(i) if i < self.settings.endpoints.len() && self.settings.form.is_none() => {
                self.settings.sel = i;
                if double {
                    self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            Action::FormField(i) => {
                if let Some(f) = &mut self.settings.form {
                    f.field = i.min(crate::settings::FIELDS.len() - 1);
                }
            }
            Action::CodeFile(i) => {
                if i < self.code.diffs.len() {
                    self.code.file = i;
                    self.code.hunk = 0;
                }
            }
            Action::FocusInput => {
                if self.view == View::Chat {
                    self.focus = Focus::Input;
                }
            }
            Action::DismissToast => {
                if self.notice_tone != super::Tone::Ask {
                    self.notice = None;
                }
            }
            Action::PickerTarget(_) | Action::PickerModel(_) | Action::SettingsRow(_) | Action::Noop => {}
        }
    }

    /// A tab: that screen. Chat is the chat itself, not a new one (the "+ New" button is).
    fn goto(&mut self, v: View) {
        match v {
            View::Chat => {
                self.view = View::Chat;
                self.auto_link = false;
            }
            v => self.open(v),
        }
    }

    /// A chat opens on a double click, a folder folds on a single one.
    fn click_sidebar(&mut self, i: usize, double: bool) {
        if i >= self.rows.len() {
            return;
        }
        self.focus = Focus::Chats;
        self.chat_sel = i;
        match self.rows[i] {
            Row::Folder { .. } => {
                if !double {
                    self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            Row::Chat { .. } => {
                if double {
                    self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
        }
    }

    fn wheel(&mut self, x: u16, y: u16, up: bool) -> bool {
        self.hover = Some((x, y));
        let Some(w) = self.hits.wheel_at(x, y) else { return false };
        let step = |v: usize, max: usize| if up { v.saturating_sub(1) } else { (v + 1).min(max) };
        match w {
            Wheel::Transcript => {
                self.scroll = if up { self.scroll + WHEEL_LINES } else { self.scroll.saturating_sub(WHEEL_LINES) };
            }
            Wheel::Help => {
                self.help_scroll = if up { self.help_scroll.saturating_sub(WHEEL_LINES) } else { self.help_scroll + WHEEL_LINES };
            }
            Wheel::Sidebar => {
                self.focus = Focus::Chats;
                self.chat_sel = step(self.chat_sel, self.rows.len().saturating_sub(1));
            }
            Wheel::PickerTargets => {
                self.picker_models_pane = false;
                self.picker_target = step(self.picker_target, self.targets.len().saturating_sub(1));
                self.model_sel = 0;
                self.load_models();
            }
            Wheel::PickerModels => {
                let n = self.picker_models().len();
                self.picker_models_pane = n > 0;
                self.model_sel = step(self.model_sel, n.saturating_sub(1));
            }
            Wheel::Settings => self.settings.sel = step(self.settings.sel, self.settings.endpoints.len().saturating_sub(1)),
            Wheel::Hunks => {
                if self.code.phase == Phase::Review {
                    self.code.step(!up);
                }
            }
        }
        true
    }
}
