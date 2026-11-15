//! Lowers editor expressions to Rust code.
//!
//! Everything here assumes the project passed validation; failures are reported as
//! [`CodegenError::Internal`] rather than panics so the UI can still show them.

use sb_model::typing::{
    Adapt, Literal, ResolvedPath, Scope, TemplatePart, TypeRef, coerce_literal, expr_type,
    infer_literal, parse_template, resolve_path,
};
use sb_model::{Assignment, CmpOp, Condition, Expr, FieldType, Func, LogLevel, Project};

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
        Expr::Now => internal("`now()` only exists in an SQL step"),
        Expr::Template { .. } | Expr::Call { .. } => {
            let got = expr_type(project, scope, expr, Some(to))
                .map_err(CodegenError::Internal)?
                .ok_or_else(|| CodegenError::Internal("untyped operation".into()))?;
            let code = typed_code(project, scope, expr, &got)?;
            if to.accepts(&got) {
                Ok(code)
            } else if to.optional && !to.repeated && to.ty.assignable_from(&got.ty) {
                Ok(format!("Some({code})"))
            } else {
                internal(format!("a {got} cannot fill a {to}"))
            }
        }
    }
}

/// The code of a template or an operation, a value of type `ty` (what `expr_type` found).
fn typed_code(
    project: &Project,
    scope: &Scope,
    expr: &Expr,
    ty: &TypeRef,
) -> Result<String, CodegenError> {
    match expr {
        Expr::Template { template } => template_code(project, scope, template),
        Expr::Call { func, args } => call_code(project, scope, *func, args, ty),
        _ => lower(project, scope, expr, ty),
    }
}

/// `format!("Order {} paid", id)`: every hole is a plain value.
fn template_code(project: &Project, scope: &Scope, template: &str) -> Result<String, CodegenError> {
    let mut fmt = String::new();
    let mut args = String::new();
    for part in parse_template(template).map_err(CodegenError::Internal)? {
        match part {
            TemplatePart::Text(t) => {
                let escaped = format!("{t:?}");
                let inner = &escaped[1..escaped.len() - 1];
                fmt.push_str(&inner.replace('{', "{{").replace('}', "}}"));
            }
            TemplatePart::Path(p) => {
                let rp = resolve_path(project, scope, &p).map_err(CodegenError::Internal)?;
                fmt.push_str("{}");
                args.push_str(&format!(", {}", path_code(&rp)));
            }
        }
    }
    Ok(format!("format!(\"{fmt}\"{args})"))
}

