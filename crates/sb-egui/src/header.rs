//! The toolbar of the web version: logo, project name, the views, the state of the project and the file actions.

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Vec2};
use sb_model::Severity;

use crate::app::{App, Page};
use crate::files::{ask_new_project, export_code, pick_project, save_as};
use crate::icons::icon;
use crate::theme::*;
use crate::widgets::{ghost, primary};

/// The views the tabs switch between: (page, label, icon).
pub const TABS: [(Page, &str, &str); 4] = [
    (Page::Architecture, "Architecture", "graph"),
    (Page::Tables, "Tables", "table"),
    (Page::Mcp, "MCP", "plug"),
    (Page::Code, "Code généré", "code"),
];

pub fn toolbar(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::top("toolbar")
        .exact_size(52.0)
        .frame(
            Frame::new()
                .fill(PANEL)
                .inner_margin(Margin::symmetric(12, 0))
                .stroke(Stroke::new(1.0, BORDER)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                bar(app, ui);
            });
        });
}

fn bar(app: &mut App, ui: &mut egui::Ui) {
    // The brand.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 9.0;
        ui.add(
            egui::Image::new(egui::include_image!("../data/service-builder.png"))
                .fit_to_exact_size(Vec2::splat(26.0)),
        );
        ui.label(RichText::new("Service Builder").font(bold(MD)));
    });

    // The name of the project: edited in place, committed with Enter or when the focus leaves.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let name = app.state.project.name.clone();
        if let Some(new) = project_name(ui, &name)
            && !new.is_empty()
        {
            app.edit(|p| p.name = new);
        }
        if app.state.dirty {
            ui.label(RichText::new("●").size(11.0).color(ACCENT))
                .on_hover_text("Modifications non enregistrées");
        }
        let file = app
            .state
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("non enregistré".to_string(), |n| {
                n.to_string_lossy().into_owned()
            });
        ui.label(RichText::new(file).font(mono(SM)).color(FAINT));
    });

    tabs(app, ui);

    // From the right end.
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        menu(app, ui);

        let errors = app
            .analysis()
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let export =
            primary(ui, "export", "Exporter le code", errors == 0).on_hover_text(if errors == 0 {
                "Générer le workspace Cargo"
            } else {
                "Corrigez les erreurs avant d'exporter"
            });
        if export.clicked() {
            export_code(app);
        }

        if let Some(update) = &app.state.update {
            let version = format!("v{}", update.version.trim_start_matches('v'));
            let b = egui::Button::image_and_text(
                icon("update", 15.0, ACCENT),
                RichText::new(format!("Mettre à jour  {version}")).color(ACCENT),
            )
            .fill(alpha(ACCENT, 26))
            .stroke(Stroke::new(1.0, alpha(ACCENT, 90)))
            .corner_radius(CornerRadius::same(8));
            if ui
                .add_enabled(!app.updating, b)
                .on_hover_text("Installer la nouvelle version")
                .clicked()
            {
                crate::updates::offer_update(app);
            }
        }

        // The file actions, right to left.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ghost(ui, "copy", "", "Enregistrer sous… (Ctrl+Maj+S)").clicked() {
                    save_as(app);
                }
                if ghost(ui, "save", "", "Enregistrer (Ctrl+S)").clicked() {
                    app.save();
                }
                if ghost(ui, "open", "", "Ouvrir (Ctrl+O)").clicked() {
                    app.guard(pick_project);
                }
                if ghost(
                    ui,
                    "schema",
                    "",
                    "Charger l’exemple complet : tous les types de blocs",
                )
                .clicked()
                {
                    app.guard(|app| app.load_sample("showcase"));
                }
                if ghost(ui, "sparkles", "", "Charger le projet d’exemple").clicked() {
                    app.guard(|app| app.load_sample("orders"));
                }
                if ghost(ui, "new", "", "Nouveau projet vide").clicked() {
                    app.guard(ask_new_project);
                }
            });
        });

        pill(app, ui);
    });
}

