//! Step fragments: named sequences of steps played in place by `include` steps.
//!
//! The model keeps the `include` steps (that is what the editor shows and what is saved). The validator and the
//! generators work on an expanded copy, [`Project::with_fragments`], where every `include` is replaced by the steps of
//! its fragment. The steps taken from a fragment get the id `<include id>__<step id>`, so ids stay unique when a
//! fragment is included twice, and [`original_path`] turns a diagnostic of the expanded copy back into a path of the
//! project.

use std::collections::HashMap;

use crate::project::{LogLevel, Project, Service, Step};
use crate::validate::{Diagnostic, Severity};

/// Separates the id of an `include` from the id of a step it brought in.
pub const SEP: &str = "__";

/// Fragments that include each other deeper than this are refused (a cycle is refused anyway).
const MAX_DEPTH: usize = 8;

impl Project {
    /// A copy where every `include` step is replaced by the steps of its fragment. An include that cannot be played
    /// (unknown fragment, empty, cycle, too deep) becomes a no-op log step, so that the other steps keep their index:
    /// the problem itself is reported by [`diagnostics`].
    pub fn with_fragments(&self) -> Project {
        let mut out = self.clone();
        for svc in &mut out.services {
            if !svc.has_includes() {
                continue;
            }
            let table: HashMap<String, Vec<Step>> = svc
                .fragments
                .iter()
                .map(|f| (f.name.clone(), f.steps.clone()))
                .collect();
            let play = |steps: &mut Vec<Step>| {
                *steps = expand(steps, &table, "", &mut Vec::new());
            };
            svc.rpcs.iter_mut().for_each(|h| play(&mut h.steps));
            svc.routes.iter_mut().for_each(|h| play(&mut h.steps));
            svc.consumers.iter_mut().for_each(|h| play(&mut h.steps));
            svc.jobs.iter_mut().for_each(|h| play(&mut h.steps));
        }
        out
    }
}

impl Service {
    /// Whether a handler of the service has an `include` step.
    pub fn has_includes(&self) -> bool {
        let mut found = false;
        for step in self.handler_steps() {
            step.walk(&mut |s| found |= matches!(s, Step::Include { .. }));
        }
        found
    }
}

fn expand(
    steps: &[Step],
    table: &HashMap<String, Vec<Step>>,
    prefix: &str,
    stack: &mut Vec<String>,
) -> Vec<Step> {
    let mut out = Vec::new();
    for step in steps {
        if let Step::Include { id, fragment } = step {
            let name = format!("{prefix}{id}");
            match table.get(fragment) {
                Some(body)
                    if !body.is_empty() && !stack.contains(fragment) && stack.len() < MAX_DEPTH =>
                {
                    stack.push(fragment.clone());
                    out.extend(expand(body, table, &format!("{name}{SEP}"), stack));
                    stack.pop();
                }
                _ => out.push(Step::Log {
                    id: name,
                    level: LogLevel::Debug,
                    message: String::new(),
                }),
            }
            continue;
        }
        let mut step = step.clone();
        if !prefix.is_empty() {
            let renamed = format!("{prefix}{}", step.id());
            set_id(&mut step, renamed);
        }
        match &mut step {
            Step::Branch {
                then, otherwise, ..
            } => {
                *then = expand(then, table, prefix, stack);
                *otherwise = expand(otherwise, table, prefix, stack);
            }
            Step::ForEach { steps, .. } => *steps = expand(steps, table, prefix, stack),
            _ => {}
        }
        out.push(step);
    }
    out
}

/// Gives a step another id (every variant has an `id` field, serialized under that name).
fn set_id(step: &mut Step, id: String) {
    if let Ok(mut v) = serde_json::to_value(&*step) {
        v["id"] = serde_json::Value::String(id);
        if let Ok(s) = serde_json::from_value(v) {
            *step = s;
        }
    }
}

