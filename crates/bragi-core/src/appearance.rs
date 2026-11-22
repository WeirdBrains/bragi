//! The look, for every shell: which theme and light/dark mode are chosen, and the themes
//! themselves (`weirdnet-themes`' TOML format, so a theme works in the desktop app, the Flutter
//! app and the terminal app, and in any other Weirdnet app).
//!
//! Themes live in `<data dir>/themes/<name>/interface.toml` and the choice in
//! `<data dir>/appearance.json`. The shells draw a [`Palette`] and call this to switch, edit,
//! duplicate, rename, delete, reset, import and export. Built-ins are never edited in place:
//! [`Appearance::editable`] hands back a copy to edit.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
pub use weirdnet_themes::Variant;
use weirdnet_themes::{OPTIONAL_ROLES, ROLES, ThemeFile, UserDir, detect::Probe};

use crate::error::{CoreError, ErrorKind, Result};

/// The theme Bragi starts with.
pub const DEFAULT_THEME: &str = "bragi";
const PREF_FILE: &str = "appearance.json";

/// Light, dark, or follow the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModePref {
    Light,
    Dark,
    #[default]
    System,
}

impl ModePref {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            "system" => Some(Self::System),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::System => "system",
        }
    }

    /// The variant this mode means now. `System` reads `COLORFGBG` and the OS preference
    /// (never the terminal: that needs the tty), else dark.
    pub fn variant(self) -> Variant {
        match self {
            Self::Light => Variant::Light,
            Self::Dark => Variant::Dark,
            Self::System => weirdnet_themes::resolve_variant_with(weirdnet_themes::Mode::System, Probe::NONE.os(true)),
        }
    }
}

/// What is chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preference {
    pub theme: String,
    pub mode: ModePref,
}

impl Default for Preference {
    fn default() -> Self {
        Self { theme: DEFAULT_THEME.into(), mode: ModePref::default() }
    }
}

/// One row of a theme list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeRow {
    pub name: String,
    pub display_name: String,
    pub builtin: bool,
    /// A file in the user's themes folder: a user theme, or an edited built-in.
    pub user_file: bool,
    pub error: Option<String>,
}

/// Every role of one variant as `#rrggbb` (optional roles derived when the theme leaves them
/// out), in [`roles`] order: what a shell paints with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub theme: String,
    pub variant: Variant,
    pub roles: Vec<(String, String)>,
    /// A step above `surface` (raised cards, key caps): the theme's `raised` palette entry when
    /// it has one, else `bg` mixed 11% toward `fg`.
    pub raised: String,
}

impl Palette {
    pub fn get(&self, role: &str) -> Option<&str> {
        self.roles.iter().find(|(r, _)| r == role).map(|(_, h)| h.as_str())
    }

    /// The role as `0xRRGGBB` (gray when unknown).
    pub fn rgb(&self, role: &str) -> u32 {
        self.get(role)
            .and_then(weirdnet_themes::parse_hex)
            .map(|(r, g, b)| u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
            .unwrap_or(0x80_80_80)
    }

    /// `raised` as `0xRRGGBB`.
    pub fn raised_rgb(&self) -> u32 {
        weirdnet_themes::parse_hex(&self.raised).map(|(r, g, b)| u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)).unwrap_or(0x80_80_80)
    }

    /// Whether this is the dark variant.
    pub fn is_dark(&self) -> bool {
        self.variant == Variant::Dark
    }
}

/// Every role a theme has, required first.
pub fn roles() -> Vec<&'static str> {
    ROLES.iter().chain(OPTIONAL_ROLES.iter()).copied().collect()
}

