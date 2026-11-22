//! The Theme screen (F7, `^G`, the "Theme" tab): a picker with a live preview, and an editor for
//! a theme's roles. State and keys live here; the drawing is `ui::draw_themes`. Themes are
//! `bragi_core::Appearance`'s (weirdnet-themes' format), so what is made here shows up in the
//! desktop and phone apps on the same device data, and the choice is saved with Bragi's settings.

use std::path::PathBuf;

use bragi_core::{Appearance, ModePref, Preference, ThemeRow};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use weirdnet_themes::{ThemeFile, Variant};

use super::App;
use crate::editor::Editor;
use crate::theme::{self, Theme};

/// What a name or path prompt is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// A new theme from the one under the cursor, then edited.
    New,
    Duplicate,
    Rename,
    Import,
    Export,
}

impl Ask {
    pub fn label(self) -> &'static str {
        match self {
            Ask::New => "New theme named",
            Ask::Duplicate => "Duplicate as",
            Ask::Rename => "Rename to",
            Ask::Import => "Import a theme file (path)",
            Ask::Export => "Export to (path)",
        }
    }
}

/// A y/n question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm {
    Delete(String),
    Reset(String),
    Discard,
}

/// A theme being edited: the pending file, the one on disk, the variant shown and the role
/// under the cursor.
pub struct EditState {
    pub name: String,
    pub file: ThemeFile,
    pub orig: ThemeFile,
    pub variant: Variant,
    pub sel: usize,
    /// The hex box of the role under the cursor, while it is being typed.
    pub hex: Option<Editor>,
    /// What the role was before the box opened.
    pub before: Option<Option<String>>,
}

impl EditState {
    pub fn role(&self) -> &'static str {
        ROLES()[self.sel.min(ROLES().len() - 1)]
    }

    pub fn dirty(&self) -> bool {
        self.file != self.orig
    }

    /// The pending value of a role as stored (a `#rrggbb` or a palette name), if set.
    pub fn stored(&self, role: &str) -> Option<&String> {
        self.file.variant(self.variant).roles.get(role)
    }
}

#[allow(non_snake_case)]
fn ROLES() -> &'static [&'static str] {
    static ALL: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    ALL.get_or_init(bragi_core::appearance::roles)
}

pub struct ThemeScreen {
    pub rows: Vec<ThemeRow>,
    pub sel: usize,
    /// What is chosen and saved, and the look that was on when the screen opened (restored by Esc).
    pub saved: Preference,
    pub opened_with: Theme,
    /// The mode being previewed.
    pub mode: ModePref,
    pub edit: Option<EditState>,
    pub prompt: Option<(Ask, Editor)>,
    pub confirm: Option<Confirm>,
    /// What the last action said (an error shows in the window, not only as a toast).
    pub msg: Option<String>,
}

impl Default for ThemeScreen {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            sel: 0,
            saved: Preference::default(),
            opened_with: Theme::bragi_dark(),
            mode: ModePref::default(),
            edit: None,
            prompt: None,
            confirm: None,
            msg: None,
        }
    }
}

impl ThemeScreen {
    pub fn selected(&self) -> Option<&ThemeRow> {
        self.rows.get(self.sel)
    }
}

/// `#abc`/`#aabbcc` (any case) as stored form, or `None`.
pub fn parse_hex(s: &str) -> Option<String> {
    let s = s.trim();
    let s = if s.starts_with('#') { s.to_string() } else { format!("#{s}") };
    let h = &s[1..];
    let full = match h.len() {
        3 if h.chars().all(|c| c.is_ascii_hexdigit()) => h.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 if h.chars().all(|c| c.is_ascii_hexdigit()) => h.to_string(),
        _ => return None,
    };
    Some(format!("#{}", full.to_ascii_lowercase()))
}

impl App {
    pub(super) fn open_themes(&mut self) {
        let a = self.core.appearance();
        self.themes.saved = a.preference();
        self.themes.mode = self.themes.saved.mode;
        self.themes.opened_with = theme::get().clone();
        self.themes.edit = None;
        self.themes.prompt = None;
        self.themes.confirm = None;
        self.themes.msg = None;
        self.reload_themes();
        self.themes.sel = self.themes.rows.iter().position(|r| r.name == self.themes.saved.theme).unwrap_or(0);
        self.preview();
    }

