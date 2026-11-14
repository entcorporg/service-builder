//! Syntax check of the raw Rust typed in the editor (expressions, conditions, code
//! blocks). The model validator cannot do it, and a syntax error would otherwise only
//! surface when compiling the generated workspace.

use sb_model::{Condition, Diagnostic, Expr, Project, Severity, Step};

pub fn check(project: &Project) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (si, s) in project.services.iter().enumerate() {
        mail_templates(si, s, &mut out);
        for (ri, r) in s.rpcs.iter().enumerate() {
            steps(
                &format!("/services/{si}/rpcs/{ri}/steps"),
                &r.steps,
                &mut out,
            );
        }
        for (ri, r) in s.routes.iter().enumerate() {
            steps(
                &format!("/services/{si}/routes/{ri}/steps"),
                &r.steps,
                &mut out,
            );
        }
        for (ci, c) in s.consumers.iter().enumerate() {
            steps(
                &format!("/services/{si}/consumers/{ci}/steps"),
                &c.steps,
                &mut out,
            );
        }
    }
    out
}

/// The mails of a mail service are Tera templates: a syntax error would otherwise only show when the worker loads them, and
/// stop every mail.
fn mail_templates(si: usize, s: &sb_model::Service, out: &mut Vec<Diagnostic>) {
    for (ti, t) in s.templates.iter().enumerate() {
        // A mail composed as blocks is checked as the HTML and the text it derives to.
        let (html, text) = t.bodies();
        for (field, source) in [("subject", &t.subject), ("html", &html), ("text", &text)] {
            if let Err(e) = tera::Tera::default().add_raw_template(field, source) {
                // Tera's message spans several lines (the cause is on the last).
                let why = std::error::Error::source(&e)
                    .map(|cause| cause.to_string())
                    .unwrap_or_else(|| e.to_string());
                out.push(Diagnostic {
                    severity: Severity::Error,
                    code: "E_MTA_TEMPLATE_SYNTAX".into(),
                    message: format!(
                        "the {field} of the mail for `{}` is not a valid template: {}",
                        t.event_type,
                        why.trim()
                    ),
                    path: format!("/services/{si}/templates/{ti}/{field}"),
                });
            }
        }
    }
}

