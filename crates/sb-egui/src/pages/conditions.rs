//! The editor of a condition, in words and without code: a comparison, « et » / « ou » / « non » over other
//! conditions, a value that is present, absent or empty, a value among a list, a yes/no operation, or Rust.
//! A condition is a tree: the groups hold conditions that are edited by this same function.

use eframe::egui::{self, Frame, Margin, RichText, Stroke, Ui};
use sb_app::scope::Var;
use sb_model::{CmpOp, Condition, Expr};

use super::assignments::{expr_editor, pick};
use super::common::{commit_entry, icon_btn};
use crate::app::App;
use crate::theme::*;

const KINDS: [&str; 10] = [
    "comparaison",
    "tous ces cas (et)",
    "l’un de ces cas (ou)",
    "le contraire de (non)",
    "est présent",
    "est absent",
    "est vide",
    "fait partie de",
    "est vrai (oui/non)",
    "Rust",
];

const OPS: [(&str, CmpOp); 6] = [
    ("=", CmpOp::Eq),
    ("≠", CmpOp::Ne),
    (">", CmpOp::Gt),
    ("≥", CmpOp::Ge),
    ("<", CmpOp::Lt),
    ("≤", CmpOp::Le),
];

fn empty_path() -> Expr {
    Expr::Path {
        path: String::new(),
    }
}

pub fn default_compare() -> Condition {
    Condition::Compare {
        left: empty_path(),
        op: CmpOp::Eq,
        right: Expr::Literal {
            value: serde_json::Value::from(0),
        },
    }
}

fn kind_of(c: &Condition) -> &'static str {
    match c {
        Condition::Compare { .. } => KINDS[0],
        Condition::And { .. } => KINDS[1],
        Condition::Or { .. } => KINDS[2],
        Condition::Not { .. } => KINDS[3],
        Condition::Present { .. } => KINDS[4],
        Condition::Absent { .. } => KINDS[5],
        Condition::Empty { .. } => KINDS[6],
        Condition::In { .. } => KINDS[7],
        Condition::Test { .. } => KINDS[8],
        Condition::Raw { .. } => KINDS[9],
    }
}

/// What picking a kind gives: the group kinds wrap the current condition, so that nothing typed is lost.
fn switched(current: &Condition, kind: &str) -> Condition {
    let value = || match current {
        Condition::Compare { left, .. } => left.clone(),
        Condition::Present { value }
        | Condition::Absent { value }
        | Condition::Empty { value }
        | Condition::Test { value }
        | Condition::In { value, .. } => value.clone(),
        _ => empty_path(),
    };
    match kind {
        k if k == KINDS[1] => Condition::And {
            conditions: vec![current.clone(), default_compare()],
        },
        k if k == KINDS[2] => Condition::Or {
            conditions: vec![current.clone(), default_compare()],
        },
        k if k == KINDS[3] => Condition::Not {
            condition: Box::new(current.clone()),
        },
        k if k == KINDS[4] => Condition::Present { value: value() },
        k if k == KINDS[5] => Condition::Absent { value: value() },
        k if k == KINDS[6] => Condition::Empty { value: value() },
        k if k == KINDS[7] => Condition::In {
            value: value(),
            values: vec![Expr::Literal {
                value: serde_json::Value::from(0),
            }],
        },
        k if k == KINDS[8] => Condition::Test { value: value() },
        k if k == KINDS[9] => Condition::Raw {
            code: String::new(),
        },
        _ => default_compare(),
    }
}