/// The name of the project, edited in place: plain text until hovered or focused (`entry.project-name`).
fn project_name(ui: &mut egui::Ui, current: &str) -> Option<String> {
    let id = ui.make_persistent_id("project-name");
    let key = id.with("buffer");
    let focused = ui.memory(|m| m.has_focus(id));
    let mut text: String = if focused {
        ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| current.to_string())
    } else {
        current.to_string()
    };
    let background = ui.painter().add(egui::Shape::Noop);
    let r = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(semibold(BASE))
            .margin(Margin::symmetric(8, 4))
            .vertical_align(egui::Align::Center)
            .min_size(Vec2::new(150.0, 26.0))
            .desired_width(150.0),
    );
    if r.hovered() || r.has_focus() {
        ui.painter().set(
            background,
            egui::epaint::RectShape::new(
                r.rect,
                CornerRadius::same(6),
                BG,
                Stroke::new(1.0, BORDER),
                egui::StrokeKind::Inside,
            ),
        );
    }
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, text.clone()));
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(key));
        let new = text.trim().to_string();
        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) && new != current {
            return Some(new);
        }
    }
    None
}

/// The views, in a dark track with the current one raised.
fn tabs(app: &mut App, ui: &mut egui::Ui) {
    // The API page is the architecture, zoomed on a route; the flows list belongs to it too.
    let current = match app.page {
        Page::Endpoint | Page::Flows => Page::Architecture,
        p => p,
    };
    Frame::new()
        .fill(BG)
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (page, label, icon_name) in TABS {
                    let on = current == page;
                    let galley = ui
                        .painter()
                        .layout_no_wrap(label.to_string(), prop(BASE), TEXT);
                    let size = Vec2::new(12.0 + 15.0 + 6.0 + galley.size().x + 12.0, 26.0);
                    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
                    let hovered = r.hovered();
                    if on {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(7), RAISED);
                    }
                    let color = if on || hovered { TEXT } else { MUTED };
                    let icon_rect = egui::Rect::from_center_size(
                        rect.left_center() + Vec2::new(12.0 + 7.5, 0.0),
                        Vec2::splat(15.0),
                    );
                    crate::icons::paint(ui, icon_name, icon_rect, color);
                    ui.painter().text(
                        rect.left_center() + Vec2::new(12.0 + 15.0 + 6.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        label,
                        prop(BASE),
                        color,
                    );
                    if r.on_hover_text(label).clicked() {
                        app.page = page;
                    }
                }
            });
        });
}

/// `Valide`, or the number of errors and warnings.
fn pill(app: &App, ui: &mut egui::Ui) {
    let a = app.analysis();
    let errors = a
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warnings = a.diagnostics.len() - errors;
    let (text, icon_name, color) = match (errors, warnings) {
        (0, 0) => ("Valide".to_string(), "ok", OK),
        (0, w) => (
            format!("{w} avertissement{}", if w > 1 { "s" } else { "" }),
            "warn",
            WARN,
        ),
        (e, _) => (
            format!("{e} erreur{}", if e > 1 { "s" } else { "" }),
            "error",
            ERR,
        ),
    };
    let fill = alpha(color, if errors > 0 { 31 } else { 26 });
    let galley = ui.painter().layout_no_wrap(text, semibold(SM), color);
    let size = Vec2::new(10.0 + 14.0 + 6.0 + galley.size().x + 10.0, 26.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(255), fill);
    let ir = egui::Rect::from_center_size(rect.left_center() + Vec2::new(10.0 + 7.0, 0.0), Vec2::splat(14.0));
    crate::icons::paint(ui, icon_name, ir, color);
    let at = egui::pos2(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, color);
}

/// The less used actions: a menu at the end.
fn menu(app: &mut App, ui: &mut egui::Ui) {
    let button = egui::Button::image(icon("grip", 15.0, MUTED))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .min_size(Vec2::splat(30.0));
    egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        if ui.button("Renommer le projet…").clicked() {
            app.later(crate::files::rename_project);
        }
        if ui.button("Rechercher une mise à jour").clicked() {
            app.later(|app| crate::updates::check_for_update(app, true));
        }
    });
}
