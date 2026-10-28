//! Light type system used to check the mappings drawn in the editor
//! (`input.order_id` -> `PaymentCompleted.order_id`) before any code is generated.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;

use crate::project::{Field, FieldType, Project};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
    pub ty: FieldType,
    pub repeated: bool,
    pub optional: bool,
}

impl TypeRef {
    pub fn scalar(ty: FieldType) -> Self {
        Self {
            ty,
            repeated: false,
            optional: false,
        }
    }

    pub fn message(name: impl Into<String>) -> Self {
        Self::scalar(FieldType::Message { name: name.into() })
    }

    pub fn of_field(f: &Field) -> Self {
        let is_msg = f.ty.is_message();
        Self {
            ty: f.ty.clone(),
            repeated: f.repeated,
            // Presence is implicit for messages and meaningless for repeated fields.
            optional: f.optional && !is_msg && !f.repeated,
        }
    }

    pub fn is_message(&self) -> bool {
        self.ty.is_message()
    }

    /// Plain scalar that can be printed with `{}` and compared with `==`.
    pub fn is_simple_scalar(&self) -> bool {
        !self.repeated
            && !self.optional
            && !matches!(
                self.ty,
                FieldType::Bytes | FieldType::Timestamp | FieldType::Message { .. }
            )
    }
}

impl fmt::Display for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.repeated {
            f.write_str("repeated ")?;
        }
        if self.optional {
            f.write_str("optional ")?;
        }
        match &self.ty {
            FieldType::String => f.write_str("string"),
            FieldType::Bool => f.write_str("bool"),
            FieldType::Int32 => f.write_str("int32"),
            FieldType::Int64 => f.write_str("int64"),
            FieldType::Float => f.write_str("float"),
            FieldType::Double => f.write_str("double"),
            FieldType::Bytes => f.write_str("bytes"),
            FieldType::Timestamp => f.write_str("timestamp"),
            FieldType::Message { name } => f.write_str(name),
        }
    }
}

/// Variables visible at a point of a handler. Message variables always hold a bare
/// message in the generated code (never an `Option`).
#[derive(Debug, Clone, Default)]
pub struct Scope {
    vars: BTreeMap<String, TypeRef>,
}

impl Scope {
    pub fn with_input(message: &str) -> Self {
        let mut s = Self::default();
        s.bind("input", TypeRef::message(message));
        s
    }

    pub fn bind(&mut self, name: impl Into<String>, ty: TypeRef) {
        self.vars.insert(name.into(), ty);
    }

