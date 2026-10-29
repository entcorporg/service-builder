use std::path::PathBuf;

use sb_app::mcp::{Activation, ToolSummary};
use sb_app::{Analysis, ExportReport};
use sb_model::Project;
use serde::Serialize;

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

/// What a client must run to start the MCP server: this program. An AppImage runs from a
/// temporary mount, so its own path is what `APPIMAGE` holds.
fn program() -> Result<String, String> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Ok(PathBuf::from(appimage).display().to_string());
    }
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .map_err(|e| format!("cannot locate the program: {e}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct McpInfo {
    command: String,
    tools: Vec<ToolSummary>,
    /// Absent while the project has no file yet: the server works on a saved project.
    activation: Option<Activation>,
}

#[tauri::command(async)]
fn mcp_info(path: Option<PathBuf>) -> Result<McpInfo, String> {
    let command = program()?;
    let activation = path
        .map(|p| sb_app::mcp::activation(&p, &command))
        .transpose()?;
    Ok(McpInfo {
        command,
        tools: sb_app::mcp::tool_summaries(),
        activation,
    })
}

/// Adds or removes the project's entry in the `.mcp.json` next to the project file.
#[tauri::command(async)]
fn mcp_set_enabled(path: PathBuf, enabled: bool) -> Result<Activation, String> {
    let command = program()?;
    if enabled {
        sb_app::mcp::enable(&path, &command)
    } else {
        sb_app::mcp::disable(&path, &command)
    }
}

/// Modification time in milliseconds: how the app notices that a client edited the file.
#[tauri::command(async)]
fn project_stamp(path: PathBuf) -> Result<u64, String> {
    let modified = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0))
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
            mcp_info,
            mcp_set_enabled,
            project_stamp,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Service Builder");
}
