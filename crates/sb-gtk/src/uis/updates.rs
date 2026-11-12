//! Self-update: search, offer, install.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use gtk::gio;
use gtk::glib::{self, clone};
use gtk::prelude::*;

use super::mcp::open_url;
use super::ui::Shared;
use super::widgets::{fail, say};
use crate::updater;

/// `verbose`: the user asked, so say it when there is nothing new or the search failed.
pub fn check_for_update(ui: &Shared, verbose: bool) {
    if verbose {
        say(ui, "Recherche d'une mise à jour…");
    }
    glib::spawn_future_local(clone!(
        #[strong]
        ui,
        async move {
            let found = gio::spawn_blocking(updater::check).await;
            match found {
                Ok(Ok(Some(update))) => {
                    super::header::update_content(&ui.update_button, &update.version);
                    ui.update_button.set_visible(true);
                    ui.state.borrow_mut().update = Some(update);
                    if verbose {
                        offer_update(&ui);
                    }
                }
                Ok(Ok(None)) => {
                    ui.update_button.set_visible(false);
                    if verbose {
                        say(
                            &ui,
                            &format!("Service Builder {} est à jour.", updater::CURRENT_VERSION),
                        );
                    }
                }
                // Offline or no release: silent at startup, reported on request.
                Ok(Err(e)) if verbose => fail(&ui, &e),
                _ => {}
            }
        }
    ));
}

pub fn offer_update(ui: &Shared) {
    let Some(update) = ui.state.borrow().update.clone() else {
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
    let buttons: [&str; 2] = [
        "Plus tard",
        if can_install {
            "Installer"
        } else {
            "Ouvrir la page"
        },
    ];
    gtk::AlertDialog::builder()
        .message(format!("Service Builder {} est disponible", update.version))
        .detail(detail.trim())
        .buttons(buttons)
        .cancel_button(0)
        .default_button(1)
        .build()
        .choose(
            Some(&ui.window),
            gio::Cancellable::NONE,
            clone!(
                #[strong]
                ui,
                move |choice| {
                    if choice.ok() != Some(1) {
                        return;
                    }
                    match (update.download.clone(), target.clone()) {
                        (Some(download), Some(target)) if can_install => {
                            install_update(&ui, download, target)
                        }
                        _ => open_url(&ui, &update.release_page),
                    }
                }
            ),
        );
}

pub fn install_update(ui: &Shared, download: updater::Download, target: PathBuf) {
    ui.update_button.set_sensitive(false);
    // -1: size unknown; 0..=100: percent. Read by a timer, since widgets stay on the UI thread.
    let progress = Arc::new(AtomicI32::new(-1));
    let reader = progress.clone();
    let ticker = glib::timeout_add_local(
        Duration::from_millis(200),
        clone!(
            #[strong]
            ui,
            move || {
                match reader.load(Ordering::Relaxed) {
                    p if p >= 0 => say(&ui, &format!("Téléchargement de la mise à jour… {p} %")),
                    _ => say(&ui, "Téléchargement de la mise à jour…"),
                }
                glib::ControlFlow::Continue
            }
        ),
    );
    glib::spawn_future_local(clone!(
        #[strong]
        ui,
        async move {
            let path = target.clone();
            let done = gio::spawn_blocking(move || {
                updater::install(&download, &path, |p| {
                    progress.store(p.map_or(-1, i32::from), Ordering::Relaxed)
                })
            })
            .await;
            ticker.remove();
            ui.update_button.set_sensitive(true);
            match done {
                Ok(Ok(())) => match updater::relaunch(&target) {
                    Ok(()) => ui.window.application().iter().for_each(|a| a.quit()),
                    Err(e) => fail(&ui, &e),
                },
                Ok(Err(e)) => fail(&ui, &e),
                Err(_) => fail(&ui, "la mise à jour s'est arrêtée brutalement"),
            }
        }
    ));
}
