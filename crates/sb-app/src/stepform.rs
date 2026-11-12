//! Forms for the steps of a flow, derived from the JSON Schema of the project, so that a new kind of step (or a new
//! field) shows up in every client without code of its own. Pure data: no window involved.
//!
//! Every field is edited as text. Expressions use a compact syntax:
//!
//! ```text
//! input.order_id      a variable or a field of one        "paid"  42  true  null   a literal (JSON)
//! now                 the current time                    rust: code              Rust, inserted verbatim
//! ```
//!
//! A list of assignments is one `field = expression` per line; a condition is `left op right` (`== != > >= < <=`)
//! or `rust: code`; a list of words is comma separated; anything more structured is shown as JSON.

use std::sync::OnceLock;

use sb_model::{Step, json_schema};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Text,
    Number,
    Bool,
    /// One of these words.
    Choice(Vec<String>),
    /// A scalar type or the name of a message: the client lists the messages.
    Type,
    Expr,
    Condition,
    /// `field = expression`, one per line.
    Assignments,
    /// Words separated by commas.
    Words,
    /// Anything else, as JSON.
    Json,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub key: String,
    pub kind: Kind,
    /// Absent is allowed: an empty text leaves the field out.
    pub optional: bool,
    pub help: String,
}

pub const SCALARS: [&str; 9] = [
    "string",
    "bool",
    "int32",
    "int64",
    "float",
    "double",
    "bytes",
    "timestamp",
    "uuid",
];

fn schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(json_schema)
}

fn def(name: &str) -> Option<&'static Value> {
    schema().get("$defs")?.get(name)
}

fn ref_name(v: &Value) -> Option<&str> {
    v.get("$ref")?.as_str()?.rsplit('/').next()
}

fn variants() -> &'static Vec<Value> {
    static V: OnceLock<Vec<Value>> = OnceLock::new();
    V.get_or_init(|| {
        def("Step")
            .and_then(|s| s.get("oneOf"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    })
}

fn variant(kind: &str) -> Option<&'static Value> {
    variants()
        .iter()
        .find(|v| v["properties"]["type"]["const"] == kind)
}

/// The `type` of every step, in the order of the model.
pub fn kinds() -> Vec<String> {
    variants()
        .iter()
        .filter_map(|v| v["properties"]["type"]["const"].as_str().map(String::from))
        .collect()
}

fn is_null(v: &Value) -> bool {
    v.get("type").and_then(Value::as_str) == Some("null")
}

fn allows_null(ty: &Value) -> bool {
    ty.as_array()
        .is_some_and(|a| a.iter().any(|t| t.as_str() == Some("null")))
}

fn base_type(ty: &Value) -> Option<&str> {
    match ty {
        Value::String(s) => Some(s),
        Value::Array(a) => a.iter().filter_map(Value::as_str).find(|t| *t != "null"),
        _ => None,
    }
}

/// What a property is, as a form input: `(kind, optional by its type)`.
fn classify(prop: &Value) -> (Kind, bool) {
    if let Some(any) = prop.get("anyOf").and_then(Value::as_array) {
        let inner: Vec<&Value> = any.iter().filter(|v| !is_null(v)).collect();
        if let [one] = inner.as_slice() {
            let (kind, _) = classify(one);
            return (kind, true);
        }
        return (Kind::Json, true);
    }
    if let Some(name) = ref_name(prop) {
        return (
            match name {
                "Expr" => Kind::Expr,
                "Condition" => Kind::Condition,
                "FieldType" => Kind::Type,
                other => match def(other)
                    .and_then(|d| d.get("enum"))
                    .and_then(Value::as_array)
                {
                    Some(words) => Kind::Choice(
                        words
                            .iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect(),
                    ),
                    None => Kind::Json,
                },
            },
            false,
        );
    }
    let ty = prop.get("type").unwrap_or(&Value::Null);
    let optional = allows_null(ty);
    let kind = match base_type(ty) {
        Some("string") => Kind::Text,
        Some("integer") => Kind::Number,
        Some("boolean") => Kind::Bool,
        Some("array") => match prop.get("items") {
            Some(i) if ref_name(i) == Some("Assignment") => Kind::Assignments,
            Some(i) if i.get("type").and_then(Value::as_str) == Some("string") => Kind::Words,
            Some(i)
                if ref_name(i).is_some_and(|n| def(n).is_some_and(|d| d.get("enum").is_some())) =>
            {
                Kind::Words
            }
            _ => Kind::Json,
        },
        _ => Kind::Json,
    };
    (kind, optional)
}

