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
                record(&mut t.renamed_from, from, to);
            }
            for f in &mut t.foreign_keys {
                if f.ref_table == from {
                    f.ref_table = to.into();
                }
            }
        }
        s.dropped_tables.retain(|d| d != to);
    }
    for steps in handlers(p, |n| n == service) {
        rewrite(steps, |o| {
            if kind(o) == "sql" {
                set_if(o, "table", from, to);
            }
        });
    }
}

/// Writes a rename in a journal of old names (oldest first). Going back to a name the thing already had cuts the history
/// there: a database at any of the names then reaches the current one, with no loop to replay.
fn record(history: &mut Vec<String>, from: &str, to: &str) {
    if let Some(at) = history.iter().position(|n| n == to) {
        history.truncate(at);
    } else if !history.iter().any(|n| n == from) {
        history.push(from.into());
    }
}

/// Renames a column and everything that names it: the other columns' constraints, the foreign keys that point at it, the
/// tenant column and the SQL steps on the table. The rename is recorded in the column's journal (`renamedFrom`), which
/// is what lets the migration rename the column of an existing database instead of adding an empty one.
pub fn column(p: &mut Project, service: &str, table: &str, from: &str, to: &str) {
    if let Some(s) = p.services.iter_mut().find(|s| s.name == service) {
        for t in &mut s.tables {
            if t.name == table {
                for c in &mut t.columns {
                    if c.name == from {
                        c.name = to.into();
                        record(&mut c.renamed_from, from, to);
                    }
                }
                let fix = |list: &mut Vec<String>| {
                    list.iter_mut()
                        .filter(|c| *c == from)
                        .for_each(|c| *c = to.into())
                };
                t.uniques.iter_mut().for_each(|u| fix(&mut u.columns));
                t.indexes.iter_mut().for_each(|i| fix(&mut i.columns));
                t.foreign_keys.iter_mut().for_each(|f| fix(&mut f.columns));
                if t.tenant_column.as_deref() == Some(from) {
                    t.tenant_column = Some(to.into());
                }
                t.dropped_columns.retain(|d| d != to);
            }
            for f in &mut t.foreign_keys {
                if f.ref_table == table {
                    f.ref_columns
                        .iter_mut()
                        .filter(|c| *c == from)
                        .for_each(|c| *c = to.into());
                }
            }
        }
    }
    for steps in handlers(p, |n| n == service) {
        rewrite(steps, |o| {
            if kind(o) != "sql" || o.get("table").and_then(Value::as_str) != Some(table) {
                return;
            }
            set_if(o, "orderBy", from, to);
            if let Some(Value::Array(columns)) = o.get_mut("columns") {
                for c in columns {
                    if c.as_str() == Some(from) {
                        *c = to.into();
                    }
                }
            }
            for list in ["where", "values"] {
                if let Some(Value::Array(items)) = o.get_mut(list) {
                    for item in items.iter_mut().filter_map(Value::as_object_mut) {
                        set_if(item, "column", from, to);
                        set_if(item, "field", from, to);
                    }
                }
            }
        });
    }
}

/// Removes a column from the design and records the drop: the migration then drops it, with its data, from a database
/// that still has it. Its old names are dropped too (a database may still carry one), and so is everything of the table
/// that was built on the column.
pub fn drop_column(p: &mut Project, service: &str, table: &str, name: &str) {
    let Some(s) = p.services.iter_mut().find(|s| s.name == service) else {
        return;
    };
    for t in &mut s.tables {
        if t.name == table {
            let Some(at) = t.columns.iter().position(|c| c.name == name) else {
                return;
            };
            let gone = t.columns.remove(at);
            for old in gone.renamed_from.into_iter().chain([name.to_string()]) {
                if !t.dropped_columns.contains(&old) {
                    t.dropped_columns.push(old);
                }
            }
            t.uniques.retain(|u| !u.columns.iter().any(|c| c == name));
            t.indexes.retain(|i| !i.columns.iter().any(|c| c == name));
            t.foreign_keys
                .retain(|f| !f.columns.iter().any(|c| c == name));
            if t.tenant_column.as_deref() == Some(name) {
                t.tenant_column = None;
            }
        }
    }
    // A foreign key pointing at the column cannot survive it.
    for t in &mut s.tables {
        t.foreign_keys
            .retain(|f| !(f.ref_table == table && f.ref_columns.iter().any(|c| c == name)));
    }
}

/// Removes a table from the design and records the drop (its old names too), with the foreign keys that point at it.
pub fn drop_table(p: &mut Project, service: &str, table: &str) {
    let Some(s) = p.services.iter_mut().find(|s| s.name == service) else {
        return;
    };
    let Some(at) = s.tables.iter().position(|t| t.name == table) else {
        return;
    };
    let gone = s.tables.remove(at);
    for old in gone.renamed_from.into_iter().chain([table.to_string()]) {
        if !s.dropped_tables.contains(&old) {
            s.dropped_tables.push(old);
        }
    }
    for t in &mut s.tables {
        t.foreign_keys.retain(|f| f.ref_table != table);
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

    #[test]
    fn renaming_a_column_is_recorded_and_followed_by_the_steps() {
        let mut p = sample::orders();
        column(&mut p, "Orders", "orders", "amount", "total");
        let c = &p.services[0].tables[0].columns[1];
        assert_eq!(
            (c.name.as_str(), c.renamed_from.clone()),
            ("total", vec!["amount".to_string()])
        );
        assert_eq!(errors(&p), 0);
        // A second rename extends the history; going back to a former name cuts it.
        column(&mut p, "Orders", "orders", "total", "sum");
        assert_eq!(
            p.services[0].tables[0].columns[1].renamed_from,
            ["amount", "total"]
        );
        column(&mut p, "Orders", "orders", "sum", "amount");
        assert!(p.services[0].tables[0].columns[1].renamed_from.is_empty());
        assert_eq!(errors(&p), 0);
    }

    #[test]
    fn renaming_a_table_is_recorded() {
        let mut p = sample::orders();
        table(&mut p, "Orders", "orders", "purchases");
        assert_eq!(p.services[0].tables[0].renamed_from, ["orders"]);
        assert_eq!(errors(&p), 0);
    }

    #[test]
    fn dropping_is_recorded_with_the_old_names_and_cleans_what_leaned_on_it() {
        let mut p = sample::orders();
        column(&mut p, "Orders", "orders", "amount", "total");
        p.services[0].tables[0].indexes.push(sb_model::Index {
            name: "orders_total_idx".into(),
            columns: vec!["total".into()],
            unique: false,
        });
        // The steps that still read the column are what the validation then points at.
        drop_column(&mut p, "Orders", "orders", "total");
        let t = &p.services[0].tables[0];
        assert_eq!(t.dropped_columns, ["amount", "total"]);
        assert!(t.indexes.is_empty() && t.column("total").is_none());
        let mut p = sample::orders();
        drop_table(&mut p, "Orders", "orders");
        assert_eq!(p.services[0].dropped_tables, ["orders"]);
        assert!(p.services[0].tables.is_empty());
    }
}
