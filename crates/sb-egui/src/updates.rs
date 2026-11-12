//! Self-update: search, offer, install.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::app::{App, Done};
use crate::updater;

/// Opens a page of the project in the browser; any other address is refused.
pub fn open_url(app: &mut App, url: &str) {
    if !url.starts_with("https://github.com/entcorporg/service-builder/") {
        app.fail("adresse refusée");
        return;
    }
    app.ctx.open_url(eframe::egui::OpenUrl::new_tab(url));
}

/// `verbose`: the user asked, so say it when there is nothing new or the search failed.
pub fn check_for_update(app: &mut App, verbose: bool) {
    if verbose {
        app.say("Recherche d'une mise à jour…");
    }
    app.spawn(updater::check, move |app, found| match found {
        Ok(Some(update)) => {
            app.state.update = Some(update);
            if verbose {
                offer_update(app);
            }
        }
        Ok(None) => {
            app.state.update = None;
            if verbose {
                app.say(&format!(
                    "Service Builder {} est à jour.",
                    updater::CURRENT_VERSION
                ));
            }
        }
        // Offline or no release: silent at startup, reported on request.
        Err(e) if verbose => app.fail(&e),
        Err(_) => {}
    });
}

pub fn offer_update(app: &mut App) {
    let Some(update) = app.state.update.clone() else {
        return;
    };
    let target = updater::appimage_path();
    let can_install = update.download.is_some() && target.is_some();
    let notes = update.notes.clone().unwrap_or_default();
    let detail = if can_install {
        format!("{notes}\n\nL'application redémarrera après l'installation.")
    } else {
        format!(
            "{notes}\n\nCette installation ne peut pas se mettre à jour seule (seul l'AppImage le peut) : la page de la release s'ouvre."
        )
    };
    let title = format!("Service Builder {} est disponible", update.version);
    let action: Box<dyn FnOnce(&mut App)> =
        Box::new(move |app| match (update.download.clone(), target) {
            (Some(download), Some(target)) if can_install => install_update(app, download, target),
            _ => open_url(app, &update.release_page),
        });
    app.confirm(
        &title,
        detail.trim(),
        vec![
            ("Plus tard".into(), false, None),
            (
                if can_install {
                    "Installer"
                } else {
                    "Ouvrir la page"
                }
                .into(),
                true,
                Some(action),
            ),
        ],
    );
}

pub fn install_update(app: &mut App, download: updater::Download, target: PathBuf) {
    app.updating = true;
    app.say("Téléchargement de la mise à jour…");
    // -1: size unknown; 0..=100: percent.
    let progress = Arc::new(AtomicI32::new(-1));
    let (tx, ctx) = app.sender();
    let path = target.clone();
    let reporter = progress.clone();
    app.spawn(
        move || {
            updater::install(&download, &path, |p| {
                let p = p.map_or(-1, i32::from);
                if reporter.swap(p, Ordering::Relaxed) != p {
                    let _ = tx.send(Done::Run(Box::new(move |app: &mut App| {
                        if app.updating {
                            app.say(&match p {
                                p if p >= 0 => format!("Téléchargement de la mise à jour… {p} %"),
                                _ => "Téléchargement de la mise à jour…".to_string(),
                            });
                        }
                    })));
                    ctx.request_repaint();
                }
            })
        },
        move |app, done| {
            app.updating = false;
            match done {
                Ok(()) => match updater::relaunch(&target) {
                    Ok(()) => app
                        .ctx
                        .send_viewport_cmd(eframe::egui::ViewportCommand::Close),
                    Err(e) => app.fail(&e),
                },
                Err(e) => app.fail(&e),
            }
        },
    );
}
