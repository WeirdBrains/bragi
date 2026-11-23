//! A small text box: a string and a cursor (in characters), with the usual terminal keys.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Editor {
    text: String,
    /// Cursor position in characters.
    cursor: usize,
}

fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map(|(i, _)| i).unwrap_or(s.len())
}

impl Editor {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.text.chars().count();
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    pub fn insert(&mut self, s: &str) {
        let at = byte_at(&self.text, self.cursor);
        // Pasted text: tabs as spaces, CRLF as LF, no other control characters.
        let clean: String =
            s.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    ").chars().filter(|c| *c == '\n' || !c.is_control()).collect();
        self.text.insert_str(at, &clean);
        self.cursor += clean.chars().count();
    }

    /// The cursor's (line, column) in characters.
    pub fn line_col(&self) -> (usize, usize) {
        let before: String = self.text.chars().take(self.cursor).collect();
        let line = before.matches('\n').count();
        let col = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0);
        (line, col)
    }

    /// Whether the cursor is on the first line (Up there goes to the prompt history).
    pub fn on_first_line(&self) -> bool {
        self.line_col().0 == 0
    }

    /// Whether the cursor is on the last line (Down there goes to the prompt history).
    pub fn on_last_line(&self) -> bool {
        self.line_col().0 == self.text.matches('\n').count()
    }

    /// Moves the cursor a line up or down, keeping its column where the line allows; returns
    /// whether it moved.
    pub fn line_step(&mut self, up: bool) -> bool {
        let (line, col) = self.line_col();
        let lines: Vec<&str> = self.text.split('\n').collect();
        let target = if up {
            match line.checked_sub(1) {
                Some(l) => l,
                None => return false,
            }
        } else if line + 1 < lines.len() {
            line + 1
        } else {
            return false;
        };
        let before: usize = lines[..target].iter().map(|l| l.chars().count() + 1).sum();
        self.cursor = before + col.min(lines[target].chars().count());
        true
    }

    /// Handles an editing key; returns whether it was one.
    pub fn key(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let len = self.text.chars().count();
        match k.code {
            KeyCode::Char('a') if ctrl => self.cursor = self.line_start(),
            KeyCode::Char('e') if ctrl => self.cursor = self.line_end(),
            KeyCode::Char('u') if ctrl => {
                self.text.clear();
                self.cursor = 0;
            }
            KeyCode::Char('w') if ctrl => {
                let chars: Vec<char> = self.text.chars().collect();
                let mut i = self.cursor;
                while i > 0 && chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                while i > 0 && !chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                let (a, b) = (byte_at(&self.text, i), byte_at(&self.text, self.cursor));
                self.text.replace_range(a..b, "");
                self.cursor = i;
            }
            KeyCode::Char(_) if ctrl => return false,
            KeyCode::Char(c) => self.insert(&c.to_string()),
            KeyCode::Backspace if self.cursor > 0 => {
                let (a, b) = (byte_at(&self.text, self.cursor - 1), byte_at(&self.text, self.cursor));
                self.text.replace_range(a..b, "");
                self.cursor -= 1;
            }
            KeyCode::Backspace => {}
            KeyCode::Delete if self.cursor < len => {
                let (a, b) = (byte_at(&self.text, self.cursor), byte_at(&self.text, self.cursor + 1));
                self.text.replace_range(a..b, "");
            }
            KeyCode::Delete => {}
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = self.line_start(),
            KeyCode::End => self.cursor = self.line_end(),
            _ => return false,
        }
        true
    }

    fn line_start(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        while i > 0 && chars[i - 1] != '\n' {
            i -= 1;
        }
        i
    }

    fn line_end(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        while i < chars.len() && chars[i] != '\n' {
            i += 1;
        }
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn edits_like_a_shell_line() {
        let mut e = Editor::default();
        e.insert("héllo world");
        e.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(e.text(), "héllo ");
        e.key(key(KeyCode::Backspace));
        e.key(key(KeyCode::Left));
        e.key(key(KeyCode::Char('X')));
        assert_eq!(e.text(), "héllXo");
        e.key(key(KeyCode::Home));
        e.key(key(KeyCode::Delete));
        assert_eq!(e.text(), "éllXo");
        e.insert("\r\nnext\tline");
        assert_eq!(e.text(), "\nnext    lineéllXo");
        assert_eq!(e.line_col(), (1, 12));
        assert_eq!(e.take(), "\nnext    lineéllXo");
        assert!(e.is_empty());
    }

    #[test]
    fn lines_and_steps() {
        let mut e = Editor::default();
        e.set("first line\nsecond\nthird one");
        assert!(e.on_last_line() && !e.on_first_line());
        assert!(e.line_step(true));
        assert_eq!(e.line_col(), (1, 6));
        assert!(e.line_step(true));
        assert_eq!(e.line_col(), (0, 6));
        assert!(e.on_first_line());
        assert!(!e.line_step(true));
        assert!(e.line_step(false) && e.line_step(false));
        assert!(!e.line_step(false));
        let mut one = Editor::default();
        one.set("one line");
        assert!(one.on_first_line() && one.on_last_line());
    }
}