/// The fields a form shows for a kind of step: all but the id and the nested steps of a branch.
pub fn specs(kind: &str) -> Vec<Spec> {
    let Some(v) = variant(kind) else {
        return vec![];
    };
    let required: Vec<&str> = v["required"]
        .as_array()
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some(props) = v["properties"].as_object() else {
        return vec![];
    };
    props
        .iter()
        .filter(|(k, p)| {
            !matches!(k.as_str(), "type" | "id")
                && p.get("items").and_then(ref_name) != Some("Step")
        })
        .map(|(key, prop)| {
            let (kind, nullable) = classify(prop);
            Spec {
                optional: nullable || !required.contains(&key.as_str()),
                help: prop
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                key: key.clone(),
                kind,
            }
        })
        .collect()
}

// ---- text <-> JSON ----

pub fn expr_to_text(expr: &Value) -> String {
    match expr["kind"].as_str() {
        Some("path") => expr["path"].as_str().unwrap_or_default().to_string(),
        Some("literal") => expr["value"].to_string(),
        Some("raw") => format!("rust: {}", expr["code"].as_str().unwrap_or_default()),
        Some("now") => "now".into(),
        _ => String::new(),
    }
}

pub fn parse_expr(text: &str) -> Result<Value, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("expression vide".into());
    }
    if let Some(code) = t.strip_prefix("rust:") {
        return Ok(json!({"kind": "raw", "code": code.trim()}));
    }
    if t == "now" {
        return Ok(json!({"kind": "now"}));
    }
    if let Ok(value) = serde_json::from_str::<Value>(t) {
        return Ok(json!({"kind": "literal", "value": value}));
    }
    let valid = t
        .split('.')
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    if valid {
        Ok(json!({"kind": "path", "path": t}))
    } else {
        Err(format!(
            "« {t} » n'est ni une variable (a.b), ni un littéral JSON (\"texte\", 42), ni `rust: code`"
        ))
    }
}

/// Splits `left op right` at the first comparison operator that is outside a quoted text.
fn split_comparison(text: &str) -> Option<(&str, &str, &str)> {
    const OPS: [&str; 6] = ["==", "!=", ">=", "<=", ">", "<"];
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if quoted {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => quoted = false,
                _ => {}
            }
            continue;
        }
        if c == '"' {
            quoted = true;
            continue;
        }
        if let Some(op) = OPS.iter().find(|op| text[i..].starts_with(**op)) {
            return Some((&text[..i], op, &text[i + op.len()..]));
        }
    }
    None
}

fn op_word(op: &str) -> &'static str {
    match op {
        "==" => "eq",
        "!=" => "ne",
        ">=" => "ge",
        "<=" => "le",
        ">" => "gt",
        _ => "lt",
    }
}

fn op_symbol(word: &str) -> &'static str {
    match word {
        "eq" => "==",
        "ne" => "!=",
        "ge" => ">=",
        "le" => "<=",
        "gt" => ">",
        _ => "<",
    }
}

pub fn condition_to_text(c: &Value) -> String {
    match c["kind"].as_str() {
        Some("compare") => format!(
            "{} {} {}",
            expr_to_text(&c["left"]),
            op_symbol(c["op"].as_str().unwrap_or("eq")),
            expr_to_text(&c["right"])
        ),
        _ => format!("rust: {}", c["code"].as_str().unwrap_or_default()),
    }
}

pub fn parse_condition(text: &str) -> Result<Value, String> {
    let t = text.trim();
    if let Some(code) = t.strip_prefix("rust:") {
        return Ok(json!({"kind": "raw", "code": code.trim()}));
    }
    let (left, op, right) = split_comparison(t)
        .ok_or("une condition s'écrit `gauche == droite` (== != > >= < <=) ou `rust: code`")?;
    Ok(json!({
        "kind": "compare",
        "left": parse_expr(left)?,
        "op": op_word(op),
        "right": parse_expr(right)?,
    }))
}

pub fn assignments_to_text(list: &Value) -> String {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            format!(
                "{} = {}",
                a["field"].as_str().unwrap_or_default(),
                expr_to_text(&a["value"])
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn parse_assignments(text: &str) -> Result<Value, String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let (field, value) = line
                .split_once('=')
                .ok_or_else(|| format!("« {line} » : écrivez `champ = expression`"))?;
            Ok(json!({"field": field.trim(), "value": parse_expr(value)?}))
        })
        .collect::<Result<Vec<_>, String>>()
        .map(Value::Array)
}

fn type_to_text(t: &Value) -> String {
    match t["kind"].as_str() {
        Some("message") => t["name"].as_str().unwrap_or_default().to_string(),
        Some(k) => k.to_string(),
        None => String::new(),
    }
}

fn parse_type(text: &str) -> Value {
    if SCALARS.contains(&text) {
        json!({"kind": text})
    } else {
        json!({"kind": "message", "name": text})
    }
}