/// A short description of a role for an editor.
pub fn role_label(role: &str) -> &'static str {
    match role {
        "bg" => "Background",
        "fg" => "Text",
        "muted" => "Secondary text",
        "border" => "Borders",
        "border_focus" => "Focused border",
        "selection_bg" => "Selection",
        "selection_fg" => "Selected text",
        "accent" => "Accent",
        "header" => "Headings",
        "up" => "Added",
        "down" => "Removed",
        "warning" => "Warning",
        "error" => "Error",
        "success" => "Success",
        "info" => "Info",
        "chart_1" | "chart_2" | "chart_3" | "chart_4" | "chart_5" | "chart_6" => "Chart",
        "surface" => "Panels",
        "code_bg" => "Code background",
        "diff_add_bg" => "Added background",
        "diff_remove_bg" => "Removed background",
        "link" => "Links",
        "dim" => "Hints",
        _ => "",
    }
}

fn bad(msg: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, msg.to_string())
}

fn themes_err(e: weirdnet_themes::ThemeError) -> CoreError {
    bad(e)
}

/// The themes and the choice under one data dir.
#[derive(Debug, Clone)]
pub struct Appearance {
    themes: UserDir,
    pref: PathBuf,
}

impl Appearance {
    pub fn open(dir: &Path) -> Self {
        Self { themes: UserDir::new(dir.join("themes")), pref: dir.join(PREF_FILE) }
    }

    /// The themes folder.
    pub fn dir(&self) -> &Path {
        self.themes.path()
    }

    // ---- the choice ----

    /// What is chosen; the default (Bragi, system mode) when nothing is saved or the file is
    /// unreadable. A theme that no longer loads falls back to Bragi when drawn.
    pub fn preference(&self) -> Preference {
        std::fs::read_to_string(&self.pref).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    /// Saves the choice. The theme has to exist and load.
    pub fn set_preference(&self, p: &Preference) -> Result<()> {
        weirdnet_themes::load(&self.themes, &p.theme).map_err(themes_err)?;
        if let Some(d) = self.pref.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = self.pref.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(p).map_err(|e| CoreError::new(ErrorKind::Storage, e.to_string()))?)?;
        std::fs::rename(&tmp, &self.pref)?;
        Ok(())
    }

    pub fn set_theme(&self, name: &str) -> Result<Preference> {
        let p = Preference { theme: name.into(), ..self.preference() };
        self.set_preference(&p)?;
        Ok(p)
    }

    pub fn set_mode(&self, mode: ModePref) -> Result<Preference> {
        let p = Preference { mode, ..self.preference() };
        self.set_preference(&p)?;
        Ok(p)
    }

    /// The next (or previous) theme after the chosen one, saved: for a "next theme" key.
    pub fn cycle(&self, forward: bool) -> Result<Preference> {
        let names: Vec<String> = self.rows()?.into_iter().filter(|r| r.error.is_none()).map(|r| r.name).collect();
        let cur = self.preference();
        let next = weirdnet_themes::cycle(&names, &cur.theme, forward).ok_or_else(|| bad("there are no themes"))?.to_string();
        self.set_theme(&next)
    }

    // ---- reading ----

    pub fn rows(&self) -> Result<Vec<ThemeRow>> {
        Ok(weirdnet_themes::entries(&self.themes)
            .map_err(themes_err)?
            .into_iter()
            .map(|e| ThemeRow { name: e.name, display_name: e.display_name, builtin: e.builtin, user_file: e.user_file, error: e.error })
            .collect())
    }

    pub fn load(&self, name: &str) -> Result<ThemeFile> {
        weirdnet_themes::load(&self.themes, name).map_err(themes_err)
    }

    /// The chosen theme (Bragi when it doesn't load) and its variant now.
    pub fn current(&self) -> (ThemeFile, Variant) {
        let p = self.preference();
        let (file, _) = weirdnet_themes::active(&self.themes, &p.theme);
        let file = if file.meta.name == p.theme { file } else { weirdnet_themes::builtin(DEFAULT_THEME).unwrap_or(file) };
        (file, p.mode.variant())
    }