fn steps(base: &str, list: &[Step], out: &mut Vec<Diagnostic>) {
    for (i, step) in list.iter().enumerate() {
        let path = format!("{base}/{i}");
        match step {
            Step::Log { .. } => {}
            Step::Let { value, .. } => expr(value, &format!("{path}/value"), out),
            Step::Publish { payload, .. } => assignments(payload, &format!("{path}/payload"), out),
            Step::PublishLive {
                account_id,
                tenant_id,
                account_ids,
                origin,
                data,
                ..
            } => {
                for (name, e) in [
                    ("accountId", account_id),
                    ("tenantId", tenant_id),
                    ("accountIds", account_ids),
                    ("origin", origin),
                ] {
                    if let Some(e) = e {
                        expr(e, &format!("{path}/{name}"), out);
                    }
                }
                assignments(data, &format!("{path}/data"), out);
            }
            Step::CallRpc { request, .. } => assignments(request, &format!("{path}/request"), out),
            Step::Respond {
                fields, cookies, ..
            } => {
                assignments(fields, &format!("{path}/fields"), out);
                for (j, c) in cookies.iter().enumerate() {
                    expr(&c.value, &format!("{path}/cookies/{j}/value"), out);
                }
            }
            Step::RateLimit { key, .. } => expr(key, &format!("{path}/key"), out),
            Step::VerifyHmac { .. } => {}
            Step::SignJwt { claims, .. } => assignments(claims, &format!("{path}/claims"), out),
            Step::VerifyJwt { token, .. } => expr(token, &format!("{path}/token"), out),
            Step::Jwks { .. } => {}
            Step::TotpGenerate { account, .. } => expr(account, &format!("{path}/account"), out),
            Step::TotpVerify {
                sealed,
                code,
                account,
                ..
            } => {
                expr(sealed, &format!("{path}/sealed"), out);
                expr(code, &format!("{path}/code"), out);
                expr(account, &format!("{path}/account"), out);
            }
            Step::PasskeyRegisterStart {
                account,
                name,
                app_url,
                ..
            } => {
                expr(account, &format!("{path}/account"), out);
                expr(name, &format!("{path}/name"), out);
                app_url_expr(app_url, &path, out);
            }
            Step::PasskeyRegisterFinish {
                state_id,
                credential,
                account,
                app_url,
                ..
            } => {
                expr(state_id, &format!("{path}/stateId"), out);
                expr(credential, &format!("{path}/credential"), out);
                expr(account, &format!("{path}/account"), out);
                app_url_expr(app_url, &path, out);
            }
            Step::PasskeyLoginStart {
                account, app_url, ..
            } => {
                if let Some(account) = account {
                    expr(account, &format!("{path}/account"), out)
                }
                app_url_expr(app_url, &path, out);
            }
            Step::PasskeyLoginFinish {
                state_id,
                credential,
                app_url,
                ..
            } => {
                expr(state_id, &format!("{path}/stateId"), out);
                expr(credential, &format!("{path}/credential"), out);
                app_url_expr(app_url, &path, out);
            }
            Step::ImageSniff { input, .. } => expr(input, &format!("{path}/input"), out),
            Step::ObjectPut {
                key,
                body,
                content_type,
                ..
            } => {
                expr(key, &format!("{path}/key"), out);
                expr(body, &format!("{path}/body"), out);
                expr(content_type, &format!("{path}/contentType"), out);
            }
            Step::ObjectGet { key, .. }
            | Step::ObjectDelete { key, .. }
            | Step::ObjectPresignGet { key, .. } => expr(key, &format!("{path}/key"), out),
            Step::ObjectPresignPut {
                key,
                content_type,
                size,
                ..
            } => {
                expr(key, &format!("{path}/key"), out);
                expr(content_type, &format!("{path}/contentType"), out);
                expr(size, &format!("{path}/size"), out);
            }
            Step::RecoveryCodes { account, .. } => expr(account, &format!("{path}/account"), out),
            Step::RecoveryCodeUse { account, code, .. } => {
                expr(account, &format!("{path}/account"), out);
                expr(code, &format!("{path}/code"), out);
            }
            Step::HashPassword { input, .. }
            | Step::Sha256 { input, .. }
            | Step::Encrypt { input, .. }
            | Step::Decrypt { input, .. } => expr(input, &format!("{path}/input"), out),
            Step::VerifyPassword {
                password, stored, ..
            } => {
                expr(password, &format!("{path}/password"), out);
                expr(stored, &format!("{path}/stored"), out);
            }
            Step::RandomToken { .. } => {}
            Step::HttpCall {
                query,
                headers,
                body,
                idempotency_key,
                ..
            } => {
                assignments(query, &format!("{path}/query"), out);
                assignments(headers, &format!("{path}/headers"), out);
                assignments(body, &format!("{path}/body"), out);
                if let Some(key) = idempotency_key {
                    expr(key, &format!("{path}/idempotencyKey"), out);
                }
            }
            Step::Fail { retry_after, .. } => {
                if let Some(wait) = retry_after {
                    expr(wait, &format!("{path}/retryAfter"), out);
                }
            }
            Step::Sql {
                conditions,
                values,
                cursor,
                limit_from,
                offset,
                ..
            } => {
                for (j, c) in conditions.iter().enumerate() {
                    expr(&c.value, &format!("{path}/where/{j}/value"), out);
                }
                for (j, c) in cursor.iter().enumerate() {
                    expr(c, &format!("{path}/cursor/{j}"), out);
                }
                if let Some(e) = limit_from {
                    expr(e, &format!("{path}/limitFrom"), out);
                }
                if let Some(e) = offset {
                    expr(e, &format!("{path}/offset"), out);
                }
                assignments(values, &format!("{path}/values"), out);
            }
            Step::Branch {
                condition,
                then,
                otherwise,
                ..
            } => {
                condition_exprs(condition, &format!("{path}/condition"), out);
                steps(&format!("{path}/then"), then, out);
                steps(&format!("{path}/otherwise"), otherwise, out);
            }
            Step::ForEach {
                list, steps: body, ..
            } => {
                expr(list, &format!("{path}/list"), out);
                steps(&format!("{path}/steps"), body, out);
            }
            Step::Code { code, .. } => {
                if !code.trim().is_empty()
                    && let Err(e) = syn::parse_str::<syn::Block>(&format!("{{ {code} }}"))
                {
                    push(out, format!("{path}/code"), e);
                }
            }
        }
    }
}

