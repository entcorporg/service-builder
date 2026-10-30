//! Syntax check of the raw Rust typed in the editor (expressions, conditions, code
//! blocks). The model validator cannot do it, and a syntax error would otherwise only
//! surface when compiling the generated workspace.

use sb_model::{Condition, Diagnostic, Expr, Project, Severity, Step};

pub fn check(project: &Project) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (si, s) in project.services.iter().enumerate() {
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

fn steps(base: &str, list: &[Step], out: &mut Vec<Diagnostic>) {
    for (i, step) in list.iter().enumerate() {
        let path = format!("{base}/{i}");
        match step {
            Step::Log { .. } => {}
            Step::Let { value, .. } => expr(value, &format!("{path}/value"), out),
            Step::Publish { payload, .. } => assignments(payload, &format!("{path}/payload"), out),
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
            Step::HashPassword { input, .. } | Step::Sha256 { input, .. } => {
                expr(input, &format!("{path}/input"), out)
            }
            Step::VerifyPassword {
                password, stored, ..
            } => {
                expr(password, &format!("{path}/password"), out);
                expr(stored, &format!("{path}/stored"), out);
            }
            Step::RandomToken { .. } => {}
            Step::Fail { retry_after, .. } => {
                if let Some(wait) = retry_after {
                    expr(wait, &format!("{path}/retryAfter"), out);
                }
            }
            Step::Sql {
                conditions, values, ..
            } => {
                for (j, c) in conditions.iter().enumerate() {
                    expr(&c.value, &format!("{path}/where/{j}/value"), out);
                }
                assignments(values, &format!("{path}/values"), out);
            }
            Step::Branch {
                condition,
                then,
                otherwise,
                ..
            } => {
                match condition {
                    Condition::Raw { code } => parse_expr(code, &format!("{path}/condition"), out),
                    Condition::Compare { left, right, .. } => {
                        expr(left, &format!("{path}/condition/left"), out);
                        expr(right, &format!("{path}/condition/right"), out);
                    }
                }
                steps(&format!("{path}/then"), then, out);
                steps(&format!("{path}/otherwise"), otherwise, out);
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

fn expr(e: &Expr, path: &str, out: &mut Vec<Diagnostic>) {
    if let Expr::Raw { code } = e {
        parse_expr(code, path, out);
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
