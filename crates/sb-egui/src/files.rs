//! File actions: open, save as, export, new project, rename.

use sb_model::Project;

use crate::app::{App, stamp_of};
use crate::form::{Input, Kind, Value, ask};

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
        Ok(()) => {
            app.state.stamp = stamp_of(&path);
            app.state.path = Some(path.clone());
            app.state.dirty = false;
            app.say(&format!("Enregistré dans {}", path.display()));
        }
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
                "{} fichiers écrits dans {}",
                report.files, report.dir
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
            app.set_project(Project::new(name), None);
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
