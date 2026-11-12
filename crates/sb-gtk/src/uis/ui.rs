use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::state::State;

pub const TITLE: &str = "Service Builder";

pub struct Ui {
    pub window: gtk::ApplicationWindow,
    pub state: RefCell<State>,
    /// The name of the project, editable in the toolbar.
    pub title: gtk::Entry,
    pub dirty_mark: gtk::Label,
    pub file_label: gtk::Label,
    pub toast: gtk::Label,
    pub problems_head: gtk::Box,
    pub problems_panel: gtk::Revealer,
    pub update_button: gtk::Button,
    /// `Valide`, or how many errors and warnings there are.
    pub pill: gtk::Box,
    pub pill_text: gtk::Label,
    pub export_button: gtk::Button,
    pub stack: gtk::Stack,
    pub canvas: super::canvas::Canvas,
    pub services: gtk::ListBox,
    pub contracts: gtk::ListBox,
    pub flows: gtk::ListBox,
    pub tables: gtk::Box,
    /// The route open on the API page: (service, route).
    pub endpoint: RefCell<Option<(String, String)>>,
    pub endpoint_page: gtk::Box,
    /// The structure tree of the API page.
    pub endpoint_tree: gtk::Box,
    /// The table open on the Tables page: (service, table).
    pub table_sel: RefCell<Option<(String, String)>>,
    /// The tree of the generated files.
    pub files: gtk::Box,
    pub current_file: RefCell<Option<String>>,
    pub code_path: gtk::Label,
    pub code: gtk::TextView,
    pub problems: gtk::ListBox,
    pub mcp: gtk::Box,
    /// Set while the MCP switches are being set by the code, so that it does not look like a click.
    pub syncing: Cell<bool>,
}

pub type Shared = Rc<Ui>;
