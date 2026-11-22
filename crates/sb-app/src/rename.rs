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
                "httpCall" => set_if(o, "response", from, to),
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
            // The audit event of a refusing check.
            if matches!(kind(o), "verifyPassword" | "totpVerify" | "recoveryCodeUse")
                && let Some(Value::Object(publish)) = o
                    .get_mut("onFail")
                    .and_then(Value::as_object_mut)
                    .and_then(|f| f.get_mut("publish"))
            {
                set_if(publish, "topic", from, to);
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
            if matches!(
                kind(o),
                "callRpc" | "publishLive" | "liveGrants" | "sessionTrace" | "sessionRevoke"
            ) {
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
                requalify(o, from, to);
            }
        });
    }
}

/// The joins of an SQL step name tables, and the columns of a joined table are written `table.column`. The fields of the
/// row are named after the table (`table_column`), and the schemas they are converted into hold those names: the old name is
/// kept as the alias of the join so that nothing there changes.
fn requalify(o: &mut serde_json::Map<String, Value>, from: &str, to: &str) {
    let prefix = format!("{from}.");
    let fix = |v: &mut Value| {
        if let Some(rest) = v.as_str().and_then(|s| s.strip_prefix(&prefix)) {
            *v = format!("{to}.{rest}").into();
        }
    };
    let fix_key = |v: &mut Value, key: &str| {
        if let Some(c) = v.get_mut(key) {
            fix(c);
        }
    };
    if let Some(joins) = o.get_mut("joins").and_then(Value::as_array_mut) {
        for j in joins {
            if j["table"] == from {
                if j.get("alias").is_none_or(Value::is_null) {
                    j["alias"] = from.into();
                }
                j["table"] = to.into();
            }
            for on in j["on"].as_array_mut().into_iter().flatten() {
                fix_key(on, "column");
            }
        }
    }
    if let Some(c) = o.get_mut("columns").and_then(Value::as_array_mut) {
        c.iter_mut().for_each(fix);
    }
    rekey_aliases(o, |name| {
        name.strip_prefix(&prefix)
            .map(|rest| format!("{to}.{rest}"))
    });
    if let Some(c) = o.get_mut("orderBy") {
        fix(c);
    }
    for key in ["thenBy", "where"] {
        for k in o
            .get_mut(key)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            fix_key(k, "column");
        }
    }
}

