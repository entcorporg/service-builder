//! The code of the `httpCall` step: a request built from the values of the flow, sent by `eda_runtime::http_client`, and
//! the answer bound to variables.

use std::collections::{BTreeMap, BTreeSet};

use sb_model::typing::{Scope, TemplatePart, TypeRef, parse_template, resolve_path};
use sb_model::{
    Assignment, FieldType, HttpAuthKind, HttpBodyFormat, HttpOnError, Project, Service, Step,
};
use serde_json::Value;

use crate::CodegenError;
use crate::expr::{lower, path_code};
use crate::writer::Code;

/// Whether a service has an outbound call: it then needs the HTTP client of the runtime (the `http-client` feature).
pub fn uses_http_client(service: &Service) -> bool {
    !http_targets(service).is_empty()
}

/// The external services a service calls: the variable of each base address, with the variables of the secrets sent to it.
/// The service checks them at start-up.
pub fn http_targets(service: &Service) -> BTreeMap<&str, BTreeSet<&str>> {
    let mut targets: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for step in service.handler_steps() {
        step.walk(&mut |s| {
            if let Step::HttpCall {
                base_url_env,
                auth_env,
                auth_password_env,
                ..
            } = s
            {
                let secrets = targets.entry(base_url_env.as_str()).or_default();
                secrets.extend(
                    [auth_env, auth_password_env]
                        .into_iter()
                        .flatten()
                        .map(String::as_str),
                );
            }
        });
    }
    targets
}

/// `fields.set("name", value);` for each assignment: an optional value is only sent when present.
fn fields_code(
    c: &mut Code,
    project: &Project,
    scope: &Scope,
    var: &str,
    list: &[Assignment],
) -> Result<(), CodegenError> {
    let kw = if list.is_empty() { "let" } else { "let mut" };
    c.line(format!(
        "{kw} {var} = eda_runtime::http_client::Fields::default();"
    ));
    for a in list {
        let name = format!("{:?}", a.field);
        match &a.value {
            sb_model::Expr::Path { path } => {
                let rp = resolve_path(project, scope, path).map_err(CodegenError::Internal)?;
                let call = if rp.ty.optional { "set_opt" } else { "set" };
                c.line(format!("{var}.{call}({name}, {});", path_code(&rp)));
            }
            sb_model::Expr::Literal { value } => {
                match value {
                    Value::String(s) => c.line(format!("{var}.set({name}, {s:?});")),
                    Value::Number(_) | Value::Bool(_) => {
                        c.line(format!("{var}.set({name}, {value});"))
                    }
                    Value::Null => c.line("// an absent value is not sent"),
                    other => c.line(format!("{var}.set({name}, {:?});", other.to_string())),
                };
            }
            sb_model::Expr::Raw { code } => {
                c.line(format!("{var}.set({name}, {});", code.trim()));
            }
            sb_model::Expr::Now => {
                return Err(CodegenError::Internal(
                    "`now()` only exists in an SQL step".into(),
                ));
            }
        }
    }
    Ok(())
}

pub fn http_call_step(
    c: &mut Code,
    project: &Project,
    scope: &mut Scope,
    step: &Step,
) -> Result<(), CodegenError> {
    let Step::HttpCall {
        id,
        method,
        base_url_env,
        path,
        query,
        headers,
        auth,
        auth_env,
        auth_password_env,
        auth_header,
        body,
        body_format,
        timeout_secs,
        retries,
        idempotency_key,
        response,
        bind,
        status_bind,
        on_error,
        error_status,
        error_message,
    } = step
    else {
        return Err(CodegenError::Internal("not an httpCall".into()));
    };
    let internal = |what: &str| CodegenError::Internal(format!("httpCall `{id}`: {what}"));
    c.open("let __answer =");
    // The path: fixed text and values, each one encoded as a single segment.
    c.line("let mut __path = String::new();");
    for part in parse_template(path).map_err(CodegenError::Internal)? {
        match part {
            TemplatePart::Text(t) => {
                c.line(format!("__path.push_str({t:?});"));
            }
            TemplatePart::Path(p) => {
                let rp = resolve_path(project, scope, &p).map_err(CodegenError::Internal)?;
                c.line(format!(
                    "__path.push_str(&eda_runtime::http_client::segment(&{}.to_string())?);",
                    path_code(&rp)
                ));
            }
        }
    }
    fields_code(c, project, scope, "__query", query)?;
    fields_code(c, project, scope, "__headers", headers)?;
    fields_code(c, project, scope, "__body", body)?;
    let key = match idempotency_key {
        Some(e) => format!(
            "Some({})",
            lower(project, scope, e, &TypeRef::scalar(FieldType::String))?
        ),
        None => "None".to_string(),
    };
    let auth = match (auth, auth_env) {
        (None, _) => "eda_runtime::http_client::Auth::None".to_string(),
        (Some(HttpAuthKind::Bearer), Some(env)) => {
            format!("eda_runtime::http_client::Auth::Bearer({env:?})")
        }
        (Some(HttpAuthKind::Basic), Some(user)) => {
            let password = auth_password_env
                .as_deref()
                .ok_or_else(|| internal("basic authentication without a password"))?;
            format!("eda_runtime::http_client::Auth::Basic({user:?}, {password:?})")
        }
        (Some(HttpAuthKind::ApiKey), Some(env)) => {
            let header = auth_header
                .as_deref()
                .ok_or_else(|| internal("API key without a header"))?;
            format!("eda_runtime::http_client::Auth::ApiKey({header:?}, {env:?})")
        }
        _ => return Err(internal("authentication without a secret")),
    };
    let format = match body_format {
        HttpBodyFormat::Json => "Json",
        HttpBodyFormat::Form => "Form",
    };
    c.line("eda_runtime::http_client::send(eda_runtime::http_client::Call {");
    c.line(format!("    id: {id:?},"));
    c.line(format!("    method: {:?},", method.as_str()));
    c.line(format!("    base_url_env: {base_url_env:?},"));
    c.line("    path: &__path,");
    c.line("    query: __query,");
    c.line("    headers: __headers,");
    c.line(format!("    auth: {auth},"));
    c.line("    body: __body,");
    c.line(format!(
        "    format: eda_runtime::http_client::BodyFormat::{format},"
    ));
    c.line(format!("    timeout_secs: {timeout_secs},"));
    c.line(format!("    retries: {retries},"));
    c.line(format!("    idempotency_key: {key},"));
    c.line("})");
    c.line(".await?");
    c.close_with(";");
    if *on_error == HttpOnError::Fail {
        // The transaction is dropped with the early return: nothing the flow wrote is kept.
        c.line(format!(
            "let __answer = __answer.require_success({error_status}, {error_message:?})?;"
        ));
    }
    if let Some(var) = status_bind {
        c.line(format!("let {var}: i32 = i32::from(__answer.status);"));
        scope.bind(var.clone(), TypeRef::scalar(FieldType::Int32));
    }
    if let (Some(var), Some(schema)) = (bind, response) {
        c.line(format!("let {var}: pb::{schema} = __answer.parse()?;"));
        scope.bind(var.clone(), TypeRef::message(schema));
    }
    Ok(())
}
