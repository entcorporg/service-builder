//! The building blocks shared by every page: buttons, cards, titles, inputs that commit, the toast.
//!
//! The GTK version styled widgets with CSS classes; here each class of the web version is a function.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Frame, Margin, Response, RichText, Sense, Stroke, Ui,
    Vec2, WidgetText,
};

use crate::app::App;
use crate::icons::icon;
use crate::theme::{self, *};

// ---- text ----

/// The small uppercase titles of the panels (`.palette-title`, `.section-title`).
pub fn section_title(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(11.0)
            .color(FAINT)
            .extra_letter_spacing(0.9)
            .family(egui::FontFamily::Name(crate::fonts::BOLD.into())),
    );
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(MUTED)
}

pub fn faint(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(FAINT)
}

pub fn mono(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).font(theme::mono(SM))
}

/// `None` or empty: an em dash.
pub fn names<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let all: Vec<&str> = items.into_iter().collect();
    if all.is_empty() {
        "—".into()
    } else {
        all.join(", ")
    }
}

// ---- buttons ----

/// A flat button holding an icon and, optionally, a text (`button.ghost`).
pub fn ghost(ui: &mut Ui, icon_name: &str, text: &str, tip: &str) -> Response {
    ghost_colored(ui, icon_name, text, tip, TEXT)
}

pub fn ghost_colored(
    ui: &mut Ui,
    icon_name: &str,
    text: &str,
    tip: &str,
    color: Color32,
) -> Response {
    let button = if text.is_empty() {
        egui::Button::image(icon(icon_name, 15.0, color))
    } else {
        egui::Button::image_and_text(
            icon(icon_name, 15.0, color),
            RichText::new(text).color(color),
        )
    };
    let r = ui.add(
        button
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(8))
            .min_size(Vec2::new(30.0, 30.0)),
    );
    if tip.is_empty() {
        r
    } else {
        r.on_hover_text(tip)
    }
}

/// The orange button of the main action (`button.primary`): a gradient from the accent to the second accent.
pub fn primary(ui: &mut Ui, icon_name: &str, text: &str, enabled: bool) -> Response {
    let font = theme::bold(BASE);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, INK);
    let icon_w = if icon_name.is_empty() {
        0.0
    } else {
        15.0 + 6.0
    };
    let size = Vec2::new(10.0 + icon_w + galley.size().x + 10.0, 30.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let fade = |c: Color32| {
            let c = if r.hovered() {
                c.gamma_multiply(1.0).lerp_to_gamma(Color32::WHITE, 0.08)
            } else {
                c
            };
            if enabled { c } else { c.gamma_multiply(0.45) }
        };
        gradient(ui.painter(), rect, 8.0, fade(ACCENT), fade(ACCENT2));
        let mut x = rect.left() + 10.0;
        if !icon_name.is_empty() {
            let ir = egui::Rect::from_center_size(
                egui::pos2(x + 7.5, rect.center().y),
                Vec2::splat(15.0),
            );
            crate::icons::paint(ui, icon_name, ir, INK);
            x += icon_w;
        }
        ui.painter().galley(
            egui::pos2(x, rect.center().y - galley.size().y / 2.0),
            galley,
            INK,
        );
    }
    if enabled {
        r.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        r
    }
}

/// A rounded rectangle filled with a horizontal gradient.
pub fn gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    radius: f32,
    from: Color32,
    to: Color32,
) {
    let r = radius.min(rect.height() / 2.0).min(rect.width() / 2.0);
    // The ends are rounded rectangles in the end colours; the middle is a gradient mesh.
    let left = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + 2.0 * r, rect.bottom()));
    let right = egui::Rect::from_min_max(egui::pos2(rect.right() - 2.0 * r, rect.top()), rect.max);
    painter.rect_filled(left, CornerRadius::same(r as u8), from);
    painter.rect_filled(right, CornerRadius::same(r as u8), to);
    let mid = egui::Rect::from_min_max(
        egui::pos2(rect.left() + r, rect.top()),
        egui::pos2(rect.right() - r, rect.bottom()),
    );
    let t = |x: f32| (x - rect.left()) / rect.width();
    let at = |x: f32| from.lerp_to_gamma(to, t(x));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(mid.left_top(), at(mid.left()));
    mesh.colored_vertex(mid.right_top(), at(mid.right()));
    mesh.colored_vertex(mid.right_bottom(), at(mid.right()));
    mesh.colored_vertex(mid.left_bottom(), at(mid.left()));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// A bordered button (`button` of the forms, `.btn`).
