//! Colour handling shared by the terminal engine and the UI.
//!
//! Tabby stores colour schemes as hex strings; the terminal additionally needs
//! the standard xterm 256-colour cube and grayscale ramp.

use serde::{Deserialize, Serialize};

/// An 8-bit sRGB colour. Kept independent from the UI framework so the core can
/// be used headlessly (tests, CLI tools, future frontends).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb8 {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn from_u32(value: u32) -> Self {
        Self {
            r: (value >> 16) as u8,
            g: (value >> 8) as u8,
            b: value as u8,
        }
    }

    pub fn to_u32(self) -> u32 {
        (u32::from(self.r) << 16) | (u32::from(self.g) << 8) | u32::from(self.b)
    }

    /// Parse `#rrggbb`, `rrggbb`, `#rgb` or `rgb`.
    pub fn parse_hex(value: &str) -> Option<Self> {
        let value = value.trim().trim_start_matches('#');
        match value.len() {
            3 => {
                let r = u8::from_str_radix(&value[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&value[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&value[2..3], 16).ok()? * 17;
                Some(Self::new(r, g, b))
            }
            6 => Some(Self::from_u32(u32::from_str_radix(value, 16).ok()?)),
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Mix towards `other` by `amount` in 0..=1.
    pub fn mix(self, other: Rgb8, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| {
            (f32::from(a) + (f32::from(b) - f32::from(a)) * amount)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Self::new(
            mix(self.r, other.r),
            mix(self.g, other.g),
            mix(self.b, other.b),
        )
    }

    /// Perceived luminance, useful for choosing contrasting text.
    pub fn luminance(self) -> f32 {
        let channel = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }
}

/// A Tabby-compatible terminal colour scheme.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorScheme {
    #[serde(default = "ColorScheme::default_name")]
    pub name: String,
    #[serde(default = "ColorScheme::default_foreground")]
    pub foreground: String,
    #[serde(default = "ColorScheme::default_background")]
    pub background: String,
    #[serde(default = "ColorScheme::default_cursor")]
    pub cursor: String,
    /// The 16 ANSI colours.
    #[serde(default = "ColorScheme::default_colors")]
    pub colors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_foreground: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_accent: Option<String>,
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self::tabby_default()
    }
}

impl ColorScheme {
    fn default_name() -> String {
        "Tabby Default".into()
    }
    fn default_foreground() -> String {
        "#cacaca".into()
    }
    fn default_background() -> String {
        "#171717".into()
    }
    fn default_cursor() -> String {
        "#bbbbbb".into()
    }
    fn default_colors() -> Vec<String> {
        Self::default_ansi().iter().map(|c| c.to_hex()).collect()
    }

    fn default_ansi() -> [Rgb8; 16] {
        [
            Rgb8::new(0x00, 0x00, 0x00),
            Rgb8::new(0xff, 0x61, 0x5a),
            Rgb8::new(0xb1, 0xe9, 0x69),
            Rgb8::new(0xeb, 0xd9, 0x9c),
            Rgb8::new(0x5d, 0xa9, 0xf6),
            Rgb8::new(0xe8, 0x6a, 0xff),
            Rgb8::new(0x82, 0xff, 0xf7),
            Rgb8::new(0xde, 0xda, 0xcf),
            Rgb8::new(0x31, 0x31, 0x31),
            Rgb8::new(0xf5, 0x8c, 0x80),
            Rgb8::new(0xdd, 0xf8, 0x8f),
            Rgb8::new(0xee, 0xe5, 0xb2),
            Rgb8::new(0xa5, 0xc7, 0xff),
            Rgb8::new(0xdd, 0xaa, 0xff),
            Rgb8::new(0xb7, 0xff, 0xf9),
            Rgb8::new(0xff, 0xff, 0xff),
        ]
    }

    /// The scheme Tabby ships as `TerminalColorScheme` default.
    pub fn tabby_default() -> Self {
        Self {
            name: Self::default_name(),
            foreground: Self::default_foreground(),
            background: Self::default_background(),
            cursor: Self::default_cursor(),
            colors: Self::default_colors(),
            selection: None,
            selection_foreground: None,
            cursor_accent: None,
        }
    }

    /// The light scheme from `tabby-terminal/src/colorSchemes.ts`.
    pub fn tabby_light() -> Self {
        let colors = [
            "#000000", "#c82829", "#718c00", "#eab700", "#4271ae", "#8959a8", "#3e999f", "#ffffff",
            "#000000", "#c82829", "#718c00", "#eab700", "#4271ae", "#8959a8", "#3e999f", "#ffffff",
        ];
        Self {
            name: "Tabby Default Light".into(),
            foreground: "#4d4d4c".into(),
            background: "#ffffff".into(),
            cursor: "#4d4d4c".into(),
            colors: colors.iter().map(|c| (*c).to_string()).collect(),
            selection: None,
            selection_foreground: None,
            cursor_accent: None,
        }
    }

    /// A small set of well-known schemes so the UI has something to switch to.
    pub fn builtin_schemes() -> Vec<ColorScheme> {
        vec![
            Self::tabby_default(),
            Self::tabby_light(),
            Self::solarized_dark(),
            Self::gruvbox_dark(),
            Self::dracula(),
        ]
    }

    fn from_parts(name: &str, fg: &str, bg: &str, cursor: &str, colors: &[&str; 16]) -> Self {
        Self {
            name: name.into(),
            foreground: fg.into(),
            background: bg.into(),
            cursor: cursor.into(),
            colors: colors.iter().map(|c| (*c).to_string()).collect(),
            selection: None,
            selection_foreground: None,
            cursor_accent: None,
        }
    }

    pub fn solarized_dark() -> Self {
        Self::from_parts(
            "Solarized Dark",
            "#839496",
            "#002b36",
            "#839496",
            &[
                "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198",
                "#eee8d5", "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4",
                "#93a1a1", "#fdf6e3",
            ],
        )
    }

    pub fn gruvbox_dark() -> Self {
        Self::from_parts(
            "Gruvbox Dark",
            "#ebdbb2",
            "#282828",
            "#ebdbb2",
            &[
                "#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a",
                "#a89984", "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b",
                "#8ec07c", "#ebdbb2",
            ],
        )
    }

    pub fn dracula() -> Self {
        Self::from_parts(
            "Dracula",
            "#f8f8f2",
            "#282a36",
            "#f8f8f2",
            &[
                "#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd",
                "#f8f8f2", "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df",
                "#a4ffff", "#ffffff",
            ],
        )
    }

    pub fn foreground_rgb(&self) -> Rgb8 {
        Rgb8::parse_hex(&self.foreground).unwrap_or(Rgb8::new(0xca, 0xca, 0xca))
    }

    pub fn background_rgb(&self) -> Rgb8 {
        Rgb8::parse_hex(&self.background).unwrap_or(Rgb8::new(0x17, 0x17, 0x17))
    }

    pub fn cursor_rgb(&self) -> Rgb8 {
        Rgb8::parse_hex(&self.cursor).unwrap_or_else(|| self.foreground_rgb())
    }

    pub fn selection_rgb(&self) -> Rgb8 {
        self.selection
            .as_deref()
            .and_then(Rgb8::parse_hex)
            .unwrap_or_else(|| self.foreground_rgb().mix(self.background_rgb(), 0.72))
    }

    pub fn ansi(&self, index: usize) -> Rgb8 {
        self.colors
            .get(index)
            .and_then(|c| Rgb8::parse_hex(c))
            .unwrap_or_else(|| {
                let defaults = Self::default_ansi();
                defaults[index % 16]
            })
    }

    /// xterm 256-colour palette entry.
    pub fn palette(&self, index: u8) -> Rgb8 {
        match index {
            0..=15 => self.ansi(index as usize),
            16..=231 => {
                let i = u32::from(index) - 16;
                let steps = [0u8, 95, 135, 175, 215, 255];
                let r = steps[(i / 36) as usize];
                let g = steps[((i % 36) / 6) as usize];
                let b = steps[(i % 6) as usize];
                Rgb8::new(r, g, b)
            }
            _ => {
                let level = 8 + (u32::from(index) - 232) * 10;
                let level = level.min(255) as u8;
                Rgb8::new(level, level, level)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing_handles_short_and_long_forms() {
        assert_eq!(Rgb8::parse_hex("#fff"), Some(Rgb8::new(255, 255, 255)));
        assert_eq!(Rgb8::parse_hex("0a0b0c"), Some(Rgb8::new(10, 11, 12)));
        assert_eq!(Rgb8::parse_hex("#xyz"), None);
    }

    #[test]
    fn xterm_palette_matches_standard_cube() {
        let scheme = ColorScheme::tabby_default();
        assert_eq!(scheme.palette(16), Rgb8::new(0, 0, 0));
        assert_eq!(scheme.palette(231), Rgb8::new(255, 255, 255));
        assert_eq!(scheme.palette(232), Rgb8::new(8, 8, 8));
        assert_eq!(scheme.palette(15), Rgb8::new(255, 255, 255));
    }

    #[test]
    fn schemes_roundtrip_yaml() {
        let scheme = ColorScheme::tabby_light();
        let text = serde_yaml::to_string(&scheme).unwrap();
        let back: ColorScheme = serde_yaml::from_str(&text).unwrap();
        assert_eq!(scheme, back);
    }
}