/// The keys of `columnAs` name columns: they follow the rename. The names they give are the author's, they stay.
fn rekey_aliases(o: &mut serde_json::Map<String, Value>, new_key: impl Fn(&str) -> Option<String>) {
    if let Some(map) = o.get_mut("columnAs").and_then(Value::as_object_mut) {
        let old = std::mem::take(map);
        for (k, v) in old {
            map.insert(new_key(&k).unwrap_or(k), v);
        }
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
    let mut fields = vec![];
    for steps in handlers(p, |n| n == service) {
        let mut rows = vec![];
        rewrite(steps, |o| {
            if kind(o) == "sql" {
                requalify_column(o, table, from, to, &mut rows, &mut fields);
            }
        });
        follow_row_fields(steps, &rows);
    }
    // The schemas the rows are converted into hold the field names: `<table>_<column>` follows the column.
    for m in &mut p.messages {
        for (message, old, new) in &fields {
            if m.name == *message && !m.fields.iter().any(|f| f.name == *new) {
                m.fields
                    .iter_mut()
                    .filter(|f| f.name == *old)
                    .for_each(|f| f.name = new.clone());
            }
        }
    }
}

/// What a flow reads of a renamed row field: `(name it binds, is it a list of rows, old field, new field)`.
type RowField = (String, bool, String, String);

/// What an SQL step names of a renamed column: the column of its own table (`column` or `table.column`) and of a joined
/// one (`table.column`), in the select, the conditions, the sort keys and the join pairs. The row field of a joined column
/// is `<alias or table>_<column>`: the rows the step binds are noted for [`follow_row_fields`] and the schema it fills
/// for the caller.
fn requalify_column(
    o: &mut serde_json::Map<String, Value>,
    table: &str,
    from: &str,
    to: &str,
    rows: &mut Vec<RowField>,
    fields: &mut Vec<(String, String, String)>,
) {
    let own = o.get("table").and_then(Value::as_str) == Some(table);
    let join = o
        .get("joins")
        .and_then(Value::as_array)
        .and_then(|js| js.iter().find(|j| j["table"] == table));
    if !own && join.is_none() {
        return;
    }
    let fix = |v: &mut Value| {
        let Some(name) = v.as_str() else { return };
        let new = match name.split_once('.') {
            Some((t, c)) if t == table && c == from => Some(format!("{table}.{to}")),
            None if own && name == from => Some(to.to_string()),
            _ => None,
        };
        if let Some(new) = new {
            *v = new.into();
        }
    };
    let fix_key = |v: &mut Value, key: &str| {
        if let Some(c) = v.get_mut(key) {
            fix(c);
        }
    };
    let aliased = o
        .get("columnAs")
        .and_then(Value::as_object)
        .is_some_and(|m| m.contains_key(&format!("{table}.{from}")));
    if let Some(j) = join.filter(|_| !own && !aliased) {
        let prefix = j.get("alias").and_then(Value::as_str).unwrap_or(table);
        let (old, new) = (format!("{prefix}_{from}"), format!("{prefix}_{to}"));
        let text = |k: &str| o.get(k).and_then(Value::as_str).map(str::to_string);
        if let Some(bind) = text("bind") {
            rows.push((
                bind,
                o.get("many") == Some(&Value::Bool(true)),
                old.clone(),
                new.clone(),
            ));
        }
        if let Some(last) = text("lastBind") {
            rows.push((last, false, old.clone(), new.clone()));
        }
        if let Some(message) = text("into") {
            fields.push((message, old, new));
        }
    }
    if let Some(joins) = o.get_mut("joins").and_then(Value::as_array_mut) {
        for j in joins {
            let joined = j["table"] == table;
            for on in j["on"].as_array_mut().into_iter().flatten() {
                if joined && let Some(pair) = on.as_object_mut() {
                    set_if(pair, "refColumn", from, to);
                }
                fix_key(on, "column");
            }
        }
    }
    if let Some(c) = o.get_mut("columns").and_then(Value::as_array_mut) {
        c.iter_mut().for_each(fix);
    }
    rekey_aliases(o, |name| match name.split_once('.') {
        Some((t, c)) if t == table && c == from => Some(format!("{table}.{to}")),
        None if own && name == from => Some(to.to_string()),
        _ => None,
    });
    if let Some(c) = o.get_mut("orderBy") {
        fix(c);
    }
    for key in ["thenBy", "where"] {
        for k in o
            .get_mut(key)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            fix_key(k, "column");
        }
    }
    if own {
        for item in o
            .get_mut("values")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object_mut)
        {
            set_if(item, "field", from, to);
        }
    }
}

/// The flow reads the fields of the rows an SQL step binds (`row.authors_name`): the paths and the texts follow the
/// rename of the field. A row is the bind of a step that gives one, its last row, or the item of a loop over its list.
fn follow_row_fields(steps: &mut Vec<Step>, rows: &[RowField]) {
    if rows.is_empty() {
        return;
    }
    let mut names: Vec<(String, String, String)> = rows
        .iter()
        .filter(|(_, list, ..)| !list)
        .map(|(n, _, old, new)| (n.clone(), old.clone(), new.clone()))
        .collect();
    rewrite(steps, |o| {
        if kind(o) != "forEach" {
            return;
        }
        let list = o
            .get("list")
            .and_then(|l| l.get("path"))
            .and_then(Value::as_str);
        let item = o.get("item").and_then(Value::as_str);
        if let (Some(list), Some(item)) = (list, item) {
            for (n, is_list, old, new) in rows {
                if *is_list && n == list {
                    names.push((item.into(), old.clone(), new.clone()));
                }
            }
        }
    });
    // One path or one text, with the old field name of a row replaced.
    let swap = |text: &str, in_text: bool| -> Option<String> {
        names.iter().find_map(|(row, old, new)| {
            let (a, b) = (format!("{row}.{old}"), format!("{row}.{new}"));
            if in_text {
                let (a, b) = (format!("{{{a}}}"), format!("{{{b}}}"));
                text.contains(&a).then(|| text.replace(&a, &b))
            } else if text == a {
                Some(b)
            } else {
                text.strip_prefix(&format!("{a}."))
                    .map(|rest| format!("{b}.{rest}"))
            }
        })
    };
    rewrite(steps, |o| {
        let key = match (o.get("kind").and_then(Value::as_str), kind(o)) {
            (Some("path"), _) => ("path", false),
            (Some("template"), _) => ("template", true),
            (_, "log") => ("message", true),
            _ => return,
        };
        // A text may hold several fields: swap until nothing changes.
        while let Some(new) = o
            .get(key.0)
            .and_then(Value::as_str)
            .and_then(|t| swap(t, key.1))
        {
            o.insert(key.0.into(), new.into());
        }
    });
}

