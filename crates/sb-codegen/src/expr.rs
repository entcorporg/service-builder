//! Lowers editor expressions to Rust code.
//!
//! Everything here assumes the project passed validation; failures are reported as
//! [`CodegenError::Internal`] rather than panics so the UI can still show them.

use sb_model::typing::{
    Adapt, Literal, ResolvedPath, Scope, TemplatePart, TypeRef, coerce_literal, infer_literal,
    parse_template, resolve_path,
};
use sb_model::{Assignment, CmpOp, Condition, Expr, FieldType, LogLevel, Project};

use crate::CodegenError;

fn internal<T>(msg: impl Into<String>) -> Result<T, CodegenError> {
    Err(CodegenError::Internal(msg.into()))
}

/// Rust type of a plain scalar (used by `Let`).
pub fn rust_scalar(ty: &FieldType) -> &'static str {
    match ty {
        FieldType::String | FieldType::Uuid => "String",
        FieldType::Bool => "bool",
        FieldType::Int32 => "i32",
        FieldType::Int64 => "i64",
        FieldType::Float => "f32",
        FieldType::Double => "f64",
        _ => "()",
    }
}

/// Rust type of a variable declared by a Let step: a scalar, or a schema (`pb::OrderView`),
/// alone, optional (`Option<..>`) or a list (`Vec<..>`).
pub fn rust_let_type(ty: &FieldType, optional: bool, repeated: bool) -> String {
    let base = match ty {
        FieldType::Message { name } => format!("pb::{name}"),
        other => rust_scalar(other).to_string(),
    };
    match (optional, repeated) {
        (_, true) => format!("Vec<{base}>"),
        (true, false) => format!("Option<{base}>"),
        (false, false) => base,
    }
}

/// Expression reading a variable or a (nested) field, as an owned value whose Rust type
/// is the declared type of the last field.
///
/// Message fields are `Option<Msg>` in prost, so walking through one yields `None`, and
/// therefore the default value for scalars, when the intermediate message is absent.
pub fn path_code(rp: &ResolvedPath) -> String {
    let var = &rp.var;
    let Some((last, middle)) = rp.hops.split_last() else {
        return format!("{var}.clone()");
    };
    let Some((first, rest)) = middle.split_first() else {
        return format!("{var}.{}.clone()", last.name);
    };
    let mut code = format!("{var}.{}.as_ref()", first.name);
    for hop in rest {
        code.push_str(&format!(".and_then(|m| m.{}.as_ref())", hop.name));
    }
    let last_ty = TypeRef::of_field(last);
    if last_ty.is_message() && !last_ty.repeated || last_ty.optional {
        code.push_str(&format!(".and_then(|m| m.{}.clone())", last.name));
    } else {
        code.push_str(&format!(
            ".map(|m| m.{}.clone()).unwrap_or_default()",
            last.name
        ));
    }
    code
}

pub fn literal_code(lit: &Literal, to: &TypeRef) -> String {
    let code = match lit {
        Literal::None => return "None".to_string(),
        Literal::List(items) => {
            let item = TypeRef::scalar(to.ty.clone());
            let items: Vec<String> = items.iter().map(|i| literal_code(i, &item)).collect();
            return format!("vec![{}]", items.join(", "));
        }
        Literal::Str(s) => format!("{s:?}.to_string()"),
        Literal::Int(i) => i.to_string(),
        Literal::Float(f) => format!("{f:?}"),
        Literal::Bool(b) => b.to_string(),
    };
    if to.optional {
        format!("Some({code})")
    } else {
        code
    }
}

/// Code for `expr` assigned to a field/variable of type `to`.
pub fn lower(
    project: &Project,
    scope: &Scope,
    expr: &Expr,
    to: &TypeRef,
) -> Result<String, CodegenError> {
    match expr {
        Expr::Path { path } => {
            let rp = resolve_path(project, scope, path).map_err(CodegenError::Internal)?;
            let code = path_code(&rp);
            match rp.adapt_to(to) {
                Some(Adapt::Identity) => Ok(code),
                Some(Adapt::WrapSome) => Ok(format!("Some({code})")),
                None => internal(format!("`{path}` ({}) cannot fill a {to}", rp.ty)),
            }
        }
        Expr::Literal { value } => {
            let lit = coerce_literal(value, to).map_err(CodegenError::Internal)?;
            Ok(literal_code(&lit, to))
        }
        Expr::Raw { code } => Ok(code.trim().to_string()),
    }
}

