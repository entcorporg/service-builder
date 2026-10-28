//! Project validation. Everything the generator relies on is checked here, so a project
//! without errors is guaranteed to produce code that type-checks (barring raw Rust blocks).
//!
//! Diagnostics point at the faulty element with a JSON pointer into the project
//! document, e.g. `/services/0/rpcs/1/steps/2/then/0`.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use crate::names::{is_pascal_ident, is_snake_ident, is_topic_name, snake, topic_const};
use crate::project::*;
use crate::typing::{
    Scope, TemplatePart, TypeRef, coerce_literal, infer_literal, parse_template, resolve_path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable machine-readable code, e.g. `E_UNKNOWN_TOPIC`.
    pub code: String,
    pub message: String,
    /// JSON pointer to the faulty element.
    pub path: String,
}

pub fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(|d| d.severity == Severity::Error)
}

pub fn validate(project: &Project) -> Vec<Diagnostic> {
    let mut c = Checker {
        project,
        out: Vec::new(),
    };
    c.project();
    c.messages();
    c.topics();
    c.services();
    c.graph_warnings();
    c.out
}

struct Checker<'a> {
    project: &'a Project,
    out: Vec<Diagnostic>,
}

#[derive(Clone, Copy)]
enum Handler<'a> {
    Rpc { response: &'a str },
    Consumer,
}

struct Env<'a> {
    service: &'a Service,
    handler: Handler<'a>,
}

const RESERVED_VARS: [&str; 2] = ["input", "ctx"];

impl<'a> Checker<'a> {
    fn error(&mut self, code: &str, path: impl Into<String>, message: impl Into<String>) {
        self.push(Severity::Error, code, path, message);
    }

    fn warn(&mut self, code: &str, path: impl Into<String>, message: impl Into<String>) {
        self.push(Severity::Warning, code, path, message);
    }

    fn push(
        &mut self,
        severity: Severity,
        code: &str,
        path: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.out.push(Diagnostic {
            severity,
            code: code.to_string(),
            message: message.into(),
            path: path.into(),
        });
    }

    fn project(&mut self) {
        let p = self.project;
        let pkg = snake(&p.name);
        let valid_pkg = pkg.starts_with(|c: char| c.is_ascii_lowercase())
            && pkg
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !valid_pkg {
            self.error(
                "E_PROJECT_NAME",
                "/name",
                "the project name must start with a letter and contain only letters, digits, spaces, `-` or `_`",
            );
        }
        if p.broker.bootstrap_servers.trim().is_empty() {
            self.error(
                "E_BROKER",
                "/broker/bootstrapServers",
                "the Kafka bootstrap servers are empty",
            );
        }
    }

    // ---- messages --------------------------------------------------------------------

    fn messages(&mut self) {
        let p = self.project;
        // Messages and services share the protobuf package namespace.
        let mut names: BTreeSet<&str> = BTreeSet::new();
        for (i, s) in p.services.iter().enumerate() {
            if p.messages.iter().any(|m| m.name == s.name) {
                self.error(
                    "E_NAME_CLASH",
                    format!("/services/{i}/name"),
                    format!("`{}` is used by both a message and a service", s.name),
                );
            }
        }
        for (i, m) in p.messages.iter().enumerate() {
            let base = format!("/messages/{i}");
            if !is_pascal_ident(&m.name) {
                self.error(
                    "E_MESSAGE_NAME",
                    format!("{base}/name"),
                    format!(
                        "message name `{}` must be UpperCamelCase (e.g. `{}`)",
                        m.name,
                        crate::names::pascal(&m.name)
                    ),
                );
            }
            if !names.insert(&m.name) {
                self.error(
                    "E_DUPLICATE_MESSAGE",
                    format!("{base}/name"),
                    format!("message `{}` is defined twice", m.name),
                );
            }
            let mut fields = HashSet::new();
            let mut tags = HashSet::new();
            for (j, f) in m.fields.iter().enumerate() {
                let fp = format!("{base}/fields/{j}");
                if !is_snake_ident(&f.name) {
                    self.error(
                        "E_FIELD_NAME",
                        format!("{fp}/name"),
                        format!(
                            "field name `{}` must be snake_case and not a Rust keyword (e.g. `{}`)",
                            f.name,
                            snake(&f.name)
                        ),
                    );
                }
                if !fields.insert(f.name.as_str()) {
                    self.error(
                        "E_DUPLICATE_FIELD",
                        format!("{fp}/name"),
                        format!("field `{}` is defined twice in `{}`", f.name, m.name),
                    );
                }
                if let FieldType::Message { name } = &f.ty
                    && p.message(name).is_none()
                {
                    self.error(
                        "E_UNKNOWN_MESSAGE",
                        format!("{fp}/type"),
                        format!("field `{}` references unknown message `{name}`", f.name),
                    );
                }
                if f.repeated && f.optional {
                    self.error(
                        "E_REPEATED_OPTIONAL",
                        &fp,
                        format!("field `{}` cannot be both repeated and optional", f.name),
                    );
                }
                if let Some(tag) = f.tag {
                    let reserved = (19_000..=19_999).contains(&tag);
                    if tag == 0 || tag > 536_870_911 || reserved {
                        self.error(
                            "E_FIELD_TAG",
                            format!("{fp}/tag"),
                            format!(
                                "field number {tag} is not allowed (1 to 536870911, except 19000-19999)"
                            ),
                        );
                    } else if !tags.insert(tag) {
                        self.error(
                            "E_DUPLICATE_TAG",
                            format!("{fp}/tag"),
                            format!("field number {tag} is used twice in `{}`", m.name),
                        );
                    }
                }
            }
        }
        self.message_cycles();
    }

