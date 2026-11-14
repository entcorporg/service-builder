//! The rules of the `httpCall` step (outbound HTTP): what keeps a request from going where it should not, from sending
//! a secret in the clear, from being replayed unsafely or from holding the transaction of the flow for too long.

use std::collections::HashSet;

use super::{Checker, Scope, TypeRef, is_env_name, parse_template, resolve_path};
use crate::project::*;
use crate::typing::TemplatePart;

/// The longest wait for one attempt, in seconds.
pub const MAX_TIMEOUT_SECS: u32 = 30;
/// The most extra attempts.
pub const MAX_RETRIES: u8 = 5;
/// The longest time the flow may wait on one call, all attempts together (the transaction is open meanwhile).
pub const MAX_BUDGET_SECS: u32 = 60;

/// Headers the runtime or the protocol owns: the step must not set them, or a request could be smuggled or confused.
const FORBIDDEN_HEADERS: [&str; 9] = [
    "host",
    "content-length",
    "content-type",
    "transfer-encoding",
    "connection",
    "authorization",
    "idempotency-key",
    "proxy-authorization",
    "upgrade",
];

/// An HTTP header name: the characters of a `token` (RFC 9110).
pub fn is_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

impl Checker<'_> {
    /// A value sent in a request: a plain scalar (or an optional one, then left out when absent). Raw Rust is not checked.
    fn http_value(&mut self, scope: &Scope, value: &Expr, path: &str) {
        let Some(ty) = self.check_expr(scope, value, None, path) else {
            return;
        };
        let scalar = !ty.repeated
            && !matches!(
                ty.ty,
                FieldType::Bytes | FieldType::Timestamp | FieldType::Message { .. }
            );
        if !scalar {
            self.error(
                "E_HTTP_CALL_VALUE",
                path,
                format!("a request carries text, numbers and booleans, not a {ty}"),
            );
        }
    }

    /// A list of `name = value` (query, headers, body): names set once, values sendable.
    fn http_pairs(
        &mut self,
        scope: &Scope,
        list: &[Assignment],
        base: &str,
        name_ok: impl Fn(&str) -> Option<String>,
        ci: bool,
    ) {
        let mut seen = HashSet::new();
        for (i, a) in list.iter().enumerate() {
            let at = format!("{base}/{i}");
            if let Some(why) = name_ok(&a.field) {
                self.error("E_HTTP_CALL_NAME", format!("{at}/field"), why);
            }
            let key = if ci {
                a.field.to_ascii_lowercase()
            } else {
                a.field.clone()
            };
            if !seen.insert(key) {
                self.error(
                    "E_HTTP_CALL_DUPLICATE",
                    &at,
                    format!("`{}` is set twice", a.field),
                );
            }
            self.http_value(scope, &a.value, &format!("{at}/value"));
        }
    }

    fn http_env(&mut self, name: &Option<String>, field: &str, path: &str, why: &str) {
        match name {
            Some(n) if is_env_name(n) => {}
            Some(n) => self.error(
                "E_HTTP_CALL_ENV",
                format!("{path}/{field}"),
                format!("`{n}` is not an environment variable name (e.g. `STRIPE_API_KEY`)"),
            ),
            None => self.error("E_HTTP_CALL_AUTH", format!("{path}/{field}"), why),
        }
    }

    pub(super) fn http_call(&mut self, scope: &mut Scope, path: &str, step: &Step) {
        let Step::HttpCall {
            method,
            base_url_env,
            path: url_path,
            query,
            headers,
            auth,
            auth_env,
            auth_password_env,
            auth_header,
            body,
            timeout_secs,
            retries,
            idempotency_key,
            response,
            bind,
            status_bind,
            on_error,
            error_status,
            error_message,
            ..
        } = step
        else {
            return;
        };
        if !is_env_name(base_url_env) {
            self.error(
                "E_HTTP_CALL_BASE_ENV",
                format!("{path}/baseUrlEnv"),
                format!(
                    "`{base_url_env}` is not an environment variable name (e.g. `STRIPE_API_URL`): the address of the service is \
                     read from the environment, never from the flow"
                ),
            );
        }
        self.http_path(scope, url_path, &format!("{path}/path"));
        self.http_pairs(
            scope,
            query,
            &format!("{path}/query"),
            |n| {
                (n.is_empty() || n.chars().any(|c| c.is_control())).then(|| {
                    "a query parameter needs a name, with no control character".to_string()
                })
            },
            false,
        );
        self.http_pairs(
            scope,
            headers,
            &format!("{path}/headers"),
            |n| {
                if !is_header_name(n) {
                    Some(format!("`{n}` is not a header name"))
                } else if FORBIDDEN_HEADERS.contains(&n.to_ascii_lowercase().as_str()) {
                    Some(format!(
                        "`{n}` is set by the step itself (auth, idempotency key, body format): use its own field"
                    ))
                } else {
                    None
                }
            },
            true,
        );
        // Authentication: the fields that go with the kind, and no other.
        match auth {
            None => {
                for (name, set) in [
                    ("authEnv", auth_env.is_some()),
                    ("authPasswordEnv", auth_password_env.is_some()),
                    ("authHeader", auth_header.is_some()),
                ] {
                    if set {
                        self.error(
                            "E_HTTP_CALL_AUTH",
                            format!("{path}/{name}"),
                            "this is set but the call has no authentication: pick one (bearer, basic, API key) or clear it",
                        );
                    }
                }
            }
            Some(kind) => {
                let what = match kind {
                    HttpAuthKind::Bearer => "the environment variable that holds the token",
                    HttpAuthKind::Basic => "the environment variable that holds the user name",
                    HttpAuthKind::ApiKey => "the environment variable that holds the key",
                };
                self.http_env(auth_env, "authEnv", path, what);
                match kind {
                    HttpAuthKind::Basic => self.http_env(
                        auth_password_env,
                        "authPasswordEnv",
                        path,
                        "name the environment variable that holds the password",
                    ),
                    _ if auth_password_env.is_some() => self.error(
                        "E_HTTP_CALL_AUTH",
                        format!("{path}/authPasswordEnv"),
                        "only basic authentication has a password",
                    ),
                    _ => {}
                }
                match (kind, auth_header) {
                    (HttpAuthKind::ApiKey, Some(h)) => {
                        if !is_header_name(h)
                            || FORBIDDEN_HEADERS.contains(&h.to_ascii_lowercase().as_str())
                        {
                            self.error(
                                "E_HTTP_CALL_AUTH",
                                format!("{path}/authHeader"),
                                format!("`{h}` cannot carry an API key: use a header such as `X-Api-Key`"),
                            );
                        } else if headers.iter().any(|a| a.field.eq_ignore_ascii_case(h)) {
                            self.error(
                                "E_HTTP_CALL_DUPLICATE",
                                format!("{path}/authHeader"),
                                format!("`{h}` is also in the headers"),
                            );
                        }
                    }
                    (HttpAuthKind::ApiKey, None) => self.error(
                        "E_HTTP_CALL_AUTH",
                        format!("{path}/authHeader"),
                        "name the header that carries the key",
                    ),
                    (_, Some(_)) => self.error(
                        "E_HTTP_CALL_AUTH",
                        format!("{path}/authHeader"),
                        "only an API key goes in a header of your choice",
                    ),
                    _ => {}
                }
            }
        }
        // Body.
        if !body.is_empty() && *method == HttpMethod::Get {
            self.error(
                "E_HTTP_CALL_BODY",
                format!("{path}/body"),
                "a GET has no body: use the query, or another method",
            );
        }
        self.http_pairs(
            scope,
            body,
            &format!("{path}/body"),
            |n| {
                n.is_empty()
                    .then(|| "a body field needs a name".to_string())
            },
            false,
        );
        // Time: the call runs inside the transaction of the flow.
        if !(1..=MAX_TIMEOUT_SECS).contains(timeout_secs) {
            self.error(
                "E_HTTP_CALL_TIMEOUT",
                format!("{path}/timeoutSecs"),
                format!("the wait is between 1 and {MAX_TIMEOUT_SECS} seconds: the transaction stays open meanwhile"),
            );
        }
        if *retries > MAX_RETRIES {
            self.error(
                "E_HTTP_CALL_RETRIES",
                format!("{path}/retries"),
                format!("{MAX_RETRIES} extra attempts at most"),
            );
        } else if *retries > 0 && !method.is_idempotent() && idempotency_key.is_none() {
            self.error(
                "E_HTTP_CALL_RETRY_UNSAFE",
                format!("{path}/retries"),
                format!(
                    "a {} can be done twice by a retry: give an idempotency key, so that the provider does it once",
                    method.as_str()
                ),
            );
        }
        if (1..=MAX_TIMEOUT_SECS).contains(timeout_secs)
            && *retries <= MAX_RETRIES
            && timeout_secs * (u32::from(*retries) + 1) > MAX_BUDGET_SECS
        {
            self.error(
                "E_HTTP_CALL_BUDGET",
                format!("{path}/retries"),
                format!(
                    "{retries} retries of {timeout_secs} s hold the transaction for up to {} s: the limit is {MAX_BUDGET_SECS} s",
                    timeout_secs * (u32::from(*retries) + 1)
                ),
            );
        }
        match idempotency_key {
            Some(key) => {
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, key, Some(&text), &format!("{path}/idempotencyKey"));
                if method.is_idempotent() {
                    self.warn(
                        "W_HTTP_CALL_IDEMPOTENCY_USELESS",
                        format!("{path}/idempotencyKey"),
                        format!(
                            "a {} already is idempotent: the key is not needed",
                            method.as_str()
                        ),
                    );
                }
            }
            None => {}
        }
        // The answer.
        let schema_ok = match response {
            Some(name) => match self.project.message(name) {
                Some(m) if m.kind == MessageKind::Api => true,
                Some(_) => {
                    self.error(
                        "E_HTTP_CALL_RESPONSE",
                        format!("{path}/response"),
                        format!("`{name}` is not an API schema: the answer is JSON, read into one"),
                    );
                    false
                }
                None => {
                    self.error(
                        "E_UNKNOWN_MESSAGE",
                        format!("{path}/response"),
                        format!("unknown message `{name}`"),
                    );
                    false
                }
            },
            None => false,
        };
        if bind.is_some() != response.is_some() {
            self.error(
                "E_HTTP_CALL_BIND",
                format!("{path}/bind"),
                "`response` (the schema of the answer) and `bind` (the variable) go together",
            );
        }
        if let (Some(b), Some(r)) = (bind, status_bind)
            && b == r
        {
            self.error(
                "E_HTTP_CALL_BIND",
                format!("{path}/statusBind"),
                "the answer and the status need two variables",
            );
        }
        if let Some(b) = bind {
            self.check_var_name(b, &format!("{path}/bind"));
        }
        if let Some(b) = status_bind {
            self.check_var_name(b, &format!("{path}/statusBind"));
        }
        match on_error {
            HttpOnError::Fail => {
                if !(400..=599).contains(error_status) {
                    self.error(
                        "E_HTTP_CALL_STATUS",
                        format!("{path}/errorStatus"),
                        format!("`{error_status}` is not an error status (400 to 599)"),
                    );
                }
                if error_message.trim().is_empty() {
                    self.error(
                        "E_HTTP_CALL_MESSAGE",
                        format!("{path}/errorMessage"),
                        "say what the client is told (a 5xx answers a generic text and logs this one)",
                    );
                }
            }
            HttpOnError::Continue => {
                if status_bind.is_none() {
                    self.warn(
                        "W_HTTP_CALL_BLIND",
                        format!("{path}/statusBind"),
                        "the flow goes on whatever happens and never reads the status: bind it and branch on it, or stop on failure",
                    );
                }
            }
        }
        if let (Some(b), true, Some(r)) = (bind, schema_ok, response) {
            scope.bind(b.clone(), TypeRef::message(r));
        }
        if let Some(b) = status_bind {
            scope.bind(b.clone(), TypeRef::scalar(FieldType::Int32));
        }
    }

    /// `/v1/customers/{input.customer_id}`: fixed text and values of the flow; a value is encoded as one segment, so it
    /// cannot add a segment, a query or a fragment. The fixed text cannot either.
    fn http_path(&mut self, scope: &Scope, template: &str, path: &str) {
        let parts = match parse_template(template) {
            Ok(p) => p,
            Err(e) => {
                self.error("E_HTTP_CALL_PATH", path, e);
                return;
            }
        };
        if !template.starts_with('/') {
            self.error(
                "E_HTTP_CALL_PATH",
                path,
                "the path starts with `/` (it goes after the base address)",
            );
        }
        for part in parts {
            match part {
                TemplatePart::Text(t) => {
                    if t.chars()
                        .any(|c| c.is_whitespace() || c.is_control() || "?#\\".contains(c))
                    {
                        self.error(
                            "E_HTTP_CALL_PATH",
                            path,
                            "the path has no space, `?`, `#` or `\\`: the query is a list of its own",
                        );
                    }
                    if t.split('/').any(|s| s == "." || s == "..") {
                        self.error("E_HTTP_CALL_PATH", path, "no `.` or `..` segment in a path");
                    }
                }
                TemplatePart::Path(pp) => {
                    match resolve_path(self.project, scope, &pp) {
                        Err(e) => self.error("E_UNKNOWN_PATH", path, e),
                        Ok(rp) => {
                            if !rp.ty.is_simple_scalar() {
                                self.error(
                                "E_HTTP_CALL_VALUE",
                                path,
                                format!("`{pp}` is a {}: a path takes a plain text, number or boolean", rp.ty),
                            );
                            }
                        }
                    }
                }
            }
        }
    }

    /// A call that follows a write of the flow holds the row locks of that write while it waits.
    pub(super) fn http_call_after_write(&mut self, path: &str, before: &[Step], step: &Step) {
        if !matches!(step, Step::HttpCall { .. }) {
            return;
        }
        let mut wrote = false;
        for s in before {
            s.walk(&mut |x| {
                wrote |= matches!(
                    x,
                    Step::Sql {
                        op: SqlOp::Insert | SqlOp::Update | SqlOp::Delete,
                        ..
                    }
                );
            });
        }
        if wrote {
            self.warn(
                "W_HTTP_CALL_AFTER_WRITE",
                path,
                "this call waits inside the transaction after a write: the rows it wrote stay locked until the answer comes. Call first, write after",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::project::*;
    use crate::sample;
    use crate::validate::{Severity, validate};

    fn showcase() -> Project {
        sample::showcase()
    }

    /// Edits the first step of the Stripe route, and returns the diagnostics `(severity, code)`.
    fn with_stripe(edit: impl FnOnce(&mut Step)) -> Vec<(Severity, String)> {
        let mut p = showcase();
        let route = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "create_payment_intent")
            .unwrap();
        edit(&mut route.steps[0]);
        validate(&p)
            .into_iter()
            .map(|d| (d.severity, d.code))
            .collect()
    }

    macro_rules! fields {
        ($step:expr, $($name:ident),+ => $body:block) => {
            let Step::HttpCall { $($name),+, .. } = $step else { panic!("not an httpCall") };
            $body
        };
    }

    fn has(found: &[(Severity, String)], code: &str) -> bool {
        found.iter().any(|(_, c)| c == code)
    }

    #[test]
    fn the_stripe_and_twilio_samples_are_valid() {
        assert_eq!(with_stripe(|_| {}), vec![]);
    }

    #[test]
    fn the_address_comes_from_the_environment_and_the_path_cannot_leave_its_place() {
        let found = with_stripe(|s| {
            fields!(s, base_url_env, path => {
                *base_url_env = "https://evil.example".into();
                *path = "v1/x".into();
            });
        });
        assert!(has(&found, "E_HTTP_CALL_BASE_ENV"));
        assert!(has(&found, "E_HTTP_CALL_PATH"));
        for bad in [
            "/a?x=1",
            "/a#b",
            "/a/../b",
            "/a b",
            "/a\\b",
            "/{input.nope}",
        ] {
            let found = with_stripe(|s| {
                fields!(s, path => { *path = bad.into(); });
            });
            assert!(
                has(&found, "E_HTTP_CALL_PATH") || has(&found, "E_UNKNOWN_PATH"),
                "{bad}: {found:?}"
            );
        }
        // A value of the flow is fine in a path.
        let found = with_stripe(|s| {
            fields!(s, path => { *path = "/v1/payment_intents/{input.order_id}".into(); });
        });
        assert_eq!(found, vec![]);
    }

    #[test]
    fn authentication_goes_with_the_fields_of_its_kind() {
        let found = with_stripe(|s| {
            fields!(s, auth_env => { *auth_env = Some("not-an-env".into()); });
        });
        assert!(has(&found, "E_HTTP_CALL_ENV"));
        let found = with_stripe(|s| {
            fields!(s, auth_password_env => { *auth_password_env = Some("X_PASS".into()); });
        });
        assert!(has(&found, "E_HTTP_CALL_AUTH"), "bearer has no password");
        let found = with_stripe(|s| {
            fields!(s, auth, auth_header => {
                *auth = Some(HttpAuthKind::ApiKey);
                *auth_header = None;
            });
        });
        assert!(
            has(&found, "E_HTTP_CALL_AUTH"),
            "an API key names its header"
        );
        let found = with_stripe(|s| {
            fields!(s, auth, auth_header => {
                *auth = Some(HttpAuthKind::ApiKey);
                *auth_header = Some("Authorization".into());
            });
        });
        assert!(has(&found, "E_HTTP_CALL_AUTH"));
        let found = with_stripe(|s| {
            fields!(s, auth => { *auth = None; });
        });
        assert!(has(&found, "E_HTTP_CALL_AUTH"), "a secret with no kind");
        let found = with_stripe(|s| {
            fields!(s, auth, auth_header => {
                *auth = Some(HttpAuthKind::ApiKey);
                *auth_header = Some("X-Api-Key".into());
            });
        });
        assert_eq!(found, vec![]);
    }

    #[test]
    fn headers_the_step_owns_and_duplicates_are_refused() {
        for name in [
            "Host",
            "content-length",
            "Authorization",
            "Idempotency-Key",
            "Content-Type",
        ] {
            let found = with_stripe(|s| {
                fields!(s, headers => {
                    headers.push(Assignment { field: name.into(), value: Expr::Literal { value: "x".into() } });
                });
            });
            assert!(has(&found, "E_HTTP_CALL_NAME"), "{name}");
        }
        let found = with_stripe(|s| {
            fields!(s, headers => {
                let h = Assignment { field: "X-Trace".into(), value: Expr::Literal { value: "x".into() } };
                headers.push(h.clone());
                headers.push(Assignment { field: "x-trace".into(), ..h });
            });
        });
        assert!(has(&found, "E_HTTP_CALL_DUPLICATE"));
        let found = with_stripe(|s| {
            fields!(s, headers => {
                headers.push(Assignment { field: "bad name".into(), value: Expr::Literal { value: "x".into() } });
            });
        });
        assert!(has(&found, "E_HTTP_CALL_NAME"));
    }

    #[test]
    fn values_are_scalars_and_a_get_has_no_body() {
        let found = with_stripe(|s| {
            fields!(s, body => { body[0].value = Expr::Path { path: "input".into() }; });
        });
        assert!(has(&found, "E_HTTP_CALL_VALUE"));
        let found = with_stripe(|s| {
            fields!(s, body => { body[0].value = Expr::Path { path: "input.nope".into() }; });
        });
        assert!(has(&found, "E_UNKNOWN_PATH"));
        let found = with_stripe(|s| {
            fields!(s, method, idempotency_key => {
                *method = HttpMethod::Get;
                *idempotency_key = None;
            });
        });
        assert!(has(&found, "E_HTTP_CALL_BODY"));
    }

    #[test]
    fn time_is_bounded_because_the_transaction_stays_open() {
        for timeout in [0, 31] {
            let found = with_stripe(|s| {
                fields!(s, timeout_secs => { *timeout_secs = timeout; });
            });
            assert!(has(&found, "E_HTTP_CALL_TIMEOUT"), "{timeout}");
        }
        let found = with_stripe(|s| {
            fields!(s, retries => { *retries = 6; });
        });
        assert!(has(&found, "E_HTTP_CALL_RETRIES"));
        let found = with_stripe(|s| {
            fields!(s, timeout_secs, retries => {
                *timeout_secs = 30;
                *retries = 2;
            });
        });
        assert!(has(&found, "E_HTTP_CALL_BUDGET"), "{found:?}");
    }

    #[test]
    fn a_post_is_retried_only_with_an_idempotency_key() {
        let found = with_stripe(|s| {
            fields!(s, idempotency_key => { *idempotency_key = None; });
        });
        assert!(has(&found, "E_HTTP_CALL_RETRY_UNSAFE"));
        // A GET needs none, and a key on it is pointless.
        let found = with_stripe(|s| {
            fields!(s, method, body, retries => {
                *method = HttpMethod::Get;
                body.clear();
                *retries = 2;
            });
        });
        assert!(!has(&found, "E_HTTP_CALL_RETRY_UNSAFE"));
        assert!(has(&found, "W_HTTP_CALL_IDEMPOTENCY_USELESS"));
    }

    #[test]
    fn the_answer_is_an_api_schema_bound_to_a_variable() {
        let found = with_stripe(|s| {
            fields!(s, response => { *response = Some("Order".into()); });
        });
        assert!(
            has(&found, "E_HTTP_CALL_RESPONSE") || has(&found, "E_UNKNOWN_MESSAGE"),
            "{found:?}"
        );
        let found = with_stripe(|s| {
            fields!(s, response => { *response = Some("NoSuchSchema".into()); });
        });
        assert!(has(&found, "E_UNKNOWN_MESSAGE"));
        let found = with_stripe(|s| {
            fields!(s, bind => { *bind = None; });
        });
        assert!(has(&found, "E_HTTP_CALL_BIND"));
        let found = with_stripe(|s| {
            fields!(s, status_bind, bind => { *status_bind = bind.clone(); });
        });
        assert!(has(&found, "E_HTTP_CALL_BIND"));
        let found = with_stripe(|s| {
            fields!(s, bind => { *bind = Some("Bad Name".into()); });
        });
        assert!(has(&found, "E_VAR_NAME"));
    }

    #[test]
    fn what_a_failure_does_is_chosen() {
        let found = with_stripe(|s| {
            fields!(s, error_status, error_message => {
                *error_status = 200;
                *error_message = " ".into();
            });
        });
        assert!(has(&found, "E_HTTP_CALL_STATUS") && has(&found, "E_HTTP_CALL_MESSAGE"));
        // Going on without reading the status is a warning, not an error.
        let found = with_stripe(|s| {
            fields!(s, on_error => { *on_error = HttpOnError::Continue; });
        });
        assert!(has(&found, "W_HTTP_CALL_BLIND"));
        assert!(!found.iter().any(|(sev, _)| *sev == Severity::Error));
    }

    #[test]
    fn the_status_and_the_answer_can_be_read_by_the_steps_after() {
        // The Twilio route reads `sms_status` and `message.sid`: it is valid, and without the binding it is not.
        let mut p = showcase();
        let route = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "send_sms")
            .unwrap();
        let Step::HttpCall { status_bind, .. } = &mut route.steps[0] else {
            panic!()
        };
        *status_bind = None;
        assert!(
            validate(&p)
                .iter()
                .any(|d| d.code == "E_UNKNOWN_PATH" && d.path.ends_with("/steps/1/condition/left"))
                || validate(&p).iter().any(|d| d.code == "E_UNKNOWN_PATH"),
            "{:?}",
            validate(&p)
        );
    }

    #[test]
    fn a_call_after_a_write_warns_that_the_locks_are_held() {
        let mut p = showcase();
        let sql = p
            .services
            .iter()
            .flat_map(|s| s.routes.iter().chain(std::iter::empty()))
            .find_map(|r| {
                r.steps
                    .iter()
                    .find(|s| {
                        matches!(
                            s,
                            Step::Sql {
                                op: SqlOp::Insert,
                                ..
                            }
                        )
                    })
                    .cloned()
            });
        let sql = sql.expect("an insert in some route of the showcase");
        let route = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "create_payment_intent")
            .unwrap();
        route.steps.insert(0, sql);
        assert!(
            validate(&p)
                .iter()
                .any(|d| d.code == "W_HTTP_CALL_AFTER_WRITE")
        );
    }
}
