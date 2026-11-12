//! The look of the web version (ui/src/styles.css): deep blue-night panels, orange accent, small uppercase titles.
//! Every colour of the app is here; the GTK version's `data/css/*.css` is the reference for sizes and radii.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Visuals,
};

pub const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const BG: Color32 = hex(0x0f1119);
pub const PANEL: Color32 = hex(0x151826);
pub const PANEL2: Color32 = hex(0x1b1f31);
pub const RAISED: Color32 = hex(0x232842);
pub const BORDER: Color32 = hex(0x2a3050);
pub const BORDER_STRONG: Color32 = hex(0x3a4270);
pub const TEXT: Color32 = hex(0xe8eaf6);
pub const MUTED: Color32 = hex(0x8f97bd);
pub const FAINT: Color32 = hex(0x5c6490);
pub const ACCENT: Color32 = hex(0xff7a45);
pub const ACCENT2: Color32 = hex(0xff4d6d);
pub const SERVICE: Color32 = hex(0x7886ff);
pub const TOPIC: Color32 = hex(0xffb04a);
pub const OK: Color32 = hex(0x3ecf8e);
pub const WARN: Color32 = hex(0xffc53d);
pub const ERR: Color32 = hex(0xff6678);
/// The dark of the code and the JSON.
pub const CODE_BG: Color32 = hex(0x0c0e17);
/// The ink on the orange gradient.
pub const INK: Color32 = hex(0x1a0d0a);

/// Font sizes of the web version, in px (egui points are px at scale 1).
pub const XS: f32 = 10.5;
pub const SM: f32 = 12.0;
pub const BASE: f32 = 13.0;
pub const MD: f32 = 14.0;
pub const LG: f32 = 15.0;
pub const XL: f32 = 20.0;

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn prop(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(crate::fonts::MEDIUM.into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(crate::fonts::SEMIBOLD.into()))
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(crate::fonts::BOLD.into()))
}

pub fn mono_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(crate::fonts::MONO_BOLD.into()))
}

/// The colour with an alpha (0..=255), for tints of the accent or of a kind.
pub fn alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = BG; // text edits
    v.faint_bg_color = PANEL2;
    v.code_bg_color = CODE_BG;
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN;
    v.error_fg_color = ERR;
    v.selection.bg_fill = alpha(ACCENT, 90);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(2.0, ACCENT);
    let r = CornerRadius::same(7);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = PANEL2;
    w.inactive.weak_bg_fill = PANEL2;
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = RAISED;
    w.hovered.weak_bg_fill = RAISED;
    w.hovered.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = RAISED;
    w.active.weak_bg_fill = RAISED;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0, TEXT);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open.bg_fill = RAISED;
    w.open.weak_bg_fill = RAISED;
    w.open.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    w.open.corner_radius = r;
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, prop(XS)),
            (TextStyle::Body, prop(BASE)),
            (TextStyle::Button, prop(BASE)),
            (TextStyle::Heading, prop(LG)),
            (TextStyle::Monospace, mono(SM)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.window_margin = Margin::same(16);
        s.spacing.interact_size.y = 26.0;
        s.spacing.combo_width = 120.0;
        s.interaction.tooltip_delay = 0.4;
    });
}