/// What is wrong with the fragments of a project, with the paths of the project itself.
pub fn diagnostics(project: &Project) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut err = |code: &str, path: String, message: String| {
        out.push(Diagnostic {
            severity: Severity::Error,
            code: code.into(),
            message,
            path,
        });
    };
    for (si, svc) in project.services.iter().enumerate() {
        let base = format!("/services/{si}");
        let mut seen: Vec<&str> = Vec::new();
        for (fi, f) in svc.fragments.iter().enumerate() {
            let path = format!("{base}/fragments/{fi}");
            let ok_name = f.name.starts_with(|c: char| c.is_ascii_lowercase())
                && f.name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            if !ok_name {
                err(
                    "E_FRAGMENT_NAME",
                    format!("{path}/name"),
                    format!(
                        "fragment name `{}`: lower case letters, digits and `_`, starting with a letter",
                        f.name
                    ),
                );
            }
            if seen.contains(&f.name.as_str()) {
                err(
                    "E_FRAGMENT_DUPLICATE",
                    format!("{path}/name"),
                    format!("two fragments are named `{}`", f.name),
                );
            }
            seen.push(&f.name);
            if f.steps.is_empty() {
                err(
                    "E_FRAGMENT_EMPTY",
                    path.clone(),
                    format!("fragment `{}` has no step", f.name),
                );
            }
            if reaches(svc, &f.name, &f.name, 0) {
                err(
                    "E_FRAGMENT_CYCLE",
                    path,
                    format!("fragment `{}` includes itself, directly or not", f.name),
                );
            }
        }
        // The includes of the handlers and of the fragments themselves.
        let mut lists: Vec<(&[Step], String)> = Vec::new();
        for (i, h) in svc.rpcs.iter().enumerate() {
            lists.push((&h.steps, format!("{base}/rpcs/{i}/steps")));
        }
        for (i, h) in svc.routes.iter().enumerate() {
            lists.push((&h.steps, format!("{base}/routes/{i}/steps")));
        }
        for (i, h) in svc.consumers.iter().enumerate() {
            lists.push((&h.steps, format!("{base}/consumers/{i}/steps")));
        }
        for (i, h) in svc.jobs.iter().enumerate() {
            lists.push((&h.steps, format!("{base}/jobs/{i}/steps")));
        }
        for (i, f) in svc.fragments.iter().enumerate() {
            lists.push((&f.steps, format!("{base}/fragments/{i}/steps")));
        }
        for (steps, at) in lists {
            walk_paths(steps, &at, &mut |s, path| {
                if let Step::Include { fragment, .. } = s
                    && !svc.fragments.iter().any(|f| &f.name == fragment)
                {
                    err(
                        "E_UNKNOWN_FRAGMENT",
                        format!("{path}/fragment"),
                        format!("the service has no fragment `{fragment}`"),
                    );
                }
            });
        }
    }
    out
}

/// Whether `from` includes `target`, through any number of fragments.
fn reaches(svc: &Service, from: &str, target: &str, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return true;
    }
    let Some(f) = svc.fragments.iter().find(|f| f.name == from) else {
        return false;
    };
    let mut hit = false;
    for step in &f.steps {
        step.walk(&mut |s| {
            if let Step::Include { fragment, .. } = s {
                hit |= fragment == target || reaches(svc, fragment, target, depth + 1);
            }
        });
    }
    hit
}

/// Visits every step of a list and of the lists nested in it, with its JSON pointer.
fn walk_paths(steps: &[Step], at: &str, f: &mut impl FnMut(&Step, &str)) {
    for (i, s) in steps.iter().enumerate() {
        let path = format!("{at}/{i}");
        f(s, &path);
        match s {
            Step::Branch {
                then, otherwise, ..
            } => {
                walk_paths(then, &format!("{path}/then"), f);
                walk_paths(otherwise, &format!("{path}/otherwise"), f);
            }
            Step::ForEach { steps, .. } => walk_paths(steps, &format!("{path}/steps"), f),
            _ => {}
        }
    }
}

/// The include a step of the expanded project was brought by (the outermost one), if it comes from a fragment.
fn origin(step: &Step) -> Option<&str> {
    step.id().split_once(SEP).map(|(inc, _)| inc)
}

