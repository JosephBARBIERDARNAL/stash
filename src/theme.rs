//! A quiet, minimalist look for both light and dark mode.

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, Style, TextStyle, Theme,
    Visuals, vec2,
};

use crate::skill::Status;

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
    hover: Color32::from_rgb(0xE8, 0xE8, 0xE5),
    border: Color32::from_rgb(0xE4, 0xE4, 0xE0),
    text: Color32::from_rgb(0x1C, 0x1C, 0x1A),
    weak: Color32::from_rgb(0x78, 0x78, 0x73),
    accent: Color32::from_rgb(0x2F, 0x6F, 0xEB),
    ok: Color32::from_rgb(0x2E, 0x9E, 0x5B),
    warn: Color32::from_rgb(0xC9, 0x8A, 0x12),
    err: Color32::from_rgb(0xD1, 0x43, 0x43),
};

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x16, 0x16, 0x16),
    surface: Color32::from_rgb(0x20, 0x20, 0x20),
    hover: Color32::from_rgb(0x2A, 0x2A, 0x2A),
    border: Color32::from_rgb(0x2C, 0x2C, 0x2C),
    text: Color32::from_rgb(0xE8, 0xE8, 0xE6),
    weak: Color32::from_rgb(0x8C, 0x8C, 0x88),
    accent: Color32::from_rgb(0x6E, 0x9B, 0xFF),
    ok: Color32::from_rgb(0x4C, 0xC3, 0x8A),
    warn: Color32::from_rgb(0xE5, 0xA6, 0x30),
    err: Color32::from_rgb(0xF0, 0x6A, 0x6A),
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

/// Apply stash's style to both themes.
pub fn install(ctx: &egui::Context) {
    for theme in [Theme::Light, Theme::Dark] {
        ctx.style_mut_of(theme, |style| apply(style, theme == Theme::Dark));
    }
}

fn apply(style: &mut Style, dark: bool) {
    let p = palette(dark);

    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(22.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
    ]
    .into();

    let s = &mut style.spacing;
    s.item_spacing = vec2(8.0, 6.0);
    s.button_padding = vec2(10.0, 5.0);
    s.interact_size.y = 26.0;
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
    v.selection.stroke = Stroke::new(1.0, p.accent);
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
        wv.fg_stroke = Stroke::new(1.0, p.text);
    }
    w.noninteractive.bg_fill = p.bg;
    w.noninteractive.weak_bg_fill = p.bg;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);

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
