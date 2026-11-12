//! The steps of a flow: the tree of rows, and the form that edits one step.
//! The forms come from `sb_app::stepform`, which derives them from the schema of the project.
//! PORT TARGET: `sb-gtk/src/uis/steps.rs` (owner: steps agent). `Flow`, `Loc`, `list_mut`, `vars_at`, `new_id`
//! are already ported; the drawing and the forms are to port.

use std::cell::Cell;

use eframe::egui::Ui;
use sb_app::scope::variables;
use sb_model::{Project, Step};

use crate::app::App;

#[derive(Clone, Copy)]
pub enum Flow {
    Rpc(usize),
    Route(usize),
    Consumer(usize),
    Job(usize),
}

/// A list of steps: the body of a flow, or one side of a branch inside it.
#[derive(Clone)]
pub struct Loc {
    pub service: usize,
    pub flow: Flow,
    /// From the body of the flow inwards: `(otherwise?, index of the branch)`.
    pub nest: Vec<(bool, usize)>,
}

impl Loc {
    pub fn inside_pub(&self, otherwise: bool, branch: usize) -> Loc {
        self.inside(otherwise, branch)
    }

    fn inside(&self, otherwise: bool, branch: usize) -> Loc {
        let mut nest = self.nest.clone();
        nest.push((otherwise, branch));
        Loc {
            nest,
            ..self.clone()
        }
    }
}

pub fn list_mut<'a>(p: &'a mut Project, loc: &Loc) -> Option<&'a mut Vec<Step>> {
    let svc = p.services.get_mut(loc.service)?;
    let mut steps = match loc.flow {
        Flow::Rpc(i) => &mut svc.rpcs.get_mut(i)?.steps,
        Flow::Route(i) => &mut svc.routes.get_mut(i)?.steps,
        Flow::Consumer(i) => &mut svc.consumers.get_mut(i)?.steps,
        Flow::Job(i) => &mut svc.jobs.get_mut(i)?.steps,
    };
    for &(otherwise, branch) in &loc.nest {
        match steps.get_mut(branch)? {
            Step::Branch {
                then, otherwise: o, ..
            } => steps = if otherwise { o } else { then },
            _ => return None,
        }
    }
    Some(steps)
}

/// The variables a step at `index` of this list can read.
pub fn vars_at(app: &App, loc: &Loc, index: usize) -> Vec<sb_app::scope::Var> {
    let p = &*app.project();
    let Some(svc) = p.services.get(loc.service) else {
        return vec![];
    };
    let (initial, mut list): (Option<String>, &[Step]) = match loc.flow {
        Flow::Rpc(i) => svc
            .rpcs
            .get(i)
            .map(|r| (Some(r.request.clone()), &r.steps[..])),
        Flow::Route(i) => svc
            .routes
            .get(i)
            .map(|r| (Some(r.request.clone()), &r.steps[..])),
        Flow::Consumer(i) => svc
            .consumers
            .get(i)
            .map(|c| (p.topic(&c.topic).map(|t| t.message.clone()), &c.steps[..])),
        Flow::Job(i) => svc.jobs.get(i).map(|j| (None, &j.steps[..])),
    }
    .unwrap_or((None, &[]));
    // Variables are scoped to their block: what ran before, in the blocks that hold this one, then in this one.
    let mut before: Vec<Step> = vec![];
    for &(otherwise, branch) in &loc.nest {
        before.extend(list[..branch.min(list.len())].iter().cloned());
        match list.get(branch) {
            Some(Step::Branch {
                then, otherwise: o, ..
            }) => list = if otherwise { o } else { then },
            _ => return vec![],
        }
    }
    before.extend(list[..index.min(list.len())].iter().cloned());
    variables(p, initial.as_deref(), &before)
}

/// A short random id, unique enough within a project (the web editor uses 8 hex digits too).
pub fn new_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    thread_local!(static COUNTER: Cell<u64> = const { Cell::new(0) });
    let n = COUNTER.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    let mut h = RandomState::new().build_hasher();
    h.write_u64(n);
    format!("{:08x}", h.finish() as u32)
}

/// The cards of the steps of a list (with the dashed "add" at the end).
pub fn draw_steps(app: &mut App, ui: &mut Ui, list: &[Step], loc: &Loc) {
    super::stepcards::steps_box(app, ui, list, loc);
}

/// Moves a step of the list at `loc` from `from` to `to`.
pub fn step_move(app: &mut App, loc: &Loc, from: usize, to: usize) {
    let _ = (app, loc, from, to);
}

/// Asks which step to add at the end of the list at `loc`, then adds it.
pub fn add_step(app: &mut App, loc: &Loc) {
    let _ = (app, loc);
}

/// The modal form that edits one step (`sb_app::stepform`); `on_ok` gets the step as edited.
pub fn step_form(
    app: &mut App,
    title: &str,
    step: &Step,
    variables: &str,
    on_ok: impl Fn(&mut App, Step) + 'static,
) {
    let _ = (app, title, step, variables, on_ok);
}