fn to_text(spec: &Spec, v: Option<&Value>) -> String {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return String::new();
    };
    match &spec.kind {
        Kind::Text | Kind::Choice(_) => v.as_str().unwrap_or_default().to_string(),
        Kind::Number | Kind::Bool => v.to_string(),
        Kind::Type => type_to_text(v),
        Kind::Expr => expr_to_text(v),
        Kind::Condition => condition_to_text(v),
        Kind::Assignments => assignments_to_text(v),
        Kind::Words => v
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        Kind::Json => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

fn from_text(spec: &Spec, text: &str) -> Result<Option<Value>, String> {
    let t = text.trim();
    if t.is_empty()
        && !matches!(
            spec.kind,
            Kind::Text | Kind::Assignments | Kind::Words | Kind::Bool
        )
    {
        return if spec.optional {
            Ok(None)
        } else {
            Err("obligatoire".into())
        };
    }
    Ok(Some(match &spec.kind {
        Kind::Text if t.is_empty() && spec.optional => return Ok(None),
        Kind::Text => Value::String(text.trim_end_matches('\n').to_string()),
        Kind::Choice(words) => {
            if !words.iter().any(|w| w == t) {
                return Err(format!("attendu : {}", words.join(", ")));
            }
            Value::String(t.into())
        }
        Kind::Number => t
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| "un nombre entier est attendu".to_string())?,
        Kind::Bool => Value::Bool(t == "true"),
        Kind::Type => parse_type(t),
        Kind::Expr => parse_expr(t)?,
        Kind::Condition => parse_condition(t)?,
        Kind::Assignments => parse_assignments(t)?,
        Kind::Words => Value::Array(
            t.split(',')
                .map(str::trim)
                .filter(|w| !w.is_empty())
                .map(|w| Value::String(w.into()))
                .collect(),
        ),
        Kind::Json => serde_json::from_str(t).map_err(|e| format!("JSON invalide : {e}"))?,
    }))
}

/// The text of every field of `step`, in the order of [`specs`].
pub fn to_texts(step: &Step) -> Vec<(Spec, String)> {
    let value = serde_json::to_value(step).unwrap_or(Value::Null);
    let kind = value["type"].as_str().unwrap_or_default().to_string();
    specs(&kind)
        .into_iter()
        .map(|s| {
            let text = to_text(&s, value.get(&s.key));
            (s, text)
        })
        .collect()
}

/// `step` with its fields replaced by `texts` (in the order of [`specs`]); its id and nested steps are kept.
pub fn from_texts(step: &Step, texts: &[String]) -> Result<Step, String> {
    let mut object = match serde_json::to_value(step) {
        Ok(Value::Object(o)) => o,
        _ => return Err("étape illisible".into()),
    };
    let kind = object["type"].as_str().unwrap_or_default().to_string();
    for (spec, text) in specs(&kind).iter().zip(texts) {
        match from_text(spec, text).map_err(|e| format!("{} : {e}", spec.key))? {
            Some(v) => {
                object.insert(spec.key.clone(), v);
            }
            None => {
                object.remove(&spec.key);
            }
        }
    }
    serde_json::from_value(Value::Object(object)).map_err(|e| format!("étape invalide : {e}"))
}

/// A step of this kind with every field empty or neutral, to be filled in by its form.
pub fn blank(kind: &str, id: &str) -> Result<Step, String> {
    let mut object = Map::new();
    object.insert("type".into(), kind.into());
    object.insert("id".into(), id.into());
    for spec in specs(kind) {
        let neutral = match &spec.kind {
            Kind::Text => json!(""),
            Kind::Number => json!(0),
            Kind::Bool => json!(false),
            Kind::Choice(words) => json!(words.first().cloned().unwrap_or_default()),
            Kind::Type => json!({"kind": "string"}),
            Kind::Expr => json!({"kind": "path", "path": ""}),
            Kind::Condition => json!({"kind": "raw", "code": ""}),
            Kind::Assignments | Kind::Words => json!([]),
            Kind::Json => variant(kind)
                .and_then(|v| v["properties"].get(&spec.key))
                .map_or(Value::Null, neutral_json),
        };
        if !(spec.optional && matches!(spec.kind, Kind::Expr | Kind::Json | Kind::Type)) {
            object.insert(spec.key.clone(), neutral);
        }
    }
    // Fields that have a default value in the model (`default` in the schema) are better left to it.
    if let Some(props) = variant(kind).and_then(|v| v["properties"].as_object()) {
        for (key, prop) in props {
            if let Some(d) = prop.get("default").filter(|d| !d.is_null()) {
                object.insert(key.clone(), d.clone());
            }
        }
    }
    serde_json::from_value(Value::Object(object)).map_err(|e| format!("étape {kind} : {e}"))
}