/// Turns a path of the expanded project into a path of the project: what is inside an included fragment is reported on
/// the `include` step, and the index of the steps after one is shifted back.
pub fn original_path(expanded: &Project, path: &str) -> String {
    let segs: Vec<&str> = path.split('/').collect();
    // /services/N/<kind>/M/steps/...
    if segs.len() < 7 || segs[1] != "services" || segs[5] != "steps" {
        return path.to_string();
    }
    let svc = segs[2]
        .parse::<usize>()
        .ok()
        .and_then(|i| expanded.services.get(i));
    let (Some(svc), Ok(m)) = (svc, segs[4].parse::<usize>()) else {
        return path.to_string();
    };
    let found = match segs[3] {
        "rpcs" => svc.rpcs.get(m).map(|h| h.steps.as_slice()),
        "routes" => svc.routes.get(m).map(|h| h.steps.as_slice()),
        "consumers" => svc.consumers.get(m).map(|h| h.steps.as_slice()),
        "jobs" => svc.jobs.get(m).map(|h| h.steps.as_slice()),
        _ => None,
    };
    let Some(mut list) = found else {
        return path.to_string();
    };
    let mut out: Vec<String> = segs[..6].iter().map(|s| s.to_string()).collect();
    let mut k = 6;
    loop {
        let Some(i) = segs.get(k).and_then(|s| s.parse::<usize>().ok()) else {
            out.extend(segs[k..].iter().map(|s| s.to_string()));
            return out.join("/");
        };
        let Some(step) = list.get(i) else {
            return path.to_string();
        };
        // The index among the steps the user wrote: one per run of steps brought by the same include.
        let mut index = 0usize;
        let mut prev: Option<&str> = None;
        for (j, s) in list[..=i].iter().enumerate() {
            let o = origin(s);
            if j > 0 && !(o.is_some() && o == prev) {
                index += 1;
            }
            prev = o;
        }
        out.push(index.to_string());
        if origin(step).is_some() {
            // Inside an include: the include is where the problem is shown.
            return out.join("/");
        }
        k += 1;
        match (step, segs.get(k).copied()) {
            (Step::Branch { then, .. }, Some("then")) => list = then,
            (Step::Branch { otherwise, .. }, Some("otherwise")) => list = otherwise,
            (Step::ForEach { steps, .. }, Some("steps")) => list = steps,
            _ => {
                out.extend(segs[k..].iter().map(|s| s.to_string()));
                return out.join("/");
            }
        }
        out.push(segs[k].to_string());
        k += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Fragment;
    use crate::sample;

    fn log(id: &str) -> Step {
        Step::Log {
            id: id.into(),
            level: LogLevel::Info,
            message: id.into(),
        }
    }

    fn include(id: &str, fragment: &str) -> Step {
        Step::Include {
            id: id.into(),
            fragment: fragment.into(),
        }
    }

    fn with_job(steps: Vec<Step>) -> Project {
        let mut p = sample::orders();
        let svc = &mut p.services[0];
        svc.fragments = vec![
            Fragment {
                name: "guard".into(),
                steps: vec![log("a"), log("b")],
            },
            Fragment {
                name: "outer".into(),
                steps: vec![log("x"), include("in", "guard")],
            },
        ];
        svc.rpcs.clear();
        svc.routes.clear();
        svc.consumers.clear();
        svc.jobs = vec![crate::project::Job {
            name: "j".into(),
            every_secs: 60,
            steps,
        }];
        p
    }

    fn ids(p: &Project) -> Vec<String> {
        p.services[0].jobs[0]
            .steps
            .iter()
            .map(|s| s.id().to_string())
            .collect()
    }

    #[test]
    fn an_include_is_replaced_by_the_steps_of_its_fragment() {
        let p = with_job(vec![
            log("first"),
            include("g1", "guard"),
            include("g2", "outer"),
            log("last"),
        ]);
        assert_eq!(
            ids(&p.with_fragments()),
            [
                "first",
                "g1__a",
                "g1__b",
                "g2__x",
                "g2__in__a",
                "g2__in__b",
                "last"
            ]
        );
    }

    #[test]
    fn a_bad_include_keeps_its_place() {
        let mut p = with_job(vec![include("u", "nope"), log("after")]);
        p.services[0].fragments[0]
            .steps
            .push(include("self", "guard"));
        assert_eq!(ids(&p.with_fragments())[0], "u");
        let codes: Vec<String> = diagnostics(&p).into_iter().map(|d| d.code).collect();
        assert!(codes.contains(&"E_UNKNOWN_FRAGMENT".to_string()));
        assert!(codes.contains(&"E_FRAGMENT_CYCLE".to_string()));
    }

    #[test]
    fn paths_go_back_to_the_include() {
        let p = with_job(vec![log("first"), include("g1", "guard"), log("last")]);
        let e = p.with_fragments();
        let at = |s: &str| original_path(&e, s);
        assert_eq!(
            at("/services/0/jobs/0/steps/0/message"),
            "/services/0/jobs/0/steps/0/message"
        );
        assert_eq!(
            at("/services/0/jobs/0/steps/2/message"),
            "/services/0/jobs/0/steps/1"
        );
        assert_eq!(
            at("/services/0/jobs/0/steps/3/message"),
            "/services/0/jobs/0/steps/2/message"
        );
    }

    fn bind(id: &str, name: &str, value: &str) -> Step {
        Step::Let {
            id: id.into(),
            name: name.into(),
            value_type: crate::project::FieldType::String,
            optional: false,
            repeated: false,
            mutable: false,
            value: crate::project::Expr::Literal {
                value: serde_json::json!(value),
            },
        }
    }

    fn use_of(id: &str, name: &str) -> Step {
        Step::Let {
            id: id.into(),
            name: format!("{name}_again"),
            value_type: crate::project::FieldType::String,
            optional: false,
            repeated: false,
            mutable: false,
            value: crate::project::Expr::Path { path: name.into() },
        }
    }

    #[test]
    fn what_a_fragment_binds_is_visible_after_it_and_its_errors_land_on_the_include() {
        let mut p = with_job(vec![include("g", "bind"), use_of("u", "who")]);
        p.services[0].fragments = vec![crate::project::Fragment {
            name: "bind".into(),
            steps: vec![bind("b", "who", "me")],
        }];
        let errors = |p: &Project| -> Vec<(String, String)> {
            crate::validate(p)
                .into_iter()
                .filter(|d| d.severity == Severity::Error && d.path.starts_with("/services/0/"))
                .map(|d| (d.code, d.path))
                .collect()
        };
        assert_eq!(errors(&p), vec![]);
        // Reading what the fragment never binds is an error, on the step that reads it (its index is the user's).
        p.services[0].jobs[0].steps[1] = use_of("u", "nobody");
        let e = errors(&p);
        assert!(
            e.iter()
                .any(|(_, path)| path.starts_with("/services/0/jobs/0/steps/1/")),
            "{e:?}"
        );
        // A mistake inside the fragment is shown on the include.
        p.services[0].jobs[0].steps[1] = use_of("u", "who");
        p.services[0].fragments[0].steps[0] = use_of("b", "ghost");
        let e = errors(&p);
        assert!(
            e.iter()
                .any(|(_, path)| path == "/services/0/jobs/0/steps/0"),
            "{e:?}"
        );
    }

    #[test]
    fn names_are_checked() {
        let mut p = with_job(vec![]);
        p.services[0].fragments[0].name = "Bad Name".into();
        p.services[0].fragments[1].name = "Bad Name".into();
        let codes: Vec<String> = diagnostics(&p).into_iter().map(|d| d.code).collect();
        assert!(codes.contains(&"E_FRAGMENT_NAME".to_string()));
        assert!(codes.contains(&"E_FRAGMENT_DUPLICATE".to_string()));
    }
}
