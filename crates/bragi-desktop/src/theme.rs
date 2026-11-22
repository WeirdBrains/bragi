//! Colour roles as `0xRRGGBB`, matching the Flutter app's dark theme (`app/lib/src/theme.dart`) and brand/BRAND.md.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub background: u32,
    pub surface: u32,
    pub surface_variant: u32,
    pub text_primary: u32,
    pub text_secondary: u32,
    pub accent: u32,
    pub border: u32,
    pub selection: u32,
    pub error: u32,
    /// Glacier, the colour Bragi shares with Ymir; links.
    pub link: u32,
    /// Done and found (a node connected): the Flutter app's `warm`.
    pub success: u32,
    /// Needs a look, not an error (a node not found yet): the Flutter app's `busy`.
    pub warning: u32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: 0x14161B,
            surface: 0x1C1F26,
            surface_variant: 0x2A2E37,
            text_primary: 0xF4F1E8,
            text_secondary: 0xA9ABB3,
            accent: 0xF5B83D,
            border: 0x3A3F4B,
            selection: 0x5A4518,
            error: 0xFF5C70,
            link: 0x7FD4F5,
            success: 0x4CD07D,
            warning: 0xFF9F43,
        }
    }
}

impl Theme {
    /// The colours of a `bragi_core` palette (a weirdnet-themes theme): the same twelve roles the
    /// shell paints with, so any theme restyles the whole window.
    pub fn from_palette(p: &bragi_core::Palette) -> Self {
        Self {
            background: p.rgb("bg"),
            surface: p.rgb("surface"),
            surface_variant: p.raised_rgb(),
            text_primary: p.rgb("fg"),
            text_secondary: p.rgb("muted"),
            accent: p.rgb("accent"),
            border: p.rgb("border"),
            selection: p.rgb("selection_bg"),
            error: p.rgb("error"),
            link: p.rgb("link"),
            success: p.rgb("success"),
            warning: p.rgb("warning"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bragi_theme_is_the_default_look() {
        let a = bragi_core::Appearance::open(std::path::Path::new("/nonexistent-bragi-test"));
        let p = a.palette_for("bragi", weirdnet_themes::Variant::Dark).unwrap();
        assert_eq!(Theme::from_palette(&p), Theme::default());
    }
}
