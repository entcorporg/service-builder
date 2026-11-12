//! The application: the project being edited, its history, the background analysis, and the frame loop.
//!
//! # How the pages are written (read this before porting a page)
//!
//! egui is immediate mode: every frame, each page is drawn again from the state, by a function
//! `fn show(app: &mut App, ui: &mut egui::Ui)`. There are no widgets to keep and no signals to connect.
//!
//! * **Reading the project**: `let p = app.project();` gives an `Rc<Project>` snapshot, cheap to clone, that stays
//!   valid while `app` is borrowed mutably. Read from `p`, never hold `&app.state.project` across an edit.
//!   Same for the analysis: `let a = app.analysis();`.
//! * **Changing the project**: `app.edit(|p| …)`, exactly as `edit(ui, |p| …)` in the GTK version: one entry in the
//!   history, the unsaved mark, a new analysis. The next frame shows the result (the snapshot `p` you hold is the
//!   old one until then, which is what you want while drawing).
//! * **Text inputs** commit on Enter or when the focus leaves, as in the GTK version: use
//!   [`crate::widgets::text_field`] and friends, which return `Some(new value)` once, at commit.
//! * **Modal forms** (`form::ask`), confirmations (`app.confirm`) and errors (`app.fail`) are queued on the app
//!   and drawn over the window by the frame loop.
//! * **Work to do after drawing** (anything that needs `&mut App` from inside a closure given to egui):
//!   `app.later(|app| …)`.
//! * Per-page UI state (selection, open cards, scroll targets) lives in the page's own struct, a field of `App`.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, UNIX_EPOCH};

use eframe::egui;
use sb_app::Analysis;
use sb_model::Project;

use crate::form::Form;
use crate::pages;
use crate::updater::Update;

pub const TITLE: &str = "Service Builder";
const HISTORY: usize = 100;

/// The views of the window. `Endpoint` is the API page, reached from the canvas (its tab is Architecture).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Page {
    #[default]
    Architecture,
    Endpoint,
    Flows,
    Tables,
    Mcp,
    Code,
}

pub struct State {
    pub project: Project,
    pub path: Option<PathBuf>,
    /// Modification time of `path` when it was last read: how an edit made by an AI client is noticed.
    pub stamp: u64,
    pub analysis: Rc<Analysis>,
    /// Bumped on every change, so that a slow analysis of an older project is dropped.
    pub generation: u64,
    pub update: Option<Update>,
    /// Edited since it was read or saved.
    pub dirty: bool,
    pub undo: Vec<Project>,
    pub redo: Vec<Project>,
}

/// A question with buttons; each button runs its action (or nothing).
pub struct Confirm {
    pub title: String,
    pub detail: String,
    /// (label, primary?, action). The first one is what Escape does.
    pub buttons: Vec<(String, bool, Option<Box<dyn FnOnce(&mut App)>>)>,
}

/// What a background thread hands back to the frame loop.
pub enum Done {
    Analysis(u64, Analysis),
    /// Run on the UI thread with the app (results of exports, update checks, downloads).
    Run(Box<dyn FnOnce(&mut App) + Send>),
}

pub struct App {
    pub ctx: egui::Context,
    pub state: State,
    snapshot: Rc<Project>,
    pub page: Page,
    pub toast: Option<(String, Instant)>,
    pub forms: Vec<Form>,
    pub confirms: Vec<Confirm>,
    pending: Vec<Box<dyn FnOnce(&mut App)>>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    last_watch: Instant,
    /// The problems panel at the bottom is open.
    pub problems_open: bool,
    /// Set while an update downloads: the button is off.
    pub updating: bool,
    /// The window may close for real (the user answered the unsaved-changes question).
    closing: bool,
    /// The cards that are open, by id (`common::card`), so that an edit does not close them.
    pub open: std::collections::HashSet<String>,

    // ---- the state of each page ----
    pub canvas: pages::canvas::CanvasState,
    pub inspector: pages::inspector::InspectorState,
    pub endpoint: pages::endpoint::EndpointState,
    pub tables: pages::tablesview::TablesState,
    pub code: pages::codeview::CodeState,
    pub mcp: pages::mcp::McpState,
    pub flows: pages::flows::FlowsState,
}

