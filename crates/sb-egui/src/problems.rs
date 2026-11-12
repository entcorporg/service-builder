//! The problems panel at the bottom: a chevron, the title and the counts; it opens on the list.

use eframe::egui::{self, Frame, Margin, RichText, Sense, Stroke};
use sb_model::Severity;

use crate::app::App;
use crate::icons::icon;
use crate::theme::*;

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    let a = app.analysis();
    let errors = a
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warnings = a.diagnostics.len() - errors;
    egui::Panel::bottom("problems")
        .resizable(false)
        .frame(Frame::new().fill(PANEL).stroke(Stroke::new(1.0, BORDER)))
        .show(ui, |ui| {
            let head = Frame::new()
                .inner_margin(Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        ui.add(icon(
                            if app.problems_open { "down" } else { "right" },
                            13.0,
                            FAINT,
                        ));
                        ui.label(RichText::new("PROBLÈMES").size(XS).strong().color(MUTED));
                        let count = |ui: &mut egui::Ui, n: String, icon_name: &str, color| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                ui.add(icon(icon_name, 12.0, color));
                                ui.label(RichText::new(n).size(SM).color(color));
                            });
                        };
                        if errors > 0 {
                            count(ui, errors.to_string(), "error", ERR);
                        }
                        if warnings > 0 {
                            count(ui, warnings.to_string(), "warn", WARN);
                        }
                        if errors == 0 && warnings == 0 {
                            count(ui, "Aucun problème".into(), "ok", OK);
                        }
                    });
                })
                .response
                .interact(Sense::click());
            if head.clicked() {
                app.problems_open = !app.problems_open;
            }
            if app.problems_open {
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        Frame::new()
                            .inner_margin(Margin::symmetric(16, 6))
                            .show(ui, |ui| {
                                if a.diagnostics.is_empty() {
                                    ui.label(RichText::new("Aucun problème").color(MUTED));
                                }
                                for d in &a.diagnostics {
                                    let (mark, color) = match d.severity {
                                        Severity::Error => ("Erreur", ERR),
                                        Severity::Warning => ("Avertissement", WARN),
                                    };
                                    ui.label(
                                        RichText::new(format!("{mark} · {}", d.code))
                                            .color(color)
                                            .strong(),
                                    );
                                    ui.label(&d.message);
                                    ui.label(
                                        RichText::new(&d.path).font(mono(XS + 1.0)).color(FAINT),
                                    );
                                    ui.add_space(6.0);
                                }
                            });
                    });
            }
        });
}