/// The joins of an SQL step that cannot work any more, because the table or the column they stand on is gone, are removed
/// with the select columns and the sort keys that read them (and the joins that stood on those). A condition on the
/// dropped column stays: removing it would silently widen a query, the validation points at it instead. With no
/// `column`, the whole table is gone.
fn drop_from_joins(o: &mut serde_json::Map<String, Value>, table: &str, column: Option<&str>) {
    let own = o
        .get("table")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // `table.column` of the dropped thing, or `column` of the own table when that is the dropped table.
    let is_gone = |name: &str, dead: &[String]| match name.split_once('.') {
        Some((t, c)) => {
            dead.iter().any(|d| d == t) || (t == table && column.is_none_or(|k| k == c))
        }
        None => own == table && column == Some(name),
    };
    let mut dead: Vec<String> = vec![];
    loop {
        let Some(joins) = o.get_mut("joins").and_then(Value::as_array_mut) else {
            return;
        };
        let Some(at) = joins.iter().position(|j| {
            let name = j["table"].as_str().unwrap_or_default();
            let on = || j["on"].as_array().into_iter().flatten();
            (name == table && (column.is_none() || on().any(|p| p["refColumn"].as_str() == column)))
                || on().any(|p| is_gone(p["column"].as_str().unwrap_or_default(), &dead))
        }) else {
            break;
        };
        let gone = joins.remove(at);
        dead.push(gone["table"].as_str().unwrap_or_default().to_string());
    }
    let qualified_gone = |v: &Value| {
        v.as_str().is_some_and(|name| {
            name.split_once('.').is_some_and(|(t, c)| {
                t != own
                    && (dead.iter().any(|d| d == t)
                        || (t == table && column.is_none_or(|k| k == c)))
            })
        })
    };
    if let Some(c) = o.get_mut("columns").and_then(Value::as_array_mut) {
        c.retain(|v| !qualified_gone(v));
    }
    if let Some(m) = o.get_mut("columnAs").and_then(Value::as_object_mut) {
        m.retain(|k, _| !qualified_gone(&Value::String(k.clone())));
        if m.is_empty() {
            o.remove("columnAs");
        }
    }
    if o.get("orderBy").is_some_and(qualified_gone) {
        o.insert("orderBy".into(), Value::Null);
    }
    if let Some(c) = o.get_mut("thenBy").and_then(Value::as_array_mut) {
        c.retain(|k| !qualified_gone(&k["column"]));
    }
}