    fn appearance(&self) -> Appearance {
        self.core.appearance()
    }

    fn reload_themes(&mut self) {
        self.themes.rows = self.appearance().rows().unwrap_or_default();
        self.themes.sel = self.themes.sel.min(self.themes.rows.len().saturating_sub(1));
    }

    /// Paints the whole app with what the screen is showing: the pending edit, or the theme
    /// under the cursor.
    pub(super) fn preview(&mut self) {
        let (file, mode) = match &self.themes.edit {
            Some(e) => (Some(e.file.clone()), if e.variant == Variant::Dark { ModePref::Dark } else { ModePref::Light }),
            None => (self.themes.selected().and_then(|r| self.appearance().load(&r.name).ok()), self.themes.mode),
        };
        if let Some(f) = file {
            theme::init(theme::build(&f, mode));
        }
    }

    /// Leaves the screen putting back the look it opened with.
    fn leave_themes(&mut self) {
        theme::init(self.themes.opened_with.clone());
        self.themes.edit = None;
        self.view = self.back;
    }

    pub(super) fn themes_paste(&mut self, text: &str) {
        if let Some((_, ed)) = &mut self.themes.prompt {
            ed.insert(text.trim());
        } else if let Some(e) = &mut self.themes.edit
            && let Some(h) = &mut e.hex
        {
            h.insert(text.trim());
            self.hex_changed();
        }
    }

    /// Saves the chosen theme and mode.
    fn apply_theme(&mut self) {
        let Some(row) = self.themes.selected().cloned() else { return };
        let a = self.appearance();
        let p = Preference { theme: row.name.clone(), mode: self.themes.mode };
        match a.set_preference(&p) {
            Ok(()) => {
                self.themes.saved = p;
                self.themes.opened_with = theme::get().clone();
                self.leave_themes();
                self.say(format!("Theme: {} ({}).", row.display_name, self.themes.mode.name()));
            }
            Err(e) => self.themes.msg = Some(e.message),
        }
    }