/// `pb::Message { field: value, ..Default::default() }`
pub fn struct_literal(
    project: &Project,
    scope: &Scope,
    message: &str,
    assignments: &[Assignment],
) -> Result<String, CodegenError> {
    let Some(msg) = project.message(message) else {
        return internal(format!("unknown message `{message}`"));
    };
    let mut out = format!("pb::{message} {{\n");
    for a in assignments {
        let Some(field) = msg.field(&a.field) else {
            return internal(format!("message `{message}` has no field `{}`", a.field));
        };
        let value = lower(project, scope, &a.value, &TypeRef::of_field(field))?;
        out.push_str(&format!("    {}: {value},\n", a.field));
    }
    out.push_str("    ..Default::default()\n}");
    Ok(out)
}

pub fn condition_code(
    project: &Project,
    scope: &Scope,
    cond: &Condition,
) -> Result<String, CodegenError> {
    match cond {
        Condition::Raw { code } => Ok(code.trim().to_string()),
        Condition::Compare { left, op, right } => {
            let side_type = |e: &Expr| -> Result<Option<TypeRef>, CodegenError> {
                match e {
                    Expr::Path { path } => Ok(Some(
                        resolve_path(project, scope, path)
                            .map_err(CodegenError::Internal)?
                            .ty,
                    )),
                    _ => Ok(None),
                }
            };
            let target = match (side_type(left)?, side_type(right)?) {
                (Some(t), _) | (None, Some(t)) => Some(t),
                (None, None) => match left {
                    Expr::Literal { value } => infer_literal(value),
                    _ => None,
                },
            };
            let side = |e: &Expr| -> Result<String, CodegenError> {
                match (e, &target) {
                    (Expr::Raw { code }, _) => Ok(code.trim().to_string()),
                    (e, Some(t)) => lower(project, scope, e, t),
                    (Expr::Literal { value }, None) => {
                        // The other side is raw Rust: use the literal's natural type.
                        let t = infer_literal(value)
                            .ok_or_else(|| CodegenError::Internal("unsupported literal".into()))?;
                        lower(project, scope, e, &t)
                    }
                    (Expr::Path { .. }, None) => internal("unresolved comparison side"),
                }
            };
            Ok(format!("({}) {} ({})", side(left)?, cmp(*op), side(right)?))
        }
    }
}

fn cmp(op: CmpOp) -> &'static str {
    op.rust()
}

