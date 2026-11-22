//! The terminal app's look, from `weirdnet-themes`: a theme file (Bragi's own palette by
//! default, or any of the crate's built-ins or a `.toml` in its format), a light or dark
//! variant, and the terminal's colour depth, mapped to ratatui styles.
//!
//! What the crate doesn't cover yet is filled in here:
//! - `NO_COLOR` (no colour at all, only bold, reverse and the like);
//! - an override for the depth (`BRAGI_TUI_COLOR`), since `ColorDepth::detect` can't tell a
//!   truecolor tmux;
//! - in 16 colours, the terminal's own background and text instead of the nearest ANSI colour;
//! - surfaces, code and diff backgrounds and a link colour: read from the theme's palette when
//!   it has them (`surface`, `raised`, `code_bg`, `dim`, `link`, `diff_add_bg`,
//!   `diff_remove_bg`, `shadow`), else blended from its roles;
//! - Unicode or ASCII icons.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use weirdnet_themes::{ColorDepth, ThemeFile, Variant, VariantDef};

/// Bragi's palette (BRAND.md) as a weirdnet-themes file.
pub const BRAGI_THEME: &str = include_str!("../themes/bragi.toml");

/// How much colour the terminal gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    TrueColor,
    Ansi256,
    Ansi16,
    /// `NO_COLOR`: text attributes only.
    None,
}

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

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

impl Theme {
    /// `file`'s `variant` at `depth`.
    pub fn new(file: &ThemeFile, variant: Variant, depth: Depth, unicode: bool) -> Self {
        let v: &VariantDef = file.variant(variant);
        let role = |r: &str| v.resolve(r).unwrap_or((128, 128, 128));
        let extra = |name: &str| v.palette.get(name).and_then(|h| weirdnet_themes::parse_hex(h));
        let (bg, fg, muted) = (role("bg"), role("fg"), role("muted"));
        let (up, down) = (role("up"), role("down"));
        let surface = extra("surface").unwrap_or_else(|| mix(bg, fg, 0.05));
        let raised = extra("raised").unwrap_or_else(|| mix(bg, fg, 0.11));
        // The terminal's own colours stand in for backgrounds and body text below truecolor
        // and 256 colours (the nearest of 16 ANSI colours is rarely the theme's).
        let paint = |c: Rgb| match depth {
            Depth::TrueColor => weirdnet_themes::to_color(c, ColorDepth::TrueColor),
            Depth::Ansi256 => weirdnet_themes::to_color(c, ColorDepth::Ansi256),
            Depth::Ansi16 => weirdnet_themes::to_color(c, ColorDepth::Ansi16),
            Depth::None => Color::Reset,
        };
        let ground = |c: Rgb| if matches!(depth, Depth::Ansi16 | Depth::None) { Color::Reset } else { paint(c) };
        Self {
            name: file.meta.name.clone(),
            variant,
            depth,
            icons: if unicode { UNICODE } else { ASCII },
            bg: ground(bg),
            surface: ground(surface),
            raised: if depth == Depth::Ansi16 { Color::Indexed(8) } else { ground(raised) },
            code_bg: ground(extra("code_bg").unwrap_or(surface)),
            shadow: ground(extra("shadow").unwrap_or_else(|| mix(bg, (0, 0, 0), 0.5))),
            fg: ground(fg),
            muted: paint(muted),
            dim: paint(extra("dim").unwrap_or_else(|| mix(muted, bg, 0.35))),
            border: paint(role("border")),
            border_focus: paint(role("border_focus")),
            selection_bg: paint(role("selection_bg")),
            selection_fg: paint(role("selection_fg")),
            accent: paint(role("accent")),
            header: paint(role("header")),
            link: paint(extra("link").unwrap_or_else(|| role("info"))),
            success: paint(role("success")),
            warning: paint(role("warning")),
            error: paint(role("error")),
            info: paint(role("info")),
            add: paint(up),
            remove: paint(down),
            add_bg: ground(extra("diff_add_bg").unwrap_or_else(|| mix(bg, up, 0.16))),
            remove_bg: ground(extra("diff_remove_bg").unwrap_or_else(|| mix(bg, down, 0.16))),
        }
    }

    /// Bragi's dark theme in truecolor: what tests and screenshots draw with.
    pub fn bragi_dark() -> Self {
        Self::new(&ThemeFile::parse(BRAGI_THEME).expect("the Bragi theme parses"), Variant::Dark, Depth::TrueColor, true)
    }