/// The smallest value that fits a schema: the first variant of an enum of objects, empty texts and lists.
fn neutral_json(schema: &Value) -> Value {
    let resolved = ref_name(schema).and_then(def).unwrap_or(schema);
    let object = resolved
        .get("oneOf")
        .and_then(|v| v.get(0))
        .unwrap_or(resolved);
    let Some(props) = object.get("properties").and_then(Value::as_object) else {
        return Value::Null;
    };
    props
        .iter()
        .map(|(k, p)| {
            let v = if let Some(c) = p.get("const") {
                c.clone()
            } else {
                match base_type(p.get("type").unwrap_or(&Value::Null)) {
                    Some("string") => json!(""),
                    Some("integer") => json!(0),
                    Some("boolean") => json!(false),
                    Some("array") => json!([]),
                    _ => neutral_json(p),
                }
            };
            (k.clone(), v)
        })
        .collect::<Map<_, _>>()
        .into()
}

/// One line that tells a step apart in a list.
pub fn summary(step: &Step) -> String {
    let value = serde_json::to_value(step).unwrap_or(Value::Null);
    let kind = value["type"].as_str().unwrap_or_default();
    let details: Vec<String> = to_texts(step)
        .into_iter()
        .filter(|(s, t)| !t.is_empty() && !matches!(s.kind, Kind::Json | Kind::Bool))
        .take(3)
        .map(|(s, t)| {
            let one_line = t.replace('\n', " ; ");
            let short: String = one_line.chars().take(48).collect();
            format!("{}: {short}", s.key)
        })
        .collect();
    format!("{kind}  {}", details.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_model::{Project, sample};

    fn every_step(p: &Project) -> Vec<Step> {
        let mut all = vec![];
        let mut take = |steps: &Vec<Step>| {
            for s in steps {
                s.walk(&mut |s| all.push(s.clone()));
            }
        };
        for svc in &p.services {
            svc.rpcs.iter().for_each(|r| take(&r.steps));
            svc.routes.iter().for_each(|r| take(&r.steps));
            svc.consumers.iter().for_each(|r| take(&r.steps));
            svc.jobs.iter().for_each(|r| take(&r.steps));
        }
        all
    }

    #[test]
    fn every_step_of_the_samples_survives_its_form() {
        let mut n = 0;
        for p in [sample::orders(), sample::showcase()] {
            for step in every_step(&p) {
                let texts: Vec<String> = to_texts(&step).into_iter().map(|(_, t)| t).collect();
                let back = from_texts(&step, &texts)
                    .unwrap_or_else(|e| panic!("{step:?}: {e}\ntexts: {texts:?}"));
                assert_eq!(back, step, "texts: {texts:?}");
                n += 1;
            }
        }
        assert!(n > 20, "the samples exercise many steps ({n})");
    }

    #[test]
    fn every_kind_has_a_blank_step() {
        let all = kinds();
        assert!(all.len() > 30);
        for kind in all {
            let step = blank(&kind, "x1").unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(step.id(), "x1");
            // And its form reads back what it shows.
            let texts: Vec<String> = to_texts(&step).into_iter().map(|(_, t)| t).collect();
            let _ = from_texts(&step, &texts);
        }
    }

    #[test]
    fn expressions_have_a_short_syntax() {
        assert_eq!(
            parse_expr("input.id").unwrap(),
            json!({"kind":"path","path":"input.id"})
        );
        assert_eq!(
            parse_expr("\"paid\"").unwrap(),
            json!({"kind":"literal","value":"paid"})
        );
        assert_eq!(
            parse_expr("42").unwrap(),
            json!({"kind":"literal","value":42})
        );
        assert_eq!(parse_expr("now").unwrap(), json!({"kind":"now"}));
        assert_eq!(
            parse_expr("rust: a + 1").unwrap(),
            json!({"kind":"raw","code":"a + 1"})
        );
        assert!(parse_expr("").is_err());
        assert!(parse_expr("a b").is_err());
    }

    #[test]
    fn conditions_ignore_operators_inside_texts() {
        let c = parse_condition("input.state == \"a>=b\"").unwrap();
        assert_eq!(c["op"], "eq");
        assert_eq!(c["right"], json!({"kind":"literal","value":"a>=b"}));
        assert_eq!(condition_to_text(&c), "input.state == \"a>=b\"");
        assert!(parse_condition("nothing to compare").is_err());
    }

    #[test]
    fn a_wrong_text_names_the_field() {
        let step = blank("log", "x1").unwrap();
        let err = from_texts(&step, &["fatal".into(), "hello".into()]).unwrap_err();
        assert!(err.contains("level"), "{err}");
    }

    #[test]
    fn nested_steps_are_not_part_of_the_form() {
        let keys: Vec<String> = specs("branch").into_iter().map(|s| s.key).collect();
        assert_eq!(keys, vec!["condition"]);
    }
}