    pub fn get(&self, name: &str) -> Option<&TypeRef> {
        self.vars.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.vars.keys().map(String::as_str)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPath {
    pub var: String,
    /// Fields walked after the variable, in order.
    pub hops: Vec<Field>,
    pub ty: TypeRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adapt {
    Identity,
    /// The source is wrapped in `Some(..)` (bare message -> message field, scalar -> optional scalar).
    WrapSome,
}

impl ResolvedPath {
    /// Whether the value is a bare message variable (no field walked).
    pub fn is_bare_message(&self) -> bool {
        self.hops.is_empty() && self.ty.is_message() && !self.ty.repeated
    }

    /// How to assign this value to a field of type `to`, or `None` if incompatible.
    pub fn adapt_to(&self, to: &TypeRef) -> Option<Adapt> {
        if self.is_bare_message() {
            return (self.ty == *to).then_some(Adapt::WrapSome);
        }
        if self.ty == *to {
            return Some(Adapt::Identity);
        }
        let widening = !self.ty.optional
            && !self.ty.repeated
            && to.optional
            && !to.repeated
            && self.ty.ty == to.ty
            && !to.is_message();
        widening.then_some(Adapt::WrapSome)
    }
}

pub fn resolve_path(project: &Project, scope: &Scope, path: &str) -> Result<ResolvedPath, String> {
    let mut parts = path.split('.');
    let var = parts.next().unwrap_or_default();
    if var.is_empty() {
        return Err("empty path".into());
    }
    let mut ty = scope
        .get(var)
        .ok_or_else(|| format!("unknown variable `{var}`"))?
        .clone();
    let mut hops = Vec::new();
    let mut walked = var.to_string();
    for seg in parts {
        let FieldType::Message { name } = &ty.ty else {
            return Err(format!("`{walked}` is a {ty} and has no field `{seg}`"));
        };
        if ty.repeated {
            return Err(format!(
                "`{walked}` is a list, its elements cannot be accessed"
            ));
        }
        let msg = project
            .message(name)
            .ok_or_else(|| format!("unknown message `{name}`"))?;
        let field = msg
            .field(seg)
            .ok_or_else(|| format!("message `{name}` has no field `{seg}`"))?;
        ty = TypeRef::of_field(field);
        hops.push(field.clone());
        walked.push('.');
        walked.push_str(seg);
    }
    Ok(ResolvedPath {
        var: var.to_string(),
        hops,
        ty,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

/// Interprets a JSON literal as a value of type `to`.
pub fn coerce_literal(value: &Value, to: &TypeRef) -> Result<Literal, String> {
    if to.repeated {
        return Err(format!(
            "a literal cannot fill a {to}; use a Rust expression"
        ));
    }
    let mismatch = || format!("literal {value} is not a valid {to}");
    match (&to.ty, value) {
        (FieldType::String, Value::String(s)) => Ok(Literal::Str(s.clone())),
        (FieldType::Bool, Value::Bool(b)) => Ok(Literal::Bool(*b)),
        (FieldType::Int32, Value::Number(n)) => n
            .as_i64()
            .filter(|i| i32::try_from(*i).is_ok())
            .map(Literal::Int)
            .ok_or_else(mismatch),
        (FieldType::Int64, Value::Number(n)) => n.as_i64().map(Literal::Int).ok_or_else(mismatch),
        (FieldType::Float | FieldType::Double, Value::Number(n)) => {
            n.as_f64().map(Literal::Float).ok_or_else(mismatch)
        }
        (FieldType::Bytes | FieldType::Timestamp | FieldType::Message { .. }, _) => Err(format!(
            "a literal cannot fill a {to}; use a Rust expression"
        )),
        _ => Err(mismatch()),
    }
}

/// Natural type of a literal when nothing constrains it.
pub fn infer_literal(value: &Value) -> Option<TypeRef> {
    match value {
        Value::String(_) => Some(TypeRef::scalar(FieldType::String)),
        Value::Bool(_) => Some(TypeRef::scalar(FieldType::Bool)),
        Value::Number(n) if n.is_i64() => Some(TypeRef::scalar(FieldType::Int64)),
        Value::Number(_) => Some(TypeRef::scalar(FieldType::Double)),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplatePart {
    Text(String),
    Path(String),
}

/// Splits `"order {input.order_id} accepted"` into text and path parts.
/// `{{` and `}}` are literal braces.
pub fn parse_template(template: &str) -> Result<Vec<TemplatePart>, String> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '{' => {
                let mut path = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(ch) => path.push(ch),
                        None => return Err("unclosed `{` (use `{{` for a literal brace)".into()),
                    }
                }
                let path = path.trim().to_string();
                if path.is_empty() {
                    return Err("empty `{}` placeholder".into());
                }
                if !text.is_empty() {
                    parts.push(TemplatePart::Text(std::mem::take(&mut text)));
                }
                parts.push(TemplatePart::Path(path));
            }
            '}' => return Err("unmatched `}` (use `}}` for a literal brace)".into()),
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        parts.push(TemplatePart::Text(text));
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Message;

    fn project() -> Project {
        let mut p = Project::new("demo");
        p.messages = vec![
            Message {
                name: "Customer".into(),
                fields: vec![
                    field("name", FieldType::String),
                    field("age", FieldType::Int32),
                ],
            },
            Message {
                name: "Order".into(),
                fields: vec![
                    field("id", FieldType::String),
                    field(
                        "customer",
                        FieldType::Message {
                            name: "Customer".into(),
                        },
                    ),
                ],
            },
        ];
        p
    }

    fn field(name: &str, ty: FieldType) -> Field {
        Field::new(name, ty)
    }

    #[test]
    fn resolves_nested_paths() {
        let p = project();
        let scope = Scope::with_input("Order");
        let r = resolve_path(&p, &scope, "input.customer.name").unwrap();
        assert_eq!(r.hops.len(), 2);
        assert_eq!(r.ty, TypeRef::scalar(FieldType::String));

        let r = resolve_path(&p, &scope, "input").unwrap();
        assert!(r.is_bare_message());
    }

    #[test]
    fn rejects_bad_paths() {
        let p = project();
        let scope = Scope::with_input("Order");
        assert!(
            resolve_path(&p, &scope, "nope.x")
                .unwrap_err()
                .contains("unknown variable")
        );
        assert!(
            resolve_path(&p, &scope, "input.zzz")
                .unwrap_err()
                .contains("no field")
        );
        assert!(
            resolve_path(&p, &scope, "input.id.len")
                .unwrap_err()
                .contains("has no field")
        );
    }

    #[test]
    fn adapt_rules() {
        let p = project();
        let scope = Scope::with_input("Order");
        let id = resolve_path(&p, &scope, "input.id").unwrap();
        let string = TypeRef::scalar(FieldType::String);
        let opt_string = TypeRef {
            optional: true,
            ..string.clone()
        };
        assert_eq!(id.adapt_to(&string), Some(Adapt::Identity));
        assert_eq!(id.adapt_to(&opt_string), Some(Adapt::WrapSome));
        assert_eq!(id.adapt_to(&TypeRef::scalar(FieldType::Int64)), None);

        let whole = resolve_path(&p, &scope, "input").unwrap();
        assert_eq!(
            whole.adapt_to(&TypeRef::message("Order")),
            Some(Adapt::WrapSome)
        );
        let cust = resolve_path(&p, &scope, "input.customer").unwrap();
        assert_eq!(
            cust.adapt_to(&TypeRef::message("Customer")),
            Some(Adapt::Identity)
        );
    }

    #[test]
    fn literals() {
        let int32 = TypeRef::scalar(FieldType::Int32);
        assert_eq!(coerce_literal(&Value::from(5), &int32), Ok(Literal::Int(5)));
        assert!(coerce_literal(&Value::from(5_000_000_000_i64), &int32).is_err());
        assert!(coerce_literal(&Value::from("x"), &int32).is_err());
        let double = TypeRef::scalar(FieldType::Double);
        assert_eq!(
            coerce_literal(&Value::from(2), &double),
            Ok(Literal::Float(2.0))
        );
    }

    #[test]
    fn templates() {
        assert_eq!(
            parse_template("order {input.id} ok {{x}}").unwrap(),
            vec![
                TemplatePart::Text("order ".into()),
                TemplatePart::Path("input.id".into()),
                TemplatePart::Text(" ok {x}".into()),
            ]
        );
        assert!(parse_template("a {b").is_err());
        assert!(parse_template("a } b").is_err());
        assert!(parse_template("{}").is_err());
    }
}
