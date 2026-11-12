//! Renaming with its consequences: everything that refers to a name by that name follows it.
//!
//! The steps refer to topics, services, RPCs, tables and schemas by name, so a rename walks them as JSON, and only
//! touches the places that really hold that kind of name (a log text that happens to read like a topic is left alone).
//! The caller checks that the new name is valid and free.

use sb_model::{FieldType, Project, Step};
use serde_json::Value;

fn each_object(v: &mut Value, f: &mut impl FnMut(&mut serde_json::Map<String, Value>)) {
    match v {
        Value::Object(o) => {
            f(o);
            for child in o.values_mut() {
                each_object(child, f);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|c| each_object(c, f)),
        _ => {}
    }
}

fn set_if(o: &mut serde_json::Map<String, Value>, key: &str, from: &str, to: &str) {
    if o.get(key).and_then(Value::as_str) == Some(from) {
        o.insert(key.into(), to.into());
    }
}

/// Runs `f` on every object of `steps` (nested branches included).
fn rewrite(steps: &mut Vec<Step>, mut f: impl FnMut(&mut serde_json::Map<String, Value>)) {
    let Ok(mut json) = serde_json::to_value(&*steps) else {
        return;
    };
    each_object(&mut json, &mut f);
    if let Ok(new) = serde_json::from_value(json) {
        *steps = new;
    }
}

fn kind(o: &serde_json::Map<String, Value>) -> &str {
    o.get("type").and_then(Value::as_str).unwrap_or_default()
}

/// The step lists of every handler of the services accepted by `only`.
fn handlers(p: &mut Project, only: impl Fn(&str) -> bool) -> impl Iterator<Item = &mut Vec<Step>> {
    p.services
        .iter_mut()
        .filter(move |s| only(&s.name))
        .flat_map(|s| {
            s.rpcs
                .iter_mut()
                .map(|r| &mut r.steps)
                .chain(s.routes.iter_mut().map(|r| &mut r.steps))
                .chain(s.consumers.iter_mut().map(|c| &mut c.steps))
                .chain(s.jobs.iter_mut().map(|j| &mut j.steps))
        })
}

pub fn message(p: &mut Project, from: &str, to: &str) {
    for m in &mut p.messages {
        if m.name == from {
            m.name = to.into();
        }
        for f in &mut m.fields {
            if let FieldType::Message { name } = &mut f.ty
                && name == from
            {
                *name = to.into();
            }
        }
    }
    for t in &mut p.topics {
        if t.message == from {
            t.message = to.into();
        }
    }
    for s in &mut p.services {
        for r in &mut s.rpcs {
            if r.request == from {
                r.request = to.into();
            }
            if r.response == from {
                r.response = to.into();
            }
        }
        for r in &mut s.routes {
            if r.request == from {
                r.request = to.into();
            }
            if r.response == from {
                r.response = to.into();
            }
        }
    }
    for steps in handlers(p, |_| true) {
        rewrite(steps, |o| {
            match kind(o) {
                "sql" => set_if(o, "into", from, to),
                "verifyJwt" => set_if(o, "schema", from, to),
                _ => {}
            }
            // A type written as `{ "kind": "message", "name": ... }`, in a `let` for instance.
            if o.get("kind").and_then(Value::as_str) == Some("message") {
                set_if(o, "name", from, to);
            }
        });
    }
}

pub fn topic(p: &mut Project, from: &str, to: &str) {
    for t in &mut p.topics {
        if t.name == from {
            t.name = to.into();
        }
    }
    for s in &mut p.services {
        for c in &mut s.consumers {
            if c.topic == from {
                c.topic = to.into();
            }
        }
        for t in &mut s.templates {
            if t.event_type == from {
                t.event_type = to.into();
            }
        }
    }
    for steps in handlers(p, |_| true) {
        rewrite(steps, |o| {
            if kind(o) == "publish" {
                set_if(o, "topic", from, to);
            }
        });
    }
}

pub fn service(p: &mut Project, from: &str, to: &str) {
    for s in &mut p.services {
        if s.name == from {
            s.name = to.into();
        }
    }
    for t in &mut p.topics {
        for producer in &mut t.producers {
            if producer == from {
                *producer = to.into();
            }
        }
    }
    for steps in handlers(p, |_| true) {
        rewrite(steps, |o| {
            if matches!(kind(o), "callRpc" | "publishLive") {
                set_if(o, "service", from, to);
            }
        });
    }
}

pub fn rpc(p: &mut Project, service: &str, from: &str, to: &str) {
    if let Some(r) = p
        .services
        .iter_mut()
        .find(|s| s.name == service)
        .and_then(|s| s.rpcs.iter_mut().find(|r| r.name == from))
    {
        r.name = to.into();
    }
    for steps in handlers(p, |_| true) {
        rewrite(steps, |o| {
            if kind(o) == "callRpc" && o.get("service").and_then(Value::as_str) == Some(service) {
                set_if(o, "method", from, to);
            }
        });
    }
}

pub fn table(p: &mut Project, service: &str, from: &str, to: &str) {
    if let Some(s) = p.services.iter_mut().find(|s| s.name == service) {
        for t in &mut s.tables {
            if t.name == from {
                t.name = to.into();
            }
            for f in &mut t.foreign_keys {
                if f.ref_table == from {
                    f.ref_table = to.into();
                }
            }
        }
    }
    for steps in handlers(p, |n| n == service) {
        rewrite(steps, |o| {
            if kind(o) == "sql" {
                set_if(o, "table", from, to);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_model::{Severity, sample};

    fn errors(p: &Project) -> usize {
        sb_codegen::diagnose(p)
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    /// How many places of the project hold exactly this name.
    fn mentions(p: &Project, name: &str) -> usize {
        serde_json::to_string(p)
            .unwrap()
            .matches(&format!("\"{name}\""))
            .count()
    }

    #[test]
    fn renaming_never_breaks_a_valid_project_and_leaves_no_old_name_behind() {
        for base in [sample::orders(), sample::showcase()] {
            assert_eq!(errors(&base), 0);

            // The steps of JWKS, TOTP and passkeys require schemas of these names: renaming one is a validation
            // error, by design (the Problems page says so).
            const RESERVED: [&str; 4] = ["Jwk", "Jwks", "TotpSetup", "PasskeyChallenge"];
            for m in base
                .messages
                .iter()
                .map(|m| m.name.clone())
                .filter(|m| !RESERVED.contains(&m.as_str()))
            {
                let mut p = base.clone();
                message(&mut p, &m, "Renamed");
                assert_eq!(mentions(&p, &m), 0, "message {m}");
                assert_eq!(errors(&p), 0, "message {m}");
            }
            for t in base.topics.iter().map(|t| t.name.clone()) {
                let mut p = base.clone();
                topic(&mut p, &t, "renamed.topic");
                // (A dangling reference is a validation error, which is what is checked: a text may read like the old name without being one.)
                assert_eq!(errors(&p), 0, "topic {t}");
            }
            for s in base.services.iter().map(|s| s.name.clone()) {
                let mut p = base.clone();
                service(&mut p, &s, "Renamed");
                assert_eq!(errors(&p), 0, "service {s}");
            }
            for s in &base.services {
                for r in &s.rpcs {
                    let mut p = base.clone();
                    rpc(&mut p, &s.name, &r.name, "RenamedCall");
                    assert_eq!(errors(&p), 0, "rpc {}", r.name);
                }
                for t in &s.tables {
                    let mut p = base.clone();
                    table(&mut p, &s.name, &t.name, "renamed_table");
                    assert_eq!(errors(&p), 0, "table {}", t.name);
                }
            }
        }
    }

    #[test]
    fn a_text_that_reads_like_a_name_is_not_a_reference() {
        let mut p = sample::orders();
        let topic_name = p.topics[0].name.clone();
        let step = sb_model::Step::Log {
            id: "z1".into(),
            level: sb_model::LogLevel::Info,
            message: topic_name.clone(),
        };
        p.services[0].rpcs[0].steps.push(step.clone());
        topic(&mut p, &topic_name, "renamed.topic");
        assert_eq!(p.services[0].rpcs[0].steps.last(), Some(&step));
    }
}
