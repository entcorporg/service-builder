//! The app opens the last project again at start, remembers the ones it opens, and gives a project that has no file its
//! working copy on the first save. Its own test binary: it sets the folders of the application for the whole process.

use sb_egui::app::{App, SyncState};
use sb_model::Project;

fn app() -> App {
    App::with_context(eframe::egui::Context::default(), None)
}

#[test]
fn the_last_project_comes_back_and_a_new_one_gets_its_working_copy() {
    let home = std::env::temp_dir().join(format!("sb-restore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    // SAFETY: set before the app starts a thread; this is the only test of the binary.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", home.join("config"));
        std::env::set_var("XDG_DATA_HOME", home.join("data"));
        std::env::remove_var("SB_NO_RESTORE");
        for var in [
            "SB_S3_ENDPOINT",
            "SB_S3_BUCKET",
            "SB_S3_ACCESS_KEY",
            "SB_S3_SECRET_KEY",
        ] {
            std::env::remove_var(var);
        }
    }
    let store = sb_app::store::Store::system();

    // Nothing opened yet: the example project, without a file.
    let first = app();
    assert!(first.state.path.is_none());
    assert_eq!(first.state.project.name, "Orders Demo");
    assert_eq!(first.state.sync, SyncState::Off);

    // Saving gives it a working copy named after it, and it becomes the last project.
    let mut first = first;
    first.save();
    let copy = store.local.working_copy("orders-demo");
    assert_eq!(first.state.path.as_deref(), Some(copy.as_path()));
    assert!(copy.is_file() && !first.state.dirty);
    assert_eq!(store.local.state().last.as_deref(), Some("orders-demo"));

    // Another project, edited and saved.
    let mut project = Project::new("Billing");
    project.name = "Billing".into();
    let path = store.local.working_copy("billing");
    sb_app::save_project(&path, &project).unwrap();
    first.set_project(project, Some(path.clone()));
    first.edit(|p| p.name = "Billing 2".into());
    first.save();
    assert_eq!(store.local.state().last.as_deref(), Some("billing"));
    let ids: Vec<_> = store
        .local
        .state()
        .recent
        .iter()
        .map(|r| r.id.clone())
        .collect();
    assert_eq!(ids, ["billing", "orders-demo"], "the most recent first");
    // The file was renamed in the project, not the identity of the project.
    assert_eq!(
        store.local.state().find("billing").unwrap().name,
        "Billing 2"
    );

    // The next start opens it again, as it was saved.
    let second = app();
    assert_eq!(second.state.path.as_deref(), Some(path.as_path()));
    assert_eq!(second.state.project.name, "Billing 2");
    assert!(!second.state.dirty);

    // A last project whose file is gone is not an error: the example is shown.
    std::fs::remove_file(&path).unwrap();
    let third = app();
    assert!(third.state.path.is_none());
    assert_eq!(third.state.project.name, "Orders Demo");

    // Opened by hand, a file keeps the id it already had, and is a recent project from then on.
    let mut fourth = app();
    fourth.set_project(sb_app::load_project(&copy).unwrap(), Some(copy.clone()));
    assert_eq!(store.local.state().last.as_deref(), Some("orders-demo"));
    let _ = std::fs::remove_dir_all(&home);
}