fn call_code(
    project: &Project,
    scope: &Scope,
    func: Func,
    args: &[Expr],
    ty: &TypeRef,
) -> Result<String, CodegenError> {
    use Func::*;
    let text = TypeRef::scalar(FieldType::String);
    let text_list = TypeRef {
        repeated: true,
        ..text.clone()
    };
    // Code of argument `i` as a value of type `t`.
    let arg = |i: usize, t: &TypeRef| lower(project, scope, &args[i], t);
    let rust = rust_scalar(&ty.ty);
    let is_int = matches!(ty.ty, FieldType::Int32 | FieldType::Int64);
    Ok(match func {
        Concat => {
            let parts = (0..args.len())
                .map(|i| arg(i, &text))
                .collect::<Result<Vec<_>, _>>()?;
            format!("[{}].concat()", parts.join(", "))
        }
        Lower => format!("({}).to_lowercase()", arg(0, &text)?),
        Upper => format!("({}).to_uppercase()", arg(0, &text)?),
        Trim => format!("({}).trim().to_string()", arg(0, &text)?),
        SanitizeHtml => format!("eda_runtime::html::sanitize(&({}))", arg(0, &text)?),
        Length => format!("({}).chars().count() as i64", arg(0, &text)?),
        Contains | StartsWith | EndsWith => {
            let method = match func {
                Contains => "contains",
                StartsWith => "starts_with",
                _ => "ends_with",
            };
            format!(
                "({}).{method}(({}).as_str())",
                arg(0, &text)?,
                arg(1, &text)?
            )
        }
        Replace => format!(
            "({}).replace(({}).as_str(), ({}).as_str())",
            arg(0, &text)?,
            arg(1, &text)?,
            arg(2, &text)?
        ),
        Split => format!(
            "({}).split(({}).as_str()).map(str::to_string).collect::<Vec<String>>()",
            arg(0, &text)?,
            arg(1, &text)?
        ),
        Join => format!(
            "({}).join(({}).as_str())",
            arg(0, &text_list)?,
            arg(1, &text)?
        ),
        Add | Sub | Mul | Div | Rem | Min | Max => {
            let (a, b) = (arg(0, ty)?, arg(1, ty)?);
            match (func, is_int) {
                (Min | Max, _) => {
                    let f = if func == Min { "min" } else { "max" };
                    if is_int {
                        format!("std::cmp::{f}({a}, {b})")
                    } else {
                        format!("{rust}::{f}({a}, {b})")
                    }
                }
                (_, true) => {
                    let method = match func {
                        Add => "checked_add",
                        Sub => "checked_sub",
                        Mul => "checked_mul",
                        Div => "checked_div",
                        _ => "checked_rem",
                    };
                    format!(
                        "eda_runtime::logic::checked({rust}::{method}({a}, {b}), {:?})?",
                        func.name()
                    )
                }
                (_, false) => {
                    let op = match func {
                        Add => "+",
                        Sub => "-",
                        Mul => "*",
                        _ => "/",
                    };
                    format!("(({a}) {op} ({b}))")
                }
            }
        }
        Round | Floor | Ceil => {
            let f = match func {
                Round => "round",
                Floor => "floor",
                _ => "ceil",
            };
            let float = TypeRef::scalar(FieldType::Double);
            format!(
                "eda_runtime::logic::f64_to_i64((({}) as f64).{f}())?",
                arg(0, &float)?
            )
        }
        ToInt32 | ToInt64 | ToDouble | ToText => {
            let from = typed(project, scope, &args[0])?;
            let a = arg(0, &from)?;
            let from_float = matches!(from.ty, FieldType::Float | FieldType::Double);
            match func {
                ToText => format!("({a}).to_string()"),
                ToDouble => format!("({a}) as f64"),
                ToInt64 if from_float => format!("eda_runtime::logic::f64_to_i64(({a}) as f64)?"),
                ToInt64 => format!("i64::from({a})"),
                _ if from_float => format!(
                    "eda_runtime::logic::int::<i32>(eda_runtime::logic::f64_to_i64(({a}) as f64)?)?"
                ),
                _ if from.ty == FieldType::Int32 => a,
                _ => format!("eda_runtime::logic::int::<i32>({a})?"),
            }
        }
        ParseInt64 => format!("({}).trim().parse::<i64>().ok()", arg(0, &text)?),
        ParseDouble => format!(
            "({}).trim().parse::<f64>().ok().filter(|v| v.is_finite())",
            arg(0, &text)?
        ),
        Default => {
            let optional = TypeRef {
                optional: true,
                ..ty.clone()
            };
            format!(
                "({}).unwrap_or_else(|| {})",
                arg(0, &optional)?,
                arg(1, ty)?
            )
        }
        NewUuid => "eda_runtime::logic::new_uuid()".to_string(),
        NewUuidV7 => "eda_runtime::logic::new_uuid_v7()".to_string(),
        ShortId => format!(
            "eda_runtime::short_id::encode({})?",
            arg(0, &TypeRef::scalar(FieldType::Int64))?
        ),
        NowSeconds => "eda_runtime::time_zone::now_seconds()".to_string(),
        LocalDate | LocalTime | StartOfDay => {
            let function = match func {
                LocalDate => "local_date",
                LocalTime => "local_time",
                _ => "start_of_day",
            };
            format!(
                "eda_runtime::time_zone::{function}({}, &({}))?",
                arg(0, &TypeRef::scalar(FieldType::Int64))?,
                arg(1, &text)?
            )
        }
        AddDays | AddMonths => {
            let function = if func == AddDays {
                "add_days"
            } else {
                "add_months"
            };
            let int = TypeRef::scalar(FieldType::Int64);
            format!(
                "eda_runtime::time_zone::{function}({}, {}, &({}))?",
                arg(0, &int)?,
                arg(1, &int)?,
                arg(2, &text)?
            )
        }
        ParseLocal => format!(
            "eda_runtime::time_zone::parse_local(&({}), &({}))?",
            arg(0, &text)?,
            arg(1, &text)?
        ),
        FromShortId => format!("eda_runtime::short_id::decode(&({}))", arg(0, &text)?),
        ListLength => {
            let from = typed(project, scope, &args[0])?;
            format!("({}).len() as i64", arg(0, &from)?)
        }
    })
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
        Condition::And { conditions } | Condition::Or { conditions } => {
            let and = matches!(cond, Condition::And { .. });
            if conditions.is_empty() {
                return Ok(and.to_string());
            }
            let parts = conditions
                .iter()
                .map(|c| condition_code(project, scope, c).map(|c| format!("({c})")))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!(
                "({})",
                parts.join(if and { " && " } else { " || " })
            ))
        }
        Condition::Not { condition } => {
            Ok(format!("!({})", condition_code(project, scope, condition)?))
        }
        Condition::Present { value } | Condition::Absent { value } => {
            let ty = typed(project, scope, value)?;
            let method = if matches!(cond, Condition::Present { .. }) {
                "is_some"
            } else {
                "is_none"
            };
            Ok(format!(
                "({}).{method}()",
                lower(project, scope, value, &ty)?
            ))
        }
        Condition::Empty { value } => {
            let ty = typed(project, scope, value)?;
            Ok(format!(
                "({}).is_empty()",
                lower(project, scope, value, &ty)?
            ))
        }
        Condition::Test { value } => {
            lower(project, scope, value, &TypeRef::scalar(FieldType::Bool))
                .map(|c| format!("({c})"))
        }
        Condition::In { value, values } => {
            let ty = match expr_type(project, scope, value, None).map_err(CodegenError::Internal)? {
                Some(t) => t,
                None => match values.first() {
                    Some(v) => typed(project, scope, v)?,
                    None => return Ok("false".into()),
                },
            };
            let items = values
                .iter()
                .map(|v| lower(project, scope, v, &ty))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!(
                "[{}].contains(&({}))",
                items.join(", "),
                lower(project, scope, value, &ty)?
            ))
        }
        Condition::Compare { left, op, right } => {
            let side_type = |e: &Expr| -> Result<Option<TypeRef>, CodegenError> {
                match e {
                    Expr::Path { .. } | Expr::Call { .. } | Expr::Template { .. } => {
                        expr_type(project, scope, e, None).map_err(CodegenError::Internal)
                    }
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
                    (
                        Expr::Path { .. } | Expr::Now | Expr::Call { .. } | Expr::Template { .. },
                        None,
                    ) => internal("unresolved comparison side"),
                }
            };
            Ok(format!("({}) {} ({})", side(left)?, cmp(*op), side(right)?))
        }
    }
}