/// Runs [`drop_from_joins`] on the SQL steps of the service.
fn clean_joins(p: &mut Project, service: &str, table: &str, column: Option<&str>) {
    for steps in handlers(p, |n| n == service) {
        rewrite(steps, |o| {
            if kind(o) == "sql" {
                drop_from_joins(o, table, column);
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
    clean_joins(p, service, table, Some(name));
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
    clean_joins(p, service, table, None);
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

            // The steps of JWKS, TOTP, passkeys and the audit journal require schemas of these names: renaming one is a
            // validation error, by design (the Problems page says so).
            const RESERVED: [&str; 5] =
                ["Jwk", "Jwks", "TotpSetup", "PasskeyChallenge", "AuditEntry"];
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

    /// The `list_posts` route of the showcase reads posts with their author's name through a join.
    fn posts_step(p: &Project) -> serde_json::Value {
        let route = p.services[0]
            .routes
            .iter()
            .find(|r| r.name == "list_posts")
            .expect("add_sql_reads");
        serde_json::to_value(
            route
                .steps
                .iter()
                .find(|s| matches!(s, Step::Sql { .. }))
                .unwrap(),
        )
        .unwrap()
    }

    fn has(list: &serde_json::Value, name: &str) -> bool {
        list.as_array().unwrap().iter().any(|v| v == name)
    }

    #[test]
    fn an_aliased_column_keeps_its_field_when_the_column_is_renamed() {
        let mut p = sample::showcase();
        for r in &mut p.services[0].routes {
            for s in &mut r.steps {
                if let Step::Sql { column_as, .. } = s
                    && r.name == "list_posts"
                {
                    column_as.insert("authors.name".into(), "authors_name".into());
                }
            }
        }
        column(&mut p, "Orders", "authors", "name", "full_name");
        let step = posts_step(&p);
        assert_eq!(step["columnAs"]["authors.full_name"], "authors_name");
        assert!(step["columnAs"].get("authors.name").is_none());
        let item = p.messages.iter().find(|m| m.name == "PostItem").unwrap();
        assert!(item.fields.iter().any(|f| f.name == "authors_name"));
        drop_column(&mut p, "Orders", "authors", "full_name");
        assert!(posts_step(&p).get("columnAs").is_none());
    }

    #[test]
    fn renaming_a_joined_column_follows_the_joins_the_row_fields_and_the_flow() {
        let mut p = sample::showcase();
        // A flow that reads the row fields of the join in a text.
        let route = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "list_posts")
            .unwrap();
        route.steps.insert(
            5,
            Step::Log {
                id: "z9".into(),
                level: sb_model::LogLevel::Info,
                message: "last by {last.authors_name}".into(),
            },
        );
        assert_eq!(errors(&p), 0, "{:?}", sb_codegen::diagnose(&p));
        column(&mut p, "Orders", "authors", "name", "full_name");
        assert!(has(&posts_step(&p)["columns"], "authors.full_name"));
        let item = p.messages.iter().find(|m| m.name == "PostItem").unwrap();
        assert!(item.fields.iter().any(|f| f.name == "authors_full_name"));
        assert!(
            serde_json::to_string(&p)
                .unwrap()
                .contains("{last.authors_full_name}")
        );
        assert_eq!(mentions(&p, "authors.name"), 0);
        assert_eq!(errors(&p), 0, "{:?}", sb_codegen::diagnose(&p));

        // The column the join stands on, on both sides.
        column(&mut p, "Orders", "authors", "author_id", "id");
        column(&mut p, "Orders", "posts", "author_id", "writer_id");
        let on = &posts_step(&p)["joins"][0]["on"][0];
        assert_eq!(
            (on["refColumn"].as_str(), on["column"].as_str()),
            (Some("id"), Some("writer_id"))
        );
        assert_eq!(errors(&p), 0, "{:?}", sb_codegen::diagnose(&p));
    }

    #[test]
    fn a_renamed_table_keeps_the_row_fields_of_its_join_and_a_column_rename_then_follows_the_alias()
    {
        let mut p = sample::showcase();
        table(&mut p, "Orders", "authors", "writers");
        assert_eq!(posts_step(&p)["joins"][0]["alias"], "authors");
        column(&mut p, "Orders", "writers", "name", "full_name");
        let item = p.messages.iter().find(|m| m.name == "PostItem").unwrap();
        assert!(item.fields.iter().any(|f| f.name == "authors_full_name"));
        assert_eq!(errors(&p), 0, "{:?}", sb_codegen::diagnose(&p));
    }

    #[test]
    fn dropping_a_joined_column_or_table_cleans_the_joins() {
        let mut p = sample::showcase();
        drop_column(&mut p, "Orders", "authors", "name");
        let step = posts_step(&p);
        assert!(!has(&step["columns"], "authors.name"));
        assert_eq!(step["joins"].as_array().unwrap().len(), 1);

        // The column the join stands on: the join goes, with what read it.
        let mut p = sample::showcase();
        drop_column(&mut p, "Orders", "authors", "author_id");
        let step = posts_step(&p);
        assert!(step.get("joins").is_none());
        assert!(!has(&step["columns"], "authors.name"));

        let mut p = sample::showcase();
        drop_table(&mut p, "Orders", "authors");
        assert!(posts_step(&p).get("joins").is_none());
        assert_eq!(mentions(&p, "authors.name"), 0);
    }
}
