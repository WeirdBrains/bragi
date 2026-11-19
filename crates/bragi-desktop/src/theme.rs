//! Colour roles as `0xRRGGBB`, matching the Flutter app's dark theme (`app/lib/theme.dart`).

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
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: 0x0E0F14,
            surface: 0x16171F,
            surface_variant: 0x22232E,
            text_primary: 0xF3F3F8,
            text_secondary: 0xA6A8B8,
            accent: 0xF2B544,
            border: 0x3A3C4C,
            selection: 0x5A4518,
            error: 0xFF5C70,
        }
    }
}
