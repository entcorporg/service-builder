//! The Projets page: where the projects are kept. The versions of the open project, the last projects opened on this
//! machine, and the projects of the store. The store is the application's own Garage: it starts by itself and has nothing
//! to set, so the page only says how it is doing.
//!
//! Everything that reaches the store runs on a thread and comes back through `App::spawn`.

use std::path::PathBuf;

use eframe::egui::{self, Frame, Margin, RichText, Stroke, Ui};
use sb_app::store::{RemoteProject, Version};

use crate::app::{App, SyncState};
use crate::fonts::Weight;
use crate::pages::common::{self, section};
use crate::theme::*;
use crate::widgets;

#[derive(Default)]
pub struct ProjectsState {
    /// Something changed (a version was stored, a project opened): the lists are read again.
    pub stale: bool,
    loading: bool,
    /// The projects of the bucket, or why they could not be read.
    pub remote: Option<Result<Vec<RemoteProject>, String>>,
    /// The versions of the open project (by its id).
    versions: Option<(String, Result<Vec<Version>, String>)>,
}

// ---- reading the bucket -----------------------------------------------------------------

fn refresh(app: &mut App) {
    if app.projects.loading || !app.store.configured() {
        return;
    }
    app.projects.loading = true;
    app.projects.stale = false;
    let store = app.store.clone();
    let current = app
        .state
        .path
        .clone()
        .map(|p| (store.id_of(&p, &app.state.project.name), p));
    app.spawn(
        move || {
            let remote = store.remote();
            let projects = remote
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|r| r.projects());
            let versions = current.map(|(id, _)| {
                let list = remote
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|r| r.versions(&id));
                (id, list)
            });
            (projects, versions)
        },
        |app, (projects, versions)| {
            app.projects.loading = false;
            app.projects.remote = Some(projects);
            app.projects.versions = versions;
        },
    );
}

// ---- actions ----------------------------------------------------------------------------

fn open_file(app: &mut App, path: PathBuf) {
    app.guard(move |app| match sb_app::load_project(&path) {
        Ok(project) => app.set_project(project, Some(path)),
        Err(e) => app.fail(&e),
    });
}

/// Opens a project of the bucket: fetched into its working copy, unless the copy of this machine holds changes the
/// bucket does not have (that one is the newer, and is not thrown away).
fn open_remote(app: &mut App, id: String) {
    app.guard(move |app| {
        let store = app.store.clone();
        app.say("Ouverture…");
        app.spawn(
            move || {
                if let Some(recent) = store.local.state().find(&id).cloned() {
                    let path = PathBuf::from(&recent.path);
                    if path.is_file() && store.unsynced(&path) {
                        return Ok((path, true));
                    }
                }
                store.pull(&id, None, false).map(|p| (p.path, false))
            },
            |app, done| match done {
                Ok((path, kept)) => match sb_app::load_project(&path) {
                    Ok(project) => {
                        app.set_project(project, Some(path));
                        if kept {
                            app.say("Copie de cette machine ouverte : elle a des changements que S3 n'a pas.");
                        } else {
                            app.say("");
                        }
                    }
                    Err(e) => app.fail(&e),
                },
                Err(e) => app.fail(&e),
            },
        );
    });
}

/// Puts an older version in the editor. It stays a change to save: saving makes a new version, the history is kept.
fn restore(app: &mut App, id: String, number: u32) {
    app.guard(move |app| {
        let store = app.store.clone();
        app.say("Restauration…");
        app.spawn(
            move || {
                // What the working copy holds that was never stored must not be lost by the restoration.
                if let Some(recent) = store.local.state().find(&id).cloned() {
                    let path = PathBuf::from(&recent.path);
                    if path.is_file() && store.unsynced(&path) {
                        store.push_file(&path)?;
                    }
                }
                store.pull(&id, Some(number), true)
            },
            move |app, done| match done {
                Ok(pulled) => match sb_app::load_project(&pulled.path) {
                    Ok(project) => {
                        app.set_project(project, Some(pulled.path));
                        app.state.dirty = true;
                        app.say(&format!(
                            "Version {number} restaurée : enregistrez pour en faire une nouvelle version."
                        ));
                    }
                    Err(e) => app.fail(&e),
                },
                Err(e) => app.fail(&e),
            },
        );
    });
}

// ---- the lists --------------------------------------------------------------------------

/// `2026-11-18T12:46:03.000Z` → `2026-11-18 12:46`.
fn when(date: &str) -> String {
    date.replace('T', " ").chars().take(16).collect()
}

fn size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} Mo", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} Ko", bytes.div_ceil(1024))
    }
}

/// A row of a list: what it is on the left, its action on the right.
fn row(ui: &mut Ui, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(PANEL2)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    left(ui);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
            });
        });
}