/// `tracing::info!("order {} paid", order_id)`
pub fn log_code(
    project: &Project,
    scope: &Scope,
    level: LogLevel,
    template: &str,
) -> Result<String, CodegenError> {
    let parts = parse_template(template).map_err(CodegenError::Internal)?;
    let mut fmt = String::new();
    let mut args = Vec::new();
    for part in parts {
        match part {
            TemplatePart::Text(t) => {
                // Debug-format for Rust escaping, strip the quotes, then escape braces.
                let escaped = format!("{t:?}");
                let inner = &escaped[1..escaped.len() - 1];
                fmt.push_str(&inner.replace('{', "{{").replace('}', "}}"));
            }
            TemplatePart::Path(p) => {
                let rp = resolve_path(project, scope, &p).map_err(CodegenError::Internal)?;
                fmt.push_str(if rp.ty.is_simple_scalar() {
                    "{}"
                } else {
                    "{:?}"
                });
                args.push(path_code(&rp));
            }
        }
    }
    let level = match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    };
    let args = args.iter().map(|a| format!(", {a}")).collect::<String>();
    Ok(format!("tracing::{level}!(\"{fmt}\"{args});"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_model::sample;

    fn orders_scope() -> (Project, Scope) {
        (sample::orders(), Scope::with_input("CreateOrderRequest"))
    }

    #[test]
    fn path_codes() {
        let (p, scope) = orders_scope();
        let code = |path: &str| path_code(&resolve_path(&p, &scope, path).unwrap());
        assert_eq!(code("input"), "input.clone()");
        assert_eq!(code("input.amount"), "input.amount.clone()");
        assert_eq!(code("input.customer"), "input.customer.clone()");
        assert_eq!(
            code("input.customer.name"),
            "input.customer.as_ref().map(|m| m.name.clone()).unwrap_or_default()"
        );
    }

    #[test]
    fn nested_message_path_keeps_option() {
        // Message -> message -> message field stays an Option.
        use sb_model::{Field, Message};
        let mut p = Project::new("x");
        let m = |name: &str, f: Field| Message {
            kind: Default::default(),
            name: name.into(),
            fields: vec![f],
        };
        let msg = |n: &str| FieldType::Message { name: n.into() };
        p.messages = vec![
            m("A", Field::new("b", msg("B"))),
            m("B", Field::new("c", msg("C"))),
            m("C", Field::new("v", FieldType::Int32)),
        ];
        let scope = Scope::with_input("A");
        let rp = resolve_path(&p, &scope, "input.b.c").unwrap();
        assert_eq!(path_code(&rp), "input.b.as_ref().and_then(|m| m.c.clone())");
        let rp = resolve_path(&p, &scope, "input.b.c.v").unwrap();
        assert_eq!(
            path_code(&rp),
            "input.b.as_ref().and_then(|m| m.c.as_ref()).map(|m| m.v.clone()).unwrap_or_default()"
        );
    }

    #[test]
    fn lowering_wraps_and_literals() {
        let (p, scope) = orders_scope();
        let note = TypeRef {
            optional: true,
            ..TypeRef::scalar(FieldType::String)
        };
        let path = Expr::Path {
            path: "input.customer.name".into(),
        };
        assert_eq!(
            lower(&p, &scope, &path, &note).unwrap(),
            "Some(input.customer.as_ref().map(|m| m.name.clone()).unwrap_or_default())"
        );
        let lit = Expr::Literal {
            value: "a\"b".into(),
        };
        assert_eq!(
            lower(&p, &scope, &lit, &TypeRef::scalar(FieldType::String)).unwrap(),
            "\"a\\\"b\".to_string()"
        );
        let whole = Expr::Path {
            path: "input".into(),
        };
        assert_eq!(
            lower(&p, &scope, &whole, &TypeRef::message("CreateOrderRequest")).unwrap(),
            "Some(input.clone())"
        );
        let float = Expr::Literal { value: 2.into() };
        assert_eq!(
            lower(&p, &scope, &float, &TypeRef::scalar(FieldType::Double)).unwrap(),
            "2.0"
        );
    }

    #[test]
    fn lowering_optional_and_list_literals() {
        let (p, scope) = orders_scope();
        let lit = |v: serde_json::Value| Expr::Literal { value: v };
        let optional = TypeRef {
            optional: true,
            ..TypeRef::scalar(FieldType::Int32)
        };
        assert_eq!(
            lower(&p, &scope, &lit(serde_json::Value::Null), &optional).unwrap(),
            "None"
        );
        assert_eq!(
            lower(&p, &scope, &lit(3.into()), &optional).unwrap(),
            "Some(3)"
        );
        let list = TypeRef {
            repeated: true,
            ..TypeRef::scalar(FieldType::String)
        };
        assert_eq!(
            lower(&p, &scope, &lit(serde_json::json!(["a", "b"])), &list).unwrap(),
            "vec![\"a\".to_string(), \"b\".to_string()]"
        );
        assert_eq!(
            lower(&p, &scope, &lit(serde_json::json!([])), &list).unwrap(),
            "vec![]"
        );
        assert_eq!(rust_let_type(&FieldType::Uuid, false, false), "String");
        assert_eq!(rust_let_type(&FieldType::Int64, true, false), "Option<i64>");
        assert_eq!(
            rust_let_type(&FieldType::Message { name: "M".into() }, false, true),
            "Vec<pb::M>"
        );
    }

    #[test]
    fn log_escapes_braces_and_quotes() {
        let (p, scope) = orders_scope();
        let code = log_code(
            &p,
            &scope,
            LogLevel::Warn,
            "say \"hi\" {{x}} {input.amount} / {input.customer}",
        )
        .unwrap();
        assert_eq!(
            code,
            "tracing::warn!(\"say \\\"hi\\\" {{x}} {} / {:?}\", input.amount.clone(), input.customer.clone());"
        );
    }

    #[test]
    fn comparison() {
        let (p, mut scope) = orders_scope();
        scope.bind("ok", TypeRef::scalar(FieldType::Bool));
        let cond = Condition::Compare {
            left: Expr::Path { path: "ok".into() },
            op: CmpOp::Eq,
            right: Expr::Literal { value: true.into() },
        };
        assert_eq!(
            condition_code(&p, &scope, &cond).unwrap(),
            "(ok.clone()) == (true)"
        );
        let cond = Condition::Compare {
            left: Expr::Raw { code: "x".into() },
            op: CmpOp::Gt,
            right: Expr::Literal { value: 3.into() },
        };
        assert_eq!(condition_code(&p, &scope, &cond).unwrap(), "(x) > (3)");
    }
}
