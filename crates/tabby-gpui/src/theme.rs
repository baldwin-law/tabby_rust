use gpui::{Hsla, rgb};

use tabby_core::{ColorScheme, Rgb8};

#[derive(Clone, Copy)]
pub struct Theme {
    pub background: Hsla,
    pub surface: Hsla,
    pub surface_active: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub error: Hsla,
}

pub fn hsla_from_rgb8(color: Rgb8) -> Hsla {
    rgb(color.to_u32()).into()
}

impl Theme {
    pub fn from_scheme(scheme: &ColorScheme) -> Self {
        let background = scheme.background_rgb();
        let foreground = scheme.foreground_rgb();
        let surface = background.mix(foreground, 0.06);
        let surface_active = background.mix(foreground, 0.12);
        let border = background.mix(foreground, 0.18);
        Self {
            background: hsla_from_rgb8(background),
            surface: hsla_from_rgb8(surface),
            surface_active: hsla_from_rgb8(surface_active),
            foreground: hsla_from_rgb8(foreground),
            muted: hsla_from_rgb8(foreground.mix(background, 0.45)),
            border: hsla_from_rgb8(border),
            accent: hsla_from_rgb8(scheme.ansi(4)),
            error: hsla_from_rgb8(scheme.ansi(1)),
        }
    }
}
