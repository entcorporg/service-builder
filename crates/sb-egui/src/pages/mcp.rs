//! The MCP page: activation in `.mcp.json`, tools, and the reports of the agents.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke, Ui, Vec2};
use sb_app::feedback::{Feedback, ReportStatus};
use sb_app::mcp::{Activation, EntryState};

use crate::fonts::Weight;
use crate::app::App;
use crate::theme::{self, BORDER, MUTED, OK, PANEL, PANEL2, RAISED, WARN};
use crate::updater;
use crate::widgets;

/// An AI client may write the files at any time: they are read again after this delay.
const REFRESH: Duration = Duration::from_millis(1500);

/// What is read from the disk (`.mcp.json`, the feedback file), kept between frames.
struct Cache {
    path: PathBuf,
    read: Instant,
    activation: Result<Activation, String>,
    feedback: Result<Feedback, String>,
}

#[derive(Default)]
pub struct McpState {
    cache: Option<Rc<Cache>>,
}

/// What a client must run to start the MCP server: this program. An AppImage runs from a temporary mount, so its
/// own path is what `APPIMAGE` holds.
pub fn program() -> Result<String, String> {
    if let Some(appimage) = updater::appimage_path() {
        return Ok(appimage.display().to_string());
    }
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .map_err(|e| format!("cannot locate the program: {e}"))
}

/// Quotes a word for a POSIX shell only when it needs it.
fn shell_quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

fn read_cache(path: &Path) -> Cache {
    Cache {
        path: path.to_path_buf(),
        read: Instant::now(),
        activation: program().and_then(|command| sb_app::mcp::activation(path, &command)),
        feedback: sb_app::feedback::load(path),
    }
}

fn small_title(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .size(theme::SM)
            .color(MUTED)
            .semibold(),
    );
    ui.add_space(2.0);
}

fn framed<R>(ui: &mut Ui, fill: Color32, margin: Margin, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A titled block with a `Copier` button and the text in a monospaced body.
fn copy_block(ui: &mut Ui, title: &str, text: &str, copied: &mut bool) {
    framed(ui, PANEL, Margin::ZERO, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        Frame::new()
            .inner_margin(Margin {
                left: 12,
                right: 6,
                top: 4,
                bottom: 4,
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).semibold());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::ghost(ui, "copy", "Copier", "").clicked() {
                            ui.ctx().copy_text(text.to_string());
                            *copied = true;
                        }
                    });
                });
            });
        let y = ui.cursor().top();
        ui.painter()
            .hline(ui.max_rect().x_range(), y, Stroke::new(1.0, BORDER));
        Frame::new()
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(RichText::new(text).font(theme::mono(theme::SM)))
                        .wrap_mode(egui::TextWrapMode::Wrap)
                        .selectable(true),
                );
            });
    });
    ui.add_space(10.0);
}

fn pill(ui: &mut Ui, text: &str, color: Color32, fill: Color32) {
    Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(99))
        .inner_margin(Margin::symmetric(9, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(theme::SM).color(color).semibold());
        });
}

