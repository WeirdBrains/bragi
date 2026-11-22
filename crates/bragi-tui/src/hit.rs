//! The mouse: a hit map built while drawing (rectangles on the screen, each with what a click
//! there does), so clicks are looked up and never worked out from coordinates scattered in the
//! drawing code. `ui::draw` builds a fresh map every frame and hands it to the app, which
//! dispatches mouse events against it (see `app::mouse`).
//!
//! Most clicks are a key in disguise: a button or hint whose key already does the job is a
//! [`Action::Key`], so the mouse and the keyboard can't drift apart.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::View;

/// What a click does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The same as pressing this key now.
    Key(KeyCode, KeyModifiers),
    /// Go to a screen (a tab).
    Goto(View),
    /// Open an address with the system opener.
    Url(String),
    /// Put text on the terminal's clipboard.
    Copy(String),
    /// A row of the chats sidebar; a double click opens it.
    Sidebar(usize),
    /// A row of the picker's targets or models; a double click uses it.
    PickerTarget(usize),
    PickerModel(usize),
    /// An endpoint in settings; a double click sends chats to it.
    SettingsRow(usize),
    /// A field of the endpoint form.
    FormField(usize),
    /// A proposed file in code review.
    CodeFile(usize),
    /// The message box and the chat pane: type here.
    FocusInput,
    /// Close the toast.
    DismissToast,
    /// A window's body: a click is taken and does nothing.
    Noop,
}

impl Action {
    pub fn key(c: char) -> Self {
        Action::Key(KeyCode::Char(c), KeyModifiers::NONE)
    }

    pub fn ctrl(c: char) -> Self {
        Action::Key(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    pub fn code(k: KeyCode) -> Self {
        Action::Key(k, KeyModifiers::NONE)
    }
}

/// Where the wheel scrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wheel {
    /// The transcript (or the code answer): lines up from the bottom.
    Transcript,
    Help,
    Sidebar,
    PickerTargets,
    PickerModels,
    Settings,
    /// Hunks in code review.
    Hunks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub rect: Rect,
    pub action: Action,
    /// A whole row of a list (hover tints its background) rather than a button or link
    /// (hover underlines it).
    pub row: bool,
}

/// Something clickable inside a block of lines: `line` counts from the block's first line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub line: usize,
    pub col: u16,
    pub w: u16,
    pub action: Action,
}

#[derive(Debug, Clone, Default)]
pub struct HitMap {
    hits: Vec<Hit>,
    wheels: Vec<(Rect, Wheel)>,
}

pub fn contains(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
}

impl HitMap {
    pub fn push(&mut self, rect: Rect, action: Action) {
        if rect.width > 0 && rect.height > 0 {
            self.hits.push(Hit { rect, action, row: false });
        }
    }

    pub fn row(&mut self, rect: Rect, action: Action) {
        if rect.width > 0 && rect.height > 0 {
            self.hits.push(Hit { rect, action, row: true });
        }
    }

    pub fn wheel(&mut self, rect: Rect, w: Wheel) {
        self.wheels.push((rect, w));
    }

    /// Forgets what a window now covers.
    pub fn cover(&mut self, area: Rect) {
        self.hits.retain(|h| !h.rect.intersects(area));
        self.wheels.retain(|(r, _)| !r.intersects(area));
    }

    /// The topmost hit at a cell.
    pub fn at(&self, x: u16, y: u16) -> Option<&Hit> {
        self.hits.iter().rev().find(|h| contains(h.rect, x, y))
    }

    pub fn wheel_at(&self, x: u16, y: u16) -> Option<Wheel> {
        self.wheels.iter().rev().find(|(r, _)| contains(*r, x, y)).map(|(_, w)| *w)
    }

    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// Registers marks of a block of lines drawn at `area`, of which lines `first..` are shown.
    pub fn marks(&mut self, area: Rect, first: usize, marks: &[Mark]) {
        for m in marks {
            let Some(row) = m.line.checked_sub(first) else { continue };
            if row >= area.height as usize || m.col >= area.width {
                continue;
            }
            let rect = Rect { x: area.x + m.col, y: area.y + row as u16, width: m.w.min(area.width - m.col), height: 1 };
            self.push(rect, m.action.clone());
        }
    }
}