fn versions(app: &mut App, ui: &mut Ui) {
    let id = app
        .state
        .path
        .as_ref()
        .map(|p| app.store.id_of(p, &app.state.project.name));
    let current = match &app.state.sync {
        SyncState::Stored(v) => Some(*v),
        _ => None,
    };
    let title = format!("Versions de « {} »", app.state.project.name);
    section(
        ui,
        &title,
        "log",
        |_| {},
        |ui| {
            let Some(id) = id else {
                common::empty(
                    ui,
                    "Ce projet n'a pas encore de fichier : Ctrl+S le crée et en enregistre la première version.",
                );
                return;
            };
            if !app.store.configured() {
                common::empty(
                    ui,
                    "Le stockage n'est pas disponible : les versions ne sont pas gardées.",
                );
                return;
            }
            match app.projects.versions.clone() {
                Some((of, Ok(list))) if of == id => {
                    if list.is_empty() {
                        common::empty(ui, "Pas encore de version : Ctrl+S enregistre la première.");
                    }
                    for v in list {
                        let number = v.number;
                        row(
                            ui,
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("v{number}")).font(mono_bold(BASE)),
                                    );
                                    if current == Some(number) {
                                        widgets::chip(ui, "actuelle", OK);
                                    }
                                });
                                ui.label(
                                    RichText::new(format!(
                                        "{}  ·  {}",
                                        when(&v.saved_at),
                                        size(v.size)
                                    ))
                                    .size(SM)
                                    .color(MUTED),
                                );
                            },
                            |ui| {
                                if widgets::button(ui, "restart", "Restaurer")
                                .on_hover_text("La remet dans l'éditeur ; enregistrer en fait une nouvelle version")
                                .clicked()
                            {
                                restore(app, id.clone(), number);
                            }
                            },
                        );
                    }
                }
                Some((of, Err(e))) if of == id => {
                    ui.label(RichText::new(e).size(SM).color(ERR));
                }
                _ => {
                    ui.spinner();
                }
            }
        },
    );
}

fn recents(app: &mut App, ui: &mut Ui) {
    let state = app.store.local.state();
    let here = app.state.path.as_ref().map(|p| p.display().to_string());
    section(
        ui,
        "Derniers projets ouverts",
        "open",
        |_| {},
        |ui| {
            if state.recent.is_empty() {
                common::empty(ui, "Aucun projet ouvert pour l'instant.");
            }
            for r in state.recent {
                let open = here.as_deref() == Some(r.path.as_str());
                row(
                    ui,
                    |ui| {
                        ui.label(RichText::new(&r.name).font(semibold(BASE)));
                        ui.label(RichText::new(&r.path).font(mono(SM)).color(FAINT));
                    },
                    |ui| {
                        if open {
                            widgets::chip(ui, "ouvert", OK);
                        } else if widgets::button(ui, "open", "Ouvrir").clicked() {
                            open_file(app, PathBuf::from(&r.path));
                        }
                    },
                );
            }
        },
    );
}

fn bucket(app: &mut App, ui: &mut Ui) {
    section(
        ui,
        "Dans le stockage",
        "folder",
        |_| {},
        |ui| {
            if !app.store.configured() {
                common::empty(ui, "Le stockage n'est pas disponible.");
                return;
            }
            let here = app
                .state
                .path
                .as_ref()
                .map(|p| app.store.id_of(p, &app.state.project.name));
            match app.projects.remote.clone() {
                Some(Ok(list)) => {
                    if list.is_empty() {
                        common::empty(
                            ui,
                            "Le stockage ne contient aucun projet : Ctrl+S enregistre celui-ci.",
                        );
                    }
                    for p in list {
                        let open = here.as_deref() == Some(p.id.as_str());
                        row(
                            ui,
                            |ui| {
                                ui.label(RichText::new(&p.name).font(semibold(BASE)));
                                ui.label(
                                    RichText::new(format!(
                                        "{}  ·  v{}  ·  {}",
                                        p.id,
                                        p.version,
                                        when(&p.saved_at)
                                    ))
                                    .font(mono(SM))
                                    .color(FAINT),
                                );
                            },
                            |ui| {
                                if open {
                                    widgets::chip(ui, "ouvert", OK);
                                } else if widgets::button(ui, "open", "Ouvrir").clicked() {
                                    open_remote(app, p.id.clone());
                                }
                            },
                        );
                    }
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).size(SM).color(ERR));
                }
                None => {
                    ui.spinner();
                }
            }
        },
    );
}

/// How the store is doing, next to the title.
fn status(app: &App, ui: &mut Ui) {
    if !app.store.configured() {
        ui.label(RichText::new("Stockage introuvable").size(SM).color(ERR))
            .on_hover_text(
                "Le stockage interne (Garage) est livré dans l'AppImage : scripts/install-local.sh",
            );
        return;
    }
    match &app.projects.remote {
        Some(Ok(_)) => {
            widgets::chip(ui, "Stockage prêt", OK);
        }
        Some(Err(e)) => {
            ui.label(
                RichText::new(format!("Stockage en échec : {e}"))
                    .size(SM)
                    .color(ERR),
            );
        }
        None => {
            ui.label(widgets::muted("Démarrage du stockage…").size(SM));
        }
    }
}

pub fn page(app: &mut App, ui: &mut Ui) {
    if (app.projects.stale || app.projects.remote.is_none()) && app.store.configured() {
        refresh(app);
    }

    egui::CentralPanel::no_frame().show(ui, |ui| {
        Frame::new()
            .inner_margin(Margin::symmetric(24, 20))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ui.label(RichText::new("Projets").size(XL).bold());
                    status(app, ui);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::button(ui, "restart", "Actualiser").clicked() {
                            app.projects.stale = true;
                        }
                        if app.projects.loading {
                            ui.spinner();
                        }
                    });
                });
                ui.add_space(10.0);
                egui::ScrollArea::vertical()
                    .id_salt("projects-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        versions(app, ui);
                        common::separator(ui);
                        recents(app, ui);
                        common::separator(ui);
                        bucket(app, ui);
                    });
            });
    });
}