    pub fn palette_of(file: &ThemeFile, variant: Variant) -> Palette {
        let v = file.variant(variant);
        let roles = roles().into_iter().filter_map(|r| v.color(r).map(|c| (r.to_string(), weirdnet_themes::to_hex(c)))).collect();
        let raised = v
            .palette
            .get("raised")
            .and_then(|h| weirdnet_themes::parse_hex(h))
            .or_else(|| Some(weirdnet_themes::mix(v.color("bg")?, v.color("fg")?, 0.11)))
            .map(weirdnet_themes::to_hex)
            .unwrap_or_else(|| "#808080".into());
        Palette { theme: file.meta.name.clone(), variant, roles, raised }
    }

    /// The palette to draw with now.
    pub fn palette(&self) -> Palette {
        let (file, variant) = self.current();
        Self::palette_of(&file, variant)
    }

    /// The palette to draw with now when the shell already knows whether the system is dark (a
    /// phone's platform brightness): `system_dark` stands in for the probe in system mode.
    pub fn palette_with_system(&self, system_dark: Option<bool>) -> Palette {
        let (file, variant) = self.current();
        let variant = match (self.preference().mode, system_dark) {
            (ModePref::System, Some(true)) => Variant::Dark,
            (ModePref::System, Some(false)) => Variant::Light,
            _ => variant,
        };
        Self::palette_of(&file, variant)
    }

    /// `name`'s palette for `variant` (the editor's and the picker's previews).
    pub fn palette_for(&self, name: &str, variant: Variant) -> Result<Palette> {
        Ok(Self::palette_of(&self.load(name)?, variant))
    }

    // ---- editing ----

    /// The theme to edit when the user asks to edit `name`: itself for a user theme, a fresh
    /// copy for a built-in (`<name>-custom`, numbered when taken).
    pub fn editable(&self, name: &str) -> Result<String> {
        if weirdnet_themes::is_builtin(name) {
            let dest = weirdnet_themes::unique_name(&self.themes, &format!("{name}-custom"));
            self.duplicate(name, Some(&dest))
        } else {
            self.load(name)?;
            Ok(name.into())
        }
    }

    /// Sets roles of one variant together, all or nothing. Values are `#rrggbb` (or `#rgb`) or a
    /// palette name; an empty value clears an optional role so it is derived again.
    pub fn apply(&self, name: &str, variant: Variant, edits: &[(String, String)]) -> Result<ThemeFile> {
        let mut file = self.load(name)?;
        for (role, value) in edits {
            let value = expand_hex(value.trim());
            if value.is_empty() {
                file.variant_mut(variant).clear_role(role).map_err(themes_err)?;
            } else {
                file.variant_mut(variant).set_role(role, &value).map_err(themes_err)?;
            }
        }
        weirdnet_themes::save(&self.themes, &file).map_err(themes_err)?;
        Ok(file)
    }

    pub fn duplicate(&self, name: &str, as_name: Option<&str>) -> Result<String> {
        weirdnet_themes::duplicate(&self.themes, name, as_name).map_err(themes_err)
    }