    pub(super) fn themes_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if self.themes.prompt.is_some() {
            return self.prompt_key(k);
        }
        if let Some(c) = self.themes.confirm.take() {
            return self.confirm_key(c, k);
        }
        if self.themes.edit.is_some() {
            return self.edit_key(k, ctrl);
        }
        self.themes.msg = None;
        let n = self.themes.rows.len();
        let sel = self.themes.selected().cloned();
        match (k.code, sel) {
            (KeyCode::Esc | KeyCode::Char('q'), _) => self.leave_themes(),
            (KeyCode::Up | KeyCode::Char('k'), _) => {
                self.themes.sel = self.themes.sel.saturating_sub(1);
                self.preview();
            }
            (KeyCode::Down | KeyCode::Char('j'), _) => {
                self.themes.sel = (self.themes.sel + 1).min(n.saturating_sub(1));
                self.preview();
            }
            (KeyCode::Home, _) => {
                self.themes.sel = 0;
                self.preview();
            }
            (KeyCode::End, _) => {
                self.themes.sel = n.saturating_sub(1);
                self.preview();
            }
            (KeyCode::Char('m') | KeyCode::Tab, _) => {
                self.themes.mode = match self.themes.mode {
                    ModePref::Dark => ModePref::Light,
                    ModePref::Light => ModePref::System,
                    ModePref::System => ModePref::Dark,
                };
                self.preview();
            }
            (KeyCode::Enter, Some(r)) if r.error.is_none() => self.apply_theme(),
            (KeyCode::Enter, Some(r)) => self.themes.msg = r.error,
            (KeyCode::Char('e'), Some(r)) => self.start_edit(&r.name),
            (KeyCode::Char('n'), Some(r)) => self.ask(Ask::New, &weirdnet_themes::unique_name(&self.appearance_dir(), "my-theme"), &r),
            (KeyCode::Char('d'), Some(r)) => {
                self.ask(Ask::Duplicate, &weirdnet_themes::unique_name(&self.appearance_dir(), &format!("{}-copy", r.name)), &r)
            }
            (KeyCode::Char('r'), Some(r)) if r.builtin => self.themes.msg = Some("A built-in can't be renamed; d duplicates it.".into()),
            (KeyCode::Char('r'), Some(r)) => self.ask(Ask::Rename, &r.name, &r),
            (KeyCode::Char('x'), Some(r)) if r.builtin && !r.user_file => self.themes.msg = Some("A built-in can't be deleted.".into()),
            (KeyCode::Char('x'), Some(r)) => self.themes.confirm = Some(Confirm::Delete(r.name)),
            (KeyCode::Char('R'), Some(r)) if r.builtin && r.user_file => self.themes.confirm = Some(Confirm::Reset(r.name)),
            (KeyCode::Char('R'), Some(_)) => self.themes.msg = Some("Only an edited built-in can be reset.".into()),
            (KeyCode::Char('i'), Some(r)) => self.ask(Ask::Import, "", &r),
            (KeyCode::Char('E'), Some(r)) => self.ask(Ask::Export, &format!("{}.toml", r.name), &r),
            _ => {}
        }
    }

    fn appearance_dir(&self) -> weirdnet_themes::UserDir {
        weirdnet_themes::UserDir::new(self.appearance().dir())
    }

    fn ask(&mut self, what: Ask, text: &str, _row: &ThemeRow) {
        let mut ed = Editor::default();
        ed.set(text);
        self.themes.prompt = Some((what, ed));
    }

    fn prompt_key(&mut self, k: KeyEvent) {
        let Some((what, mut ed)) = self.themes.prompt.take() else { return };
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Enter => {
                let text = ed.text().trim().to_string();
                if let Err(e) = self.run_ask(what, &text) {
                    self.themes.msg = Some(e);
                    self.themes.prompt = Some((what, ed));
                }
                return;
            }
            _ => {
                ed.key(k);
            }
        }
        self.themes.prompt = Some((what, ed));
    }

    fn run_ask(&mut self, what: Ask, text: &str) -> Result<(), String> {
        let a = self.appearance();
        let Some(row) = self.themes.selected().cloned() else { return Ok(()) };
        if text.is_empty() {
            return Err("Type something first.".into());
        }
        match what {
            Ask::New | Ask::Duplicate => {
                let name = a.duplicate(&row.name, Some(text)).map_err(|e| e.message)?;
                self.reload_themes();
                self.themes.sel = self.themes.rows.iter().position(|r| r.name == name).unwrap_or(0);
                self.themes.msg = Some(format!("Made {name}."));
                if what == Ask::New {
                    self.start_edit(&name);
                } else {
                    self.preview();
                }
            }
            Ask::Rename => {
                a.rename(&row.name, text).map_err(|e| e.message)?;
                self.reload_themes();
                self.themes.sel = self.themes.rows.iter().position(|r| r.name == text).unwrap_or(0);
                self.themes.saved = a.preference();
                self.themes.msg = Some(format!("Renamed to {text}."));
            }
            Ask::Import => {
                let name = a.import_file(&PathBuf::from(expand_home(text)), None).map_err(|e| e.message)?;
                self.reload_themes();
                self.themes.sel = self.themes.rows.iter().position(|r| r.name == name).unwrap_or(0);
                self.themes.msg = Some(format!("Imported {name}."));
                self.preview();
            }
            Ask::Export => {
                a.export_file(&row.name, &PathBuf::from(expand_home(text))).map_err(|e| e.message)?;
                self.themes.msg = Some(format!("Wrote {text}."));
            }
        }
        Ok(())
    }

    fn confirm_key(&mut self, c: Confirm, k: KeyEvent) {
        if k.code != KeyCode::Char('y') {
            self.themes.msg = Some("Kept.".into());
            return;
        }
        let a = self.appearance();
        match c {
            Confirm::Delete(name) => match a.delete(&name) {
                Ok(()) => {
                    self.reload_themes();
                    self.themes.saved = a.preference();
                    self.themes.msg = Some(format!("Deleted {name}."));
                    self.preview();
                }
                Err(e) => self.themes.msg = Some(e.message),
            },
            Confirm::Reset(name) => match a.reset(&name).and_then(|()| a.delete(&name)) {
                Ok(()) | Err(_) => {
                    self.reload_themes();
                    self.themes.msg = Some(format!("{name} is as it ships."));
                    self.preview();
                }
            },
            Confirm::Discard => {
                self.themes.edit = None;
                self.themes.msg = Some("Discarded.".into());
                self.preview();
            }
        }
    }

    // ---- the editor ----

    fn start_edit(&mut self, name: &str) {
        let a = self.appearance();
        let target = match a.editable(name) {
            Ok(t) => t,
            Err(e) => return self.themes.msg = Some(e.message),
        };
        let file = match a.load(&target) {
            Ok(f) => f,
            Err(e) => return self.themes.msg = Some(e.message),
        };
        if target != name {
            self.reload_themes();
            self.themes.sel = self.themes.rows.iter().position(|r| r.name == target).unwrap_or(0);
            self.themes.msg = Some(format!("{name} ships with Bragi, so you're editing a copy: {target}."));
        }
        let variant = self.themes.mode.variant();
        self.themes.edit = Some(EditState { name: target, orig: file.clone(), file, variant, sel: 0, hex: None, before: None });
        self.preview();
    }

    /// The hex box changed: a valid colour is live in the preview.
    fn hex_changed(&mut self) {
        let Some(e) = &mut self.themes.edit else { return };
        let (role, variant) = (e.role(), e.variant);
        if let Some(h) = &e.hex
            && let Some(v) = parse_hex(h.text())
        {
            let _ = e.file.variant_mut(variant).set_role(role, &v);
        }
        self.preview();
    }

    fn set_role_to(&mut self, value: Option<String>) {
        let Some(e) = &mut self.themes.edit else { return };
        let (role, variant) = (e.role(), e.variant);
        match value {
            Some(v) => {
                let _ = e.file.variant_mut(variant).set_role(role, &v);
            }
            None => {
                if weirdnet_themes::OPTIONAL_ROLES.contains(&role) {
                    let _ = e.file.variant_mut(variant).clear_role(role);
                } else if let Some(o) = e.orig.variant(variant).roles.get(role).cloned() {
                    e.file.variant_mut(variant).roles.insert(role.into(), o);
                }
            }
        }
        self.preview();
    }

    fn edit_key(&mut self, k: KeyEvent, ctrl: bool) {
        let n = ROLES().len();
        // The hex box.
        if let Some(e) = &mut self.themes.edit
            && let Some(h) = &mut e.hex
        {
            match k.code {
                KeyCode::Esc => {
                    e.hex = None;
                    let before = e.before.take().flatten();
                    self.set_role_to(before);
                    // A role that was unset (derived) and is a required one can't be None here.
                }
                KeyCode::Enter => {
                    if parse_hex(h.text()).is_some() {
                        e.hex = None;
                        e.before = None;
                    } else {
                        self.themes.msg = Some("A colour is #rgb or #rrggbb.".into());
                    }
                }
                _ => {
                    h.key(k);
                    self.hex_changed();
                }
            }
            return;
        }
        self.themes.msg = None;
        let Some(e) = &mut self.themes.edit else { return };
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                if e.dirty() {
                    self.themes.confirm = Some(Confirm::Discard);
                    self.themes.msg = Some("Unsaved changes: discard them? y/n".into());
                } else {
                    self.themes.edit = None;
                    self.preview();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => e.sel = e.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => e.sel = (e.sel + 1).min(n - 1),
            KeyCode::PageUp => e.sel = e.sel.saturating_sub(8),
            KeyCode::PageDown => e.sel = (e.sel + 8).min(n - 1),
            KeyCode::Home => e.sel = 0,
            KeyCode::End => e.sel = n - 1,
            KeyCode::Tab | KeyCode::BackTab => {
                e.variant = e.variant.toggle();
                self.preview();
            }
            KeyCode::Enter | KeyCode::Char('#') => {
                let role = e.role();
                let cur = bragi_core::Appearance::palette_of(&e.file, e.variant).get(role).unwrap_or("#000000").to_string();
                e.before = Some(e.stored(role).cloned());
                let mut h = Editor::default();
                h.set(&cur);
                e.hex = Some(h);
            }
            // The theme's own palette as a picker: [ and ] step through its swatches.
            KeyCode::Char(c @ ('[' | ']')) => {
                let sw = e.file.variant(e.variant).swatches();
                if sw.is_empty() {
                    return;
                }
                let cur = bragi_core::Appearance::palette_of(&e.file, e.variant).get(e.role()).map(str::to_string);
                let at = sw.iter().position(|(_, h)| Some(h.to_ascii_lowercase()) == cur);
                let i = match (at, c) {
                    (Some(i), ']') => (i + 1) % sw.len(),
                    (Some(i), _) => (i + sw.len() - 1) % sw.len(),
                    (None, _) => 0,
                };
                let hex = sw[i].1.clone();
                self.set_role_to(Some(hex));
            }
            KeyCode::Char('u') => {
                let role = e.role();
                let o = e.orig.variant(e.variant).roles.get(role).cloned();
                self.set_role_to(o);
            }
            KeyCode::Char('c') => {
                if weirdnet_themes::OPTIONAL_ROLES.contains(&e.role()) {
                    self.set_role_to(None);
                } else {
                    self.themes.msg = Some("Only an optional role (panels, code, diffs, links, hints) can be cleared.".into());
                }
            }
            KeyCode::Char('s') => self.save_edit(),
            _ if ctrl => {}
            _ => {}
        }
    }

    fn save_edit(&mut self) {
        let Some(e) = &self.themes.edit else { return };
        let a = self.appearance();
        let mut edits = Vec::new();
        for v in [Variant::Dark, Variant::Light] {
            let (now, was) = (e.file.variant(v), e.orig.variant(v));
            for r in bragi_core::appearance::roles() {
                match (now.roles.get(r), was.roles.get(r)) {
                    (a, b) if a == b => {}
                    (Some(val), _) => edits.push((v, r.to_string(), val.clone())),
                    (None, Some(_)) => edits.push((v, r.to_string(), String::new())),
                    _ => {}
                }
            }
        }
        let name = e.name.clone();
        for v in [Variant::Dark, Variant::Light] {
            let mine: Vec<(String, String)> = edits.iter().filter(|(x, _, _)| *x == v).map(|(_, r, h)| (r.clone(), h.clone())).collect();
            if mine.is_empty() {
                continue;
            }
            if let Err(err) = a.apply(&name, v, &mine) {
                return self.themes.msg = Some(err.message);
            }
        }
        if let Ok(f) = a.load(&name)
            && let Some(e) = &mut self.themes.edit
        {
            e.orig = f.clone();
            e.file = f;
        }
        self.reload_themes();
        // The chosen theme edited shows its new look once the screen closes.
        if self.themes.saved.theme == name
            && let Ok(f) = a.load(&name)
        {
            self.themes.opened_with = theme::build(&f, self.themes.saved.mode);
        }
        let warn = a.warnings(&name, self.themes.edit.as_ref().map_or(Variant::Dark, |e| e.variant)).unwrap_or_default();
        self.themes.msg = Some(if warn.is_empty() { format!("Saved {name}.") } else { format!("Saved {name}. Check: {}", warn[0]) });
    }
}

fn expand_home(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(h)) => format!("{h}/{rest}"),
        _ => p.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_input_accepts_short_and_long_forms() {
        assert_eq!(parse_hex("#ABC").as_deref(), Some("#aabbcc"));
        assert_eq!(parse_hex("f5b83d").as_deref(), Some("#f5b83d"));
        assert_eq!(parse_hex("#12"), None);
        assert_eq!(parse_hex("#ggg"), None);
    }
}
