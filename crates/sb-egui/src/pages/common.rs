//! Building blocks shared by the pages, ported from the GTK version's `inspector.rs` and `editors.rs` helpers.
//! GTK → egui correspondence (use these, do not re-invent them in a page):
//!
//! | GTK (`inspector.rs` / `editors.rs`)    | here                                                             |
//! |-----------------------------------------|------------------------------------------------------------------|
//! | `section(title, icon, action)`          | `section(ui, title, icon, |ui| action, |ui| body)`               |
//! | `icon_button(name, tip, danger, f)`     | `if icon_btn(ui, name, tip, danger).clicked() { … }`             |
//! | `field(label, control, hint)`           | `field(ui, label, hint, |ui| control)`                           |
//! | `empty(text)`                           | `empty(ui, text)`                                                |
//! | `commit_entry(value, mono, ph, allow, f)` | `if let Some(v) = commit_entry(ui, id, value, mono, ph, allow) { … }` |
//! | `number(value, min, max, f)`            | `if let Some(n) = number(ui, id, value, min, max) { … }`        |
//! | `choice(options, selected, f)`          | `if let Some(v) = choice(ui, id, options, selected) { … }`      |
//! | `card(ui, id, tone, title, summary, actions, body)` | `card(app, ui, id, tone, title, summary, |app, ui| actions, |app, ui| body)` |
//! | `link(text, f)`                         | `if link(ui, text).clicked() { … }`                              |
//! | `danger_button(text, f)`                | `if danger_button(ui, text).clicked() { … }`                     |
//! | `editors::card(title, sub, indent, actions)` | `list_card(ui, title, sub, indent, &["Modifier", "Supprimer"])` → index clicked |
//! | `editors::add_row(text, f)`             | `if widgets::add_row(ui, text).clicked() { … }`                  |
//! | `unique`, `add_message`                 | same, pure                                                       |
//!
//! `id`s: anything hashable and stable across frames for the same thing (e.g. `("svc-port", service_name)`).

use eframe::egui::{
    self, Color32, CornerRadius, Frame, Margin, Response, RichText, Sense, Stroke, Ui, Vec2,
};
use sb_model::{Field, FieldType, Message, MessageKind, Project};

use crate::fonts::Weight;
use crate::app::App;
use crate::icons::icon;
use crate::theme::*;

/// A titled block of the inspector: an icon, a small uppercase title, an optional action on the right; then the body.
pub fn section<R>(
    ui: &mut Ui,
    title: &str,
    icon_name: &str,
    action: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add(icon(icon_name, 14.0, MUTED));
        ui.label(
            RichText::new(title.to_uppercase())
                .size(11.0)
                .semibold()
                .color(MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), action);
    });
    ui.add_space(10.0);
    let r = ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        body(ui)
    });
    ui.add_space(6.0);
    r.inner
}

/// The line between two sections.
pub fn separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 9.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, BORDER));
}

/// A small icon button (`.icon-btn`); `danger` turns it red on hover.
pub fn icon_btn(ui: &mut Ui, name: &str, tip: &str, danger: bool) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(6),
            if danger {
                alpha(ERR, 38)
            } else {
                alpha(RAISED, 200)
            },
        );
    }
    let color = if danger && r.hovered() { ERR } else { MUTED };
    crate::icons::paint(
        ui,
        name,
        egui::Rect::from_center_size(rect.center(), Vec2::splat(15.0)),
        color,
    );
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tip.is_empty() {
        r
    } else {
        r.on_hover_text(tip)
    }
}

/// A label on the left (84 px), the control on the right, a faint hint under it.
pub fn field<R>(ui: &mut Ui, label: &str, hint: &str, control: impl FnOnce(&mut Ui) -> R) -> R {
    let r = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.allocate_ui_with_layout(
                Vec2::new(84.0, 24.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_size(Vec2::new(84.0, 24.0));
                    ui.add(egui::Label::new(RichText::new(label).color(MUTED)).truncate());
                },
            );
            control(ui)
        })
        .inner;
    if !hint.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(94.0);
            ui.add(egui::Label::new(RichText::new(hint).size(11.0).color(FAINT)).wrap());
        });
    }
    r
}

pub fn empty(ui: &mut Ui, text: &str) {
    Frame::new()
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).size(SM).color(FAINT)).wrap());
        });
}

/// Text that applies on Enter or when the focus leaves; Escape gives the old value back. An empty text is refused
/// unless `allow_empty`. Takes the whole width left.
pub fn commit_entry(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    value: &str,
    mono: bool,
    placeholder: &str,
    allow_empty: bool,
) -> Option<String> {
    let id = ui.make_persistent_id(id);
    let key = id.with("sb-buffer");
    let focused = ui.memory(|m| m.has_focus(id));
    let mut text: String = if focused {
        ui.data(|d| d.get_temp::<String>(key))
            .unwrap_or_else(|| value.to_string())
    } else {
        value.to_string()
    };
    let mut edit = egui::TextEdit::singleline(&mut text)
        .id(id)
        .desired_width(ui.available_width())
        .hint_text(placeholder)
        .margin(Margin::symmetric(8, 5));
    if mono {
        edit = edit.font(crate::theme::mono(SM));
    }
    let r = ui.add(edit);
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, text.clone()));
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(key));
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return None;
        }
        let v = text.trim().to_string();
        if v != value && (allow_empty || !v.is_empty()) {
            return Some(v);
        }
    }
    None
}