    /// Renames a user theme (the choice follows it).
    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        weirdnet_themes::rename(&self.themes, from, to).map_err(themes_err)?;
        let p = self.preference();
        if p.theme == from {
            self.set_preference(&Preference { theme: to.into(), ..p })?;
        }
        Ok(())
    }

    /// Deletes a user theme, or the edits of a built-in; the choice falls back to Bragi.
    pub fn delete(&self, name: &str) -> Result<()> {
        weirdnet_themes::delete(&self.themes, name).map_err(themes_err)?;
        let p = self.preference();
        if p.theme == name && !weirdnet_themes::is_builtin(name) {
            self.set_preference(&Preference { theme: DEFAULT_THEME.into(), ..p })?;
        }
        Ok(())
    }

    /// Puts a built-in back as it ships.
    pub fn reset(&self, name: &str) -> Result<()> {
        weirdnet_themes::reset(&self.themes, name).map_err(themes_err)
    }

    pub fn import_file(&self, file: &Path, as_name: Option<&str>) -> Result<String> {
        weirdnet_themes::import(&self.themes, file, as_name).map_err(themes_err)
    }

    pub fn export_file(&self, name: &str, file: &Path) -> Result<()> {
        weirdnet_themes::export(&self.themes, name, file).map_err(themes_err)
    }

    /// A theme file's text, to import from a clipboard or a picker that only gives bytes.
    pub fn import_text(&self, text: &str, as_name: Option<&str>) -> Result<String> {
        let mut t = ThemeFile::parse(text).map_err(themes_err)?;
        if let Some(n) = as_name {
            t.meta.name = n.into();
        }
        if weirdnet_themes::exists(&self.themes, &t.meta.name) {
            t.meta.name = weirdnet_themes::unique_name(&self.themes, &t.meta.name);
        }
        weirdnet_themes::save(&self.themes, &t).map_err(themes_err)?;
        Ok(t.meta.name)
    }

    pub fn export_text(&self, name: &str) -> Result<String> {
        self.load(name)?.to_toml().map_err(themes_err)
    }

    /// WCAG contrast warnings for one variant (text on background and the like).
    pub fn warnings(&self, name: &str, variant: Variant) -> Result<Vec<String>> {
        Ok(weirdnet_themes::contrast_warnings(self.load(name)?.variant(variant)))
    }

    /// Roles with the colours a theme's variant has now, as an editor lists them.
    pub fn role_map(file: &ThemeFile, variant: Variant) -> BTreeMap<String, String> {
        Self::palette_of(file, variant).roles.into_iter().collect()
    }
}

