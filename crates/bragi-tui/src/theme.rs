//! The terminal app's look, from `weirdnet-themes` 0.2: a theme (a built-in, one of the user's
//! themes under Bragi's data dir, or any `.toml` in its format), a light or dark variant, and the
//! terminal's colour depth, mapped to ratatui styles. The choice is saved with the rest of
//! Bragi's settings ([`bragi_core::Appearance`]) and changed live from the Theme screen.
//!
//! What the crate leaves to the app:
//! - the depth override `BRAGI_TUI_COLOR` (an explicit choice beats `NO_COLOR`);
//! - `raised` and `shadow`, extra palette entries read by name, else blended from the roles;
//! - Unicode or ASCII icons, and the styles themselves.

use std::sync::RwLock;

use bragi_core::{Appearance, ModePref};
use ratatui::style::{Color, Modifier, Style};
use weirdnet_themes::{ColorDepth, ThemeFile, Variant, VariantDef, mix, to_bg_color, to_color};

/// How much colour the terminal gets (`ColorDepth::None` is `NO_COLOR`: text attributes only).
pub type Depth = ColorDepth;

type Rgb = (u8, u8, u8);

/// Glyphs, with ASCII stand-ins for terminals without Unicode.
#[derive(Debug, Clone)]
pub struct Icons {
    pub open: &'static str,
    pub closed: &'static str,
    pub chat: &'static str,
    pub chat_here: &'static str,
    pub bullet: &'static str,
    pub sub_bullet: &'static str,
    pub on: &'static str,
    pub off: &'static str,
    pub half: &'static str,
    pub check: &'static str,
    pub cross: &'static str,
    pub bar: &'static str,
    pub quote: &'static str,
    pub sep: &'static str,
    pub brand: &'static str,
    pub user: &'static str,
    pub bot: &'static str,
    pub attach: &'static str,
    pub warn: &'static str,
    pub spinner: &'static [&'static str],
    pub task_done: &'static str,
    pub task_todo: &'static str,
    pub rule: &'static str,
}

const UNICODE: Icons = Icons {
    open: "▾",
    closed: "▸",
    chat: "○",
    chat_here: "●",
    bullet: "•",
    sub_bullet: "◦",
    on: "●",
    off: "○",
    half: "◐",
    check: "✓",
    cross: "✗",
    bar: "▌",
    quote: "▎",
    sep: "│",
    brand: "◆",
    user: "❯",
    bot: "◆",
    attach: "⊕",
    warn: "!",
    spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
    task_done: "☑",
    task_todo: "☐",
    rule: "─",
};

const ASCII: Icons = Icons {
    open: "v",
    closed: ">",
    chat: "-",
    chat_here: "*",
    bullet: "*",
    sub_bullet: "-",
    on: "*",
    off: "o",
    half: "~",
    check: "+",
    cross: "x",
    bar: "|",
    quote: "|",
    sep: "|",
    brand: "B",
    user: ">",
    bot: "*",
    attach: "+",
    warn: "!",
    spinner: &["|", "/", "-", "\\"],
    task_done: "[x]",
    task_todo: "[ ]",
    rule: "-",
};

/// The resolved look. Every colour is ready for the terminal's depth.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub variant: Variant,
    pub depth: Depth,
    pub icons: Icons,
    pub bg: Color,
    pub surface: Color,
    pub raised: Color,
    pub code_bg: Color,
    pub shadow: Color,
    pub fg: Color,
    pub muted: Color,
    pub dim: Color,
    /// Text behind a window: between `dim` and the background.
    pub faint: Color,
    pub border: Color,
    pub border_focus: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub accent: Color,
    pub header: Color,
    pub link: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,
    pub add: Color,
    pub remove: Color,
    pub add_bg: Color,
    pub remove_bg: Color,
}