/// A whole number, applied when committed (Enter / focus out), kept within `min..=max`.
pub fn number(ui: &mut Ui, id: impl egui::AsIdSalt, value: i64, min: i64, max: i64) -> Option<i64> {
    let v = commit_entry(ui, id, &value.to_string(), false, "", false)?;
    v.parse::<i64>()
        .ok()
        .map(|n| n.clamp(min, max))
        .filter(|n| *n != value)
}

/// A list of choices; `selected` may be absent from `options` (it is then shown as is).
pub fn choice(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    options: &[String],
    selected: &str,
) -> Option<String> {
    let mut picked = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected)
        .width(ui.available_width().max(80.0))
        .show_ui(ui, |ui| {
            for o in options {
                if ui.selectable_label(o == selected, o).clicked() && o != selected {
                    picked = Some(o.clone());
                }
            }
        });
    picked
}

/// A card with a chevron, a title (mono, bold), a summary and actions; the body shows when it is open.
/// Open cards are remembered by `id` in `app.open` (so an edit does not close them). `tone`: the colour of the
/// left edge (`SERVICE` for an RPC, `OK` for a consumer…) or `None`.
#[allow(clippy::too_many_arguments)]
pub fn card(
    app: &mut App,
    ui: &mut Ui,
    id: &str,
    tone: Option<Color32>,
    title: &str,
    summary: &str,
    actions: impl FnOnce(&mut App, &mut Ui),
    body: impl FnOnce(&mut App, &mut Ui),
) {
    // `SB_OPEN_ALL` (screenshots): every card open.
    let open = app.open.contains(id) || std::env::var_os("SB_OPEN_ALL").is_some();
    let r = Frame::new()
        .fill(PANEL2)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let head = ui.horizontal(|ui| {
                let toggle = Frame::new()
                    .inner_margin(Margin::symmetric(6, 9))
                    .show(ui, |ui| {
                        // Room for the actions on the right: the summary is cut before them.
                        ui.set_max_width(ui.available_width() - 70.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.add(icon(if open { "down" } else { "right" }, 14.0, FAINT));
                            ui.label(RichText::new(title).font(mono_bold(SM)));
                            ui.add(
                                egui::Label::new(RichText::new(summary).size(SM).color(MUTED))
                                    .truncate(),
                            );
                        });
                    });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    actions(app, ui)
                });
                toggle.response
            });
            let toggle = head.inner.interact(Sense::click());
            if toggle.clicked() {
                if open {
                    app.open.remove(id);
                } else {
                    app.open.insert(id.to_string());
                }
            }
            if open {
                let rect = ui.available_rect_before_wrap();
                ui.painter()
                    .hline(rect.x_range(), rect.top(), Stroke::new(1.0, BORDER));
                Frame::new()
                    .fill(alpha(CODE_BG, 115))
                    .inner_margin(Margin {
                        left: 12,
                        right: 12,
                        top: 4,
                        bottom: 12,
                    })
                    .corner_radius(CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: 10,
                        se: 10,
                    })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 8.0;
                        body(app, ui);
                    });
            }
        });
    if let Some(tone) = tone {
        let rect = r.response.rect;
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
            CornerRadius {
                nw: 10,
                sw: 10,
                ne: 0,
                se: 0,
            },
            tone,
        );
    }
    ui.add_space(2.0);
}

/// A text button in the colour of the services (`.link-btn`).
pub fn link(ui: &mut Ui, text: &str) -> Response {
    let r = ui.add(egui::Label::new(RichText::new(text).color(SERVICE)).sense(Sense::click()));
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    if r.hovered() {
        let rect = r.rect;
        ui.painter()
            .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, SERVICE));
    }
    r
}

/// The red outlined button (`.danger-btn`), with space above it.
pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    ui.add_space(22.0);
    ui.add(
        egui::Button::new(RichText::new(text).color(ERR))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, alpha(ERR, 90)))
            .corner_radius(CornerRadius::same(8)),
    )
}

/// A row of a list page (`editors::card`): a title, a subtitle under it, buttons on the right.
/// Returns the index of the button clicked. A button named `Supprimer` is red.
pub fn list_card(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    indent: f32,
    actions: &[&str],
) -> Option<usize> {
    let mut clicked = None;
    Frame::new()
        .fill(if indent > 0.0 { PANEL2 } else { PANEL })
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin {
            left: (12.0 + indent) as i8,
            right: 12,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).semibold());
                    if !subtitle.is_empty() {
                        ui.label(RichText::new(subtitle).color(MUTED));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (i, name) in actions.iter().enumerate().rev() {
                        let text = if *name == "Supprimer" {
                            RichText::new(*name).color(ERR)
                        } else {
                            RichText::new(*name)
                        };
                        if ui.button(text).clicked() {
                            clicked = Some(i);
                        }
                    }
                });
            });
        });
    clicked
}

pub fn unique(base: &str, taken: &[&str]) -> String {
    if !taken.contains(&base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base}{i}"))
        .find(|n| !taken.contains(&n.as_str()))
        .unwrap_or_default()
}

pub fn add_message(p: &mut Project, base: &str, kind: MessageKind) -> String {
    let taken: Vec<&str> = p.messages.iter().map(|m| m.name.as_str()).collect();
    let name = unique(base, &taken);
    p.messages.push(Message {
        name: name.clone(),
        kind,
        fields: vec![Field::new("id", FieldType::String)],
    });
    name
}
