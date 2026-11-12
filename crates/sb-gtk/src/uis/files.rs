//! File actions: open, save as, export, new project.

use gtk::gio;
use gtk::glib::{self, clone};
use gtk::prelude::*;
use sb_model::Project;

use super::edit::edit;
use super::form::{Input, Kind, Value, ask};
use super::mcp::draw_mcp;
use super::project::{refresh_title, set_project, stamp_of};
use super::ui::Shared;
use super::widgets::{fail, say};

pub fn project_filter() -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Projet Service Builder"));
    filter.add_pattern("*.sb.json");
    filter.add_pattern("*.json");
    let store = gio::ListStore::new::<gtk::FileFilter>();
    store.append(&filter);
    store
}

pub fn pick_project(ui: &Shared) {
    let dialog = gtk::FileDialog::builder()
        .title("Ouvrir un projet")
        .filters(&project_filter())
        .build();
    dialog.open(
        Some(&ui.window),
        gio::Cancellable::NONE,
        clone!(
            #[strong]
            ui,
            move |result| {
                // Cancelling is an error for GTK, and not one for us.
                let Some(path) = result.ok().and_then(|f| f.path()) else {
                    return;
                };
                match sb_app::load_project(&path) {
                    Ok(project) => set_project(&ui, project, Some(path)),
                    Err(e) => fail(&ui, &e),
                }
            }
        ),
    );
}

pub fn save_as(ui: &Shared) {
    let name = ui.state.borrow().project.name.clone();
    let dialog = gtk::FileDialog::builder()
        .title("Enregistrer le projet")
        .initial_name(format!("{name}.sb.json"))
        .filters(&project_filter())
        .build();
    dialog.save(
        Some(&ui.window),
        gio::Cancellable::NONE,
        clone!(
            #[strong]
            ui,
            move |result| {
                let Some(path) = result.ok().and_then(|f| f.path()) else {
                    return;
                };
                let saved = sb_app::save_project(&path, &ui.state.borrow().project);
                match saved {
                    Ok(()) => {
                        let mut s = ui.state.borrow_mut();
                        s.stamp = stamp_of(&path);
                        s.path = Some(path.clone());
                        s.dirty = false;
                        drop(s);
                        refresh_title(&ui);
                        say(&ui, &format!("Enregistré dans {}", path.display()));
                        draw_mcp(&ui);
                    }
                    Err(e) => fail(&ui, &e),
                }
            }
        ),
    );
}

pub fn export_code(ui: &Shared) {
    let dialog = gtk::FileDialog::builder()
        .title("Dossier de destination du code généré")
        .build();
    dialog.select_folder(
        Some(&ui.window),
        gio::Cancellable::NONE,
        clone!(
            #[strong]
            ui,
            move |result| {
                let Some(dir) = result.ok().and_then(|f| f.path()) else {
                    return;
                };
                let project = ui.state.borrow().project.clone();
                say(&ui, "Export en cours…");
                glib::spawn_future_local(clone!(
                    #[strong]
                    ui,
                    async move {
                        let done =
                            gio::spawn_blocking(move || sb_app::export_project(&project, &dir))
                                .await;
                        match done {
                            Ok(Ok(report)) => say(
                                &ui,
                                &format!("{} fichiers écrits dans {}", report.files, report.dir),
                            ),
                            Ok(Err(e)) => fail(&ui, &e),
                            Err(_) => fail(&ui, "l'export s'est arrêté brutalement"),
                        }
                    }
                ));
            }
        ),
    );
}

pub fn ask_new_project(ui: &Shared) {
    let dialog = gtk::Window::builder()
        .title("Nouveau projet")
        .transient_for(&ui.window)
        .modal(true)
        .resizable(false)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    let entry = gtk::Entry::builder()
        .placeholder_text("Nom du projet")
        .activates_default(true)
        .width_chars(32)
        .build();
    let create = gtk::Button::with_label("Créer");
    create.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label("Annuler");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&create);
    content.append(&entry);
    content.append(&buttons);
    dialog.set_child(Some(&content));
    dialog.set_default_widget(Some(&create));

    cancel.connect_clicked(clone!(
        #[weak]
        dialog,
        move |_| dialog.close()
    ));
    create.connect_clicked(clone!(
        #[strong]
        ui,
        #[weak]
        dialog,
        #[weak]
        entry,
        move |_| {
            let name = entry.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            set_project(&ui, Project::new(name), None);
            dialog.close();
        }
    ));
    dialog.present();
}

pub fn rename_project(ui: &Shared) {
    let name = ui.state.borrow().project.name.clone();
    ask(
        ui,
        "Renommer le projet",
        vec![Input::new("Nom", Kind::Text(name))],
        |ui, values| {
            let name = values
                .into_iter()
                .next()
                .map(Value::text)
                .unwrap_or_default();
            if name.is_empty() {
                return Err("Le nom ne peut pas être vide.".into());
            }
            edit(ui, |p| p.name = name);
            Ok(())
        },
    );
}
