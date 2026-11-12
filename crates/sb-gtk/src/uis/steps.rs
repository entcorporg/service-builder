//! The steps of a flow: the tree of rows, and the form that edits one step.
//!
//! The forms come from `sb_app::stepform`, which derives them from the schema of the project.

use std::cell::Cell;

use gtk::prelude::*;
use sb_app::scope::variables;
use sb_app::stepform::{self, Kind as Spec};
use sb_model::{Project, Step};

use super::edit::edit;
use super::form::{Input, Kind, Value, ask};
use super::ui::Shared;

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

/// The variables a step at `index` of this list can read, as a text for its form.
/// The variables a step at `index` of this list can read, as a text for its form.
fn variables_text(ui: &Shared, loc: &Loc, index: usize) -> String {
    let all = vars_at(ui, loc, index);
    let shown: Vec<String> = all
        .iter()
        .take(40)
        .map(|v| format!("{} ({})", v.path, v.ty))
        .collect();
    let more = all.len().saturating_sub(40);
    let mut text = shown.join("  ·  ");
    if more > 0 {
        text.push_str(&format!("  · … {more} autres"));
    }
    text
}

/// The variables a step at `index` of this list can read.
pub fn vars_at(ui: &Shared, loc: &Loc, index: usize) -> Vec<sb_app::scope::Var> {
    let s = ui.state.borrow();
    let p = &s.project;
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
pub(super) fn new_id() -> String {
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

/// The cards of the steps, in one row of a list.
pub fn draw_steps(ui: &Shared, list: &[Step], loc: &Loc, out: &gtk::ListBox) {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let b = super::stepcards::steps_box(ui, list, loc);
    b.set_margin_start(12);
    b.set_margin_end(12);
    b.set_margin_top(4);
    b.set_margin_bottom(8);
    row.set_child(Some(&b));
    row.add_css_class("step-row");
    out.append(&row);
}

pub fn step_move(ui: &Shared, loc: &Loc, from: usize, to: usize) {
    move_step(ui, loc, from, to)
}

fn move_step(ui: &Shared, loc: &Loc, from: usize, to: usize) {
    let loc = loc.clone();
    edit(ui, move |p| {
        if let Some(v) = list_mut(p, &loc).filter(|v| from < v.len() && to < v.len()) {
            v.swap(from, to);
        }
    });
}

/// Asks which kind of step, then its fields, then appends it to the list.
pub fn add_step(ui: &Shared, loc: &Loc) {
    let kinds = stepform::kinds();
    let loc = loc.clone();
    let inputs = vec![Input::new(
        "Type d'étape",
        Kind::Choice {
            selected: kinds.first().cloned(),
            options: kinds,
        },
    )];
    ask(ui, "Nouvelle étape", inputs, move |ui, values| {
        let kind = values
            .into_iter()
            .next()
            .map(Value::text)
            .unwrap_or_default();
        let blank = stepform::blank(&kind, &new_id())?;
        let vars = variables_text(ui, &loc, usize::MAX);
        let loc = loc.clone();
        let append = move |ui: &Shared, step: Step| {
            let loc = loc.clone();
            edit(ui, move |p| {
                if let Some(v) = list_mut(p, &loc) {
                    v.push(step);
                }
            })
        };
        if stepform::specs(&kind).is_empty() {
            append(ui, blank);
        } else {
            step_form(
                ui,
                &format!("Nouvelle étape : {kind}"),
                &blank,
                &vars,
                append,
            );
        }
        Ok(())
    });
}

const SYNTAX: &str = "Expressions : a.b (variable) · \"texte\", 42, true (littéral) · now · rust: code\n\
Conditions : gauche == droite (== != > >= < <=) · Affectations : champ = expression, une par ligne";

pub fn step_form(
    ui: &Shared,
    title: &str,
    step: &Step,
    variables: &str,
    on_ok: impl Fn(&Shared, Step) + 'static,
) {
    let types: Vec<String> = {
        let s = ui.state.borrow();
        stepform::SCALARS
            .iter()
            .map(|t| (*t).to_string())
            .chain(s.project.messages.iter().map(|m| m.name.clone()))
            .collect()
    };
    let fields = stepform::to_texts(step);
    let mut inputs = vec![];
    if fields
        .iter()
        .any(|(s, _)| matches!(s.kind, Spec::Expr | Spec::Condition | Spec::Assignments))
    {
        inputs.push(Input::new("Aide", Kind::Fixed(SYNTAX.into())));
    }
    for (spec, text) in &fields {
        let label = if spec.optional {
            format!("{} (facultatif)", spec.key)
        } else {
            spec.key.clone()
        };
        let kind = match &spec.kind {
            Spec::Assignments | Spec::Json => Kind::Multiline(text.clone()),
            Spec::Bool => Kind::Check(text == "true"),
            Spec::Choice(words) => Kind::Choice {
                options: words.clone(),
                selected: Some(text.clone()),
            },
            Spec::Type => Kind::Choice {
                options: types.clone(),
                selected: Some(text.clone()),
            },
            _ => Kind::Text(text.clone()),
        };
        inputs.push(Input::new(label, kind).help(spec.help.clone()));
    }
    if !variables.is_empty() {
        inputs.insert(
            0,
            Input::new("Variables disponibles", Kind::Fixed(variables.to_string())),
        );
    }
    let skip = inputs.len() - fields.len();
    let step = step.clone();
    ask(ui, title, inputs, move |ui, values| {
        let texts: Vec<String> = values
            .into_iter()
            .skip(skip)
            .map(Value::into_string)
            .collect();
        on_ok(ui, stepform::from_texts(&step, &texts)?);
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first rpc of the sample, with a branch (one step on each side) added at the end.
    fn with_branch() -> (Project, Loc, usize) {
        let mut p = sb_model::sample::orders();
        let log = |id: &str| stepform::blank("log", id).unwrap();
        let rpcs = &mut p.services[0].rpcs[0].steps;
        rpcs.push(Step::Branch {
            id: "b1".into(),
            condition: sb_model::Condition::Raw {
                code: "true".into(),
            },
            then: vec![log("t1"), log("t2")],
            otherwise: vec![log("o1")],
        });
        let at = rpcs.len() - 1;
        let root = Loc {
            service: 0,
            flow: Flow::Rpc(0),
            nest: vec![],
        };
        (p, root, at)
    }

    #[test]
    fn a_location_reaches_the_body_of_a_flow_and_the_sides_of_a_branch() {
        let (mut p, root, at) = with_branch();
        assert_eq!(list_mut(&mut p, &root.inside(false, at)).unwrap().len(), 2);
        assert_eq!(list_mut(&mut p, &root.inside(true, at)).unwrap().len(), 1);
        // A step that is not a branch holds nothing, so nothing is reached through it.
        assert!(list_mut(&mut p, &root.inside(false, 0)).is_none());
        // A step that is not a branch holds nothing; neither does a flow that is not there.
        assert!(list_mut(&mut p, &root.inside(false, at + 100)).is_none());
        let missing = Loc {
            flow: Flow::Rpc(99),
            ..root
        };
        assert!(list_mut(&mut p, &missing).is_none());
    }

    #[test]
    fn ids_do_not_repeat() {
        let ids: std::collections::HashSet<String> = (0..500).map(|_| new_id()).collect();
        assert_eq!(ids.len(), 500);
    }
}