fn assignments(list: &[sb_model::Assignment], base: &str, out: &mut Vec<Diagnostic>) {
    for (i, a) in list.iter().enumerate() {
        expr(&a.value, &format!("{base}/{i}/value"), out);
    }
}

/// The optional `app_url` of a passkey step.
fn app_url_expr(app_url: &Option<Expr>, path: &str, out: &mut Vec<Diagnostic>) {
    if let Some(app_url) = app_url {
        expr(app_url, &format!("{path}/appUrl"), out);
    }
}

fn expr(e: &Expr, path: &str, out: &mut Vec<Diagnostic>) {
    match e {
        Expr::Raw { code } => parse_expr(code, path, out),
        Expr::Call { args, .. } => {
            for (i, a) in args.iter().enumerate() {
                expr(a, &format!("{path}/args/{i}"), out);
            }
        }
        _ => {}
    }
}

fn condition_exprs(c: &Condition, path: &str, out: &mut Vec<Diagnostic>) {
    match c {
        Condition::Raw { code } => parse_expr(code, path, out),
        Condition::Compare { left, right, .. } => {
            expr(left, &format!("{path}/left"), out);
            expr(right, &format!("{path}/right"), out);
        }
        Condition::And { conditions } | Condition::Or { conditions } => {
            for (i, c) in conditions.iter().enumerate() {
                condition_exprs(c, &format!("{path}/conditions/{i}"), out);
            }
        }
        Condition::Not { condition } => {
            condition_exprs(condition, &format!("{path}/condition"), out)
        }
        Condition::Present { value }
        | Condition::Absent { value }
        | Condition::Empty { value }
        | Condition::Test { value } => expr(value, &format!("{path}/value"), out),
        Condition::In { value, values } => {
            expr(value, &format!("{path}/value"), out);
            for (i, v) in values.iter().enumerate() {
                expr(v, &format!("{path}/values/{i}"), out);
            }
        }
    }
}

fn parse_expr(code: &str, path: &str, out: &mut Vec<Diagnostic>) {
    // Empty code is reported by the model validator.
    if !code.trim().is_empty()
        && let Err(e) = syn::parse_str::<syn::Expr>(code)
    {
        push(out, path.to_string(), e);
    }
}

fn push(out: &mut Vec<Diagnostic>, path: String, e: syn::Error) {
    out.push(Diagnostic {
        severity: Severity::Error,
        code: "E_RUST_SYNTAX".into(),
        message: format!("invalid Rust: {e}"),
        path,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_model::sample;

    #[test]
    fn sample_has_valid_rust() {
        assert_eq!(check(&sample::orders()), vec![]);
    }

    #[test]
    fn reports_syntax_errors_with_their_location() {
        let mut p = sample::orders();
        let Step::Let { value, .. } = &mut p.services[0].rpcs[0].steps[0] else {
            panic!("sample layout changed")
        };
        *value = Expr::Raw {
            code: "eda_runtime::new_id(".into(),
        };
        p.services[1].consumers[0].steps.push(Step::Code {
            id: "x".into(),
            code: "let a = ;".into(),
        });
        let d = check(&p);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].path, "/services/0/rpcs/0/steps/0/value");
        assert_eq!(d[1].path, "/services/1/consumers/0/steps/2/code");
        assert!(d.iter().all(|d| d.code == "E_RUST_SYNTAX"));
    }
}