pub fn button(ui: &mut Ui, icon_name: &str, text: &str) -> Response {
    let b = if icon_name.is_empty() {
        egui::Button::new(text)
    } else {
        egui::Button::image_and_text(icon(icon_name, 14.0, TEXT), text)
    };
    ui.add(
        b.fill(PANEL2)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(7)),
    )
}

/// A small icon-only button, as on the rows of a list (delete, move up/down).
pub fn icon_button(ui: &mut Ui, icon_name: &str, tip: &str, color: Color32) -> Response {
    let r = ui.add(
        egui::Button::image(icon(icon_name, 13.0, color))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(6))
            .min_size(Vec2::new(22.0, 22.0)),
    );
    if tip.is_empty() {
        r
    } else {
        r.on_hover_text(tip)
    }
}

/// A dashed "+ add …" row (`.add-row`).
pub fn add_row(ui: &mut Ui, text: &str) -> Response {
    let width = ui.available_width();
    let (rect, r) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    let color = if r.hovered() { ACCENT } else { MUTED };
    ui.painter().rect_stroke(
        rect.shrink(0.5),
        CornerRadius::same(8),
        Stroke::new(1.0, if r.hovered() { ACCENT } else { BORDER_STRONG }),
        egui::StrokeKind::Inside,
    );
    let icon_rect =
        egui::Rect::from_center_size(rect.left_center() + Vec2::new(16.0, 0.0), Vec2::splat(13.0));
    crate::icons::paint(ui, "plus", icon_rect, color);
    ui.painter().text(
        rect.left_center() + Vec2::new(30.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        theme::prop(BASE),
        color,
    );
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A rounded tag (`.chip`): text on a tint of `color`.
pub fn chip(ui: &mut Ui, text: &str, color: Color32) -> Response {
    Frame::new()
        .fill(alpha(color, 30))
        .stroke(Stroke::new(1.0, alpha(color, 80)))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(7, 1))
        .show(ui, |ui| ui.label(RichText::new(text).size(XS).color(color)))
        .response
}

// ---- containers ----

/// A panel card (`.card`): panel2 fill, border, radius 10.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(PANEL2)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A collapsible card whose header is always visible; `open` is kept by the caller (an id in a set, usually).
/// Returns (header response, body result if open).
pub fn fold<R>(
    ui: &mut Ui,
    open: &mut bool,
    header: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui) -> R,
) -> (Response, Option<R>) {
    Frame::new()
        .fill(PANEL2)
        .stroke(Stroke::new(1.0, if *open { BORDER_STRONG } else { BORDER }))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let head = ui
                .horizontal(|ui| {
                    ui.add(icon(if *open { "down" } else { "right" }, 13.0, FAINT));
                    header(ui);
                })
                .response
                .interact(Sense::click());
            if head.clicked() {
                *open = !*open;
            }
            let out = if *open {
                ui.add_space(6.0);
                Some(body(ui))
            } else {
                None
            };
            (head, out)
        })
        .inner
}

/// A label and its input side by side, as in the forms of the inspector.
pub fn field_row<R>(ui: &mut Ui, label: &str, help: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let l = ui.add_sized([120.0, 22.0], egui::Label::new(muted(label)).truncate());
        if !help.is_empty() {
            l.on_hover_text(help);
        }
        add(ui)
    })
    .inner
}

// ---- inputs that commit ----
//
// The project is edited only when the user is done (Enter, or the focus leaves), never at each key: one entry in
// the history per change, and no analysis per keystroke. While the field has the focus, its text lives in egui's
// memory under its id.

fn buffer_id(id: egui::Id) -> egui::Id {
    id.with("sb-buffer")
}

/// A one-line text input showing `current`; returns the new text (trimmed) once, when it is committed and differs.
pub fn text_field(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    current: &str,
    width: f32,
) -> Option<String> {
    text_edit(ui, id, current, width, false, "")
}