    pub fn color(&self) -> bool {
        self.depth != Depth::None
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

/// How the theme is chosen: `--theme` / `BRAGI_TUI_THEME` (a built-in name or a `.toml` path),
/// `--mode` / `BRAGI_TUI_MODE` (`dark`, the default, `light`, or `system`), `NO_COLOR`,
/// `BRAGI_TUI_COLOR` (`truecolor`, `256`, `16`, `none`) and `BRAGI_TUI_ASCII`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    pub theme: Option<String>,
    pub mode: Option<String>,
}

/// Loads a theme by name: `bragi`, one of weirdnet-themes' built-ins, or a `.toml` file.
pub fn load(name: &str) -> Result<ThemeFile, String> {
    if name == "bragi" {
        return ThemeFile::parse(BRAGI_THEME).map_err(|e| e.to_string());
    }
    if name.ends_with(".toml") {
        let text = std::fs::read_to_string(name).map_err(|e| format!("{name}: {e}"))?;
        return ThemeFile::parse(&text).map_err(|e| format!("{name}: {e}"));
    }
    weirdnet_themes::builtin(name).ok_or_else(|| {
        let names: Vec<&str> = std::iter::once("bragi").chain(weirdnet_themes::BUILTINS.iter().map(|(n, _)| *n)).collect();
        format!("no theme called {name} (there are {}; or give a .toml path)", names.join(", "))
    })
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The depth from the environment.
pub fn depth_from_env() -> Depth {
    if env("NO_COLOR").is_some() {
        return Depth::None;
    }
    match env("BRAGI_TUI_COLOR").as_deref() {
        Some("none" | "off" | "0") => return Depth::None,
        Some("16") => return Depth::Ansi16,
        Some("256") => return Depth::Ansi256,
        Some("truecolor" | "24bit" | "24") => return Depth::TrueColor,
        _ => {}
    }
    match ColorDepth::detect() {
        ColorDepth::TrueColor => Depth::TrueColor,
        ColorDepth::Ansi256 => Depth::Ansi256,
        ColorDepth::Ansi16 => Depth::Ansi16,
    }
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

/// The theme for this run.
pub fn resolve(choice: &Choice) -> Result<Theme, String> {
    let name = choice.theme.clone().or_else(|| env("BRAGI_TUI_THEME")).unwrap_or_else(|| "bragi".into());
    let file = load(&name)?;
    let variant = match choice.mode.clone().or_else(|| env("BRAGI_TUI_MODE")).as_deref() {
        None | Some("dark") => Variant::Dark,
        Some("light") => Variant::Light,
        // Asks the OS (the portal on Linux): only when the user picks it.
        Some("system") => weirdnet_themes::resolve_variant(weirdnet_themes::Mode::System),
        Some(m) => return Err(format!("--mode is dark, light or system, not {m}")),
    };
    Ok(Theme::new(&file, variant, depth_from_env(), unicode_from_env()))
}

static THEME: OnceLock<Theme> = OnceLock::new();

/// Sets the theme for the run (first call wins).
pub fn init(t: Theme) {
    let _ = THEME.set(t);
}

/// The run's theme: Bragi dark in truecolor until [`init`] says otherwise.
pub fn get() -> &'static Theme {
    THEME.get_or_init(Theme::bragi_dark)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bragi_theme_is_valid_readable_and_maps_every_depth() {
        let f = ThemeFile::parse(BRAGI_THEME).unwrap();
        for v in [Variant::Dark, Variant::Light] {
            let d = f.variant(v);
            assert!(weirdnet_themes::contrast(d.resolve("fg").unwrap(), d.resolve("bg").unwrap()) >= weirdnet_themes::MIN_CONTRAST);
            assert!(weirdnet_themes::contrast_warnings(d).is_empty(), "{v:?}: {:?}", weirdnet_themes::contrast_warnings(d));
        }
        let t = Theme::new(&f, Variant::Dark, Depth::TrueColor, true);
        assert_eq!(t.accent, Color::Rgb(0xf5, 0xb8, 0x3d));
        assert_eq!(t.bg, Color::Rgb(0x14, 0x16, 0x1b));
        assert!(matches!(Theme::new(&f, Variant::Dark, Depth::Ansi256, true).accent, Color::Indexed(_)));
        let t16 = Theme::new(&f, Variant::Dark, Depth::Ansi16, false);
        assert_eq!((t16.bg, t16.fg), (Color::Reset, Color::Reset));
        assert_eq!(t16.icons.open, "v");
        let none = Theme::new(&f, Variant::Light, Depth::None, true);
        assert_eq!(none.accent, Color::Reset);
        assert!(none.selection().add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn other_themes_get_the_extra_colours_blended() {
        for (name, _) in weirdnet_themes::BUILTINS {
            let t = Theme::new(&load(name).unwrap(), Variant::Dark, Depth::TrueColor, true);
            assert_ne!(t.surface, t.bg, "{name}");
            assert_ne!(t.add_bg, t.remove_bg, "{name}");
        }
        assert!(load("nope").unwrap_err().contains("bragi"));
    }
}