/// The type of an expression the editor can see into.
fn typed(project: &Project, scope: &Scope, e: &Expr) -> Result<TypeRef, CodegenError> {
    expr_type(project, scope, e, None)
        .map_err(CodegenError::Internal)?
        .ok_or_else(|| CodegenError::Internal("the type of raw Rust is unknown".into()))
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

    fn path(p: &str) -> Expr {
        Expr::Path { path: p.into() }
    }

    fn lit(v: impl Into<serde_json::Value>) -> Expr {
        Expr::Literal { value: v.into() }
    }

    fn call(func: Func, args: Vec<Expr>) -> Expr {
        Expr::Call { func, args }
    }

    #[test]
    fn integer_arithmetic_is_checked_and_floats_are_plain() {
        let (p, scope) = orders_scope();
        let int = TypeRef::scalar(FieldType::Int64);
        let add = call(Func::Add, vec![path("input.amount"), lit(5)]);
        assert_eq!(
            lower(&p, &scope, &add, &int).unwrap(),
            "eda_runtime::logic::checked(i64::checked_add(input.amount.clone(), 5), \"add\")?"
        );
        let div = call(Func::Div, vec![path("input.amount"), lit(0)]);
        assert!(
            lower(&p, &scope, &div, &int)
                .unwrap()
                .contains("checked_div(input.amount.clone(), 0)")
        );
        let double = TypeRef::scalar(FieldType::Double);
        let mul = call(Func::Mul, vec![lit(1.5), lit(2.0)]);
        assert_eq!(lower(&p, &scope, &mul, &double).unwrap(), "((1.5) * (2.0))");
        let min = call(Func::Min, vec![path("input.amount"), lit(10)]);
        assert_eq!(
            lower(&p, &scope, &min, &int).unwrap(),
            "std::cmp::min(input.amount.clone(), 10)"
        );
    }

    #[test]
    fn conversions_and_text_go_through_the_runtime_or_std() {
        let (p, scope) = orders_scope();
        let int32 = TypeRef::scalar(FieldType::Int32);
        let to = call(Func::ToInt32, vec![path("input.amount")]);
        assert_eq!(
            lower(&p, &scope, &to, &int32).unwrap(),
            "eda_runtime::logic::int::<i32>(input.amount.clone())?"
        );
        let text = TypeRef::scalar(FieldType::String);
        let t = Expr::Template {
            template: "n={input.amount} {{ok}}".into(),
        };
        assert_eq!(
            lower(&p, &scope, &t, &text).unwrap(),
            "format!(\"n={} {{ok}}\", input.amount.clone())"
        );
        let up = call(
            Func::Upper,
            vec![call(Func::Trim, vec![path("input.customer.name")])],
        );
        assert!(
            lower(&p, &scope, &up, &text)
                .unwrap()
                .ends_with(".trim().to_string()).to_uppercase()")
        );
        // A value that cannot be absent fills an optional field.
        let note = TypeRef {
            optional: true,
            ..text.clone()
        };
        let low = call(Func::Lower, vec![lit("A")]);
        assert!(lower(&p, &scope, &low, &note).unwrap().starts_with("Some("));
    }

    #[test]
    fn conditions_nest_and_test_presence() {
        let (p, scope) = orders_scope();
        let gt = Condition::Compare {
            left: path("input.amount"),
            op: CmpOp::Gt,
            right: lit(10),
        };
        let both = Condition::And {
            conditions: vec![
                gt.clone(),
                Condition::Not {
                    condition: Box::new(Condition::Empty {
                        value: path("input.customer.name"),
                    }),
                },
            ],
        };
        assert_eq!(
            condition_code(&p, &scope, &both).unwrap(),
            "(((input.amount.clone()) > (10)) && (!((input.customer.as_ref().map(|m| m.name.clone()).unwrap_or_default()).is_empty())))"
        );
        let any = Condition::In {
            value: path("input.amount"),
            values: vec![lit(1), lit(2)],
        };
        assert_eq!(
            condition_code(&p, &scope, &any).unwrap(),
            "[1, 2].contains(&(input.amount.clone()))"
        );
        assert_eq!(
            condition_code(&p, &scope, &Condition::Or { conditions: vec![] }).unwrap(),
            "false"
        );
        let length = Condition::Compare {
            left: call(Func::Length, vec![path("input.customer.name")]),
            op: CmpOp::Ge,
            right: lit(3),
        };
        assert!(
            condition_code(&p, &scope, &length)
                .unwrap()
                .contains(".chars().count() as i64")
        );
    }
}
