//! The variables a step can read, for the forms: `input` and its fields, and what the steps before it bound.
//!
//! Mirrors what the web editor offered as suggestions. The validator stays authoritative: this only helps to type
//! the right path.

use std::collections::BTreeMap;

use sb_model::{Project, Step};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Var {
    pub path: String,
    pub ty: String,
}

#[derive(Clone)]
struct Ty {
    /// `{"kind": "string"}` or `{"kind": "message", "name": ...}`.
    ty: Value,
    repeated: bool,
    optional: bool,
}

fn scalar(kind: &str) -> Ty {
    Ty {
        ty: serde_json::json!({ "kind": kind }),
        repeated: false,
        optional: false,
    }
}

fn message(name: &str) -> Ty {
    Ty {
        ty: serde_json::json!({ "kind": "message", "name": name }),
        repeated: false,
        optional: false,
    }
}

fn type_label(t: &Ty) -> String {
    let base = match t.ty["kind"].as_str() {
        Some("message") => t.ty["name"].as_str().unwrap_or_default().to_string(),
        Some(k) => k.to_string(),
        None => "?".into(),
    };
    format!(
        "{}{}{base}",
        if t.repeated { "liste de " } else { "" },
        if t.optional { "optionnel " } else { "" }
    )
}

/// The scope after `step` ran: what it binds.
fn bind(project: &Project, scope: &mut BTreeMap<String, Ty>, step: &Step) {
    let v = serde_json::to_value(step).unwrap_or(Value::Null);
    let text = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let Some(kind) = text("type") else { return };
    if kind == "let" {
        if let Some(name) = text("name") {
            let repeated = v["repeated"].as_bool().unwrap_or(false);
            scope.insert(
                name.into(),
                Ty {
                    ty: v["valueType"].clone(),
                    repeated,
                    optional: v["optional"].as_bool().unwrap_or(false) && !repeated,
                },
            );
        }
        return;
    }
    // An outbound call binds its answer and, apart, its status.
    if kind == "httpCall" {
        if let (Some(var), Some(schema)) = (text("bind"), text("response")) {
            scope.insert(var.into(), message(schema));
        }
        if let Some(var) = text("statusBind") {
            scope.insert(var.into(), scalar("int32"));
        }
        return;
    }
    let Some(var) = text("bind") else { return };
    let ty = match kind {
        "verifyPassword" => scalar("bool"),
        // What is left of the quota after this request.
        "checkQuota" => scalar("int64"),
        // The score of the password, 0 to 4.
        "passwordStrength" => scalar("int32"),
        "hashPassword" | "randomToken" | "sha256" | "encrypt" | "decrypt" | "signJwt"
        | "passkeyLoginFinish" | "imageSniff" | "objectPresignGet" | "objectPresignPut" => {
            scalar("string")
        }
        "verifyJwt" => message(text("schema").unwrap_or_default()),
        "totpGenerate" => message("TotpSetup"),
        "recoveryCodes" => Ty {
            repeated: true,
            ..scalar("string")
        },
        "passkeyRegisterStart" | "passkeyLoginStart" => message("PasskeyChallenge"),
        "objectGet" => Ty {
            optional: true,
            ..scalar("string")
        },
        "jwks" => message("Jwks"),
        "sql" => {
            let has_columns = v["columns"].as_array().is_some_and(|c| !c.is_empty());
            if !has_columns {
                return;
            }
            // What a page adds: the number of rows that match, whether another page follows, the last row.
            if let Some(v) = text("totalBind") {
                scope.insert(v.into(), scalar("int64"));
            }
            if let Some(v) = text("moreBind") {
                scope.insert(v.into(), scalar("bool"));
            }
            if let (Some(v), Some(schema)) = (text("lastBind"), text("into")) {
                scope.insert(v.into(), message(schema));
            }
            let name = text("into").map_or_else(
                || {
                    sb_model::sql_row_name(
                        text("table").unwrap_or_default(),
                        text("id").unwrap_or_default(),
                    )
                },
                String::from,
            );
            Ty {
                repeated: v["many"].as_bool().unwrap_or(false),
                ..message(&name)
            }
        }
        "callRpc" => {
            let rpc = project
                .services
                .iter()
                .find(|s| Some(s.name.as_str()) == text("service"))
                .and_then(|s| {
                    s.rpcs
                        .iter()
                        .find(|r| Some(r.name.as_str()) == text("method"))
                });
            match rpc {
                Some(r) => message(&r.response),
                None => return,
            }
        }
        _ => return,
    };
    scope.insert(var.into(), ty);
}