    /// prost cannot represent a message that contains itself without boxing.
    fn message_cycles(&mut self) {
        let p = self.project;
        let edges: BTreeMap<&str, Vec<&str>> = p
            .messages
            .iter()
            .map(|m| {
                let targets = m
                    .fields
                    .iter()
                    .filter(|f| !f.repeated)
                    .filter_map(|f| match &f.ty {
                        FieldType::Message { name } => Some(name.as_str()),
                        _ => None,
                    })
                    .collect();
                (m.name.as_str(), targets)
            })
            .collect();

        fn reaches<'x>(
            edges: &BTreeMap<&'x str, Vec<&'x str>>,
            from: &'x str,
            target: &str,
            seen: &mut HashSet<&'x str>,
        ) -> bool {
            for &next in edges.get(from).into_iter().flatten() {
                if next == target {
                    return true;
                }
                if seen.insert(next) && reaches(edges, next, target, seen) {
                    return true;
                }
            }
            false
        }

        for (i, m) in p.messages.iter().enumerate() {
            let mut seen = HashSet::new();
            if reaches(&edges, &m.name, &m.name, &mut seen) {
                self.error(
                    "E_MESSAGE_CYCLE",
                    format!("/messages/{i}"),
                    format!(
                        "message `{}` contains itself; make the recursive field `repeated`",
                        m.name
                    ),
                );
            }
        }
    }

    // ---- topics ----------------------------------------------------------------------

    fn topics(&mut self) {
        let p = self.project;
        let mut names = HashSet::new();
        let mut consts: BTreeMap<String, &str> = BTreeMap::new();
        for (i, t) in p.topics.iter().enumerate() {
            let base = format!("/topics/{i}");
            if !is_topic_name(&t.name) {
                self.error(
                    "E_TOPIC_NAME",
                    format!("{base}/name"),
                    format!(
                        "`{}` is not a valid Kafka topic name (letters, digits, `.`, `_`, `-`)",
                        t.name
                    ),
                );
            }
            if !names.insert(t.name.as_str()) {
                self.error(
                    "E_DUPLICATE_TOPIC",
                    format!("{base}/name"),
                    format!("topic `{}` is defined twice", t.name),
                );
            }
            if let Some(other) = consts.insert(topic_const(&t.name), &t.name)
                && other != t.name
            {
                self.error(
                    "E_TOPIC_CONST_CLASH",
                    format!("{base}/name"),
                    format!(
                        "topics `{other}` and `{}` map to the same Rust constant `{}`",
                        t.name,
                        topic_const(&t.name)
                    ),
                );
            }
            let Some(msg) = p.message(&t.message) else {
                self.error(
                    "E_UNKNOWN_MESSAGE",
                    format!("{base}/message"),
                    format!("topic `{}` carries unknown message `{}`", t.name, t.message),
                );
                continue;
            };
            if t.partitions < 1 {
                self.error(
                    "E_PARTITIONS",
                    format!("{base}/partitions"),
                    "at least 1 partition",
                );
            }
            if t.replication_factor < 1 {
                self.error(
                    "E_REPLICATION",
                    format!("{base}/replicationFactor"),
                    "replication factor must be at least 1",
                );
            }
            if let Some(key) = &t.key_field {
                match msg.field(key) {
                    None => self.error(
                        "E_KEY_FIELD",
                        format!("{base}/keyField"),
                        format!("message `{}` has no field `{key}`", msg.name),
                    ),
                    Some(f) => {
                        let ty = TypeRef::of_field(f);
                        let ok = ty.is_simple_scalar();
                        if !ok {
                            self.error(
                                "E_KEY_FIELD",
                                format!("{base}/keyField"),
                                format!("key field `{key}` must be a plain scalar, found {ty}"),
                            );
                        }
                    }
                }
            }
        }
    }

    // ---- services --------------------------------------------------------------------

    fn services(&mut self) {
        let p = self.project;
        let mut names = HashSet::new();
        let mut ports: BTreeMap<u16, &str> = BTreeMap::new();
        for (i, s) in p.services.iter().enumerate() {
            let base = format!("/services/{i}");
            if !is_pascal_ident(&s.name) {
                self.error(
                    "E_SERVICE_NAME",
                    format!("{base}/name"),
                    format!(
                        "service name `{}` must be UpperCamelCase (e.g. `{}`)",
                        s.name,
                        crate::names::pascal(&s.name)
                    ),
                );
            }
            if !names.insert(s.name.as_str()) {
                self.error(
                    "E_DUPLICATE_SERVICE",
                    format!("{base}/name"),
                    format!("service `{}` is defined twice", s.name),
                );
            }
            if s.port == 0 {
                self.error(
                    "E_PORT",
                    format!("{base}/port"),
                    "port must be between 1 and 65535",
                );
            } else if let Some(other) = ports.insert(s.port, &s.name) {
                self.error(
                    "E_DUPLICATE_PORT",
                    format!("{base}/port"),
                    format!("port {} is already used by `{other}`", s.port),
                );
            }
            if s.rpcs.is_empty() && s.consumers.is_empty() {
                self.warn(
                    "W_EMPTY_SERVICE",
                    base.clone(),
                    format!("service `{}` has no RPC and no consumer", s.name),
                );
            }

            let mut rpc_names = HashSet::new();
            for (j, r) in s.rpcs.iter().enumerate() {
                let rp = format!("{base}/rpcs/{j}");
                if !is_pascal_ident(&r.name) {
                    self.error(
                        "E_RPC_NAME",
                        format!("{rp}/name"),
                        format!(
                            "RPC name `{}` must be UpperCamelCase (e.g. `{}`)",
                            r.name,
                            crate::names::pascal(&r.name)
                        ),
                    );
                }
                if !rpc_names.insert(r.name.as_str()) {
                    self.error(
                        "E_DUPLICATE_RPC",
                        format!("{rp}/name"),
                        format!("RPC `{}` is defined twice in `{}`", r.name, s.name),
                    );
                }
                let req_ok = self.require_message(&r.request, &format!("{rp}/request"));
                let res_ok = self.require_message(&r.response, &format!("{rp}/response"));
                if req_ok && res_ok {
                    let env = Env {
                        service: s,
                        handler: Handler::Rpc {
                            response: &r.response,
                        },
                    };
                    let mut scope = Scope::with_input(&r.request);
                    let returns = self.steps(&env, &format!("{rp}/steps"), &r.steps, &mut scope);
                    if !returns {
                        self.warn(
                            "W_NO_RESPOND",
                            rp,
                            format!(
                                "`{}.{}` does not always end with a Respond step; it will return an empty `{}`",
                                s.name, r.name, r.response
                            ),
                        );
                    }
                }
            }

            let mut consumer_names = HashSet::new();
            for (j, c) in s.consumers.iter().enumerate() {
                let cp = format!("{base}/consumers/{j}");
                if !is_snake_ident(&c.name) {
                    self.error(
                        "E_CONSUMER_NAME",
                        format!("{cp}/name"),
                        format!(
                            "consumer name `{}` must be snake_case (e.g. `{}`)",
                            c.name,
                            snake(&c.name)
                        ),
                    );
                }
                if !consumer_names.insert(c.name.as_str()) {
                    self.error(
                        "E_DUPLICATE_CONSUMER",
                        format!("{cp}/name"),
                        format!("consumer `{}` is defined twice in `{}`", c.name, s.name),
                    );
                }
                if let Some(g) = &c.group_id
                    && !is_topic_name(g)
                {
                    self.error(
                        "E_GROUP_ID",
                        format!("{cp}/groupId"),
                        format!("`{g}` is not a valid consumer group id"),
                    );
                }
                let Some(topic) = p.topic(&c.topic) else {
                    self.error(
                        "E_UNKNOWN_TOPIC",
                        format!("{cp}/topic"),
                        format!(
                            "consumer `{}` listens to unknown topic `{}`",
                            c.name, c.topic
                        ),
                    );
                    continue;
                };
                if p.message(&topic.message).is_none() {
                    continue; // already reported on the topic
                }
                let env = Env {
                    service: s,
                    handler: Handler::Consumer,
                };
                let mut scope = Scope::with_input(&topic.message);
                self.steps(&env, &format!("{cp}/steps"), &c.steps, &mut scope);
            }
        }
    }

    fn require_message(&mut self, name: &str, path: &str) -> bool {
        if self.project.message(name).is_some() {
            return true;
        }
        self.error(
            "E_UNKNOWN_MESSAGE",
            path,
            format!("unknown message `{name}`"),
        );
        false
    }

    // ---- steps -----------------------------------------------------------------------

    /// Checks a block of steps. Returns whether the block always ends with `Respond`.
    fn steps(&mut self, env: &Env, base: &str, steps: &[Step], scope: &mut Scope) -> bool {
        let mut returns = false;
        for (i, step) in steps.iter().enumerate() {
            let path = format!("{base}/{i}");
            if returns {
                self.warn(
                    "W_UNREACHABLE",
                    &path,
                    "this step is never executed because a previous step already responds",
                );
            }
            if self.step(env, &path, step, scope) {
                returns = true;
            }
        }
        returns
    }

    fn step(&mut self, env: &Env, path: &str, step: &Step, scope: &mut Scope) -> bool {
        let p = self.project;
        match step {
            Step::Log { message, .. } => {
                match parse_template(message) {
                    Err(e) => self.error("E_LOG_TEMPLATE", format!("{path}/message"), e),
                    Ok(parts) => {
                        for part in parts {
                            if let TemplatePart::Path(pp) = part
                                && let Err(e) = resolve_path(p, scope, &pp)
                            {
                                self.error("E_UNKNOWN_PATH", format!("{path}/message"), e);
                            }
                        }
                    }
                }
                false
            }
            Step::Let {
                name,
                value_type,
                value,
                ..
            } => {
                let ty = TypeRef::scalar(value_type.clone());
                if !ty.is_simple_scalar() {
                    self.error(
                        "E_LET_TYPE",
                        format!("{path}/valueType"),
                        "variables can only hold string, bool, int32, int64, float or double",
                    );
                }
                self.check_var_name(name, &format!("{path}/name"));
                self.check_expr(scope, value, Some(&ty), &format!("{path}/value"));
                scope.bind(name.clone(), ty);
                false
            }
            Step::Publish { topic, payload, .. } => {
                match p.topic(topic) {
                    None => self.error(
                        "E_UNKNOWN_TOPIC",
                        format!("{path}/topic"),
                        format!("unknown topic `{topic}`"),
                    ),
                    Some(t) => {
                        if let Some(msg) = p.message(&t.message) {
                            self.assignments(msg, payload, scope, &format!("{path}/payload"));
                            if let Some(key) = &t.key_field
                                && !payload.iter().any(|a| &a.field == key)
                            {
                                self.warn(
                                        "W_KEY_FIELD_UNSET",
                                        format!("{path}/payload"),
                                        format!(
                                            "key field `{key}` of topic `{topic}` is not set; records will get an empty key"
                                        ),
                                    );
                            }
                        }
                    }
                }
                false
            }
            Step::CallRpc {
                service,
                method,
                request,
                bind,
                ..
            } => {
                if let Some(b) = bind {
                    self.check_var_name(b, &format!("{path}/bind"));
                }
                let Some(target) = p.service(service) else {
                    self.error(
                        "E_UNKNOWN_SERVICE",
                        format!("{path}/service"),
                        format!("unknown service `{service}`"),
                    );
                    return false;
                };
                if target.name == env.service.name {
                    self.error(
                        "E_SELF_CALL",
                        format!("{path}/service"),
                        "a service cannot call itself over RPC",
                    );
                }
                let Some(rpc) = target.rpc(method) else {
                    self.error(
                        "E_UNKNOWN_RPC",
                        format!("{path}/method"),
                        format!("service `{service}` has no RPC `{method}`"),
                    );
                    return false;
                };
                if let Some(msg) = p.message(&rpc.request) {
                    self.assignments(msg, request, scope, &format!("{path}/request"));
                }
                if let Some(b) = bind {
                    scope.bind(b.clone(), TypeRef::message(&rpc.response));
                }
                false
            }
            Step::Branch {
                condition,
                then,
                otherwise,
                ..
            } => {
                self.condition(scope, condition, &format!("{path}/condition"));
                let a = self.steps(env, &format!("{path}/then"), then, &mut scope.clone());
                let b = self.steps(
                    env,
                    &format!("{path}/otherwise"),
                    otherwise,
                    &mut scope.clone(),
                );
                a && b
            }
            Step::Respond { fields, .. } => {
                match env.handler {
                    Handler::Consumer => self.error(
                        "E_RESPOND_IN_CONSUMER",
                        path,
                        "Respond only makes sense in an RPC handler, consumers have nobody to answer",
                    ),
                    Handler::Rpc { response } => {
                        if let Some(msg) = p.message(response) {
                            self.assignments(msg, fields, scope, &format!("{path}/fields"));
                        }
                    }
                }
                true
            }
            Step::Code { code, .. } => {
                if code.trim().is_empty() {
                    self.warn("W_EMPTY_CODE", format!("{path}/code"), "empty Rust block");
                }
                false
            }
        }
    }

    fn check_var_name(&mut self, name: &str, path: &str) {
        if !is_snake_ident(name) {
            self.error(
                "E_VAR_NAME",
                path,
                format!(
                    "variable name `{name}` must be snake_case and not a Rust keyword (e.g. `{}`)",
                    snake(name)
                ),
            );
        } else if RESERVED_VARS.contains(&name) {
            self.error("E_RESERVED_NAME", path, format!("`{name}` is reserved"));
        }
    }

    fn assignments(&mut self, target: &Message, assigns: &[Assignment], scope: &Scope, base: &str) {
        let mut seen = HashSet::new();
        for (i, a) in assigns.iter().enumerate() {
            let path = format!("{base}/{i}");
            if !seen.insert(a.field.as_str()) {
                self.error(
                    "E_DUPLICATE_ASSIGNMENT",
                    &path,
                    format!("field `{}` is set twice", a.field),
                );
                continue;
            }
            let Some(field) = target.field(&a.field) else {
                self.error(
                    "E_UNKNOWN_FIELD",
                    &path,
                    format!("message `{}` has no field `{}`", target.name, a.field),
                );
                continue;
            };
            let ty = TypeRef::of_field(field);
            self.check_expr(scope, &a.value, Some(&ty), &format!("{path}/value"));
        }
    }

    /// Checks an expression against an expected type. Returns the expression's own type
    /// when it is known (raw Rust is opaque).
    fn check_expr(
        &mut self,
        scope: &Scope,
        expr: &Expr,
        expected: Option<&TypeRef>,
        path: &str,
    ) -> Option<TypeRef> {
        match expr {
            Expr::Path { path: pp } => match resolve_path(self.project, scope, pp) {
                Err(e) => {
                    self.error("E_UNKNOWN_PATH", path, e);
                    None
                }
                Ok(rp) => {
                    if let Some(to) = expected
                        && rp.adapt_to(to).is_none()
                    {
                        self.error(
                            "E_TYPE_MISMATCH",
                            path,
                            format!("`{pp}` is a {} but a {to} is expected", rp.ty),
                        );
                    }
                    Some(rp.ty)
                }
            },
            Expr::Literal { value } => match expected {
                Some(to) => {
                    if let Err(e) = coerce_literal(value, to) {
                        self.error("E_TYPE_MISMATCH", path, e);
                    }
                    Some(to.clone())
                }
                None => infer_literal(value),
            },
            Expr::Raw { code } => {
                if code.trim().is_empty() {
                    self.error("E_EMPTY_EXPR", path, "empty Rust expression");
                }
                None
            }
        }
    }

    fn condition(&mut self, scope: &Scope, cond: &Condition, path: &str) {
        match cond {
            Condition::Raw { code } => {
                if code.trim().is_empty() {
                    self.error("E_EMPTY_EXPR", path, "empty Rust condition");
                }
            }
            Condition::Compare { left, right, .. } => {
                // Comparisons need both sides to be exactly the same plain scalar type.
                let path_type = |this: &mut Self, e: &Expr, at: &str| -> Option<TypeRef> {
                    match e {
                        Expr::Path { .. } => this.check_expr(scope, e, None, at),
                        _ => None,
                    }
                };
                let lp = path_type(self, left, &format!("{path}/left"));
                let rp = path_type(self, right, &format!("{path}/right"));
                let known = lp.clone().or(rp.clone());
                if let Some(t) = &known
                    && !t.is_simple_scalar()
                {
                    self.error(
                        "E_COMPARE_TYPE",
                        path,
                        format!("only plain scalars can be compared, found {t}"),
                    );
                    return;
                }
                if let (Some(l), Some(r)) = (&lp, &rp)
                    && l != r
                {
                    self.error(
                        "E_TYPE_MISMATCH",
                        path,
                        format!("cannot compare a {l} with a {r}"),
                    );
                }
                let target = known.or_else(|| match left {
                    Expr::Literal { value } => infer_literal(value),
                    _ => None,
                });
                for (side, name) in [(left, "left"), (right, "right")] {
                    match side {
                        // With no known type the other side is raw Rust: unchecked.
                        Expr::Literal { value } => {
                            if let Some(t) = &target
                                && let Err(e) = coerce_literal(value, t)
                            {
                                self.error("E_TYPE_MISMATCH", format!("{path}/{name}"), e);
                            }
                        }
                        Expr::Raw { code } if code.trim().is_empty() => {
                            self.error(
                                "E_EMPTY_EXPR",
                                format!("{path}/{name}"),
                                "empty Rust expression",
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // ---- topology --------------------------------------------------------------------

    fn graph_warnings(&mut self) {
        let p = self.project;
        let mut published: BTreeSet<&str> = BTreeSet::new();
        for s in &p.services {
            let handler_steps = s
                .rpcs
                .iter()
                .flat_map(|r| &r.steps)
                .chain(s.consumers.iter().flat_map(|c| &c.steps));
            for step in handler_steps {
                step.walk(&mut |st| {
                    if let Step::Publish { topic, .. } = st {
                        published.insert(topic);
                    }
                });
            }
        }
        let consumed: BTreeSet<&str> = p
            .services
            .iter()
            .flat_map(|s| s.consumers.iter().map(|c| c.topic.as_str()))
            .collect();
        for (i, t) in p.topics.iter().enumerate() {
            if !published.contains(t.name.as_str()) {
                self.warn(
                    "W_TOPIC_NO_PRODUCER",
                    format!("/topics/{i}"),
                    format!("nothing publishes on topic `{}`", t.name),
                );
            }
            if !consumed.contains(t.name.as_str()) {
                self.warn(
                    "W_TOPIC_NO_CONSUMER",
                    format!("/topics/{i}"),
                    format!("nothing consumes topic `{}`", t.name),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample;

    fn codes(p: &Project) -> Vec<String> {
        validate(p).into_iter().map(|d| d.code).collect()
    }

    fn errors(p: &Project) -> Vec<String> {
        validate(p)
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.code)
            .collect()
    }

    #[test]
    fn sample_is_valid_without_warnings() {
        let d = validate(&sample::orders());
        assert_eq!(d, vec![], "sample project must be clean");
    }

    #[test]
    fn empty_project_is_valid() {
        assert!(errors(&Project::new("empty")).is_empty());
    }

    #[test]
    fn bad_names() {
        let mut p = Project::new("1 bad");
        p.messages.push(Message {
            name: "orderCreated".into(),
            fields: vec![Field::new("OrderId", FieldType::String)],
        });
        let e = errors(&p);
        assert!(e.contains(&"E_PROJECT_NAME".into()));
        assert!(e.contains(&"E_MESSAGE_NAME".into()));
        assert!(e.contains(&"E_FIELD_NAME".into()));
    }

    #[test]
    fn unknown_references() {
        let mut p = sample::orders();
        p.topics[0].message = "Nope".into();
        p.services[1].consumers[0].topic = "nope.topic".into();
        let e = errors(&p);
        assert!(e.contains(&"E_UNKNOWN_MESSAGE".into()));
        assert!(e.contains(&"E_UNKNOWN_TOPIC".into()));
    }

    #[test]
    fn field_numbers_must_be_unique_and_allowed() {
        let mut p = sample::orders();
        p.messages[0].fields[0].tag = Some(1);
        p.messages[0].fields[1].tag = Some(1);
        assert!(errors(&p).contains(&"E_DUPLICATE_TAG".into()));
        p.messages[0].fields[1].tag = Some(19_500);
        assert!(errors(&p).contains(&"E_FIELD_TAG".into()));
        p.messages[0].fields[1].tag = Some(2);
        assert!(errors(&p).is_empty());
    }

    #[test]
    fn detects_message_cycle() {
        let mut p = Project::new("cyc");
        let msg = |name: &str, other: &str| Message {
            name: name.into(),
            fields: vec![Field::new(
                "other",
                FieldType::Message { name: other.into() },
            )],
        };
        p.messages = vec![msg("A", "B"), msg("B", "A")];
        assert!(errors(&p).contains(&"E_MESSAGE_CYCLE".into()));
        // A list breaks the cycle.
        p.messages[1].fields[0].repeated = true;
        assert!(!errors(&p).contains(&"E_MESSAGE_CYCLE".into()));
    }

    #[test]
    fn type_mismatch_in_publish() {
        let mut p = sample::orders();
        // `amount` is int64, assign the customer's name (a string).
        let Step::Publish { payload, .. } = &mut p.services[0].rpcs[0].steps[2] else {
            panic!("sample layout changed")
        };
        payload[2].value = Expr::Path {
            path: "input.customer.name".into(),
        };
        let d = validate(&p);
        let hit = d
            .iter()
            .find(|d| d.code == "E_TYPE_MISMATCH")
            .expect("mismatch");
        assert_eq!(hit.path, "/services/0/rpcs/0/steps/2/payload/2/value");
    }

    #[test]
    fn respond_in_consumer_is_an_error() {
        let mut p = sample::orders();
        p.services[1].consumers[0].steps.push(Step::Respond {
            id: "x".into(),
            fields: vec![],
        });
        assert!(errors(&p).contains(&"E_RESPOND_IN_CONSUMER".into()));
    }

    #[test]
    fn variables_are_scoped_to_their_block() {
        let mut p = sample::orders();
        // Payments consumer: branch defines nothing; add a use of a branch-local variable after it.
        let steps = &mut p.services[1].consumers[0].steps;
        let Step::Branch { then, .. } = &mut steps[1] else {
            panic!("sample layout changed")
        };
        then.insert(
            0,
            Step::Let {
                id: "l".into(),
                name: "local".into(),
                value_type: FieldType::String,
                value: Expr::Literal { value: "x".into() },
            },
        );
        steps.push(Step::Log {
            id: "z".into(),
            level: LogLevel::Info,
            message: "{local}".into(),
        });
        assert!(errors(&p).contains(&"E_UNKNOWN_PATH".into()));
    }

    #[test]
    fn rpc_without_respond_warns() {
        let mut p = sample::orders();
        p.services[0].rpcs[0].steps.pop();
        assert!(codes(&p).contains(&"W_NO_RESPOND".into()));
    }

    #[test]
    fn self_call_and_unknown_rpc() {
        let mut p = sample::orders();
        p.services[2].consumers[1].steps.push(Step::CallRpc {
            id: "c".into(),
            service: "Notifications".into(),
            method: "X".into(),
            request: vec![],
            bind: None,
        });
        p.services[2].consumers[1].steps.push(Step::CallRpc {
            id: "d".into(),
            service: "Orders".into(),
            method: "Missing".into(),
            request: vec![],
            bind: None,
        });
        let e = errors(&p);
        assert!(e.contains(&"E_SELF_CALL".into()));
        assert!(e.contains(&"E_UNKNOWN_RPC".into()));
    }

    #[test]
    fn duplicate_ports() {
        let mut p = sample::orders();
        p.services[1].port = p.services[0].port;
        assert!(errors(&p).contains(&"E_DUPLICATE_PORT".into()));
    }

    #[test]
    fn orphan_topic_warns() {
        let mut p = sample::orders();
        p.topics.push(Topic {
            name: "audit.log".into(),
            message: "OrderCreated".into(),
            partitions: 1,
            replication_factor: 1,
            key_field: None,
            position: Position::default(),
        });
        let c = codes(&p);
        assert!(c.contains(&"W_TOPIC_NO_PRODUCER".into()));
        assert!(c.contains(&"W_TOPIC_NO_CONSUMER".into()));
    }
}
