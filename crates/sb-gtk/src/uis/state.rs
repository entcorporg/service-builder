use std::path::PathBuf;

use sb_app::Analysis;
use sb_model::Project;

use crate::updater::Update;

pub struct State {
    pub project: Project,
    pub path: Option<PathBuf>,
    /// Modification time of `path` when it was last read: how an edit made by an AI client is noticed.
    pub stamp: u64,
    pub analysis: Analysis,
    /// Bumped on every change, so that a slow analysis of an older project is dropped.
    pub generation: u64,
    pub update: Option<Update>,
    /// Edited since it was read or saved.
    pub dirty: bool,
    pub undo: Vec<Project>,
    pub redo: Vec<Project>,
}