/// A condition: its kind, then what goes with it. Returns the new condition when something changed.
pub fn condition_editor(
    app: &mut App,
    ui: &mut Ui,
    id: egui::Id,
    cond: &Condition,
    vars: &[Var],
) -> Option<Condition> {
    let mut next: Option<Condition> = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let now = kind_of(cond);
        let kinds: Vec<String> = KINDS.iter().map(|k| (*k).to_string()).collect();
        if let Some(k) = pick(ui, id.with("kind"), now, &kinds, f32::MAX)
            && k != now
        {
            next = Some(switched(cond, &k));
        }
        match cond {
            Condition::Compare { left, op, right } => {
                if let Some(Some(e)) =
                    expr_editor(app, ui, id.with("left"), None, false, Some(left), vars)
                {
                    next = Some(Condition::Compare {
                        left: e,
                        op: *op,
                        right: right.clone(),
                    });
                }
                let names: Vec<String> = OPS.iter().map(|(n, _)| (*n).to_string()).collect();
                let shown = OPS.iter().find(|(_, o)| o == op).map_or("=", |(n, _)| *n);
                if let Some(v) = pick(ui, id.with("op"), shown, &names, 60.0)
                    && let Some((_, o)) = OPS.iter().find(|(n, _)| *n == v)
                {
                    next = Some(Condition::Compare {
                        left: left.clone(),
                        op: *o,
                        right: right.clone(),
                    });
                }
                if let Some(Some(e)) =
                    expr_editor(app, ui, id.with("right"), None, false, Some(right), vars)
                {
                    next = Some(Condition::Compare {
                        left: left.clone(),
                        op: *op,
                        right: e,
                    });
                }
            }
            Condition::And { conditions } | Condition::Or { conditions } => {
                let and = matches!(cond, Condition::And { .. });
                let rebuild = |list: Vec<Condition>| {
                    if and {
                        Condition::And { conditions: list }
                    } else {
                        Condition::Or { conditions: list }
                    }
                };
                let word = if and { "ET" } else { "OU" };
                for (i, c) in conditions.iter().enumerate() {
                    if i > 0 {
                        ui.label(RichText::new(word).size(11.0).color(FAINT));
                    }
                    let mut remove = false;
                    Frame::new()
                        .stroke(Stroke::new(1.0, BORDER))
                        .inner_margin(Margin::same(8))
                        .corner_radius(8)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            if let Some(n) = condition_editor(app, ui, id.with(i), c, vars) {
                                let mut list = conditions.clone();
                                list[i] = n;
                                next = Some(rebuild(list));
                            }
                            if icon_btn(ui, "trash", "Retirer ce cas", true).clicked() {
                                remove = true;
                            }
                        });
                    if remove {
                        let mut list = conditions.clone();
                        list.remove(i);
                        next = Some(rebuild(list));
                    }
                }
                if super::common::link(ui, "+ Ajouter un cas").clicked() {
                    let mut list = conditions.clone();
                    list.push(default_compare());
                    next = Some(rebuild(list));
                }
            }
            Condition::Not { condition } => {
                if let Some(n) = condition_editor(app, ui, id.with("not"), condition, vars) {
                    next = Some(Condition::Not {
                        condition: Box::new(n),
                    });
                }
            }
            Condition::Present { value }
            | Condition::Absent { value }
            | Condition::Empty { value }
            | Condition::Test { value } => {
                if let Some(Some(e)) =
                    expr_editor(app, ui, id.with("value"), None, false, Some(value), vars)
                {
                    next = Some(match cond {
                        Condition::Present { .. } => Condition::Present { value: e },
                        Condition::Absent { .. } => Condition::Absent { value: e },
                        Condition::Empty { .. } => Condition::Empty { value: e },
                        _ => Condition::Test { value: e },
                    });
                }
            }
            Condition::In { value, values } => {
                if let Some(Some(e)) =
                    expr_editor(app, ui, id.with("value"), None, false, Some(value), vars)
                {
                    next = Some(Condition::In {
                        value: e,
                        values: values.clone(),
                    });
                }
                ui.label(RichText::new("fait partie de :").size(11.0).color(FAINT));
                for (i, v) in values.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if icon_btn(ui, "trash", "Retirer cette valeur", true).clicked() {
                            let mut list = values.clone();
                            list.remove(i);
                            next = Some(Condition::In {
                                value: value.clone(),
                                values: list,
                            });
                        }
                        if let Some(Some(e)) =
                            expr_editor(app, ui, id.with(("item", i)), None, false, Some(v), vars)
                        {
                            let mut list = values.clone();
                            list[i] = e;
                            next = Some(Condition::In {
                                value: value.clone(),
                                values: list,
                            });
                        }
                    });
                }
                if super::common::link(ui, "+ Ajouter une valeur").clicked() {
                    let mut list = values.clone();
                    list.push(Expr::Literal {
                        value: serde_json::Value::from(0),
                    });
                    next = Some(Condition::In {
                        value: value.clone(),
                        values: list,
                    });
                }
            }
            Condition::Raw { code } => {
                if let Some(v) = commit_entry(
                    ui,
                    id.with("raw"),
                    code,
                    true,
                    "condition Rust, par exemple input.amount > 0",
                    true,
                ) {
                    next = Some(Condition::Raw { code: v });
                }
            }
        }
    });
    next
}