/// A row of buttons (` label ` pills, two spaces apart): the line, and each button's column
/// and width for [`HitMap::marks`] (with `line` filled in by the caller).
pub fn button_line(buttons: &[(String, Style, Action)]) -> (Line<'static>, Vec<(u16, u16, Action)>) {
    let mut spans = Vec::new();
    let mut at = Vec::new();
    let mut col = 0u16;
    for (label, style, action) in buttons {
        let w = UnicodeWidthStr::width(label.as_str()) as u16;
        spans.push(Span::styled(label.clone(), *style));
        at.push((col, w, action.clone()));
        spans.push(Span::raw("  "));
        col += w + 2;
    }
    (Line::from(spans), at)
}

/// Marks for the spans of `line` (drawn from column `x0`) whose text, trimmed, is one of
/// `keys`: the span and the one after it (its label) become a click.
pub fn scan_pills(line_idx: usize, line: &Line, x0: u16, keys: &[(&str, Action)]) -> Vec<Mark> {
    let mut out = Vec::new();
    let mut col = x0;
    let spans = &line.spans;
    for (i, s) in spans.iter().enumerate() {
        let w = UnicodeWidthStr::width(s.content.as_ref()) as u16;
        if let Some((_, a)) = keys.iter().find(|(k, _)| s.content.trim() == *k) {
            let next = spans.get(i + 1).map(|n| UnicodeWidthStr::width(n.content.trim_end()) as u16 + 1).unwrap_or(0);
            out.push(Mark { line: line_idx, col, w: w + next, action: a.clone() });
        }
        col += w;
    }
    out
}

/// The key a hint in the status bar names (`Esc`, `Enter`, `^O`, `F1`, `y`), if it names one key.
pub fn hint_action(k: &str) -> Option<Action> {
    Some(match k {
        "Esc" => Action::code(KeyCode::Esc),
        "Enter" => Action::code(KeyCode::Enter),
        "Tab" => Action::code(KeyCode::Tab),
        "Space" => Action::key(' '),
        "PgUp" => Action::code(KeyCode::PageUp),
        "PgDn" => Action::code(KeyCode::PageDown),
        f if f.len() == 2 && f.starts_with('F') && f.as_bytes()[1].is_ascii_digit() => Action::code(KeyCode::F(f[1..].parse().ok()?)),
        c if c.len() == 2 && c.starts_with('^') => Action::ctrl(c.chars().nth(1)?.to_ascii_lowercase()),
        c if c.chars().count() == 1 && c.chars().all(|c| c.is_ascii_graphic()) => Action::key(c.chars().next()?),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect { x, y, width: w, height: h }
    }

    #[test]
    fn the_topmost_hit_wins_and_covering_forgets() {
        let mut m = HitMap::default();
        m.push(r(0, 0, 10, 3), Action::key('a'));
        m.push(r(2, 1, 4, 1), Action::key('b'));
        assert_eq!(m.at(3, 1).map(|h| &h.action), Some(&Action::key('b')));
        assert_eq!(m.at(0, 0).map(|h| &h.action), Some(&Action::key('a')));
        assert!(m.at(10, 0).is_none() && m.at(0, 3).is_none());
        m.cover(r(1, 0, 3, 3));
        assert!(m.at(0, 0).is_none());
    }

    #[test]
    fn marks_follow_the_scrolled_window() {
        let mut m = HitMap::default();
        let marks =
            [Mark { line: 5, col: 2, w: 4, action: Action::Url("https://x.test".into()) }, Mark { line: 1, col: 0, w: 1, action: Action::key('z') }];
        m.marks(r(10, 20, 30, 3), 4, &marks);
        assert_eq!(m.hits().len(), 1);
        assert_eq!(m.hits()[0].rect, r(12, 21, 4, 1));
    }

    #[test]
    fn hints_name_one_key() {
        assert_eq!(hint_action("^O"), Some(Action::ctrl('o')));
        assert_eq!(hint_action("F1"), Some(Action::code(KeyCode::F(1))));
        assert_eq!(hint_action("y"), Some(Action::key('y')));
        assert_eq!(hint_action("↑↓"), None);
        assert_eq!(hint_action("J/K"), None);
    }

    #[test]
    fn buttons_are_placed_in_a_row() {
        let (line, at) = button_line(&[(" a ".into(), Style::new(), Action::key('a')), (" bb ".into(), Style::new(), Action::key('b'))]);
        assert_eq!(line.width(), 3 + 2 + 4 + 2);
        assert_eq!(at, vec![(0, 3, Action::key('a')), (5, 4, Action::key('b'))]);
    }
}