/// `#abc` as `#aabbcc`; anything else unchanged.
fn expand_hex(s: &str) -> String {
    match s.strip_prefix('#') {
        Some(h) if h.len() == 3 && h.chars().all(|c| c.is_ascii_hexdigit()) => {
            format!("#{}", h.chars().flat_map(|c| [c, c]).collect::<String>())
        }
        _ => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (tempfile::TempDir, Appearance) {
        let d = tempfile::tempdir().unwrap();
        let a = Appearance::open(d.path());
        (d, a)
    }

    #[test]
    fn the_default_is_bragi_in_system_mode_and_the_choice_persists() {
        let (d, a) = app();
        assert_eq!(a.preference(), Preference { theme: "bragi".into(), mode: ModePref::System });
        a.set_theme("nord").unwrap();
        a.set_mode(ModePref::Light).unwrap();
        let again = Appearance::open(d.path());
        assert_eq!(again.preference(), Preference { theme: "nord".into(), mode: ModePref::Light });
        assert_eq!(again.palette().theme, "nord");
        assert!(!again.palette().is_dark());
        assert!(a.set_theme("nope").is_err());
        assert_eq!(a.preference().theme, "nord");
    }

    #[test]
    fn a_palette_has_every_role_and_bragi_matches_the_brand() {
        let (_d, a) = app();
        let p = a.palette_for("bragi", Variant::Dark).unwrap();
        assert_eq!(p.roles.len(), roles().len());
        assert_eq!(p.get("accent"), Some("#f5b83d"));
        assert_eq!(p.rgb("bg"), 0x14161b);
    }

    #[test]
    fn editing_a_builtin_edits_a_copy_and_apply_is_all_or_nothing() {
        let (_d, a) = app();
        let n = a.editable("dracula").unwrap();
        assert_eq!(n, "dracula-custom");
        assert_eq!(a.editable("dracula").unwrap(), "dracula-custom-2");
        a.apply(&n, Variant::Dark, &[("accent".into(), "#abc".into()), ("surface".into(), "#102030".into())]).unwrap();
        let p = a.palette_for(&n, Variant::Dark).unwrap();
        assert_eq!((p.get("accent"), p.get("surface")), (Some("#aabbcc"), Some("#102030")));
        assert!(a.apply(&n, Variant::Dark, &[("accent".into(), "#000000".into()), ("bg".into(), "notacolor".into())]).is_err());
        assert_eq!(a.palette_for(&n, Variant::Dark).unwrap().get("accent"), Some("#aabbcc"), "nothing half-applied");
        assert!(a.apply(&n, Variant::Dark, &[("nonsense".into(), "#000000".into())]).is_err());
        // An optional role cleared is derived again.
        a.apply(&n, Variant::Dark, &[("surface".into(), String::new())]).unwrap();
        assert_ne!(a.palette_for(&n, Variant::Dark).unwrap().get("surface"), Some("#102030"));
        assert!(a.apply(&n, Variant::Dark, &[("bg".into(), String::new())]).is_err(), "a required role can't be cleared");
        // The shipped dracula is untouched.
        assert_eq!(
            a.palette_for("dracula", Variant::Dark).unwrap(),
            Appearance::palette_of(&weirdnet_themes::builtin("dracula").unwrap(), Variant::Dark).with_name("dracula")
        );
    }

    #[test]
    fn duplicate_rename_delete_follow_the_choice() {
        let (_d, a) = app();
        let n = a.duplicate("nord", Some("mine")).unwrap();
        a.set_theme(&n).unwrap();
        a.rename("mine", "ours").unwrap();
        assert_eq!(a.preference().theme, "ours");
        assert!(a.rename("nord", "x").is_err(), "built-ins are copied, not renamed");
        a.delete("ours").unwrap();
        assert_eq!(a.preference().theme, "bragi");
        assert!(a.delete("bragi").is_err());
        let rows = a.rows().unwrap();
        assert!(rows.iter().any(|r| r.name == "bragi" && r.builtin) && !rows.iter().any(|r| r.name == "ours"));
    }

    #[test]
    fn export_import_round_trip_and_reset() {
        let (d, a) = app();
        let n = a.duplicate("gruvbox", Some("warm")).unwrap();
        a.apply(&n, Variant::Light, &[("accent".into(), "#112233".into())]).unwrap();
        let text = a.export_text(&n).unwrap();
        let (_d2, b) = app();
        assert_eq!(b.import_text(&text, None).unwrap(), "warm");
        assert_eq!(b.import_text(&text, None).unwrap(), "warm-2", "an import never overwrites");
        assert_eq!(b.palette_for("warm", Variant::Light).unwrap().get("accent"), Some("#112233"));
        let file = d.path().join("out.toml");
        a.export_file(&n, &file).unwrap();
        assert_eq!(b.import_file(&file, Some("again")).unwrap(), "again");
        assert!(b.import_text("not a theme", None).is_err());
        // A built-in's edits can be reset.
        let mut f = b.load("nord").unwrap();
        f.dark.set_role("accent", "#010203").unwrap();
        weirdnet_themes::save(&b.themes, &f).unwrap();
        assert_eq!(b.palette_for("nord", Variant::Dark).unwrap().get("accent"), Some("#010203"));
        b.reset("nord").unwrap();
        assert_ne!(b.palette_for("nord", Variant::Dark).unwrap().get("accent"), Some("#010203"));
    }

    #[test]
    fn cycling_wraps_and_a_broken_theme_falls_back_to_bragi() {
        let (d, a) = app();
        let first = a.rows().unwrap()[0].name.clone();
        a.set_theme("bragi").unwrap();
        let n = a.cycle(true).unwrap();
        assert_ne!(n.theme, "bragi");
        a.set_theme(&first).unwrap();
        assert_eq!(a.cycle(false).unwrap().theme, a.rows().unwrap().last().unwrap().name);
        let dir = d.path().join("themes/broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("interface.toml"), "nope").unwrap();
        std::fs::write(d.path().join("appearance.json"), r#"{"theme":"broken","mode":"dark"}"#).unwrap();
        assert_eq!(a.palette().theme, "bragi");
        assert!(a.rows().unwrap().iter().any(|r| r.name == "broken" && r.error.is_some()));
    }
}

impl Palette {
    #[cfg(test)]
    fn with_name(mut self, n: &str) -> Self {
        self.theme = n.into();
        self
    }
}
