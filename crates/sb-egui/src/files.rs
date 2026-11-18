//! File actions: open, save as, export (the code, the infrastructure), new project, rename.

use sb_app::store::slug;
use sb_model::Project;

use crate::app::App;
use crate::form::{Input, Kind, Value, ask};

/// Opens a page of the project in the browser; any other address is refused.
pub fn open_url(app: &mut App, url: &str) {
    if !url.starts_with("https://github.com/entcorporg/service-builder/") {
        app.fail("adresse refusée");
        return;
    }
    app.ctx.open_url(eframe::egui::OpenUrl::new_tab(url));
}

fn project_dialog(title: &str) -> rfd::FileDialog {
    rfd::FileDialog::new()
        .set_title(title)
        .add_filter("Projet Service Builder", &["json"])
}

pub fn pick_project(app: &mut App) {
    // Cancelling is no error.
    let Some(path) = project_dialog("Ouvrir un projet").pick_file() else {
        return;
    };
    match sb_app::load_project(&path) {
        Ok(project) => app.set_project(project, Some(path)),
        Err(e) => app.fail(&e),
    }
}

pub fn save_as(app: &mut App) {
    let name = app.state.project.name.clone();
    let Some(path) = project_dialog("Enregistrer le projet")
        .set_file_name(format!("{name}.sb.json"))
        .save_file()
    else {
        return;
    };
    match sb_app::save_project(&path, &app.state.project) {
        Ok(()) => app.saved(path),
        Err(e) => app.fail(&e),
    }
}

pub fn export_code(app: &mut App) {
    let Some(dir) = rfd::FileDialog::new()
        .set_title("Dossier de destination du code généré")
        .pick_folder()
    else {
        return;
    };
    let project = app.state.project.clone();
    app.say("Export en cours…");
    app.spawn(
        move || sb_app::export_project(&project, &dir),
        |app, done| match done {
            Ok(report) => app.say(&format!(
                "{} fichiers écrits dans {} (le projet : {})",
                report.files,
                report.dir,
                std::path::Path::new(&report.project_file)
                    .file_name()
                    .map_or(String::new(), |n| n.to_string_lossy().into_owned())
            )),
            Err(e) => app.fail(&e),
        },
    );
}

/// Writes everything that runs the services in containers (the two compose files, Traefik, Garage, the image) into a
/// folder: the one of the code export, so that the image can be built from it.
pub fn export_infra(app: &mut App) {
    let Some(dir) = rfd::FileDialog::new()
        .set_title("Dossier de destination de l'infrastructure (celui du code exporté)")
        .pick_folder()
    else {
        return;
    };
    let project = app.state.project.clone();
    app.say("Export de l'infrastructure en cours…");
    app.spawn(
        move || sb_app::infra::export_infra(&project, &dir),
        |app, done| match done {
            Ok(report) => app.say(&format!(
                "{} fichiers d'infrastructure écrits dans {}",
                report.files.len(),
                report.dir
            )),
            Err(e) => app.fail(&e),
        },
    );
}

pub fn ask_new_project(app: &mut App) {
    ask(
        app,
        "Nouveau projet",
        vec![Input::new("Nom du projet", Kind::Text(String::new()))],
        |app, values| {
            let name = values
                .into_iter()
                .next()
                .map(Value::text)
                .unwrap_or_default();
            if name.is_empty() {
                return Err("Le nom ne peut pas être vide.".into());
            }
            // A project belongs to the store from its first moment: its working copy is made right away.
            let id = slug(&name);
            let path = app.store.local.working_copy(&id);
            if path.exists() || app.store.local.state().find(&id).is_some() {
                return Err(format!(
                    "Un projet « {id} » existe déjà : choisissez un autre nom."
                ));
            }
            let project = Project::new(name);
            sb_app::save_project(&path, &project)?;
            app.set_project(project, Some(path));
            Ok(())
        },
    );
}

pub fn rename_project(app: &mut App) {
    let name = app.state.project.name.clone();
    ask(
        app,
        "Renommer le projet",
        vec![Input::new("Nom", Kind::Text(name))],
        |app, values| {
            let name = values
                .into_iter()
                .next()
                .map(Value::text)
                .unwrap_or_default();
            if name.is_empty() {
                return Err("Le nom ne peut pas être vide.".into());
            }
            app.edit(|p| p.name = name);
            Ok(())
        },
    );
}