pub fn text_field_hint(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    current: &str,
    width: f32,
    hint: &str,
) -> Option<String> {
    text_edit(ui, id, current, width, false, hint)
}

/// A multi-line monospace input; commits when the focus leaves.
pub fn text_area(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    current: &str,
    width: f32,
) -> Option<String> {
    text_edit(ui, id, current, width, true, "")
}

fn text_edit(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    current: &str,
    width: f32,
    multiline: bool,
    hint: &str,
) -> Option<String> {
    let id = ui.make_persistent_id(id);
    let key = buffer_id(id);
    let focused = ui.memory(|m| m.has_focus(id));
    let mut text: String = if focused {
        ui.data(|d| d.get_temp::<String>(key))
            .unwrap_or_else(|| current.to_string())
    } else {
        current.to_string()
    };
    let width = if width <= 0.0 {
        ui.available_width()
    } else {
        width
    };
    let edit = if multiline {
        egui::TextEdit::multiline(&mut text)
            .font(theme::mono(SM))
            .desired_rows(4)
    } else {
        egui::TextEdit::singleline(&mut text)
    };
    let r = ui.add(edit.id(id).desired_width(width).hint_text(hint));
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, text.clone()));
    }
    let enter = !multiline && r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if r.lost_focus() || enter {
        ui.data_mut(|d| d.remove::<String>(key));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let new = if multiline {
            text
        } else {
            text.trim().to_string()
        };
        if !escape && new != current {
            return Some(new);
        }
    }
    None
}

/// A number input; returns the new value once, when committed.
pub fn number_field(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    current: i64,
    min: i64,
    max: i64,
) -> Option<i64> {
    let text = text_field(ui, id, &current.to_string(), 80.0)?;
    text.parse::<i64>()
        .ok()
        .map(|n| n.clamp(min, max))
        .filter(|n| *n != current)
}

/// A drop-down of `options`; returns the option picked, when it differs from `current`.
pub fn choice(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    current: &str,
    options: &[String],
    width: f32,
) -> Option<String> {
    let mut picked = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(if current.is_empty() { "—" } else { current })
        .width(width)
        .show_ui(ui, |ui| {
            for o in options {
                if ui.selectable_label(o == current, o).clicked() && o != current {
                    picked = Some(o.clone());
                }
            }
        });
    picked
}

/// A checkbox; returns the new value when clicked.
pub fn check(ui: &mut Ui, current: bool, label: impl Into<WidgetText>) -> Option<bool> {
    let mut v = current;
    ui.checkbox(&mut v, label).changed().then_some(v)
}

/// A switch (the toggles of the MCP page and of the options).
pub fn switch(ui: &mut Ui, on: bool) -> Response {
    let size = Vec2::new(34.0, 18.0);
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    let t = ui.ctx().animate_bool_responsive(r.id, on);
    let fill =
        Color32::from(egui::Rgba::from(BORDER_STRONG) * (1.0 - t) + egui::Rgba::from(ACCENT) * t);
    ui.painter().rect_filled(rect, CornerRadius::same(9), fill);
    let x = egui::lerp((rect.left() + 9.0)..=(rect.right() - 9.0), t);
    ui.painter()
        .circle_filled(egui::pos2(x, rect.center().y), 7.0, TEXT);
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---- the toast ----

/// A message in the bottom right corner, for a few seconds (`.toast`).
pub fn toast(app: &App, ctx: &egui::Context) {
    let Some((text, _)) = &app.toast else {
        return;
    };
    egui::Area::new(egui::Id::unique("toast"))
        .order(egui::Order::Tooltip)
        .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-18.0, -52.0))
        .interactable(false)
        .show(ctx, |ui| {
            Frame::new()
                .fill(RAISED)
                .stroke(Stroke::new(1.0, BORDER_STRONG))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(14, 10))
                .shadow(egui::Shadow {
                    offset: [0, 6],
                    blur: 24,
                    spread: 0,
                    color: Color32::from_black_alpha(120),
                })
                .show(ui, |ui| {
                    ui.set_max_width(460.0);
                    ui.label(RichText::new(text).color(TEXT));
                });
        });
}