pub fn page(app: &mut App, ui: &mut Ui) {
    let path = app.state.path.clone();

    // Reads from the disk again only when the project changed or after a short delay.
    let stale = |c: &Cache| path.as_deref() != Some(c.path.as_path()) || c.read.elapsed() > REFRESH;
    if let Some(p) = &path {
        if app.mcp.cache.as_ref().is_none_or(|c| stale(c)) {
            app.mcp.cache = Some(Rc::new(read_cache(p)));
        }
        app.ctx.request_repaint_after(REFRESH);
    } else {
        app.mcp.cache = None;
    }
    let cache = app.mcp.cache.clone();

    let command = program().unwrap_or_else(|e| e);
    let mut say: Option<String> = None;
    let mut fail: Option<String> = None;
    let mut invalidate = false;
    let mut open: Option<String> = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let full = ui.max_rect();
            let w = 812.0_f32.min(full.width() - 48.0).max(200.0);
            let x = full.center().x - w / 2.0;
            let rect = egui::Rect::from_min_size(
                egui::pos2(x, full.top() + 28.0),
                Vec2::new(w, full.height()),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.set_width(w);
                ui.spacing_mut().item_spacing.y = 16.0;

                // ---- heading ----
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.label(RichText::new("Serveur MCP").size(theme::XL).semibold());
                    ui.label(widgets::muted(
                        "Un client IA (Claude Code, Claude Desktop…) lance Service Builder en mode mcp et lit, \
                         modifie, valide et exporte ce projet par le protocole MCP, sur stdio. Le serveur \
                         travaille sur le fichier du projet.",
                    ));
                });

                // ---- activation ----
                let activation = cache.as_ref().map(|c| &c.activation);
                let state = activation.and_then(|a| a.as_ref().ok()).map(|a| a.state);
                framed(ui, PANEL2, Margin::symmetric(16, 14), |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.set_max_width(ui.available_width() - 150.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Activation").semibold());
                                match state {
                                    Some(EntryState::On) => {
                                        pill(ui, "Actif", OK, theme::alpha(OK, 30))
                                    }
                                    Some(EntryState::Stale) => {
                                        pill(ui, "À mettre à jour", WARN, theme::alpha(WARN, 30))
                                    }
                                    Some(_) => pill(ui, "Inactif", MUTED, RAISED),
                                    None => {}
                                }
                            });
                            let detail = match activation {
                                Some(Ok(a)) => format!(
                                    "Ajoute l’entrée {} au fichier {}, que Claude Code lit depuis ce dossier. \
                                     Les autres entrées sont conservées.",
                                    a.server_name, a.config_path
                                ),
                                Some(Err(e)) => e.clone(),
                                None => "Enregistrez le projet pour pouvoir activer le MCP : le serveur \
                                         travaille sur son fichier."
                                    .to_string(),
                            };
                            ui.label(RichText::new(detail).size(theme::XS + 0.5).color(MUTED));
                        });
                        if let (Some(state), Some(path)) = (state, &path) {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let on = state == EntryState::On;
                                    let text = match (on, state == EntryState::Stale) {
                                        (true, _) => "Désactiver",
                                        (_, true) => "Mettre à jour",
                                        _ => "Activer",
                                    };
                                    let clicked = if on {
                                        widgets::button(ui, "", text).clicked()
                                    } else {
                                        widgets::primary(ui, "", text, true).clicked()
                                    };
                                    if clicked {
                                        let result = program().and_then(|command| {
                                            if on {
                                                sb_app::mcp::disable(path, &command)
                                            } else {
                                                sb_app::mcp::enable(path, &command)
                                            }
                                        });
                                        invalidate = true;
                                        match result {
                                            Ok(_) => {
                                                say = Some(
                                                    if on { "MCP désactivé" } else { "MCP activé" }
                                                        .into(),
                                                )
                                            }
                                            Err(e) => fail = Some(e),
                                        }
                                    }
                                },
                            );
                        }
                    });
                });

                // ---- manual configuration ----
                let name = activation
                    .and_then(|a| a.as_ref().ok())
                    .map_or("service-builder".to_string(), |a| a.server_name.clone());
                let project_arg = path
                    .as_ref()
                    .map_or("<projet.sb.json>".to_string(), |p| p.display().to_string());
                let line = [command.as_str(), "mcp", project_arg.as_str()]
                    .map(shell_quote)
                    .join(" ");
                let json = serde_json::to_string_pretty(&serde_json::json!({
                    "mcpServers": { name.clone(): { "command": command, "args": ["mcp", project_arg] } }
                }))
                .unwrap_or_default();
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    small_title(ui, "Configuration manuelle");
                    let mut copied = false;
                    copy_block(ui, "Commande du serveur", &line, &mut copied);
                    copy_block(
                        ui,
                        "Claude Code",
                        &format!("claude mcp add {name} -- {line}"),
                        &mut copied,
                    );
                    copy_block(ui, "Autres clients (Claude Desktop…)", &json, &mut copied);
                    if copied {
                        say = Some("Copié".into());
                    }
                });

                // ---- tools ----
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    small_title(ui, "Outils exposés");
                    framed(ui, PANEL, Margin::ZERO, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, tool) in sb_app::mcp::tool_summaries().iter().enumerate() {
                            if i > 0 {
                                ui.painter().hline(
                                    ui.max_rect().x_range(),
                                    ui.cursor().top(),
                                    Stroke::new(1.0, BORDER),
                                );
                            }
                            Frame::new()
                                .inner_margin(Margin::symmetric(12, 8))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal_top(|ui| {
                                        ui.spacing_mut().item_spacing.x = 12.0;
                                        ui.allocate_ui(Vec2::new(190.0, 0.0), |ui| {
                                            ui.set_width(190.0);
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(tool.name)
                                                        .font(theme::mono(theme::SM)),
                                                )
                                                .wrap_mode(egui::TextWrapMode::Wrap),
                                            );
                                        });
                                        ui.vertical(|ui| {
                                            ui.set_width(ui.available_width());
                                            ui.label(widgets::muted(tool.description));
                                        });
                                    });
                                });
                        }
                    });
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(
                            "Les modifications d’un client sont écrites dans le fichier du projet. L’application \
                             le recharge quand la fenêtre reprend le focus, sauf si des modifications non \
                             enregistrées sont en cours.",
                        )
                        .size(theme::SM)
                        .color(MUTED),
                    );
                });

                // ---- the reports of the agents ----
                if let (Some(path), Some(cache)) = (&path, &cache) {
                    match &cache.feedback {
                        Err(e) => {
                            ui.label(e);
                        }
                        Ok(feedback) => {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 8.0;
                                small_title(ui, "Suggestions des agents");
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 12.0;
                                    if widgets::switch(ui, feedback.enabled).clicked() {
                                        invalidate = true;
                                        if let Err(e) = sb_app::feedback::set_enabled(
                                            path,
                                            !feedback.enabled,
                                        ) {
                                            fail = Some(e);
                                        }
                                    }
                                    ui.label("Demander un rapport à l’agent tous les 50 appels");
                                });
                                for report in &feedback.reports {
                                    widgets::card(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = 4.0;
                                        let opened = if report.status == ReportStatus::Opened {
                                            " (issue ouverte)"
                                        } else {
                                            ""
                                        };
                                        let summary = if report.summary.is_empty() {
                                            "Rapport"
                                        } else {
                                            &report.summary
                                        };
                                        ui.label(format!("#{} · {summary}{opened}", report.id));
                                        for s in &report.suggestions {
                                            ui.label(format!("• {} {}", s.title, s.details));
                                        }
                                        ui.horizontal(|ui| {
                                            if widgets::button(ui, "", "Ouvrir l'issue GitHub")
                                                .clicked()
                                            {
                                                invalidate = true;
                                                match sb_app::feedback::issue_url_for(
                                                    path, report.id,
                                                ) {
                                                    Ok(url) => open = Some(url),
                                                    Err(e) => fail = Some(e),
                                                }
                                            }
                                            if widgets::button(ui, "", "Ignorer").clicked() {
                                                invalidate = true;
                                                if let Err(e) =
                                                    sb_app::feedback::dismiss(path, report.id)
                                                {
                                                    fail = Some(e);
                                                }
                                            }
                                        });
                                    });
                                }
                            });
                        }
                    }
                }
                ui.add_space(24.0);
            });
        });

    if invalidate {
        app.mcp.cache = None;
    }
    if let Some(url) = open {
        crate::updates::open_url(app, &url);
    }
    if let Some(t) = say {
        app.say(&t);
    }
    if let Some(e) = fail {
        app.fail(&e);
    }
}