impl Theme {
    /// `file`'s `variant` at `depth`.
    pub fn new(file: &ThemeFile, variant: Variant, depth: Depth, unicode: bool) -> Self {
        let v: &VariantDef = file.variant(variant);
        let role = |r: &str| v.color(r).unwrap_or((128, 128, 128));
        let extra = |name: &str| v.palette.get(name).and_then(|h| weirdnet_themes::parse_hex(h));
        let (bg, fg, muted) = (role("bg"), role("fg"), role("muted"));
        let (up, down) = (role("up"), role("down"));
        let surface = role("surface");
        let raised = extra("raised").unwrap_or_else(|| mix(bg, fg, 0.11));
        // The terminal's own colours stand in for backgrounds and body text in 16 colours and
        // with none (`to_bg_color`): the nearest of 16 ANSI colours is rarely the theme's.
        let paint = |c: Rgb| to_color(c, depth);
        let ground = |c: Rgb| to_bg_color(c, depth);
        Self {
            name: file.meta.name.clone(),
            variant,
            depth,
            icons: if unicode { UNICODE } else { ASCII },
            bg: ground(bg),
            surface: ground(surface),
            raised: if depth == ColorDepth::Ansi16 { Color::Indexed(8) } else { ground(raised) },
            code_bg: ground(role("code_bg")),
            shadow: ground(extra("shadow").unwrap_or_else(|| mix(bg, (0, 0, 0), 0.5))),
            fg: ground(fg),
            muted: paint(muted),
            dim: paint(role("dim")),
            faint: paint(mix(role("dim"), bg, 0.45)),
            border: paint(role("border")),
            border_focus: paint(role("border_focus")),
            selection_bg: paint(role("selection_bg")),
            selection_fg: paint(role("selection_fg")),
            accent: paint(role("accent")),
            header: paint(role("header")),
            link: paint(role("link")),
            success: paint(role("success")),
            warning: paint(role("warning")),
            error: paint(role("error")),
            info: paint(role("info")),
            add: paint(up),
            remove: paint(down),
            add_bg: ground(role("diff_add_bg")),
            remove_bg: ground(role("diff_remove_bg")),
        }
    }

    /// Bragi's dark theme in truecolor: what tests and screenshots draw with.
    pub fn bragi_dark() -> Self {
        Self::new(&weirdnet_themes::builtin("bragi").expect("the Bragi theme ships with the crate"), Variant::Dark, ColorDepth::TrueColor, true)
    }

    pub fn color(&self) -> bool {
        self.depth != ColorDepth::None
    }

    // ---- styles ----

    /// Body text on the app background.
    pub fn base(&self) -> Style {
        Style::new().fg(self.fg).bg(self.bg)
    }

    pub fn text(&self) -> Style {
        Style::new().fg(self.fg)
    }

    pub fn muted(&self) -> Style {
        Style::new().fg(self.muted)
    }

    pub fn dim(&self) -> Style {
        if self.color() { Style::new().fg(self.dim) } else { Style::new().add_modifier(Modifier::DIM) }
    }

    pub fn accent(&self) -> Style {
        Style::new().fg(self.accent)
    }

    pub fn bold_accent(&self) -> Style {
        self.accent().add_modifier(Modifier::BOLD)
    }

    pub fn fg(&self, c: Color) -> Style {
        Style::new().fg(c)
    }

    /// A pane's border: the accent when it has the focus.
    pub fn border(&self, focused: bool) -> Style {
        match (self.color(), focused) {
            (true, true) => Style::new().fg(self.border_focus),
            (true, false) => Style::new().fg(self.border),
            (false, true) => Style::new().add_modifier(Modifier::BOLD),
            (false, false) => Style::new().add_modifier(Modifier::DIM),
        }
    }

    /// A pane title: accent and bold when focused, muted otherwise.
    pub fn title(&self, focused: bool) -> Style {
        if focused { self.bold_accent() } else { self.muted().add_modifier(Modifier::BOLD) }
    }

    /// The selection bar.
    pub fn selection(&self) -> Style {
        if self.color() {
            Style::new().fg(self.selection_fg).bg(self.selection_bg).add_modifier(Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::REVERSED)
        }
    }

