//! Light type system used to check the mappings drawn in the editor
//! (`input.order_id` -> `PaymentCompleted.order_id`) before any code is generated.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;

use crate::project::{Expr, Field, FieldType, Func, Project};

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

    /// The same shape (list or optional) and a type that fits: `uuid` and `string` are both text.
    pub fn accepts(&self, from: &TypeRef) -> bool {
        self.repeated == from.repeated
            && self.optional == from.optional
            && self.ty.assignable_from(&from.ty)
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
            FieldType::Uuid => f.write_str("uuid"),
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
        if to.accepts(&self.ty) {
            return Some(Adapt::Identity);
        }
        let widening = !self.ty.optional
            && !self.ty.repeated
            && to.optional
            && !to.repeated
            && to.ty.assignable_from(&self.ty.ty)
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
    /// `null` for an optional value: absent.
    None,
    /// A JSON array for a list.
    List(Vec<Literal>),
}

/// `8-4-4-4-12` hexadecimal digits.
pub fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Interprets a JSON literal as a value of type `to`.
pub fn coerce_literal(value: &Value, to: &TypeRef) -> Result<Literal, String> {
    if to.repeated {
        let Value::Array(items) = value else {
            return Err(format!(
                "literal {value} is not a valid {to}: write a JSON array"
            ));
        };
        let item = TypeRef::scalar(to.ty.clone());
        return items
            .iter()
            .map(|v| coerce_literal(v, &item))
            .collect::<Result<_, _>>()
            .map(Literal::List);
    }
    if to.optional && value.is_null() {
        return Ok(Literal::None);
    }
    let mismatch = || format!("literal {value} is not a valid {to}");
    match (&to.ty, value) {
        (FieldType::String, Value::String(s)) => Ok(Literal::Str(s.clone())),
        (FieldType::Uuid, Value::String(s)) if is_uuid(s) => Ok(Literal::Str(s.clone())),
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

/// Whether `name` is a time zone of the IANA database (`Europe/Paris`, `UTC`): the database is bundled with the editor.
pub fn is_time_zone(name: &str) -> bool {
    jiff::tz::TimeZone::get(name).is_ok()
}

impl Func {
    /// Every operation, in the order the editors list them.
    pub const ALL: [Func; 41] = [
        Func::Concat,
        Func::Lower,
        Func::Upper,
        Func::Trim,
        Func::SanitizeHtml,
        Func::Length,
        Func::Contains,
        Func::StartsWith,
        Func::EndsWith,
        Func::Replace,
        Func::Split,
        Func::Join,
        Func::Add,
        Func::Sub,
        Func::Mul,
        Func::Div,
        Func::Rem,
        Func::Min,
        Func::Max,
        Func::Round,
        Func::Floor,
        Func::Ceil,
        Func::ToInt32,
        Func::ToInt64,
        Func::ToDouble,
        Func::ToText,
        Func::ParseInt64,
        Func::ParseDouble,
        Func::Default,
        Func::NewUuid,
        Func::NewUuidV7,
        Func::ShortId,
        Func::FromShortId,
        Func::NowSeconds,
        Func::LocalDate,
        Func::LocalTime,
        Func::StartOfDay,
        Func::AddDays,
        Func::AddMonths,
        Func::ParseLocal,
        Func::ListLength,
    ];

    /// The name used in JSON and in the compact syntax (`startsWith`).
    pub fn name(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    }

    pub fn from_name(name: &str) -> Option<Func> {
        Func::ALL.into_iter().find(|f| f.name() == name)
    }

    /// Fewest and most arguments (`None`: any number).
    pub fn arity(self) -> (usize, Option<usize>) {
        use Func::*;
        match self {
            NewUuid | NewUuidV7 | NowSeconds => (0, Some(0)),
            AddDays | AddMonths => (3, Some(3)),
            Concat => (1, None),
            Lower | Upper | Trim | SanitizeHtml | Length | Round | Floor | Ceil | ToInt32
            | ToInt64 | ToDouble | ToText | ParseInt64 | ParseDouble | ShortId | FromShortId
            | ListLength => (1, Some(1)),
            Replace => (3, Some(3)),
            _ => (2, Some(2)),
        }
    }

    /// Whether the arguments are numbers of one same type.
    pub fn is_arithmetic(self) -> bool {
        matches!(
            self,
            Func::Add | Func::Sub | Func::Mul | Func::Div | Func::Rem | Func::Min | Func::Max
        )
    }
}

pub fn is_number(t: &TypeRef) -> bool {
    !t.repeated
        && !t.optional
        && matches!(
            t.ty,
            FieldType::Int32 | FieldType::Int64 | FieldType::Float | FieldType::Double
        )
}

fn is_integer(t: &FieldType) -> bool {
    matches!(t, FieldType::Int32 | FieldType::Int64)
}

/// The type of an expression, or `None` when it is Rust code the editor cannot see into. `hint` is the type the
/// context wants: it types literals (`1` is an int32 where an int32 is wanted). Pure: it is the one source of
/// truth for validation and generation alike.
pub fn expr_type(
    project: &Project,
    scope: &Scope,
    expr: &Expr,
    hint: Option<&TypeRef>,
) -> Result<Option<TypeRef>, String> {
    match expr {
        Expr::Path { path } => Ok(Some(resolve_path(project, scope, path)?.ty)),
        Expr::Literal { value } => match hint {
            Some(to) => coerce_literal(value, to).map(|_| Some(to.clone())),
            None => Ok(infer_literal(value)),
        },
        Expr::Raw { .. } => Ok(None),
        Expr::Now => {
            Err("`now()` is the database clock: it only fills a column of an SQL step".into())
        }
        Expr::Template { template } => {
            for part in parse_template(template)? {
                if let TemplatePart::Path(p) = part {
                    let ty = resolve_path(project, scope, &p)?.ty;
                    if !ty.is_simple_scalar() {
                        return Err(format!("`{p}` is a {ty}: only plain values go in a text"));
                    }
                }
            }
            Ok(Some(TypeRef::scalar(FieldType::String)))
        }
        Expr::Call { func, args } => call_type(project, scope, *func, args, hint),
    }
}

fn call_type(
    project: &Project,
    scope: &Scope,
    func: Func,
    args: &[Expr],
    hint: Option<&TypeRef>,
) -> Result<Option<TypeRef>, String> {
    use Func::*;
    let name = func.name();
    let (min, max) = func.arity();
    if args.len() < min || max.is_some_and(|m| args.len() > m) {
        return Err(format!(
            "`{name}` takes {} argument(s), found {}",
            match max {
                Some(m) if m == min => m.to_string(),
                Some(m) => format!("{min} to {m}"),
                None => format!("at least {min}"),
            },
            args.len()
        ));
    }
    let text = TypeRef::scalar(FieldType::String);
    let text_list = TypeRef {
        repeated: true,
        ..text.clone()
    };
    // `want`: the type argument `i` must have; raw Rust passes.
    let check = |i: usize, want: &TypeRef| -> Result<(), String> {
        match expr_type(project, scope, &args[i], Some(want))? {
            Some(got) if !want.accepts(&got) => Err(format!(
                "argument {} of `{name}` is a {got} but a {want} is expected",
                i + 1
            )),
            _ => Ok(()),
        }
    };
    let result = |ty: FieldType| Ok(Some(TypeRef::scalar(ty)));
    match func {
        Concat | Lower | Upper | Trim | SanitizeHtml | Replace | Length | Contains | StartsWith
        | EndsWith | Split | ParseInt64 | ParseDouble => {
            for i in 0..args.len() {
                check(i, &text)?;
            }
            match func {
                Length => result(FieldType::Int64),
                Contains | StartsWith | EndsWith => result(FieldType::Bool),
                Split => Ok(Some(text_list)),
                ParseInt64 | ParseDouble => Ok(Some(TypeRef {
                    optional: true,
                    ty: if func == ParseInt64 {
                        FieldType::Int64
                    } else {
                        FieldType::Double
                    },
                    repeated: false,
                })),
                _ => result(FieldType::String),
            }
        }
        Join => {
            check(0, &text_list)?;
            check(1, &text)?;
            result(FieldType::String)
        }
        Add | Sub | Mul | Div | Rem | Min | Max => {
            match arithmetic_type(project, scope, args, hint)? {
                None => Ok(None),
                Some(ty) => {
                    if !is_number(&ty) {
                        return Err(format!("`{name}` works on numbers, found a {ty}"));
                    }
                    if func == Rem && !is_integer(&ty.ty) {
                        return Err("`rem` works on integers".into());
                    }
                    for i in 0..args.len() {
                        check(i, &ty)?;
                    }
                    Ok(Some(ty))
                }
            }
        }
        Round | Floor | Ceil => {
            let float = TypeRef::scalar(FieldType::Double);
            if let Some(t) = expr_type(project, scope, &args[0], Some(&float))?
                && !(is_number(&t) && matches!(t.ty, FieldType::Float | FieldType::Double))
            {
                return Err(format!("`{name}` rounds a float or a double, found a {t}"));
            }
            result(FieldType::Int64)
        }
        ToInt32 | ToInt64 | ToDouble | ToText => {
            if let Some(t) = expr_type(project, scope, &args[0], None)? {
                if func == ToText && !t.is_simple_scalar() {
                    return Err(format!("`toText` needs a plain value, found a {t}"));
                }
                if func != ToText && !is_number(&t) {
                    return Err(format!("`{name}` converts a number, found a {t}"));
                }
            }
            result(match func {
                ToInt32 => FieldType::Int32,
                ToInt64 => FieldType::Int64,
                ToDouble => FieldType::Double,
                _ => FieldType::String,
            })
        }
        Default => {
            let Some(first) = expr_type(project, scope, &args[0], None)? else {
                return Ok(None);
            };
            if !first.optional || first.repeated || first.is_message() {
                return Err(format!(
                    "`default` needs an optional value, which may be absent; this one is a {first}"
                ));
            }
            let plain = TypeRef {
                optional: false,
                ..first
            };
            check(1, &plain)?;
            Ok(Some(plain))
        }
        NewUuid | NewUuidV7 => result(FieldType::Uuid),
        NowSeconds => result(FieldType::Int64),
        LocalDate | LocalTime | StartOfDay | AddDays | AddMonths | ParseLocal => {
            let int = TypeRef::scalar(FieldType::Int64);
            // The instant (or the text to read), then what is added, then the zone: always the last argument.
            match func {
                ParseLocal => check(0, &text)?,
                _ => check(0, &int)?,
            }
            if matches!(func, AddDays | AddMonths) {
                check(1, &int)?;
            }
            let zone = args.len() - 1;
            check(zone, &text)?;
            // A zone written in the flow is checked now: a typo would be a 400 at every call.
            if let Expr::Literal { value } = &args[zone]
                && let Some(name) = value.as_str()
                && !is_time_zone(name)
            {
                return Err(format!(
                    "`{name}` is not a time zone of the IANA database (`Europe/Paris`, `America/New_York`, `UTC`)"
                ));
            }
            match func {
                LocalDate | LocalTime => result(FieldType::String),
                ParseLocal => Ok(Some(TypeRef {
                    optional: true,
                    ty: FieldType::Int64,
                    repeated: false,
                })),
                _ => result(FieldType::Int64),
            }
        }
        ShortId => {
            check(0, &TypeRef::scalar(FieldType::Int64))?;
            result(FieldType::String)
        }
        FromShortId => {
            check(0, &text)?;
            Ok(Some(TypeRef {
                optional: true,
                ty: FieldType::Int64,
                repeated: false,
            }))
        }
        ListLength => {
            if let Some(t) = expr_type(project, scope, &args[0], None)?
                && !t.repeated
            {
                return Err(format!("`listLength` needs a list, found a {t}"));
            }
            result(FieldType::Int64)
        }
    }
}

/// The type of the operands of an arithmetic call: the one of the first argument that has a type of its own, and
/// the literals follow it; else what the context wants; else the natural type of the literals.
fn arithmetic_type(
    project: &Project,
    scope: &Scope,
    args: &[Expr],
    hint: Option<&TypeRef>,
) -> Result<Option<TypeRef>, String> {
    let hint = hint.filter(|h| is_number(h));
    for a in args {
        if !matches!(a, Expr::Literal { .. })
            && let Some(t) = expr_type(project, scope, a, hint)?
        {
            return Ok(Some(t));
        }
    }
    if let Some(h) = hint {
        return Ok(Some(h.clone()));
    }
    if args.iter().any(|a| matches!(a, Expr::Raw { .. })) {
        return Ok(None);
    }
    let float = args
        .iter()
        .any(|a| matches!(a, Expr::Literal { value } if value.is_f64()));
    Ok(Some(TypeRef::scalar(if float {
        FieldType::Double
    } else {
        FieldType::Int64
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Message;

    fn project() -> Project {
        let mut p = Project::new("demo");
        p.messages = vec![
            Message {
                kind: Default::default(),
                name: "Customer".into(),
                fields: vec![
                    field("name", FieldType::String),
                    field("age", FieldType::Int32),
                ],
            },
            Message {
                kind: Default::default(),
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

    #[test]
    fn uuid_is_text_with_a_checked_literal() {
        assert!(is_uuid("123e4567-e89b-12d3-a456-426614174000"));
        for bad in [
            "",
            "123e4567e89b12d3a456426614174000",
            "123e4567-e89b-12d3-a456-42661417400g",
            "x",
        ] {
            assert!(!is_uuid(bad), "{bad}");
        }
        let uuid = TypeRef::scalar(FieldType::Uuid);
        assert_eq!(
            coerce_literal(&"123e4567-e89b-12d3-a456-426614174000".into(), &uuid),
            Ok(Literal::Str("123e4567-e89b-12d3-a456-426614174000".into()))
        );
        assert!(coerce_literal(&"not a uuid".into(), &uuid).is_err());
        // Both are text: either fills the other.
        assert!(uuid.accepts(&TypeRef::scalar(FieldType::String)));
        assert!(TypeRef::scalar(FieldType::String).accepts(&uuid));
        assert!(!uuid.accepts(&TypeRef::scalar(FieldType::Int64)));
        // ...but a list of uuids is not a uuid.
        assert!(!uuid.accepts(&TypeRef {
            repeated: true,
            ..uuid.clone()
        }));
    }

    #[test]
    fn literals_can_be_absent_or_lists() {
        let optional = TypeRef {
            optional: true,
            ..TypeRef::scalar(FieldType::Int32)
        };
        assert_eq!(coerce_literal(&Value::Null, &optional), Ok(Literal::None));
        assert!(coerce_literal(&Value::Null, &TypeRef::scalar(FieldType::Int32)).is_err());
        let list = TypeRef {
            repeated: true,
            ..TypeRef::scalar(FieldType::String)
        };
        assert_eq!(
            coerce_literal(&serde_json::json!(["a", "b"]), &list),
            Ok(Literal::List(vec![
                Literal::Str("a".into()),
                Literal::Str("b".into())
            ]))
        );
        assert_eq!(
            coerce_literal(&serde_json::json!([]), &list),
            Ok(Literal::List(vec![]))
        );
        // One element of the wrong type, or no array at all, is refused.
        assert!(coerce_literal(&serde_json::json!(["a", 1]), &list).is_err());
        assert!(coerce_literal(&"a".into(), &list).is_err());
    }
}