pub fn stamp_of(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        Self::with_context(cc.egui_ctx.clone(), open)
    }

    pub fn with_context(ctx: egui::Context, open: Option<PathBuf>) -> Self {
        egui_extras::install_image_loaders(&ctx);
        crate::fonts::install(&ctx);
        crate::theme::apply(&ctx);
        let (tx, rx) = channel();
        let project = sb_model::sample::orders();
        let mut app = App {
            ctx,
            snapshot: Rc::new(project.clone()),
            state: State {
                project,
                path: None,
                stamp: 0,
                analysis: Rc::new(Analysis {
                    diagnostics: vec![],
                    files: vec![],
                }),
                generation: 0,
                update: None,
                dirty: false,
                undo: vec![],
                redo: vec![],
            },
            page: Page::Architecture,
            toast: None,
            forms: vec![],
            confirms: vec![],
            pending: vec![],
            tx,
            rx,
            last_watch: Instant::now(),
            problems_open: false,
            updating: false,
            closing: false,
            open: Default::default(),
            canvas: Default::default(),
            inspector: Default::default(),
            endpoint: Default::default(),
            tables: Default::default(),
            code: Default::default(),
            mcp: Default::default(),
            flows: Default::default(),
        };
        if let Some(path) = open {
            match sb_app::load_project(&path) {
                Ok(project) => app.set_project(project, Some(path)),
                Err(e) => app.fail(&e),
            }
        }
        app.changed();
        if std::env::var_os("SB_NO_UPDATE").is_none() {
            crate::updates::check_for_update(&mut app, false);
        }
        app
    }

    // ---- reading ----

    /// The project as it was at the start of this change: read from it while drawing.
    pub fn project(&self) -> Rc<Project> {
        self.snapshot.clone()
    }

    pub fn analysis(&self) -> Rc<Analysis> {
        self.state.analysis.clone()
    }

    // ---- editing ----

    /// Applies `change` to the project. A change that changes nothing leaves no trace.
    pub fn edit(&mut self, change: impl FnOnce(&mut Project)) {
        let s = &mut self.state;
        let before = s.project.clone();
        change(&mut s.project);
        // Field numbers are frozen as they are given, so that a later deletion never renumbers the others.
        s.project.freeze_field_numbers();
        if s.project == before {
            return;
        }
        s.undo.push(before);
        if s.undo.len() > HISTORY {
            s.undo.remove(0);
        }
        s.redo.clear();
        s.dirty = true;
        self.changed();
    }

    pub fn undo(&mut self) {
        self.step(true);
    }

    pub fn redo(&mut self) {
        self.step(false);
    }

    fn step(&mut self, back: bool) {
        let s = &mut self.state;
        let (from, to) = if back {
            (&mut s.undo, &mut s.redo)
        } else {
            (&mut s.redo, &mut s.undo)
        };
        let Some(previous) = from.pop() else {
            return;
        };
        to.push(std::mem::replace(&mut s.project, previous));
        s.dirty = true;
        self.changed();
    }

    pub fn set_project(&mut self, project: Project, path: Option<PathBuf>) {
        let s = &mut self.state;
        s.stamp = path.as_deref().map_or(0, stamp_of);
        s.project = project;
        s.path = path;
        s.dirty = false;
        s.undo.clear();
        s.redo.clear();
        self.changed();
    }

    /// The project changed: the next frame draws it, and it is validated and generated in the background.
    pub fn changed(&mut self) {
        self.state.generation += 1;
        self.snapshot = Rc::new(self.state.project.clone());
        let (project, generation, tx, ctx) = (
            self.state.project.clone(),
            self.state.generation,
            self.tx.clone(),
            self.ctx.clone(),
        );
        std::thread::spawn(move || {
            let analysis = sb_app::analyze(&project);
            let _ = tx.send(Done::Analysis(generation, analysis));
            ctx.request_repaint();
        });
        self.ctx.request_repaint();
    }

    pub fn load_sample(&mut self, name: &str) {
        let project = match name {
            "showcase" => sb_model::sample::showcase(),
            _ => sb_model::sample::orders(),
        };
        self.set_project(project, None);
    }

    /// Saves in place; a project that has no file yet asks for one.
    pub fn save(&mut self) {
        let Some(path) = self.state.path.clone() else {
            return crate::files::save_as(self);
        };
        match sb_app::save_project(&path, &self.state.project) {
            Ok(()) => {
                self.state.stamp = stamp_of(&path);
                self.state.dirty = false;
                self.say(&format!("Enregistré dans {}", path.display()));
            }
            Err(e) => self.fail(&e),
        }
    }

    /// Runs `then`, after asking what to do with the unsaved changes if there are any.
    pub fn guard(&mut self, then: impl FnOnce(&mut App) + 'static) {
        if !self.state.dirty {
            return then(self);
        }
        // One of the two buttons runs it.
        type Once = std::rc::Rc<std::cell::RefCell<Option<Box<dyn FnOnce(&mut App)>>>>;
        let then: Once = std::rc::Rc::new(std::cell::RefCell::new(Some(Box::new(then))));
        let other = then.clone();
        let run = |app: &mut App, then: &Once| {
            if let Some(f) = then.borrow_mut().take() {
                f(app)
            }
        };
        self.confirm(
            "Le projet a des modifications non enregistrées",
            "Que faire de ces modifications ?",
            vec![
                ("Annuler".into(), false, None),
                (
                    "Abandonner".into(),
                    false,
                    Some(Box::new(move |app: &mut App| run(app, &then))),
                ),
                (
                    "Enregistrer".into(),
                    true,
                    Some(Box::new(move |app: &mut App| {
                        app.save();
                        // A project without a file opens the file dialog: the user retries once it is saved.
                        if !app.state.dirty {
                            run(app, &other)
                        }
                    })),
                ),
            ],
        );
    }

    // ---- messages ----

    /// A message in the corner, for a few seconds.
    pub fn say(&mut self, text: &str) {
        self.toast = (!text.is_empty()).then(|| (text.to_string(), Instant::now()));
        self.ctx.request_repaint();
    }

    pub fn fail(&mut self, error: &str) {
        self.say(error);
        self.confirm(
            "Une erreur est survenue",
            error,
            vec![("Fermer".into(), true, None)],
        );
    }

    pub fn confirm(
        &mut self,
        title: &str,
        detail: &str,
        buttons: Vec<(String, bool, Option<Box<dyn FnOnce(&mut App)>>)>,
    ) {
        self.confirms.push(Confirm {
            title: title.into(),
            detail: detail.into(),
            buttons,
        });
    }

    /// Runs `f` once the current frame is drawn.
    pub fn later(&mut self, f: impl FnOnce(&mut App) + 'static) {
        self.pending.push(Box::new(f));
    }

    /// Runs `work` on a thread, then `then` with its result on the UI thread.
    pub fn spawn<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
        then: impl FnOnce(&mut App, T) + Send + 'static,
    ) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let out = work();
            let _ = tx.send(Done::Run(Box::new(move |app| then(app, out))));
            ctx.request_repaint();
        });
    }

    /// A sender for threads that report progress (`Done::Run` closures).
    pub fn sender(&self) -> (Sender<Done>, egui::Context) {
        (self.tx.clone(), self.ctx.clone())
    }

    pub fn window_title(&self) -> String {
        let mark = if self.state.dirty { "• " } else { "" };
        format!("{mark}{} — {TITLE}", self.state.project.name)
    }

    // ---- the frame ----

    fn poll(&mut self) {
        while let Ok(done) = self.rx.try_recv() {
            match done {
                Done::Analysis(generation, analysis) => {
                    if generation == self.state.generation {
                        self.state.analysis = Rc::new(analysis);
                    }
                }
                Done::Run(f) => f(self),
            }
        }
        // An AI client edits the file on disk: reload it when it changes.
        if self.last_watch.elapsed() >= Duration::from_secs(2) {
            self.last_watch = Instant::now();
            self.watch_file();
        }
        self.ctx.request_repaint_after(Duration::from_secs(2));
        if let Some((_, at)) = &self.toast {
            let left = Duration::from_secs(4).saturating_sub(at.elapsed());
            if left.is_zero() {
                self.toast = None;
            } else {
                self.ctx.request_repaint_after(left);
            }
        }
    }

    fn watch_file(&mut self) {
        let Some(path) = self.state.path.clone() else {
            return;
        };
        let now = stamp_of(&path);
        // Unsaved edits are not thrown away for another program's: saving overwrites, as the user chose.
        if now == 0 || now == self.state.stamp || self.state.dirty {
            return;
        }
        // A file half-written by the other program fails to load: the next tick tries again, without a complaint.
        if let Ok(project) = sb_app::load_project(&path) {
            self.set_project(project, Some(path));
            self.say("Le projet a été modifié par un autre programme : rechargé.");
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let hit = |m, k| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)));
        // A modal open keeps the keys to itself.
        if !self.forms.is_empty() || !self.confirms.is_empty() {
            return;
        }
        if hit(shift, Key::S) {
            crate::files::save_as(self);
        } else if hit(Modifiers::COMMAND, Key::S) {
            self.save();
        }
        if hit(Modifiers::COMMAND, Key::O) {
            self.guard(crate::files::pick_project);
        }
        if hit(shift, Key::Z) {
            self.redo();
        } else if hit(Modifiers::COMMAND, Key::Z) {
            // Not while typing: the text field has its own undo.
            if ctx.memory(|m| m.focused().is_none()) {
                self.undo();
            }
        }
    }

    fn close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.closing || !self.state.dirty {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        if self.confirms.is_empty() {
            self.guard(|app| {
                app.closing = true;
                app.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            });
        }
    }

    /// One frame: the toolbar, the page, the problems, then what floats over them.
    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        self.poll();
        self.shortcuts(ctx);
        self.close_request(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));

        crate::header::toolbar(self, ui);
        crate::problems::panel(self, ui);
        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(crate::theme::BG))
            .show(ui, |ui| match self.page {
                Page::Architecture => pages::canvas::page(self, ui),
                Page::Endpoint => pages::endpoint::page(self, ui),
                Page::Flows => pages::flows::page(self, ui),
                Page::Tables => pages::tablesview::page(self, ui),
                Page::Mcp => pages::mcp::page(self, ui),
                Page::Code => pages::codeview::page(self, ui),
            });

        crate::widgets::toast(self, ctx);
        crate::form::show_modals(self, ctx);

        for f in std::mem::take(&mut self.pending) {
            f(self);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }
}
