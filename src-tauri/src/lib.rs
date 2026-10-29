use std::path::PathBuf;

use sb_app::{Analysis, ExportReport};
use sb_model::Project;

#[tauri::command]
fn sample_project() -> Project {
    sb_model::sample::orders()
}

#[tauri::command]
fn new_project(name: String) -> Project {
    Project::new(name)
}

/// Validation + code generation in one round trip; called after every edit.
#[tauri::command(async)]
fn analyze(project: Project) -> Analysis {
    sb_app::analyze(&project)
}

#[tauri::command(async)]
fn open_project(path: PathBuf) -> Result<Project, String> {
    sb_app::load_project(&path)
}

#[tauri::command(async)]
fn save_project(path: PathBuf, project: Project) -> Result<(), String> {
    sb_app::save_project(&path, &project)
}

#[tauri::command(async)]
fn export_project(project: Project, dir: PathBuf) -> Result<ExportReport, String> {
    sb_app::export_project(&project, &dir)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            sample_project,
            new_project,
            analyze,
            open_project,
            save_project,
            export_project,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Service Builder");
}