/// Every readable value at a point: the variables, and the fields of the messages they hold (three levels deep).
///
/// `initial` is the message of `input` (none for a scheduled job), `before` the steps that ran before, outermost first.
pub fn variables(project: &Project, initial: Option<&str>, before: &[Step]) -> Vec<Var> {
    let mut scope: BTreeMap<String, Ty> = BTreeMap::new();
    if let Some(m) = initial.filter(|m| !m.is_empty()) {
        scope.insert("input".into(), message(m));
    }
    for step in before {
        bind(project, &mut scope, step);
    }
    // The rows that SQL steps give back are messages that are never stored.
    let with_rows = project.with_sql_rows();
    let mut out = vec![];
    fn walk(p: &Project, prefix: &str, t: &Ty, depth: usize, out: &mut Vec<Var>) {
        out.push(Var {
            path: prefix.into(),
            ty: type_label(t),
        });
        if depth >= 3 || t.repeated || t.ty["kind"] != "message" {
            return;
        }
        let Some(m) = t.ty["name"].as_str().and_then(|n| p.message(n)) else {
            return;
        };
        for f in &m.fields {
            let ty = Ty {
                ty: serde_json::to_value(&f.ty).unwrap_or(Value::Null),
                repeated: f.repeated,
                optional: f.optional && !f.repeated && !f.ty.is_message(),
            };
            walk(p, &format!("{prefix}.{}", f.name), &ty, depth + 1, out);
        }
    }
    // `input` first, then the others by name.
    let mut names: Vec<&String> = scope.keys().collect();
    names.sort_by_key(|n| (n.as_str() != "input", n.to_string()));
    for name in names {
        walk(&with_rows, name, &scope[name], 0, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_model::sample;

    #[test]
    fn input_and_its_fields_are_offered_then_what_a_step_bound() {
        let p = sample::orders();
        let svc = &p.services[0];
        let rpc = &svc.rpcs[0];
        let none = variables(&p, Some(&rpc.request), &[]);
        assert_eq!(none[0].path, "input");
        assert!(none.iter().any(|v| v.path.starts_with("input.")));
        assert!(!none.iter().any(|v| v.path == "order_id"));

        // The first step of CreateOrder is `let order_id`: it is there for the steps after it.
        let after = variables(&p, Some(&rpc.request), &rpc.steps[..1]);
        assert!(
            after
                .iter()
                .any(|v| v.path == "order_id" && v.ty == "string"),
            "{after:?}"
        );
    }

    #[test]
    fn what_is_left_of_a_quota_is_offered_to_the_steps_after_it() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_usage(&mut p);
        let route = p.services[0]
            .routes
            .iter()
            .find(|r| r.name == "usage_report")
            .unwrap();
        let before = variables(&p, Some(&route.request), &route.steps[..1]);
        assert!(!before.iter().any(|v| v.path == "calls_left"));
        let after = variables(&p, Some(&route.request), &route.steps[..2]);
        let left = after.iter().find(|v| v.path == "calls_left").unwrap();
        assert_eq!(left.ty, "int64");
    }

    #[test]
    fn a_job_has_no_input() {
        let p = sample::orders();
        let vars = variables(&p, None, &[]);
        assert!(vars.is_empty());
    }

    #[test]
    fn the_rows_of_a_select_are_typed_by_their_columns() {
        let p = sample::showcase();
        // Every step of the showcase that selects rows binds a variable whose fields can then be read.
        let mut found = false;
        for svc in &p.services {
            for r in svc
                .rpcs
                .iter()
                .map(|r| &r.steps)
                .chain(svc.routes.iter().map(|r| &r.steps))
            {
                for (i, s) in r.iter().enumerate() {
                    if let Step::Sql {
                        bind: Some(b),
                        columns,
                        many: false,
                        ..
                    } = s
                        && !columns.is_empty()
                    {
                        let vars = variables(&p, None, &r[..=i]);
                        assert!(vars.iter().any(|v| v.path == *b), "{b}");
                        assert!(
                            vars.iter().any(|v| v.path.starts_with(&format!("{b}."))),
                            "{b}"
                        );
                        found = true;
                    }
                }
            }
        }
        assert!(found, "the showcase has a select that binds its row");
    }
}
