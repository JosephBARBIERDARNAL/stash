//! A quiet, minimalist look for both light and dark mode.
//!
//! Text uses Atkinson Hyperlegible (designed by the Braille Institute for
//! low-vision readers) and every text color meets WCAG AA contrast (4.5:1)
//! on the backgrounds it is drawn on.

use std::sync::Arc;

use egui::epaint::text::{FontData, FontDefinitions, FontTweak, VariationCoords};
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, Style, TextStyle, Theme,
    Visuals, vec2,
};

use crate::skill::Status;

const REGULAR: &[u8] = include_bytes!("../assets/fonts/AtkinsonHyperlegible-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../assets/fonts/AtkinsonHyperlegible-Bold.ttf");
const MONO: &[u8] = include_bytes!("../assets/fonts/AtkinsonHyperlegibleMono.ttf");

/// Smallest text size used anywhere in the app.
pub const SMALL: f32 = 12.5;

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
}

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xFB, 0xFB, 0xFA),
    surface: Color32::from_rgb(0xF1, 0xF1, 0xEF),
    hover: Color32::from_rgb(0xE6, 0xE6, 0xE3),
    border: Color32::from_rgb(0xDD, 0xDD, 0xD8),
    text: Color32::from_rgb(0x1C, 0x1C, 0x1A),
    weak: Color32::from_rgb(0x6B, 0x6B, 0x66),
    accent: Color32::from_rgb(0x25, 0x63, 0xD9),
    ok: Color32::from_rgb(0x16, 0x77, 0x33),
    warn: Color32::from_rgb(0x8A, 0x5C, 0x00),
    err: Color32::from_rgb(0xCF, 0x22, 0x2E),
};

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x16, 0x16, 0x16),
    surface: Color32::from_rgb(0x20, 0x20, 0x20),
    hover: Color32::from_rgb(0x2E, 0x2E, 0x2E),
    border: Color32::from_rgb(0x33, 0x33, 0x33),
    text: Color32::from_rgb(0xEC, 0xEC, 0xEA),
    weak: Color32::from_rgb(0x9A, 0x9A, 0x96),
    accent: Color32::from_rgb(0x7A, 0xA5, 0xFF),
    ok: Color32::from_rgb(0x4C, 0xC3, 0x8A),
    warn: Color32::from_rgb(0xE5, 0xA6, 0x30),
    err: Color32::from_rgb(0xF2, 0x77, 0x77),
};

pub fn palette(dark: bool) -> Palette {
    if dark { DARK } else { LIGHT }
}

impl Palette {
    pub fn status(&self, status: Status) -> Color32 {
        match status {
            Status::Valid => self.ok,
            Status::Warnings => self.warn,
            Status::Invalid => self.err,
        }
    }
}

/// Font family for bold text.
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Install fonts and style for both themes.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    for theme in [Theme::Light, Theme::Dark] {
        ctx.style_mut_of(theme, |style| apply(style, theme == Theme::Dark));
    }
}

/// Atkinson Hyperlegible first, egui's built-in fonts as fallback for symbols.
fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let data = |bytes| Arc::new(FontData::from_static(bytes));
    fonts.font_data.insert("atkinson".into(), data(REGULAR));
    fonts.font_data.insert("atkinson-bold".into(), data(BOLD));
    fonts.font_data.insert(
        "atkinson-mono".into(),
        Arc::new(FontData::from_static(MONO).tweak(FontTweak {
            coords: VariationCoords::new([("wght", 400.0)]),
            ..Default::default()
        })),
    );

    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let with_fallbacks = |first: &str| {
        let mut list = vec![first.to_string()];
        list.extend(fallbacks.iter().cloned());
        list
    };
    fonts
        .families
        .insert(FontFamily::Proportional, with_fallbacks("atkinson"));
    fonts
        .families
        .insert(bold(), with_fallbacks("atkinson-bold"));
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "atkinson-mono".into());
    fonts
}

fn apply(style: &mut Style, dark: bool) {
    let p = palette(dark);

    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(SMALL, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(15.0, FontFamily::Proportional),
        ),
        (TextStyle::Heading, FontId::new(24.0, bold())),
        (
            TextStyle::Monospace,
            FontId::new(13.5, FontFamily::Monospace),
        ),
    ]
    .into();

    let s = &mut style.spacing;
    s.item_spacing = vec2(8.0, 6.0);
    s.button_padding = vec2(10.0, 5.0);
    s.interact_size.y = 28.0;
    s.window_margin = Margin::same(18);
    s.menu_margin = Margin::same(6);

    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.extreme_bg_color = p.surface;
    v.faint_bg_color = p.surface;
    v.code_bg_color = p.surface;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    let shadow_alpha = if dark { 120 } else { 28 };
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(shadow_alpha),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(shadow_alpha),
    };
    v.selection.bg_fill = p.accent.gamma_multiply(0.35);
    // Keyboard focus ring: clearly visible in both themes.
    v.selection.stroke = Stroke::new(2.0, p.accent);
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.err;
    v.override_text_color = None;

    let w = &mut v.widgets;
    for wv in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        wv.corner_radius = CornerRadius::same(6);
        wv.expansion = 0.0;
        wv.fg_stroke = Stroke::new(1.5, p.text);
    }
    w.noninteractive.bg_fill = p.bg;
    w.noninteractive.weak_bg_fill = p.bg;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);

    w.inactive.bg_fill = p.surface;
    w.inactive.weak_bg_fill = p.surface;
    w.inactive.bg_stroke = Stroke::NONE;

    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::NONE;

    w.active.bg_fill = p.border;
    w.active.weak_bg_fill = p.border;
    w.active.bg_stroke = Stroke::NONE;

    w.open.bg_fill = p.hover;
    w.open.weak_bg_fill = p.hover;
    w.open.bg_stroke = Stroke::NONE;

    style.visuals = v;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Color32) -> f64 {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn text_colors_meet_wcag_aa() {
        for p in [LIGHT, DARK] {
            for bg in [p.bg, p.surface] {
                for fg in [p.text, p.weak, p.ok, p.warn, p.err] {
                    let ratio = contrast(fg, bg);
                    assert!(ratio >= 4.5, "{fg:?} on {bg:?}: {ratio:.2}");
                }
            }
            assert!(contrast(p.text, p.hover) >= 4.5);
            assert!(contrast(p.accent, p.bg) >= 3.0, "focus ring is visible");
        }
    }

    #[test]
    fn fonts_are_bundled() {
        let defs = fonts();
        assert_eq!(defs.families[&FontFamily::Proportional][0], "atkinson");
        assert_eq!(defs.families[&FontFamily::Monospace][0], "atkinson-mono");
        assert_eq!(defs.families[&bold()][0], "atkinson-bold");
    }
}