    /// A filled pill (the brand mark, key caps, the language tag of a code block).
    pub fn pill(&self, fg: Color, bg: Color) -> Style {
        if self.color() { Style::new().fg(fg).bg(bg).add_modifier(Modifier::BOLD) } else { Style::new().add_modifier(Modifier::REVERSED) }
    }

    pub fn spinner(&self, elapsed_ms: u128) -> &'static str {
        let f = self.icons.spinner;
        f[(elapsed_ms / 90) as usize % f.len()]
    }
}

/// How the theme is chosen for this run, over the saved choice: `--theme` / `BRAGI_TUI_THEME`
/// (a theme name or a `.toml` path), `--mode` / `BRAGI_TUI_MODE` (`dark`, `light` or `system`),
/// `BRAGI_TUI_COLOR` (`truecolor`, `256`, `16`, `none`; beats `NO_COLOR`), `NO_COLOR` and
/// `BRAGI_TUI_ASCII`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    pub theme: Option<String>,
    pub mode: Option<String>,
}

/// Loads a theme by name (the user's themes first, then the crate's built-ins) or from a
/// `.toml` file.
pub fn load(name: &str, appearance: &Appearance) -> Result<ThemeFile, String> {
    if name.ends_with(".toml") {
        let text = std::fs::read_to_string(name).map_err(|e| format!("{name}: {e}"))?;
        return ThemeFile::parse(&text).map_err(|e| format!("{name}: {e}"));
    }
    appearance.load(name).map_err(|e| {
        let names: Vec<String> = appearance.rows().unwrap_or_default().into_iter().map(|r| r.name).collect();
        format!("no theme called {name}: {} (there are {}; or give a .toml path)", e.message, names.join(", "))
    })
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The depth from the environment: `BRAGI_TUI_COLOR` when it parses, else `NO_COLOR`,
/// `COLORTERM` and `TERM`.
pub fn depth_from_env() -> ColorDepth {
    ColorDepth::detect_with(env("BRAGI_TUI_COLOR").as_deref())
}

/// Unicode unless asked for ASCII or the locale isn't UTF-8.
pub fn unicode_from_env() -> bool {
    if env("BRAGI_TUI_ASCII").is_some_and(|v| v != "0") {
        return false;
    }
    let locale = env("LC_ALL").or_else(|| env("LC_CTYPE")).or_else(|| env("LANG"));
    locale.is_none_or(|l| {
        let l = l.to_ascii_lowercase();
        l.contains("utf-8") || l.contains("utf8")
    })
}

/// The theme for this run: the flags, then the environment, then what is saved.
pub fn resolve(choice: &Choice, appearance: &Appearance) -> Result<Theme, String> {
    let saved = appearance.preference();
    let name = choice.theme.clone().or_else(|| env("BRAGI_TUI_THEME")).unwrap_or_else(|| saved.theme.clone());
    let file = match load(&name, appearance) {
        Ok(f) => f,
        // A saved theme that no longer loads falls back to Bragi; a typed one is an error.
        Err(_) if choice.theme.is_none() && env("BRAGI_TUI_THEME").is_none() => appearance.current().0,
        Err(e) => return Err(e),
    };
    let mode = match choice.mode.clone().or_else(|| env("BRAGI_TUI_MODE")) {
        None => saved.mode,
        Some(m) => ModePref::parse(&m).ok_or_else(|| format!("--mode is dark, light or system, not {m}"))?,
    };
    Ok(Theme::new(&file, mode.variant(), depth_from_env(), unicode_from_env()))
}

/// The look of `name` in `mode` as the terminal draws it now (the Theme screen's preview and
/// its switching).
pub fn build(file: &ThemeFile, mode: ModePref) -> Theme {
    Theme::new(file, mode.variant(), depth_from_env(), unicode_from_env())
}

static THEME: RwLock<Option<&'static Theme>> = RwLock::new(None);

/// Sets the theme (the run's first, or a switch from the Theme screen). The old one is leaked:
/// a few hundred bytes per change, and `get` hands out `&'static` borrows to the drawing code.
pub fn init(t: Theme) {
    let leaked: &'static Theme = Box::leak(Box::new(t));
    *THEME.write().unwrap_or_else(|e| e.into_inner()) = Some(leaked);
}

/// The run's theme: Bragi dark in truecolor until [`init`] says otherwise.
pub fn get() -> &'static Theme {
    if let Some(t) = *THEME.read().unwrap_or_else(|e| e.into_inner()) {
        return t;
    }
    let mut w = THEME.write().unwrap_or_else(|e| e.into_inner());
    w.get_or_insert_with(|| Box::leak(Box::new(Theme::bragi_dark())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bragi() -> ThemeFile {
        weirdnet_themes::builtin("bragi").unwrap()
    }

    #[test]
    fn bragi_theme_is_valid_readable_and_maps_every_depth() {
        let f = bragi();
        for v in [Variant::Dark, Variant::Light] {
            let d = f.variant(v);
            assert!(weirdnet_themes::contrast(d.resolve("fg").unwrap(), d.resolve("bg").unwrap()) >= weirdnet_themes::MIN_CONTRAST);
            assert!(weirdnet_themes::contrast_warnings(d).is_empty(), "{v:?}: {:?}", weirdnet_themes::contrast_warnings(d));
        }
        let t = Theme::new(&f, Variant::Dark, ColorDepth::TrueColor, true);
        assert_eq!(t.accent, Color::Rgb(0xf5, 0xb8, 0x3d));
        assert_eq!(t.bg, Color::Rgb(0x14, 0x16, 0x1b));
        assert!(matches!(Theme::new(&f, Variant::Dark, ColorDepth::Ansi256, true).accent, Color::Indexed(_)));
        let t16 = Theme::new(&f, Variant::Dark, ColorDepth::Ansi16, false);
        assert_eq!((t16.bg, t16.fg), (Color::Reset, Color::Reset));
        assert_eq!(t16.icons.open, "v");
        let none = Theme::new(&f, Variant::Light, ColorDepth::None, true);
        assert_eq!(none.accent, Color::Reset);
        assert!(none.selection().add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn every_builtin_gets_distinct_surfaces_and_diff_backgrounds() {
        for (name, _) in weirdnet_themes::BUILTINS {
            let t = Theme::new(&weirdnet_themes::builtin(name).unwrap(), Variant::Dark, ColorDepth::TrueColor, true);
            assert_ne!(t.surface, t.bg, "{name}");
            assert_ne!(t.add_bg, t.remove_bg, "{name}");
        }
    }

    #[test]
    fn the_saved_choice_is_used_and_flags_beat_it() {
        let d = tempfile::tempdir().unwrap();
        let a = Appearance::open(d.path());
        a.set_theme("nord").unwrap();
        a.set_mode(ModePref::Light).unwrap();
        // The environment may carry BRAGI_TUI_* in a developer's shell; these assertions only
        // look at the parts the flags decide.
        let t = resolve(&Choice { theme: None, mode: Some("dark".into()) }, &a).unwrap();
        assert_eq!(t.variant, Variant::Dark);
        let t = resolve(&Choice { theme: Some("gruvbox".into()), mode: Some("light".into()) }, &a).unwrap();
        assert_eq!((t.name.as_str(), t.variant), ("gruvbox", Variant::Light));
        assert!(resolve(&Choice { theme: Some("nope".into()), mode: Some("dark".into()) }, &a).unwrap_err().contains("bragi"));
        assert!(resolve(&Choice { theme: None, mode: Some("blue".into()) }, &a).is_err());
    }

    #[test]
    fn an_explicit_depth_beats_no_color() {
        assert_eq!(ColorDepth::detect_with(Some("256")), ColorDepth::Ansi256);
        assert_eq!(ColorDepth::detect_with(Some("none")), ColorDepth::None);
    }

    #[test]
    fn switching_swaps_what_get_returns() {
        let a = Theme::bragi_dark();
        init(a.clone());
        assert_eq!(get().name, "bragi");
        let nord = Theme::new(&weirdnet_themes::builtin("nord").unwrap(), Variant::Dark, ColorDepth::TrueColor, true);
        init(nord);
        assert_eq!(get().name, "nord");
        init(a);
    }
}
