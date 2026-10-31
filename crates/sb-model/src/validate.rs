//! Project validation. Everything the generator relies on is checked here, so a project
//! without errors is guaranteed to produce code that type-checks (barring raw Rust blocks).
//!
//! Diagnostics point at the faulty element with a JSON pointer into the project
//! document, e.g. `/services/0/rpcs/1/steps/2/then/0`.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use crate::names::{
    is_pascal_ident, is_snake_ident, is_topic_name, route_params, route_shape, snake, topic_const,
};
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
    // The rows read by SQL steps are typed like messages (appended: indexes do not move).
    let typed = project.with_sql_rows();
    let mut c = Checker {
        project: &typed,
        out: Vec::new(),
    };
    c.project();
    c.messages();
    c.unused_rules();
    c.unused_sources();
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
    Rpc {
        response: &'a str,
    },
    /// An HTTP route: answers like an RPC and can also end with a `Fail`.
    Route {
        response: &'a str,
    },
    Consumer,
}

struct Env<'a> {
    service: &'a Service,
    handler: Handler<'a>,
}

fn kind_label(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Event => "an event schema",
        MessageKind::Proto => "a proto schema",
        MessageKind::Api => "an API schema",
        MessageKind::Row => "a SQL row",
    }
}

/// An environment variable name: upper case letters, digits and `_`, starting with a letter.
fn is_env_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// The longest period of a job: 30 days.
const MAX_JOB_INTERVAL_SECS: u32 = 2_592_000;

/// `tx` is the database transaction every handler runs in.
const RESERVED_VARS: [&str; 3] = ["input", "ctx", "tx"];

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
        if !(1..=MAX_OUTBOX_ATTEMPTS).contains(&p.outbox.max_attempts) {
            self.error(
                "E_OUTBOX_ATTEMPTS",
                "/outbox/maxAttempts",
                format!("an event gets between 1 and {MAX_OUTBOX_ATTEMPTS} publish attempts before it is moved to the DLQ"),
            );
        }
    }

    // ---- messages --------------------------------------------------------------------

    /// Rules that no route would ever check give a false sense of safety: say so.
    fn unused_rules(&mut self) {
        let received = self.project.received_schemas();
        for (i, m) in self.project.messages.iter().enumerate() {
            let has_rules = m.fields.iter().any(|f| !f.rules.is_empty());
            if has_rules && m.kind == MessageKind::Api && !received.contains(m.name.as_str()) {
                self.warn(
                    "W_RULE_UNUSED",
                    format!("/messages/{i}"),
                    format!(
                        "the rules of `{}` are never checked: no route receives it, directly or inside another schema",
                        m.name
                    ),
                );
            }
        }
    }

    /// A field read from a header or a cookie is filled by the route that receives the schema: elsewhere (an
    /// event, a nested schema) nothing would ever fill it.
    fn unused_sources(&mut self) {
        let direct: HashSet<&str> = self
            .project
            .services
            .iter()
            .flat_map(|s| s.routes.iter().map(|r| r.request.as_str()))
            .collect();
        for (i, m) in self.project.messages.iter().enumerate() {
            if m.kind == MessageKind::Api
                && m.fields.iter().any(|f| f.source.is_some())
                && !direct.contains(m.name.as_str())
            {
                self.warn(
                    "W_SOURCE_UNUSED",
                    format!("/messages/{i}"),
                    format!(
                        "`{}` has fields read from headers or cookies, but no route receives it directly: they are never filled",
                        m.name
                    ),
                );
            }
        }
    }

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
            if m.kind == MessageKind::Row {
                continue; // derived from a SQL step, not part of the project
            }
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
                if let FieldType::Message { name } = &f.ty {
                    match p.message(name) {
                        None => self.error(
                            "E_UNKNOWN_MESSAGE",
                            format!("{fp}/type"),
                            format!("field `{}` references unknown message `{name}`", f.name),
                        ),
                        // Events and proto schemas share building blocks; API schemas live apart.
                        Some(target)
                            if (target.kind == MessageKind::Api)
                                != (m.kind == MessageKind::Api) =>
                        {
                            self.error(
                                "E_SCHEMA_KIND_MIX",
                                format!("{fp}/type"),
                                format!(
                                    "`{}` is {} and cannot contain `{name}`, which is {}",
                                    m.name,
                                    kind_label(m.kind),
                                    kind_label(target.kind)
                                ),
                            );
                        }
                        Some(_) => {}
                    }
                }
                if m.kind == MessageKind::Api
                    && matches!(f.ty, FieldType::Bytes | FieldType::Timestamp)
                {
                    self.error(
                        "E_API_FIELD_TYPE",
                        format!("{fp}/type"),
                        format!(
                            "field `{}`: `{}` has no JSON form in API schemas yet (use string, bool, int32, int64, float, double or another API schema)",
                            f.name,
                            if f.ty == FieldType::Bytes { "bytes" } else { "timestamp" }
                        ),
                    );
                }
                self.field_rules(m, f, &fp);
                self.field_source(m, f, &fp);
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
            if t.name.to_ascii_lowercase().ends_with(".dlq") {
                self.error(
                    "E_TOPIC_RESERVED",
                    format!("{base}/name"),
                    format!(
                        "`{}` ends with `.dlq`, which is reserved for dead-letter topics (`<topic>.dlq` is created for every topic)",
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
            let mut seen = HashSet::new();
            for (j, producer) in t.producers.iter().enumerate() {
                let pp = format!("{base}/producers/{j}");
                if p.service(producer).is_none() {
                    self.error(
                        "E_UNKNOWN_SERVICE",
                        pp,
                        format!("topic `{}` lists unknown producer `{producer}`", t.name),
                    );
                } else if !seen.insert(producer.as_str()) {
                    self.error(
                        "E_DUPLICATE_PRODUCER",
                        pp,
                        format!("`{producer}` is listed twice as producer of `{}`", t.name),
                    );
                }
            }
            let Some(msg) = p.message(&t.message) else {
                self.error(
                    "E_UNKNOWN_MESSAGE",
                    format!("{base}/message"),
                    format!("topic `{}` carries unknown message `{}`", t.name, t.message),
                );
                continue;
            };
            if msg.kind != MessageKind::Event {
                self.error(
                    "E_TOPIC_MESSAGE_KIND",
                    format!("{base}/message"),
                    format!(
                        "topic `{}` carries `{}`, which is {}: a topic needs an event schema",
                        t.name,
                        msg.name,
                        kind_label(msg.kind)
                    ),
                );
            }
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
            if !s.routes.is_empty() {
                match s.http_port {
                    None | Some(0) => self.error(
                        "E_HTTP_PORT",
                        format!("{base}/httpPort"),
                        format!(
                            "`{}` has HTTP routes: give it an HTTP port (1 to 65535)",
                            s.name
                        ),
                    ),
                    Some(port) => {
                        if let Some(other) = ports.insert(port, &s.name) {
                            self.error(
                                "E_DUPLICATE_PORT",
                                format!("{base}/httpPort"),
                                format!("port {port} is already used by `{other}`"),
                            );
                        }
                    }
                }
            }
            if s.rpcs.is_empty()
                && s.consumers.is_empty()
                && s.routes.is_empty()
                && s.jobs.is_empty()
            {
                self.warn(
                    "W_EMPTY_SERVICE",
                    base.clone(),
                    format!(
                        "service `{}` has no RPC, no HTTP route, no consumer and no job",
                        s.name
                    ),
                );
            }
            self.routes(s, &base);
            self.tables(s, &base);
            self.table_constraints(s, &base);

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
                self.require_proto(&r.request, &format!("{rp}/request"));
                self.require_proto(&r.response, &format!("{rp}/response"));
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
                if let Some(field) = &c.idempotent_on {
                    self.idempotent_field(&format!("{cp}/idempotentOn"), &topic.message, field);
                }
                let env = Env {
                    service: s,
                    handler: Handler::Consumer,
                };
                let mut scope = Scope::with_input(&topic.message);
                self.steps(&env, &format!("{cp}/steps"), &c.steps, &mut scope);
            }

            let mut job_names = HashSet::new();
            for (j, job) in s.jobs.iter().enumerate() {
                let jp = format!("{base}/jobs/{j}");
                if !is_snake_ident(&job.name) {
                    self.error(
                        "E_JOB_NAME",
                        format!("{jp}/name"),
                        format!(
                            "job name `{}` must be snake_case (e.g. `{}`)",
                            job.name,
                            snake(&job.name)
                        ),
                    );
                }
                if !job_names.insert(job.name.as_str()) {
                    self.error(
                        "E_DUPLICATE_JOB",
                        format!("{jp}/name"),
                        format!("job `{}` is defined twice in `{}`", job.name, s.name),
                    );
                }
                if !(1..=MAX_JOB_INTERVAL_SECS).contains(&job.every_secs) {
                    self.error(
                        "E_JOB_INTERVAL",
                        format!("{jp}/everySecs"),
                        format!("a job runs every 1 second to 30 days ({MAX_JOB_INTERVAL_SECS} s)"),
                    );
                }
                // No request, no event: nothing to read as `input`, nobody to answer.
                let env = Env {
                    service: s,
                    handler: Handler::Consumer,
                };
                let mut scope = Scope::default();
                self.steps(&env, &format!("{jp}/steps"), &job.steps, &mut scope);
            }
        }
    }

    /// The rules of a field: only on the text fields of API schemas (the ones a route reads from a request), each
    /// kind once, and a length that can be satisfied.
    fn field_rules(&mut self, m: &Message, f: &Field, fp: &str) {
        if f.rules.is_empty() {
            return;
        }
        let rp = format!("{fp}/rules");
        if m.kind != MessageKind::Api {
            self.error(
                "E_RULE_KIND",
                &rp,
                format!(
                    "field `{}` has rules, but `{}` is {}: only API schemas are checked, when a route receives them",
                    f.name,
                    m.name,
                    kind_label(m.kind)
                ),
            );
        } else if !matches!(f.ty, FieldType::String | FieldType::Uuid) {
            self.error(
                "E_RULE_TYPE",
                &rp,
                format!(
                    "field `{}` has rules, which only apply to text (string or uuid)",
                    f.name
                ),
            );
        }
        let mut kinds = HashSet::new();
        for (i, rule) in f.rules.iter().enumerate() {
            if !kinds.insert(rule.kind()) {
                self.error(
                    "E_RULE_DUPLICATE",
                    format!("{rp}/{i}"),
                    format!("field `{}` has two `{}` rules", f.name, rule.kind()),
                );
            }
            if let Rule::Length { min, max } = rule
                && (*max == 0 || min > max)
            {
                self.error(
                    "E_RULE_LENGTH",
                    format!("{rp}/{i}"),
                    format!("a length between {min} and {max} characters cannot be satisfied"),
                );
            }
        }
    }

    /// Where a field is read from the request: only the text fields of API schemas, under a name that can travel in
    /// a header or a cookie.
    fn field_source(&mut self, m: &Message, f: &Field, fp: &str) {
        let Some(source) = &f.source else {
            return;
        };
        let sp = format!("{fp}/source");
        if m.kind != MessageKind::Api {
            self.error(
                "E_SOURCE_KIND",
                &sp,
                format!(
                    "field `{}` is read from the request, but `{}` is {}: only API schemas come from a request",
                    f.name,
                    m.name,
                    kind_label(m.kind)
                ),
            );
        } else if !matches!(f.ty, FieldType::String | FieldType::Uuid) || f.repeated || f.optional {
            self.error(
                "E_SOURCE_TYPE",
                &sp,
                format!(
                    "field `{}` is read from the request: it must be a single required string or uuid (empty when the request lacks it)",
                    f.name
                ),
            );
        }
        let (what, name, extra) = match source {
            RequestSource::Header { name } => ("header", name, ""),
            RequestSource::Cookie { name } => ("cookie", name, "_."),
        };
        let valid = !name.is_empty()
            && name.len() <= 100
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || extra.as_bytes().contains(&b));
        if !valid {
            self.error(
                "E_SOURCE_NAME",
                format!("{sp}/name"),
                format!(
                    "`{name}` is not a {what} name: letters, digits and `-`{} only, 100 characters at most",
                    if extra.is_empty() { "" } else { ", `_` and `.`" }
                ),
            );
        }
    }

    /// The field that identifies an event: a single text or integer, since it becomes the key of a table.
    fn idempotent_field(&mut self, path: &str, message: &str, field: &str) {
        let Some(msg) = self.project.message(message) else {
            return;
        };
        let ok = match msg.field(field) {
            None => {
                self.error(
                    "E_IDEMPOTENT_FIELD",
                    path,
                    format!("`{message}` has no field `{field}` to tell its events apart"),
                );
                return;
            }
            Some(f) => {
                !f.repeated
                    && !f.optional
                    && matches!(
                        f.ty,
                        FieldType::String | FieldType::Uuid | FieldType::Int32 | FieldType::Int64
                    )
            }
        };
        if !ok {
            self.error(
                "E_IDEMPOTENT_FIELD",
                path,
                format!(
                    "`{message}.{field}` cannot identify an event: it has to be a single string, uuid, int32 or int64"
                ),
            );
        }
    }

    fn routes(&mut self, s: &Service, base: &str) {
        let p = self.project;
        let mut names = HashSet::new();
        let mut shapes: BTreeMap<(&'static str, String), &str> = BTreeMap::new();
        for (j, r) in s.routes.iter().enumerate() {
            let rp = format!("{base}/routes/{j}");
            if !is_snake_ident(&r.name) {
                self.error(
                    "E_ROUTE_NAME",
                    format!("{rp}/name"),
                    format!(
                        "route name `{}` must be snake_case (e.g. `{}`)",
                        r.name,
                        snake(&r.name)
                    ),
                );
            }
            if !names.insert(r.name.as_str()) {
                self.error(
                    "E_DUPLICATE_ROUTE_NAME",
                    format!("{rp}/name"),
                    format!("route `{}` is defined twice in `{}`", r.name, s.name),
                );
            }
            let params = match route_params(&r.path) {
                Ok(params) => {
                    if let Some(other) =
                        shapes.insert((r.method.as_str(), route_shape(&r.path)), &r.name)
                    {
                        self.error(
                            "E_DUPLICATE_ROUTE",
                            format!("{rp}/path"),
                            format!(
                                "`{} {}` is already served by route `{other}`",
                                r.method.as_str(),
                                r.path
                            ),
                        );
                    }
                    params
                }
                Err(why) => {
                    self.error("E_ROUTE_PATH", format!("{rp}/path"), why);
                    vec![]
                }
            };
            if !(200..=202).contains(&r.status) {
                self.error(
                    "E_ROUTE_STATUS",
                    format!("{rp}/status"),
                    "the success status must be 200, 201 or 202",
                );
            }
            let req_ok = self.require_message(&r.request, &format!("{rp}/request"));
            let res_ok = self.require_message(&r.response, &format!("{rp}/response"));
            self.require_api(&r.request, &format!("{rp}/request"));
            self.require_api(&r.response, &format!("{rp}/response"));
            if !req_ok || !res_ok {
                continue;
            }

            let request = p.message(&r.request).expect("checked");
            for param in &params {
                match request.field(param) {
                    None => self.error(
                        "E_ROUTE_PARAM",
                        format!("{rp}/path"),
                        format!("`{{{param}}}` has no field `{param}` in `{}`", r.request),
                    ),
                    Some(f)
                        if f.repeated
                            || f.optional
                            || !matches!(
                                f.ty,
                                FieldType::String
                                    | FieldType::Bool
                                    | FieldType::Int32
                                    | FieldType::Int64
                            ) =>
                    {
                        self.error(
                            "E_ROUTE_PARAM",
                            format!("{rp}/path"),
                            format!(
                                "field `{param}` of `{}` fills `{{{param}}}`: it must be a required string, bool, int32 or int64",
                                r.request
                            ),
                        );
                    }
                    Some(_) => {}
                }
            }
            for f in request
                .fields
                .iter()
                .filter(|f| f.source.is_some() && params.contains(&f.name))
            {
                self.error(
                    "E_SOURCE_PARAM",
                    format!("{rp}/path"),
                    format!(
                        "field `{}` of `{}` is read from the request, and `{{{}}}` of the path fills it too",
                        f.name, r.request, f.name
                    ),
                );
            }
            if !r.method.has_body() {
                for f in request.fields.iter().filter(|f| !params.contains(&f.name)) {
                    if f.repeated || f.ty.is_message() {
                        self.error(
                            "E_ROUTE_QUERY_FIELD",
                            format!("{rp}/request"),
                            format!(
                                "field `{}` of `{}` comes from the query string of a {}: only single scalar values fit (use POST, PUT or PATCH for a body)",
                                f.name,
                                r.request,
                                r.method.as_str()
                            ),
                        );
                    }
                }
            }

            self.hmac_placement(r, &format!("{rp}/steps"));
            let env = Env {
                service: s,
                handler: Handler::Route {
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
                        "`{} {}` does not always end with a Respond step; it will answer with an empty `{}`",
                        r.method.as_str(),
                        r.path,
                        r.response
                    ),
                );
            }
        }
    }

    // ---- tables and SQL ---------------------------------------------------------------

    fn tables(&mut self, s: &Service, base: &str) {
        let mut names = HashSet::new();
        for (i, t) in s.tables.iter().enumerate() {
            let tp = format!("{base}/tables/{i}");
            if !is_snake_ident(&t.name) {
                self.error(
                    "E_TABLE_NAME",
                    format!("{tp}/name"),
                    format!(
                        "table name `{}` must be snake_case (e.g. `{}`)",
                        t.name,
                        snake(&t.name)
                    ),
                );
            } else if t.name == "outbox" {
                self.error(
                    "E_TABLE_RESERVED",
                    format!("{tp}/name"),
                    "`outbox` is the table of the published events, pick another name",
                );
            }
            if !names.insert(t.name.as_str()) {
                self.error(
                    "E_DUPLICATE_TABLE",
                    format!("{tp}/name"),
                    format!("table `{}` is defined twice in `{}`", t.name, s.name),
                );
            }
            if t.columns.is_empty() {
                self.error(
                    "E_TABLE_EMPTY",
                    &tp,
                    format!("table `{}` has no column", t.name),
                );
            } else if !t.columns.iter().any(|c| c.primary_key) {
                self.warn(
                    "W_TABLE_NO_PK",
                    &tp,
                    format!("table `{}` has no primary key", t.name),
                );
            }
            let mut columns = HashSet::new();
            for (j, c) in t.columns.iter().enumerate() {
                let cp = format!("{tp}/columns/{j}");
                if !is_snake_ident(&c.name) {
                    self.error(
                        "E_COLUMN_NAME",
                        format!("{cp}/name"),
                        format!(
                            "column name `{}` must be snake_case (e.g. `{}`)",
                            c.name,
                            snake(&c.name)
                        ),
                    );
                }
                if !columns.insert(c.name.as_str()) {
                    self.error(
                        "E_DUPLICATE_COLUMN",
                        format!("{cp}/name"),
                        format!("column `{}` is defined twice in `{}`", c.name, t.name),
                    );
                }
                if c.primary_key && c.nullable {
                    self.error(
                        "E_PK_NULLABLE",
                        &cp,
                        format!("`{}` is a primary key: it cannot be null", c.name),
                    );
                }
                if let Some(d) = &c.default
                    && (d.contains(';') || d.contains("--") || d.contains('\n'))
                {
                    self.error(
                        "E_COLUMN_DEFAULT",
                        format!("{cp}/default"),
                        "a default is one SQL expression (no `;`, no comment, no line break)",
                    );
                }
            }
        }
    }

    /// Unique constraints, indexes and foreign keys. A unique constraint and an index are both
    /// relations of the schema in Postgres: their names are shared with the tables of the service.
    fn table_constraints(&mut self, s: &Service, base: &str) {
        let mut relations: HashSet<String> = s.tables.iter().map(|t| t.name.clone()).collect();
        relations.insert("outbox".into());
        relations.insert("outbox_pending_idx".into());
        relations.extend(s.tables.iter().map(|t| format!("{}_pkey", t.name)));

        for (i, t) in s.tables.iter().enumerate() {
            let tp = format!("{base}/tables/{i}");
            let named =
                t.uniques
                    .iter()
                    .enumerate()
                    .map(|(j, u)| {
                        (
                            format!("{tp}/uniques/{j}"),
                            "unique constraint",
                            &u.name,
                            &u.columns,
                        )
                    })
                    .chain(
                        t.indexes.iter().enumerate().map(|(j, x)| {
                            (format!("{tp}/indexes/{j}"), "index", &x.name, &x.columns)
                        }),
                    );
            for (path, what, name, columns) in named {
                self.constraint_name(&path, what, name);
                if !relations.insert(name.clone()) {
                    self.error(
                        "E_DUPLICATE_CONSTRAINT",
                        format!("{path}/name"),
                        format!(
                            "`{name}` is already the name of a table, an index or a constraint of `{}`",
                            s.name
                        ),
                    );
                }
                self.constraint_columns(&path, t, columns);
            }

            let mut fk_names = HashSet::new();
            for (j, fk) in t.foreign_keys.iter().enumerate() {
                let fp = format!("{tp}/foreignKeys/{j}");
                self.constraint_name(&fp, "foreign key", &fk.name);
                if !fk_names.insert(fk.name.as_str()) {
                    self.error(
                        "E_DUPLICATE_CONSTRAINT",
                        format!("{fp}/name"),
                        format!("foreign key `{}` is defined twice in `{}`", fk.name, t.name),
                    );
                }
                self.foreign_key(s, t, &fp, fk);
            }
        }
    }

    /// Names end up quoted in DDL and are cut by Postgres beyond 63 bytes.
    fn constraint_name(&mut self, path: &str, what: &str, name: &str) {
        if !is_snake_ident(name) || name.len() > 63 {
            self.error(
                "E_CONSTRAINT_NAME",
                format!("{path}/name"),
                format!("{what} name `{name}` must be snake_case, 63 characters at most"),
            );
        }
    }

    /// At least one column, each of the table and named once.
    fn constraint_columns(&mut self, path: &str, t: &Table, columns: &[String]) {
        if columns.is_empty() {
            self.error(
                "E_CONSTRAINT_NO_COLUMNS",
                format!("{path}/columns"),
                "pick at least one column",
            );
        }
        let mut seen = HashSet::new();
        for (k, name) in columns.iter().enumerate() {
            if t.column(name).is_none() {
                self.error(
                    "E_CONSTRAINT_UNKNOWN_COLUMN",
                    format!("{path}/columns/{k}"),
                    format!("table `{}` has no column `{name}`", t.name),
                );
            } else if !seen.insert(name.as_str()) {
                self.error(
                    "E_CONSTRAINT_DUPLICATE_COLUMN",
                    format!("{path}/columns/{k}"),
                    format!("column `{name}` is listed twice"),
                );
            }
        }
    }

    fn foreign_key(&mut self, s: &Service, t: &Table, path: &str, fk: &ForeignKey) {
        self.constraint_columns(path, t, &fk.columns);
        let Some(target) = s.table(&fk.ref_table) else {
            self.error(
                "E_FK_UNKNOWN_TABLE",
                format!("{path}/refTable"),
                format!(
                    "`{}` has no table `{}`: a foreign key stays within the database of its service",
                    s.name, fk.ref_table
                ),
            );
            return;
        };
        if fk.columns.len() != fk.ref_columns.len() {
            self.error(
                "E_FK_ARITY",
                format!("{path}/refColumns"),
                format!(
                    "{} column(s) cannot reference {} column(s): pair them one to one",
                    fk.columns.len(),
                    fk.ref_columns.len()
                ),
            );
            return;
        }
        let mut referenced_ok = true;
        for (k, name) in fk.ref_columns.iter().enumerate() {
            if target.column(name).is_none() {
                referenced_ok = false;
                self.error(
                    "E_CONSTRAINT_UNKNOWN_COLUMN",
                    format!("{path}/refColumns/{k}"),
                    format!("table `{}` has no column `{name}`", target.name),
                );
            }
        }
        for (k, (from, to)) in fk.columns.iter().zip(&fk.ref_columns).enumerate() {
            if let (Some(a), Some(b)) = (t.column(from), target.column(to))
                && a.ty != b.ty
            {
                self.error(
                    "E_FK_TYPE",
                    format!("{path}/columns/{k}"),
                    format!(
                        "`{}.{from}` is {} but `{}.{to}` is {}: a foreign key pairs columns of the same type",
                        t.name,
                        a.ty.sql(),
                        target.name,
                        b.ty.sql()
                    ),
                );
            }
        }
        if referenced_ok && !target.is_unique_key(&fk.ref_columns) {
            self.error(
                "E_FK_NOT_UNIQUE",
                format!("{path}/refColumns"),
                format!(
                    "({}) is neither the primary key nor unique in `{}`: add a UNIQUE constraint on it first",
                    fk.ref_columns.join(", "),
                    target.name
                ),
            );
        }
        for (field, action) in [("onDelete", fk.on_delete), ("onUpdate", fk.on_update)] {
            let columns = fk.columns.iter().filter_map(|c| t.column(c));
            match action {
                ReferentialAction::SetNull => {
                    if let Some(c) = columns.clone().find(|c| !c.nullable) {
                        self.error(
                            "E_FK_SET_NULL",
                            format!("{path}/{field}"),
                            format!("SET NULL would null `{}`, which is not nullable", c.name),
                        );
                    }
                }
                ReferentialAction::SetDefault => {
                    if let Some(c) = columns
                        .clone()
                        .find(|c| c.default.as_deref().is_none_or(|d| d.trim().is_empty()))
                    {
                        self.error(
                            "E_FK_SET_DEFAULT",
                            format!("{path}/{field}"),
                            format!("SET DEFAULT needs a default on `{}`", c.name),
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// The type of the values a column holds, as a flow sees it.
    fn column_type(c: &Column) -> TypeRef {
        let mut t = TypeRef::scalar(c.ty.field_type());
        t.optional = c.nullable;
        t
    }

    fn sql(&mut self, env: &Env, path: &str, step: &Step, scope: &mut Scope) {
        let Step::Sql {
            op,
            table,
            bind,
            columns,
            many,
            conditions,
            values,
            order_by,
            missing,
            ..
        } = step
        else {
            return;
        };
        let Some(t) = env.service.table(table) else {
            self.error(
                "E_UNKNOWN_TABLE",
                format!("{path}/table"),
                format!(
                    "`{}` has no table `{table}`: design it in the Tables view",
                    env.service.name
                ),
            );
            return;
        };
        let column = |name: &str| t.column(name);

        for (i, c) in conditions.iter().enumerate() {
            let cp = format!("{path}/where/{i}");
            match column(&c.column) {
                None => self.error(
                    "E_UNKNOWN_COLUMN",
                    format!("{cp}/column"),
                    format!("table `{table}` has no column `{}`", c.column),
                ),
                Some(col) => self.sql_value(scope, &c.value, col, &format!("{cp}/value")),
            }
        }
        if matches!(op, SqlOp::Update | SqlOp::Delete) && conditions.is_empty() {
            self.error(
                "E_SQL_NO_WHERE",
                format!("{path}/where"),
                "an update or a delete without a condition would touch every row: add one",
            );
        }

        match op {
            SqlOp::Select => {
                if columns.is_empty() {
                    self.error(
                        "E_SQL_NO_COLUMNS",
                        format!("{path}/columns"),
                        "tick at least one column to read",
                    );
                }
                if let Some(o) = order_by
                    && column(o).is_none()
                {
                    self.error(
                        "E_UNKNOWN_COLUMN",
                        format!("{path}/orderBy"),
                        format!("table `{table}` has no column `{o}`"),
                    );
                }
                self.sql_result(path, step, t, scope);
            }
            SqlOp::Insert | SqlOp::Update => {
                let mut seen = HashSet::new();
                for (i, a) in values.iter().enumerate() {
                    let vp = format!("{path}/values/{i}");
                    if !seen.insert(a.field.as_str()) {
                        self.error(
                            "E_SQL_DUPLICATE_COLUMN",
                            &vp,
                            format!("`{}` is set twice", a.field),
                        );
                    }
                    match column(&a.field) {
                        None => self.error(
                            "E_UNKNOWN_COLUMN",
                            format!("{vp}/field"),
                            format!("table `{table}` has no column `{}`", a.field),
                        ),
                        Some(col) => self.sql_value(scope, &a.value, col, &format!("{vp}/value")),
                    }
                }
                if *op == SqlOp::Update && values.is_empty() {
                    self.error(
                        "E_SQL_NO_VALUES",
                        format!("{path}/values"),
                        "give at least one column a new value",
                    );
                }
                if !columns.is_empty() {
                    self.sql_result(path, step, t, scope);
                }
                if *op == SqlOp::Insert {
                    for col in t.columns.iter().filter(|c| c.is_required_on_insert()) {
                        if !values.iter().any(|a| a.field == col.name) {
                            self.error(
                                "E_SQL_MISSING_VALUE",
                                format!("{path}/values"),
                                format!("column `{}` has no default and is not nullable: give it a value", col.name),
                            );
                        }
                    }
                }
            }
            SqlOp::Delete => {
                if !columns.is_empty() {
                    self.sql_result(path, step, t, scope);
                }
            }
        }

        if let Some(m) = missing {
            let mp = format!("{path}/missing");
            let one_row = !columns.is_empty() && bind.is_some() && !*many;
            if !one_row {
                self.error(
                    "E_SQL_MISSING_USELESS",
                    &mp,
                    "an answer for a missing row needs a step that reads one row (tick columns, name the variable, not a list)",
                );
            }
            if !matches!(env.handler, Handler::Route { .. }) {
                self.error(
                    "E_SQL_MISSING_OUTSIDE_ROUTE",
                    &mp,
                    "the answer to a missing row is an HTTP status: it only makes sense in an HTTP route",
                );
            }
            if !(400..=599).contains(&m.status) {
                self.error(
                    "E_SQL_MISSING_STATUS",
                    format!("{mp}/status"),
                    format!("`{}` is not an error status (400 to 599)", m.status),
                );
            }
            if m.message.trim().is_empty() {
                self.error(
                    "E_SQL_MISSING_MESSAGE",
                    format!("{mp}/message"),
                    "say what the client is told (a 5xx answers a generic text)",
                );
            }
        }
    }

    /// A value written or compared in an SQL step: typed after its column, or the database clock for a timestamp.
    fn sql_value(&mut self, scope: &Scope, value: &Expr, col: &Column, path: &str) {
        if matches!(value, Expr::Now) {
            if col.ty != ColumnType::Timestamp {
                self.error(
                    "E_NOW_COLUMN",
                    path,
                    format!(
                        "`now()` is a timestamp: column `{}` is {}",
                        col.name,
                        col.ty.sql()
                    ),
                );
            }
            return;
        }
        self.check_expr(scope, value, Some(&Self::column_type(col)), path);
    }

    /// The variable that receives the rows of a step: what a select reads, or what an insert, update or delete
    /// returns (`RETURNING`). Typed after the ticked columns, or after the schema they are converted into.
    fn sql_result(&mut self, path: &str, step: &Step, t: &Table, scope: &mut Scope) {
        let Step::Sql {
            id,
            table,
            bind,
            columns,
            many,
            into,
            ..
        } = step
        else {
            return;
        };
        for (i, name) in columns.iter().enumerate() {
            if t.column(name).is_none() {
                self.error(
                    "E_UNKNOWN_COLUMN",
                    format!("{path}/columns/{i}"),
                    format!("table `{table}` has no column `{name}`"),
                );
            }
        }
        let Some(var) = bind else {
            self.error(
                "E_SQL_BIND",
                format!("{path}/bind"),
                "name the variable that receives the result",
            );
            return;
        };
        self.check_var_name(var, &format!("{path}/bind"));
        let ty = match into {
            Some(schema) => {
                self.sql_into(path, table, columns, t, schema);
                TypeRef {
                    repeated: *many,
                    ..TypeRef::message(schema)
                }
            }
            None => {
                if *many {
                    self.error(
                        "E_SQL_MANY_INTO",
                        format!("{path}/into"),
                        "a list of rows has to be converted into a schema (`into`) to be used",
                    );
                }
                TypeRef::message(sql_row_name(table, id))
            }
        };
        scope.bind(var.clone(), ty);
    }

    /// `into`: every field of the schema is filled by the column of the same name, with the same type.
    fn sql_into(&mut self, path: &str, table: &str, columns: &[String], t: &Table, schema: &str) {
        let Some(msg) = self.project.message(schema) else {
            self.error(
                "E_UNKNOWN_MESSAGE",
                format!("{path}/into"),
                format!("unknown message `{schema}`"),
            );
            return;
        };
        if msg.kind == MessageKind::Row {
            return;
        }
        for f in &msg.fields {
            let ok = match t.column(&f.name).filter(|_| columns.contains(&f.name)) {
                None => Err(format!("no ticked column `{}` fills it", f.name)),
                Some(c) if f.repeated || f.ty != c.ty.field_type() => Err(format!(
                    "`{}` is a {} column, the field is {}",
                    c.name,
                    c.ty.sql(),
                    TypeRef::of_field(f)
                )),
                Some(c) if c.nullable && !f.optional => {
                    Err("the column can be null but the field is not optional".to_string())
                }
                Some(_) => Ok(()),
            };
            if let Err(why) = ok {
                self.error(
                    "E_SQL_INTO_FIELD",
                    format!("{path}/into"),
                    format!("`{schema}.{}` cannot be read from `{table}`: {why}", f.name),
                );
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

    /// The request and response of an HTTP route must be API schemas.
    fn require_api(&mut self, name: &str, path: &str) {
        if let Some(m) = self.project.message(name)
            && m.kind != MessageKind::Api
        {
            self.error(
                "E_ROUTE_SCHEMA_KIND",
                path,
                format!(
                    "`{name}` is {}: an HTTP route needs an API schema",
                    kind_label(m.kind)
                ),
            );
        }
    }

    /// A gRPC request or response must be a proto schema.
    fn require_proto(&mut self, name: &str, path: &str) {
        if let Some(m) = self.project.message(name)
            && m.kind != MessageKind::Proto
        {
            self.error(
                "E_RPC_MESSAGE_KIND",
                path,
                format!(
                    "`{name}` is {}: a gRPC request or response needs a proto schema",
                    kind_label(m.kind)
                ),
            );
        }
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
                optional,
                repeated,
                value,
                ..
            } => {
                let ty = TypeRef {
                    ty: value_type.clone(),
                    repeated: *repeated,
                    optional: *optional,
                };
                if *optional && *repeated {
                    self.error(
                        "E_LET_TYPE",
                        format!("{path}/optional"),
                        "a list is never optional: an empty list says there is nothing",
                    );
                }
                match value_type {
                    // A whole schema, built with Rust (queries over several tables, say), that a
                    // Respond step or a Publish can then read field by field.
                    FieldType::Message { name } => {
                        if p.message(name).is_none() {
                            self.error(
                                "E_UNKNOWN_MESSAGE",
                                format!("{path}/valueType"),
                                format!("unknown message `{name}`"),
                            );
                        }
                        if *optional {
                            self.error(
                                "E_LET_TYPE",
                                format!("{path}/optional"),
                                "a schema variable is never optional: its presence is implicit",
                            );
                        }
                        if !matches!(value, Expr::Raw { .. }) {
                            self.error(
                                "E_LET_VALUE",
                                format!("{path}/value"),
                                format!("a `{name}` variable is built with Rust code, e.g. `pb::{name} {{ .. }}`"),
                            );
                        }
                    }
                    _ if !TypeRef::scalar(value_type.clone()).is_simple_scalar() => self.error(
                        "E_LET_TYPE",
                        format!("{path}/valueType"),
                        "variables hold a string, uuid, bool, int32, int64, float, double (alone, optional or as a list), or a schema built with Rust",
                    ),
                    _ => {}
                }
                self.check_var_name(name, &format!("{path}/name"));
                self.check_expr(scope, value, Some(&ty), &format!("{path}/value"));
                scope.bind(name.clone(), ty);
                false
            }
            Step::Sql { .. } => {
                self.sql(env, path, step, scope);
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
                        if !t.producers.contains(&env.service.name) {
                            self.error(
                                "E_NOT_PRODUCER",
                                format!("{path}/topic"),
                                format!(
                                    "`{}` is not a producer of `{topic}`: declare it on the topic (draw the arrow from the service to the topic)",
                                    env.service.name
                                ),
                            );
                        }
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
            Step::Respond {
                fields,
                status,
                cookies,
                ..
            } => {
                match env.handler {
                    Handler::Consumer => self.error(
                        "E_RESPOND_IN_CONSUMER",
                        path,
                        "Respond only makes sense in an RPC or an HTTP route: consumers and jobs have nobody to answer",
                    ),
                    Handler::Rpc { response } => {
                        if let Some(msg) = p.message(response) {
                            self.assignments(msg, fields, scope, &format!("{path}/fields"));
                        }
                        if status.is_some() || !cookies.is_empty() {
                            self.error(
                                "E_RESPOND_ROUTE_ONLY",
                                path,
                                "a status and cookies are HTTP: they only make sense in an HTTP route",
                            );
                        }
                    }
                    Handler::Route { response } => {
                        if let Some(msg) = p.message(response) {
                            self.assignments(msg, fields, scope, &format!("{path}/fields"));
                        }
                        self.respond_extras(path, scope, *status, cookies);
                    }
                }
                true
            }
            Step::SignJwt {
                key_env,
                issuer,
                ttl_secs,
                claims,
                bind,
                ..
            } => {
                self.check_var_name(bind, &format!("{path}/bind"));
                self.jwt_key_env(&format!("{path}/keyEnv"), key_env);
                self.jwt_issuer(&format!("{path}/issuer"), issuer);
                if !(1..=2_592_000).contains(ttl_secs) {
                    self.error(
                        "E_JWT_TTL",
                        format!("{path}/ttlSecs"),
                        "a token lives 1 second to 30 days (2592000)",
                    );
                }
                let mut seen = HashSet::new();
                for (i, claim) in claims.iter().enumerate() {
                    let cp = format!("{path}/claims/{i}");
                    let name = claim.field.as_str();
                    let valid_name = name.starts_with(|c: char| c.is_ascii_lowercase())
                        && name.len() <= 32
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
                    if !valid_name {
                        self.error(
                            "E_JWT_CLAIM_NAME",
                            format!("{cp}/field"),
                            format!("claim name `{name}` must be lower case letters, digits and `_`, 32 characters at most"),
                        );
                    } else if matches!(name, "iss" | "iat" | "exp") {
                        self.error(
                            "E_JWT_CLAIM_RESERVED",
                            format!("{cp}/field"),
                            format!("`{name}` is set by the step: the issuer, and the issue and expiry times"),
                        );
                    }
                    if !seen.insert(name) {
                        self.error(
                            "E_JWT_CLAIM_DUPLICATE",
                            format!("{cp}/field"),
                            format!("claim `{name}` is set twice"),
                        );
                    }
                    // A claim is JSON: text, number, boolean, or a list or an optional of those.
                    if let Some(t) =
                        self.check_expr(scope, &claim.value, None, &format!("{cp}/value"))
                        && (t.is_message()
                            || matches!(t.ty, FieldType::Bytes | FieldType::Timestamp))
                    {
                        self.error(
                            "E_JWT_CLAIM_TYPE",
                            format!("{cp}/value"),
                            format!("claim `{name}` is a {t}: a claim is a text, a number, a boolean, or a list of those"),
                        );
                    }
                }
                if !seen.contains("sub") {
                    self.error(
                        "E_JWT_NO_SUB",
                        format!("{path}/claims"),
                        "an access token names who it is for: add a `sub` claim (verifying a token requires it)",
                    );
                }
                scope.bind(bind.clone(), TypeRef::scalar(FieldType::String));
                false
            }
            Step::VerifyJwt {
                token,
                source,
                issuer,
                schema,
                bind,
                ..
            } => {
                if !matches!(env.handler, Handler::Route { .. }) {
                    self.error(
                        "E_JWT_OUTSIDE_ROUTE",
                        path,
                        "a refused token is a 401: verifying one only makes sense in an HTTP route",
                    );
                }
                self.check_var_name(bind, &format!("{path}/bind"));
                self.jwt_issuer(&format!("{path}/issuer"), issuer);
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, token, Some(&text), &format!("{path}/token"));
                match source {
                    JwksSource::Keys { key_envs } => {
                        if key_envs.is_empty() {
                            self.error(
                                "E_JWT_KEYS",
                                format!("{path}/source/keyEnvs"),
                                "name at least one seed whose public key checks the token",
                            );
                        }
                        for (i, k) in key_envs.iter().enumerate() {
                            self.jwt_key_env(&format!("{path}/source/keyEnvs/{i}"), k);
                        }
                    }
                    JwksSource::Url { url_env } => {
                        self.jwt_key_env(&format!("{path}/source/urlEnv"), url_env)
                    }
                }
                match p.message(schema) {
                    Some(m) if m.kind == MessageKind::Api => {}
                    Some(_) => self.error(
                        "E_JWT_SCHEMA",
                        format!("{path}/schema"),
                        format!("`{schema}` is not an API schema: the claims are read as JSON"),
                    ),
                    None => self.error(
                        "E_JWT_SCHEMA",
                        format!("{path}/schema"),
                        format!("unknown schema `{schema}`: declare an API schema with the claims to read (`sub`, `roles`...)"),
                    ),
                }
                scope.bind(bind.clone(), TypeRef::message(schema));
                false
            }
            Step::Jwks { key_envs, bind, .. } => {
                self.check_var_name(bind, &format!("{path}/bind"));
                if key_envs.is_empty() {
                    self.error(
                        "E_JWT_KEYS",
                        format!("{path}/keyEnvs"),
                        "name at least one seed to publish the public key of",
                    );
                }
                for (i, k) in key_envs.iter().enumerate() {
                    self.jwt_key_env(&format!("{path}/keyEnvs/{i}"), k);
                }
                self.jwks_schema(path);
                scope.bind(bind.clone(), TypeRef::message("Jwks"));
                false
            }
            Step::Encrypt {
                key_env,
                input,
                bind,
                ..
            }
            | Step::Decrypt {
                key_env,
                input,
                bind,
                ..
            } => {
                self.check_var_name(bind, &format!("{path}/bind"));
                if !is_env_name(key_env) {
                    self.error(
                        "E_KEY_ENV",
                        format!("{path}/keyEnv"),
                        format!("`{key_env}` is not an environment variable name (e.g. `TOTP_ENCRYPTION_KEY`)"),
                    );
                }
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, input, Some(&text), &format!("{path}/input"));
                scope.bind(bind.clone(), text);
                false
            }
            Step::VerifyHmac {
                header,
                secret_env,
                tolerance_secs,
                ..
            } => {
                if !matches!(env.handler, Handler::Route { .. }) {
                    self.error(
                        "E_HMAC_OUTSIDE_ROUTE",
                        path,
                        "a signature is read from an HTTP header: it only makes sense in an HTTP route",
                    );
                }
                if header.is_empty()
                    || !header
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    self.error(
                        "E_HMAC_HEADER",
                        format!("{path}/header"),
                        format!("`{header}` is not a header name (letters, digits and `-`)"),
                    );
                }
                if !is_env_name(secret_env) {
                    self.error(
                        "E_HMAC_SECRET_ENV",
                        format!("{path}/secretEnv"),
                        format!("`{secret_env}` is not an environment variable name (e.g. `STRIPE_WEBHOOK_SECRET`)"),
                    );
                }
                if !(1..=86_400).contains(tolerance_secs) {
                    self.error(
                        "E_HMAC_TOLERANCE",
                        format!("{path}/toleranceSecs"),
                        "the clock tolerance is 1 second to 24 hours (86400)",
                    );
                }
                false
            }
            Step::RateLimit {
                key,
                max,
                window_secs,
                ..
            } => {
                if !matches!(env.handler, Handler::Route { .. }) {
                    self.error(
                        "E_RATE_LIMIT_OUTSIDE_ROUTE",
                        path,
                        "a rate limit answers 429 and `Retry-After`: it only makes sense in an HTTP route",
                    );
                }
                if *max == 0 {
                    self.error(
                        "E_RATE_LIMIT_MAX",
                        format!("{path}/max"),
                        "at least one attempt must be allowed",
                    );
                }
                if !(1..=86_400).contains(window_secs) {
                    self.error(
                        "E_RATE_LIMIT_WINDOW",
                        format!("{path}/windowSecs"),
                        "the window is 1 second to 24 hours (86400)",
                    );
                }
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, key, Some(&text), &format!("{path}/key"));
                false
            }
            Step::HashPassword { input, bind, .. } => {
                self.check_var_name(bind, &format!("{path}/bind"));
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, input, Some(&text), &format!("{path}/input"));
                scope.bind(bind.clone(), text);
                false
            }
            Step::VerifyPassword {
                password,
                stored,
                bind,
                ..
            } => {
                self.check_var_name(bind, &format!("{path}/bind"));
                let text = TypeRef::scalar(FieldType::String);
                // The stored hash may be absent: an unknown account.
                let maybe = TypeRef {
                    optional: true,
                    ..text.clone()
                };
                self.check_expr(scope, password, Some(&text), &format!("{path}/password"));
                self.check_expr(scope, stored, Some(&maybe), &format!("{path}/stored"));
                scope.bind(bind.clone(), TypeRef::scalar(FieldType::Bool));
                false
            }
            Step::RandomToken { bind, .. } => {
                self.check_var_name(bind, &format!("{path}/bind"));
                scope.bind(bind.clone(), TypeRef::scalar(FieldType::String));
                false
            }
            Step::Sha256 { input, bind, .. } => {
                self.check_var_name(bind, &format!("{path}/bind"));
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, input, Some(&text), &format!("{path}/input"));
                scope.bind(bind.clone(), text);
                false
            }
            Step::Fail {
                status,
                message,
                retry_after,
                ..
            } => {
                if !matches!(env.handler, Handler::Route { .. }) {
                    self.error(
                        "E_FAIL_OUTSIDE_ROUTE",
                        path,
                        "Fail answers an HTTP status: it only makes sense in an HTTP route",
                    );
                }
                if !(400..=599).contains(status) {
                    self.error(
                        "E_FAIL_STATUS",
                        format!("{path}/status"),
                        format!("`{status}` is not an error status (400 to 599)"),
                    );
                }
                if message.trim().is_empty() {
                    self.error(
                        "E_FAIL_MESSAGE",
                        format!("{path}/message"),
                        "say what the client is told (a 5xx answers a generic text and logs this one)",
                    );
                }
                if let Some(wait) = retry_after {
                    if !matches!(status, 429 | 503) {
                        self.error(
                            "E_FAIL_RETRY_AFTER",
                            format!("{path}/retryAfter"),
                            "`Retry-After` goes with 429 and 503 only",
                        );
                    }
                    let seconds = TypeRef::scalar(FieldType::Int64);
                    self.check_expr(scope, wait, Some(&seconds), &format!("{path}/retryAfter"));
                }
                // Nothing runs after it, like a Respond.
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

    /// The name of an environment variable.
    fn jwt_key_env(&mut self, path: &str, name: &str) {
        if !is_env_name(name) {
            self.error(
                "E_KEY_ENV",
                path,
                format!("`{name}` is not an environment variable name (e.g. `JWT_SIGNING_KEY`)"),
            );
        }
    }

    fn jwt_issuer(&mut self, path: &str, issuer: &str) {
        if issuer.trim().is_empty() || issuer.chars().count() > 100 {
            self.error(
                "E_JWT_ISSUER",
                path,
                "the issuer is a name of 1 to 100 characters, the same on the service that signs and on those that verify",
            );
        }
    }

    /// The step `jwks` fills the API schema `Jwks`, a list of `Jwk`: the shape of a JWKS (RFC 7517).
    fn jwks_schema(&mut self, path: &str) {
        let p = self.project;
        let ok = p.message("Jwks").is_some_and(|m| {
            m.kind == MessageKind::Api
                && m.field("keys").is_some_and(|f| {
                    f.repeated && matches!(&f.ty, FieldType::Message { name } if name == "Jwk")
                })
        }) && p.message("Jwk").is_some_and(|m| {
            m.kind == MessageKind::Api
                && ["kty", "crv", "alg", "kid", "x"].iter().all(|n| {
                    m.field(n)
                        .is_some_and(|f| f.ty == FieldType::String && !f.repeated && !f.optional)
                })
        });
        if !ok {
            self.error(
                "E_JWKS_SCHEMA",
                path,
                "publishing a JWKS needs the API schemas `Jwks` (`keys`: a list of `Jwk`) and `Jwk` (`kty`, `crv`, `alg`, `kid`, `x`: texts)",
            );
        }
    }

    /// A signed webhook is checked on the raw body, before it is read: the step is the first of the route, only
    /// once, and the route carries a body.
    fn hmac_placement(&mut self, route: &Route, steps_path: &str) {
        let mut count = 0;
        for step in &route.steps {
            step.walk(&mut |s| count += usize::from(matches!(s, Step::VerifyHmac { .. })));
        }
        if count == 0 {
            return;
        }
        if !matches!(route.steps.first(), Some(Step::VerifyHmac { .. })) {
            self.error(
                "E_HMAC_POSITION",
                steps_path,
                "the signature is checked before anything else: make it the first step of the route, not inside a condition",
            );
        }
        if count > 1 {
            self.error(
                "E_HMAC_DUPLICATE",
                steps_path,
                "a route checks one signature",
            );
        }
        if !route.method.has_body() {
            self.error(
                "E_HMAC_METHOD",
                steps_path,
                format!(
                    "a signature covers the body: a {} route has none (use POST, PUT or PATCH)",
                    route.method.as_str()
                ),
            );
        }
    }

    /// The status and the cookies a route can answer with.
    fn respond_extras(
        &mut self,
        path: &str,
        scope: &Scope,
        status: Option<u16>,
        cookies: &[Cookie],
    ) {
        if let Some(s) = status
            && !matches!(s, 200..=202)
        {
            self.error(
                "E_RESPOND_STATUS",
                format!("{path}/status"),
                format!("`{s}` is not a success status of a route (200, 201 or 202)"),
            );
        }
        let text = TypeRef::scalar(FieldType::String);
        let mut names = HashSet::new();
        for (i, c) in cookies.iter().enumerate() {
            let cp = format!("{path}/cookies/{i}");
            // The characters of an HTTP token, the safe part of them.
            if c.name.is_empty()
                || !c
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            {
                self.error(
                    "E_COOKIE_NAME",
                    format!("{cp}/name"),
                    format!(
                        "cookie name `{}` must be letters, digits, `_`, `-` or `.`",
                        c.name
                    ),
                );
            }
            if !names.insert(c.name.as_str()) {
                self.error(
                    "E_COOKIE_DUPLICATE",
                    format!("{cp}/name"),
                    format!("cookie `{}` is set twice", c.name),
                );
            }
            if c.same_site == SameSite::None && !c.secure {
                self.error(
                    "E_COOKIE_SAMESITE",
                    format!("{cp}/sameSite"),
                    "`SameSite=None` is only accepted by browsers on a `Secure` cookie",
                );
            }
            if let Some(p) = &c.path
                && (!p.starts_with('/') || p.bytes().any(|b| b <= b' ' || b == b';' || b == 0x7f))
            {
                self.error(
                    "E_COOKIE_PATH",
                    format!("{cp}/path"),
                    "a cookie path starts with `/` and has no space, `;` or control character",
                );
            }
            self.check_expr(scope, &c.value, Some(&text), &format!("{cp}/value"));
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
            Expr::Now => {
                self.error(
                    "E_NOW_OUTSIDE_SQL",
                    path,
                    "`now()` is the database clock: it only fills a timestamp column of an SQL step",
                );
                None
            }
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
                    && !l.accepts(r)
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
                        Expr::Now => self.error(
                            "E_NOW_OUTSIDE_SQL",
                            format!("{path}/{name}"),
                            "`now()` is the database clock: it only fills a timestamp column of an SQL step",
                        ),
                        _ => {}
                    }
                }
            }
        }
    }

    // ---- topology --------------------------------------------------------------------

    fn graph_warnings(&mut self) {
        let p = self.project;
        // (service, topic) pairs with a Publish step.
        let mut published: BTreeSet<(&str, &str)> = BTreeSet::new();
        for s in &p.services {
            for step in s.handler_steps() {
                step.walk(&mut |st| {
                    if let Step::Publish { topic, .. } = st {
                        published.insert((s.name.as_str(), topic.as_str()));
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
            if t.producers.is_empty() {
                self.warn(
                    "W_TOPIC_NO_PRODUCER",
                    format!("/topics/{i}"),
                    format!("no service produces topic `{}`", t.name),
                );
            }
            for (j, producer) in t.producers.iter().enumerate() {
                if p.service(producer).is_some()
                    && !published.contains(&(producer.as_str(), t.name.as_str()))
                {
                    self.warn(
                        "W_PRODUCER_NEVER_PUBLISHES",
                        format!("/topics/{i}/producers/{j}"),
                        format!(
                            "`{producer}` is a producer of `{}` but none of its steps publishes there",
                            t.name
                        ),
                    );
                }
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
    fn the_showcase_is_valid_without_warnings_and_shows_every_step_type() {
        let p = sample::showcase();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        let mut kinds = std::collections::BTreeSet::new();
        for s in &p.services {
            for step in s.handler_steps() {
                step.walk(&mut |x| {
                    kinds.insert(
                        serde_json::to_value(x).unwrap()["type"]
                            .as_str()
                            .unwrap()
                            .to_string(),
                    );
                });
            }
        }
        // A new step type has to be added to the showcase, or an agent never sees it used.
        let schema = json_schema();
        let all: Vec<String> = schema["$defs"]["Step"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                v["properties"]["type"]["const"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        let missing: Vec<_> = all.iter().filter(|t| !kinds.contains(*t)).collect();
        assert!(missing.is_empty(), "the showcase never uses {missing:?}");
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
            kind: Default::default(),
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
            kind: Default::default(),
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
    fn topics_carry_events_and_rpcs_use_proto_schemas() {
        let mut p = sample::orders();
        assert!(errors(&p).iter().all(|c| !c.contains("KIND")));
        // A proto schema on a topic.
        p.topics[0].message = "Customer".into();
        assert!(errors(&p).contains(&"E_TOPIC_MESSAGE_KIND".into()));
        p.topics[0].message = "OrderCreated".into();
        // An event as a gRPC response.
        p.services[0].rpcs[0].response = "OrderCreated".into();
        assert!(errors(&p).contains(&"E_RPC_MESSAGE_KIND".into()));
    }

    #[test]
    fn api_schemas_stay_apart_from_protobuf_ones() {
        let mut p = sample::orders();
        let mut api = p.messages[0].clone();
        api.name = "CustomerDto".into();
        api.kind = MessageKind::Api;
        p.messages.push(api);
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        // An API schema is not a topic payload, nor a gRPC message, and cannot be nested in them.
        p.topics[0].message = "CustomerDto".into();
        assert!(errors(&p).contains(&"E_TOPIC_MESSAGE_KIND".into()));
        p.topics[0].message = "OrderCreated".into();
        p.messages[5].fields[1].ty = FieldType::Message {
            name: "CustomerDto".into(),
        };
        assert!(errors(&p).contains(&"E_SCHEMA_KIND_MIX".into()));
        p.messages[5].fields[1].ty = FieldType::Message {
            name: "Customer".into(),
        };
        // ...and the reverse.
        let last = p.messages.len() - 1;
        p.messages[last].fields[0].ty = FieldType::Message {
            name: "Customer".into(),
        };
        assert!(errors(&p).contains(&"E_SCHEMA_KIND_MIX".into()));
    }

    #[test]
    fn a_schema_cannot_take_the_name_of_a_service() {
        let mut p = sample::orders();
        p.messages.push(Message {
            name: "Orders".into(),
            kind: MessageKind::Api,
            fields: vec![],
        });
        assert!(errors(&p).contains(&"E_NAME_CLASH".into()));
    }

    fn route_of(p: &mut Project) -> &mut Route {
        &mut p.services[0].routes[0]
    }

    #[test]
    fn the_sample_routes_are_valid() {
        assert!(errors(&sample::orders()).is_empty());
    }

    #[test]
    fn routes_need_an_http_port_that_is_free() {
        let mut p = sample::orders();
        p.services[0].http_port = None;
        assert!(errors(&p).contains(&"E_HTTP_PORT".into()));
        p.services[0].http_port = Some(50052); // Payments' gRPC port
        assert!(errors(&p).contains(&"E_DUPLICATE_PORT".into()));
        p.services[0].http_port = Some(8081);
        assert!(errors(&p).is_empty());
    }

    #[test]
    fn route_paths_methods_and_names_are_checked() {
        let mut p = sample::orders();
        route_of(&mut p).path = "orders".into();
        assert!(errors(&p).contains(&"E_ROUTE_PATH".into()));
        route_of(&mut p).path = "/orders".into();

        route_of(&mut p).name = "PlaceOrder".into();
        assert!(errors(&p).contains(&"E_ROUTE_NAME".into()));
        route_of(&mut p).name = "get_order".into();
        assert!(errors(&p).contains(&"E_DUPLICATE_ROUTE_NAME".into()));
        route_of(&mut p).name = "place_order".into();

        // Same method and same shape, whatever the parameter is called.
        let mut other = p.services[0].routes[1].clone();
        other.name = "get_order_again".into();
        other.path = "/orders/{order_id}".into();
        p.services[0].routes.push(other);
        assert!(errors(&p).contains(&"E_DUPLICATE_ROUTE".into()));
        // Another method on the same path is fine.
        p.services[0].routes[2].method = HttpMethod::Delete;
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));

        route_of(&mut p).status = 404;
        assert!(errors(&p).contains(&"E_ROUTE_STATUS".into()));
    }

    #[test]
    fn route_schemas_must_be_api_schemas() {
        let mut p = sample::orders();
        route_of(&mut p).request = "CreateOrderRequest".into();
        assert!(errors(&p).contains(&"E_ROUTE_SCHEMA_KIND".into()));
        route_of(&mut p).request = "PlaceOrderBody".into();
        route_of(&mut p).response = "OrderCreated".into();
        assert!(errors(&p).contains(&"E_ROUTE_SCHEMA_KIND".into()));
    }

    #[test]
    fn path_parameters_fill_scalar_required_fields() {
        let mut p = sample::orders();
        p.services[0].routes[1].path = "/orders/{id}".into();
        assert!(
            errors(&p).contains(&"E_ROUTE_PARAM".into()),
            "no such field"
        );
        p.services[0].routes[1].path = "/orders/{order_id}".into();
        let field = |p: &mut Project| {
            p.messages
                .iter_mut()
                .find(|m| m.name == "GetOrderRequest")
                .unwrap()
                .fields
                .remove(0)
        };
        let mut f = field(&mut p);
        let restore = |p: &mut Project, f: Field| {
            p.messages
                .iter_mut()
                .find(|m| m.name == "GetOrderRequest")
                .unwrap()
                .fields
                .insert(0, f)
        };
        f.repeated = true;
        restore(&mut p, f.clone());
        assert!(errors(&p).contains(&"E_ROUTE_PARAM".into()), "repeated");
        field(&mut p);
        f.repeated = false;
        f.optional = true;
        restore(&mut p, f);
        assert!(errors(&p).contains(&"E_ROUTE_PARAM".into()), "optional");
    }

    #[test]
    fn a_query_string_only_carries_single_scalars() {
        let mut p = sample::orders();
        let get = p
            .messages
            .iter_mut()
            .find(|m| m.name == "GetOrderRequest")
            .unwrap();
        let mut tags = Field::new("tags", FieldType::String);
        tags.repeated = true;
        get.fields.push(tags);
        assert!(errors(&p).contains(&"E_ROUTE_QUERY_FIELD".into()));
        // The same schema is fine behind a body.
        p.services[0].routes[1].method = HttpMethod::Put;
        assert!(!errors(&p).contains(&"E_ROUTE_QUERY_FIELD".into()));
    }

    #[test]
    fn api_schemas_have_no_bytes_or_timestamp_yet() {
        let mut p = sample::orders();
        let view = p
            .messages
            .iter_mut()
            .find(|m| m.name == "OrderView")
            .unwrap();
        view.fields
            .push(Field::new("created_at", FieldType::Timestamp));
        assert!(errors(&p).contains(&"E_API_FIELD_TYPE".into()));
    }

    #[test]
    fn route_steps_are_checked_like_rpc_steps() {
        let mut p = sample::orders();
        // `tx` is reserved, publishing needs the producer declaration, a route answers with Respond.
        let publish = p.services[0].routes[0]
            .steps
            .iter_mut()
            .find(|s| matches!(s, Step::Publish { .. }))
            .unwrap();
        let Step::Publish { topic, .. } = publish else {
            panic!("sample layout changed")
        };
        *topic = "payments.completed".into();
        assert!(errors(&p).contains(&"E_NOT_PRODUCER".into()));
        let steps = &mut p.services[0].routes[1].steps;
        steps.pop();
        let d = validate(&p);
        assert!(
            d.iter()
                .any(|d| d.code == "W_NO_RESPOND" && d.path == "/services/0/routes/1")
        );
    }

    #[test]
    fn outbox_attempts_must_be_in_range() {
        let mut p = sample::orders();
        assert!(!errors(&p).contains(&"E_OUTBOX_ATTEMPTS".into()));
        for bad in [0, MAX_OUTBOX_ATTEMPTS + 1] {
            p.outbox.max_attempts = bad;
            assert!(errors(&p).contains(&"E_OUTBOX_ATTEMPTS".into()), "{bad}");
        }
        p.outbox.max_attempts = 1;
        assert!(!errors(&p).contains(&"E_OUTBOX_ATTEMPTS".into()));
    }

    #[test]
    fn dead_letter_suffix_is_reserved() {
        let mut p = sample::orders();
        p.topics[0].name = "orders.created.dlq".into();
        assert!(errors(&p).contains(&"E_TOPIC_RESERVED".into()));
    }

    #[test]
    fn a_variable_can_be_a_uuid_optional_or_a_list() {
        let mut p = sample::orders();
        let var = |ty: FieldType, optional: bool, repeated: bool, value: Expr| Step::Let {
            id: "v".into(),
            name: "x".into(),
            value_type: ty,
            optional,
            repeated,
            value,
        };
        let lit = |v: serde_json::Value| Expr::Literal { value: v };
        let id = "123e4567-e89b-12d3-a456-426614174000";
        let mut check = |step: Step, code: Option<&str>| {
            p.services[0].routes[0].steps.insert(0, step);
            let found = errors(&p);
            p.services[0].routes[0].steps.remove(0);
            match code {
                None => assert!(found.is_empty(), "{found:?}"),
                Some(c) => assert!(found.contains(&c.to_string()), "{c}: {found:?}"),
            }
        };
        check(var(FieldType::Uuid, false, false, lit(id.into())), None);
        check(
            var(FieldType::Uuid, false, false, lit("nope".into())),
            Some("E_TYPE_MISMATCH"),
        );
        check(
            var(FieldType::Int64, true, false, lit(serde_json::Value::Null)),
            None,
        );
        check(var(FieldType::Int64, true, false, lit(4.into())), None);
        check(
            var(FieldType::Int64, false, false, lit(serde_json::Value::Null)),
            Some("E_TYPE_MISMATCH"),
        );
        check(
            var(
                FieldType::String,
                false,
                true,
                lit(serde_json::json!(["a", "b"])),
            ),
            None,
        );
        check(
            var(
                FieldType::String,
                false,
                true,
                lit(serde_json::json!(["a", 1])),
            ),
            Some("E_TYPE_MISMATCH"),
        );
        // A list of the request's items fills a list; a single string does not.
        check(
            var(
                FieldType::String,
                false,
                true,
                Expr::Path {
                    path: "input.items".into(),
                },
            ),
            None,
        );
        check(
            var(
                FieldType::String,
                false,
                false,
                Expr::Path {
                    path: "input.items".into(),
                },
            ),
            Some("E_TYPE_MISMATCH"),
        );
        // Never both, and a schema is never optional.
        check(
            var(FieldType::String, true, true, lit(serde_json::json!([]))),
            Some("E_LET_TYPE"),
        );
        check(
            var(
                FieldType::Message {
                    name: "OrderView".into(),
                },
                true,
                false,
                Expr::Raw {
                    code: "pb::OrderView::default()".into(),
                },
            ),
            Some("E_LET_TYPE"),
        );
        // The variable has the shape it was declared with: reading it as a plain string is refused.
        p.services[0].routes[0].steps.insert(
            0,
            var(FieldType::String, true, false, lit(serde_json::Value::Null)),
        );
        if let Step::Respond { fields, .. } = p.services[0].routes[0].steps.last_mut().unwrap() {
            fields[0].value = Expr::Path { path: "x".into() };
        }
        assert!(
            errors(&p).contains(&"E_TYPE_MISMATCH".to_string()),
            "{:?}",
            errors(&p)
        );
    }

    #[test]
    fn a_variable_can_hold_a_schema_built_with_rust() {
        let mut p = sample::orders();
        let let_step = |value_type: FieldType, value: Expr| Step::Let {
            id: "v".into(),
            name: "view".into(),
            value_type,
            optional: false,
            repeated: false,
            value,
        };
        let view = |p: &Project| p.message("OrderView").is_some();
        assert!(view(&p));
        let message = || FieldType::Message {
            name: "OrderView".into(),
        };
        let raw = |code: &str| Expr::Raw { code: code.into() };

        p.services[0].routes[1]
            .steps
            .insert(0, let_step(message(), raw("pb::OrderView::default()")));
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));

        // Only Rust code can build it.
        p.services[0].routes[1].steps[0] = let_step(message(), Expr::Literal { value: "x".into() });
        assert!(errors(&p).contains(&"E_LET_VALUE".into()));
        // The schema must exist.
        p.services[0].routes[1].steps[0] = let_step(
            FieldType::Message {
                name: "Nope".into(),
            },
            raw("pb::Nope::default()"),
        );
        assert!(errors(&p).contains(&"E_UNKNOWN_MESSAGE".into()));
        // Other types are still refused.
        p.services[0].routes[1].steps[0] = let_step(FieldType::Bytes, raw("vec![]"));
        assert!(errors(&p).contains(&"E_LET_TYPE".into()));
    }

    #[test]
    fn a_schema_variable_feeds_the_response() {
        let mut p = sample::orders();
        p.services[0].routes[1].steps = vec![
            Step::Let {
                id: "v".into(),
                name: "view".into(),
                value_type: FieldType::Message {
                    name: "OrderView".into(),
                },
                optional: false,
                repeated: false,
                value: Expr::Raw {
                    code: "pb::OrderView::default()".into(),
                },
            },
            Step::Respond {
                id: "r".into(),
                fields: vec![
                    Assignment {
                        field: "order_id".into(),
                        value: Expr::Path {
                            path: "view.order_id".into(),
                        },
                    },
                    Assignment {
                        field: "status".into(),
                        value: Expr::Path {
                            path: "view.status".into(),
                        },
                    },
                ],
                status: None,
                cookies: vec![],
            },
        ];
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
    }

    fn sql_step(p: &mut Project, route: usize, at: usize) -> &mut Step {
        &mut p.services[0].routes[route].steps[at]
    }

    #[test]
    fn tables_are_checked() {
        let mut p = sample::orders();
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        let table = |p: &mut Project| p.services[0].tables[0].clone();
        let mut t = table(&mut p);
        t.name = "Orders".into();
        p.services[0].tables = vec![t.clone()];
        assert!(errors(&p).contains(&"E_TABLE_NAME".into()));
        t.name = "outbox".into();
        p.services[0].tables = vec![t.clone()];
        assert!(errors(&p).contains(&"E_TABLE_RESERVED".into()));
        t.name = "orders".into();
        p.services[0].tables = vec![t.clone(), t.clone()];
        assert!(errors(&p).contains(&"E_DUPLICATE_TABLE".into()));
        t.columns.push(t.columns[0].clone());
        p.services[0].tables = vec![t.clone()];
        assert!(errors(&p).contains(&"E_DUPLICATE_COLUMN".into()));
        t.columns.pop();
        t.columns[0].nullable = true;
        p.services[0].tables = vec![t.clone()];
        assert!(errors(&p).contains(&"E_PK_NULLABLE".into()));
        t.columns[0].nullable = false;
        t.columns[2].default = Some("'x'; drop table orders".into());
        p.services[0].tables = vec![t.clone()];
        assert!(errors(&p).contains(&"E_COLUMN_DEFAULT".into()));
        t.columns.clear();
        p.services[0].tables = vec![t];
        assert!(errors(&p).contains(&"E_TABLE_EMPTY".into()));
    }

    /// `order_lines` (keyed by `order_id, line_no`) and `shipments`, which points at one line.
    fn with_composite_key() -> Project {
        let mut p = sample::orders();
        let key = |name: &str, ty| {
            let mut c = Column::new(name, ty);
            c.primary_key = true;
            c
        };
        let mut payload = Column::new("payload", ColumnType::Jsonb);
        payload.default = Some("'{}'::jsonb".into());
        let lines = Table::new(
            "order_lines",
            vec![
                key("order_id", ColumnType::String),
                key("line_no", ColumnType::Int32),
                payload,
            ],
        );
        let mut shipments = Table::new(
            "shipments",
            vec![
                key("id", ColumnType::Uuid),
                Column::new("order_id", ColumnType::String),
                Column::new("line_no", ColumnType::Int32),
            ],
        );
        shipments.foreign_keys.push(ForeignKey {
            name: "shipments_line_fk".into(),
            columns: vec!["order_id".into(), "line_no".into()],
            ref_table: "order_lines".into(),
            ref_columns: vec!["order_id".into(), "line_no".into()],
            on_delete: ReferentialAction::Cascade,
            on_update: ReferentialAction::NoAction,
        });
        shipments.indexes.push(Index {
            name: "shipments_order_idx".into(),
            columns: vec!["order_id".into()],
            unique: false,
        });
        shipments.uniques.push(UniqueConstraint {
            name: "shipments_line_uq".into(),
            columns: vec!["order_id".into(), "line_no".into()],
        });
        p.services[0].tables.extend([lines, shipments]);
        p
    }

    #[test]
    fn composite_foreign_keys_indexes_and_uniques_are_valid() {
        let p = with_composite_key();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
    }

    #[test]
    fn foreign_keys_are_checked() {
        fn fk(p: &mut Project) -> &mut ForeignKey {
            &mut p.services[0].tables[2].foreign_keys[0]
        }
        // (expected code, how to break the key, what it breaks)
        type Case = (&'static str, fn(&mut ForeignKey), &'static str);
        let cases: Vec<Case> = vec![
            (
                "E_FK_UNKNOWN_TABLE",
                |f| f.ref_table = "nowhere".into(),
                "table",
            ),
            (
                "E_FK_ARITY",
                |f| {
                    f.ref_columns.pop();
                },
                "arity",
            ),
            (
                "E_CONSTRAINT_UNKNOWN_COLUMN",
                |f| f.columns[0] = "ghost".into(),
                "column",
            ),
            (
                "E_CONSTRAINT_UNKNOWN_COLUMN",
                |f| f.ref_columns[0] = "ghost".into(),
                "ref column",
            ),
            (
                "E_CONSTRAINT_DUPLICATE_COLUMN",
                |f| {
                    f.columns[1] = "order_id".into();
                    f.ref_columns[1] = "order_id".into();
                },
                "twice",
            ),
            // Half of the key does not identify a line.
            (
                "E_FK_NOT_UNIQUE",
                |f| {
                    f.columns.pop();
                    f.ref_columns.pop();
                },
                "partial key",
            ),
            // `line_no` is an integer, `order_id` text.
            (
                "E_FK_TYPE",
                |f| {
                    f.ref_columns.swap(0, 1);
                },
                "type",
            ),
            (
                "E_FK_SET_NULL",
                |f| f.on_delete = ReferentialAction::SetNull,
                "set null",
            ),
            (
                "E_FK_SET_DEFAULT",
                |f| f.on_update = ReferentialAction::SetDefault,
                "set default",
            ),
            ("E_CONSTRAINT_NAME", |f| f.name = "Bad-Name".into(), "name"),
        ];
        for (code, change, what) in cases {
            let mut p = with_composite_key();
            change(fk(&mut p));
            assert!(
                errors(&p).contains(&code.to_string()),
                "{what}: {:?}",
                errors(&p)
            );
        }
    }

    #[test]
    fn a_foreign_key_may_reference_a_unique_set_in_any_order() {
        let mut p = with_composite_key();
        // Not the primary key of the target, but a UNIQUE constraint on the same columns.
        for c in &mut p.services[0].tables[1].columns[..2] {
            c.primary_key = false;
        }
        assert!(errors(&p).contains(&"E_FK_NOT_UNIQUE".to_string()));
        p.services[0].tables[1].uniques.push(UniqueConstraint {
            name: "order_lines_uq".into(),
            columns: vec!["line_no".into(), "order_id".into()],
        });
        assert!(
            !errors(&p).contains(&"E_FK_NOT_UNIQUE".to_string()),
            "{:?}",
            errors(&p)
        );
    }

    #[test]
    fn indexes_and_uniques_are_checked() {
        let mut p = with_composite_key();
        let t = &mut p.services[0].tables[2];
        t.indexes[0].columns = vec![];
        assert!(errors(&p).contains(&"E_CONSTRAINT_NO_COLUMNS".to_string()));
        let t = &mut p.services[0].tables[2];
        t.indexes[0].columns = vec!["ghost".into()];
        assert!(errors(&p).contains(&"E_CONSTRAINT_UNKNOWN_COLUMN".to_string()));
        // Constraints and indexes share one namespace with the tables.
        let t = &mut p.services[0].tables[2];
        t.indexes[0].columns = vec!["order_id".into()];
        t.indexes[0].name = "shipments_line_uq".into();
        assert!(errors(&p).contains(&"E_DUPLICATE_CONSTRAINT".to_string()));
        let t = &mut p.services[0].tables[2];
        t.indexes[0].name = "orders".into();
        assert!(errors(&p).contains(&"E_DUPLICATE_CONSTRAINT".to_string()));
        let t = &mut p.services[0].tables[2];
        t.indexes[0].name = "x".repeat(64);
        assert!(errors(&p).contains(&"E_CONSTRAINT_NAME".to_string()));
    }

    fn fail(status: u16, retry_after: Option<Expr>) -> Step {
        Step::Fail {
            id: "f1".into(),
            status,
            message: "too many attempts".into(),
            retry_after,
        }
    }

    #[test]
    fn fail_ends_a_route_and_is_checked() {
        let mut p = sample::orders();
        let seconds = |path: &str| Expr::Path { path: path.into() };
        // A route that always ends with a Fail does not warn about a missing Respond.
        route_of(&mut p).steps = vec![fail(429, Some(Expr::Literal { value: 30.into() }))];
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));

        // Only error statuses, a message, and `Retry-After` on 429 / 503.
        for (step, code) in [
            (fail(200, None), "E_FAIL_STATUS"),
            (fail(600, None), "E_FAIL_STATUS"),
            (
                fail(409, Some(seconds("input.amount"))),
                "E_FAIL_RETRY_AFTER",
            ),
            (fail(429, Some(seconds("input.nothing"))), "E_UNKNOWN_PATH"),
            (fail(429, Some(seconds("input.items"))), "E_TYPE_MISMATCH"),
        ] {
            route_of(&mut p).steps = vec![step];
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
        let mut blank = fail(400, None);
        if let Step::Fail { message, .. } = &mut blank {
            message.clear();
        }
        route_of(&mut p).steps = vec![blank];
        assert!(errors(&p).contains(&"E_FAIL_MESSAGE".to_string()));
    }

    #[test]
    fn fail_only_makes_sense_in_a_route() {
        let mut p = sample::orders();
        // An RPC and a consumer answer with a gRPC status or a retry, not an HTTP one.
        p.services[0].rpcs[0].steps.insert(0, fail(400, None));
        assert!(errors(&p).contains(&"E_FAIL_OUTSIDE_ROUTE".to_string()));
        let mut p = sample::orders();
        p.services[1].consumers[0].steps.insert(0, fail(400, None));
        assert!(errors(&p).contains(&"E_FAIL_OUTSIDE_ROUTE".to_string()));
    }

    fn path(p: &str) -> Expr {
        Expr::Path { path: p.into() }
    }

    #[test]
    fn password_and_token_steps_bind_typed_variables() {
        let mut p = sample::orders();
        // Hash the token's hash, verify it, and read the results: a `string` and a `bool`.
        route_of(&mut p).steps.splice(
            0..0,
            [
                Step::RandomToken {
                    id: "a".into(),
                    bind: "token".into(),
                },
                Step::Sha256 {
                    id: "b".into(),
                    input: path("token"),
                    bind: "digest".into(),
                },
                Step::HashPassword {
                    id: "c".into(),
                    input: path("digest"),
                    bind: "hash".into(),
                },
                // The stored hash may be a plain string, an optional one, or absent.
                Step::VerifyPassword {
                    id: "d".into(),
                    password: path("token"),
                    stored: path("hash"),
                    bind: "ok".into(),
                },
                Step::VerifyPassword {
                    id: "e".into(),
                    password: path("token"),
                    stored: Expr::Literal {
                        value: serde_json::Value::Null,
                    },
                    bind: "ghost_ok".into(),
                },
                Step::Branch {
                    id: "f".into(),
                    condition: Condition::Compare {
                        left: path("ok"),
                        op: CmpOp::Eq,
                        right: Expr::Literal { value: true.into() },
                    },
                    then: vec![],
                    otherwise: vec![],
                },
            ],
        );
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));

        // A number is not a password; `ok` is a bool, not a text.
        route_of(&mut p).steps[2] = Step::HashPassword {
            id: "c".into(),
            input: path("input.amount"),
            bind: "hash".into(),
        };
        assert!(
            errors(&p).contains(&"E_TYPE_MISMATCH".to_string()),
            "{:?}",
            errors(&p)
        );
        route_of(&mut p).steps[2] = Step::HashPassword {
            id: "c".into(),
            input: path("ok"),
            bind: "hash".into(),
        };
        assert!(
            errors(&p).contains(&"E_UNKNOWN_PATH".to_string()),
            "`ok` is bound later: {:?}",
            errors(&p)
        );
        // Names follow the variable rules.
        route_of(&mut p).steps[0] = Step::RandomToken {
            id: "a".into(),
            bind: "Bad-Name".into(),
        };
        assert!(errors(&p).contains(&"E_VAR_NAME".to_string()));
        route_of(&mut p).steps[0] = Step::RandomToken {
            id: "a".into(),
            bind: "tx".into(),
        };
        assert!(errors(&p).contains(&"E_RESERVED_NAME".to_string()));
    }

    #[test]
    fn a_stored_hash_can_come_from_an_optional_column() {
        let mut p = sample::orders();
        // `get_order` reads `status`. Make it nullable, as the hash column of a table whose rows may lack one;
        // the response then gives its own status.
        p.services[0].tables[0].columns[2].nullable = true;
        p.services[0].tables[0].columns[2].default = None;
        let route = &mut p.services[0].routes[1];
        if let Step::Respond { fields, .. } = &mut route.steps[1] {
            fields[1].value = Expr::Literal { value: "x".into() };
        }
        route.steps.insert(
            1,
            Step::VerifyPassword {
                id: "v".into(),
                password: path("input.order_id"),
                stored: path("order.status"),
                bind: "ok".into(),
            },
        );
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
    }

    fn respond_of(p: &mut Project) -> &mut Step {
        route_of(p).steps.last_mut().unwrap()
    }

    #[test]
    fn a_route_can_set_a_status_and_cookies() {
        let mut p = sample::orders();
        assert_eq!(
            validate(&p),
            vec![],
            "the sample sets both: {:?}",
            validate(&p)
        );
        let set = |p: &mut Project, f: &dyn Fn(&mut Cookie)| {
            let Step::Respond { cookies, .. } = respond_of(p) else {
                panic!("layout changed")
            };
            f(&mut cookies[0]);
        };
        // (expected code, how to break the cookie)
        type Case<'a> = (&'a str, &'a dyn Fn(&mut Cookie));
        let cases: [Case; 6] = [
            ("E_COOKIE_NAME", &|c| c.name = "bad name;".into()),
            ("E_COOKIE_NAME", &|c| c.name.clear()),
            ("E_COOKIE_SAMESITE", &|c| {
                (c.same_site, c.secure) = (SameSite::None, false)
            }),
            ("E_COOKIE_PATH", &|c| c.path = Some("no-slash".into())),
            ("E_COOKIE_PATH", &|c| c.path = Some("/a; Domain=x".into())),
            ("E_UNKNOWN_PATH", &|c| {
                c.value = Expr::Path {
                    path: "nope".into(),
                }
            }),
        ];
        for (code, change) in cases {
            let mut p = p.clone();
            set(&mut p, change);
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
        // `SameSite=None` is fine on a Secure cookie; a value must be text; names are not repeated.
        set(&mut p, &|c| c.same_site = SameSite::None);
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        set(&mut p, &|c| {
            c.value = Expr::Path {
                path: "input.amount".into(),
            }
        });
        assert!(errors(&p).contains(&"E_TYPE_MISMATCH".to_string()));
        set(&mut p, &|c| {
            c.value = Expr::Path {
                path: "order_id".into(),
            }
        });
        if let Step::Respond { cookies, .. } = respond_of(&mut p) {
            cookies.push(cookies[0].clone());
        }
        assert!(errors(&p).contains(&"E_COOKIE_DUPLICATE".to_string()));
        // Only success statuses.
        if let Step::Respond {
            cookies, status, ..
        } = respond_of(&mut p)
        {
            cookies.pop();
            *status = Some(404);
        }
        assert!(errors(&p).contains(&"E_RESPOND_STATUS".to_string()));
    }

    #[test]
    fn a_status_and_cookies_are_for_routes_only() {
        let mut p = sample::orders();
        let step = Step::Respond {
            id: "r".into(),
            fields: vec![],
            status: Some(201),
            cookies: vec![],
        };
        p.services[0].rpcs[0].steps.push(step);
        assert!(
            errors(&p).contains(&"E_RESPOND_ROUTE_ONLY".to_string()),
            "{:?}",
            errors(&p)
        );
    }

    #[test]
    fn a_rate_limit_is_checked() {
        let mut p = sample::orders();
        let limit = |max: u32, window_secs: u32, key: Expr| Step::RateLimit {
            id: "rl".into(),
            key,
            max,
            window_secs,
        };
        let lit = |v: &str| Expr::Literal { value: v.into() };
        let mut check = |step: Step, code: Option<&str>| {
            route_of(&mut p).steps.insert(0, step);
            let found = errors(&p);
            route_of(&mut p).steps.remove(0);
            match code {
                None => assert!(found.is_empty(), "{found:?}"),
                Some(c) => assert!(found.contains(&c.to_string()), "{c}: {found:?}"),
            }
        };
        check(limit(5, 900, lit("k")), None);
        check(
            limit(
                5,
                900,
                Expr::Raw {
                    code: "input.items.join(\",\")".into(),
                },
            ),
            None,
        );
        check(limit(0, 900, lit("k")), Some("E_RATE_LIMIT_MAX"));
        check(limit(5, 0, lit("k")), Some("E_RATE_LIMIT_WINDOW"));
        check(limit(5, 86_401, lit("k")), Some("E_RATE_LIMIT_WINDOW"));
        // The key is text: a number is refused.
        check(
            limit(
                5,
                60,
                Expr::Path {
                    path: "input.amount".into(),
                },
            ),
            Some("E_TYPE_MISMATCH"),
        );
        // 429 and Retry-After are HTTP.
        p.services[0].rpcs[0]
            .steps
            .insert(0, limit(5, 60, lit("k")));
        assert!(errors(&p).contains(&"E_RATE_LIMIT_OUTSIDE_ROUTE".to_string()));
    }

    fn with_token() -> Project {
        let mut p = sample::orders();
        sample::add_single_use_token(&mut p);
        p
    }

    fn token_step(p: &mut Project) -> &mut Step {
        &mut p.services[0].routes[2].steps[1]
    }

    #[test]
    fn an_update_can_return_the_row_it_changed() {
        let mut p = with_token();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));

        // The row it gives back is typed after the ticked columns, so the response can read it...
        let respond_reads = |p: &mut Project, path: &str| {
            if let Step::Respond { fields, .. } = &mut p.services[0].routes[2].steps[2] {
                fields[0].value = Expr::Path { path: path.into() };
            }
        };
        // ...as what they are: `new_email` is nullable, so it is an optional string, not a text.
        respond_reads(&mut p, "consumed.new_email");
        assert!(
            errors(&p).contains(&"E_TYPE_MISMATCH".to_string()),
            "{:?}",
            errors(&p)
        );
        // ...and only them: `used` was not ticked.
        respond_reads(&mut p, "consumed.used");
        assert!(
            errors(&p).contains(&"E_UNKNOWN_PATH".to_string()),
            "{:?}",
            errors(&p)
        );
        respond_reads(&mut p, "consumed.account_id");

        // A step that gives rows back names where they go; a list needs a schema, like a select.
        {
            let Step::Sql { bind, .. } = token_step(&mut p) else {
                panic!()
            };
            *bind = None;
        }
        assert!(errors(&p).contains(&"E_SQL_BIND".to_string()));
        {
            let Step::Sql { bind, columns, .. } = token_step(&mut p) else {
                panic!()
            };
            *bind = Some("consumed".into());
            columns.push("ghost".into());
        }
        assert!(errors(&p).contains(&"E_UNKNOWN_COLUMN".to_string()));
        {
            let Step::Sql { columns, many, .. } = token_step(&mut p) else {
                panic!()
            };
            columns.pop();
            *many = true;
        }
        assert!(errors(&p).contains(&"E_SQL_MANY_INTO".to_string()));
        // A delete can give rows back too.
        {
            let Step::Sql { many, op, .. } = token_step(&mut p) else {
                panic!()
            };
            *many = false;
            *op = SqlOp::Delete;
        }
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
    }

    #[test]
    fn now_is_the_database_clock_of_a_timestamp_column() {
        // `used` is a bool: it cannot be compared with a clock.
        let mut p = with_token();
        {
            let Step::Sql { conditions, .. } = token_step(&mut p) else {
                panic!()
            };
            conditions[2].value = Expr::Now;
        }
        assert!(
            errors(&p).contains(&"E_NOW_COLUMN".to_string()),
            "{:?}",
            errors(&p)
        );
        // It also fills a timestamp column.
        let mut p = with_token();
        {
            let Step::Sql { values, .. } = token_step(&mut p) else {
                panic!()
            };
            values.push(Assignment {
                field: "expires_at".into(),
                value: Expr::Now,
            });
        }
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        // But it is no value for a Rust expression: a variable, a branch or a response.
        route_of(&mut p).steps.insert(
            0,
            Step::Branch {
                id: "b".into(),
                condition: Condition::Compare {
                    left: Expr::Now,
                    op: CmpOp::Gt,
                    right: Expr::Literal { value: "x".into() },
                },
                then: vec![],
                otherwise: vec![],
            },
        );
        assert!(
            errors(&p).contains(&"E_NOW_OUTSIDE_SQL".to_string()),
            "{:?}",
            errors(&p)
        );
    }

    #[test]
    fn the_answer_to_a_missing_row_is_checked() {
        // (expected code, how to break the answer)
        type Case<'a> = (&'a str, &'a dyn Fn(&mut SqlMissing));
        let cases: [Case; 3] = [
            ("E_SQL_MISSING_STATUS", &|m| m.status = 200),
            ("E_SQL_MISSING_STATUS", &|m| m.status = 600),
            ("E_SQL_MISSING_MESSAGE", &|m| m.message = " ".into()),
        ];
        for (code, change) in cases {
            let mut p = with_token();
            if let Step::Sql { missing, .. } = token_step(&mut p) {
                change(missing.as_mut().unwrap());
            }
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
        // It only makes sense for a step that reads one row: not a list, not a step that gives nothing back.
        let mut p = with_token();
        {
            let Step::Sql { many, .. } = token_step(&mut p) else {
                panic!()
            };
            *many = true;
        }
        assert!(errors(&p).contains(&"E_SQL_MISSING_USELESS".to_string()));
        let mut p = with_token();
        {
            let Step::Sql { columns, bind, .. } = token_step(&mut p) else {
                panic!()
            };
            columns.clear();
            *bind = None;
        }
        assert!(errors(&p).contains(&"E_SQL_MISSING_USELESS".to_string()));
        // A gRPC handler has no status to answer with.
        let mut p = with_token();
        let step = token_step(&mut p).clone();
        p.services[0].rpcs[0].steps.insert(0, step);
        assert!(
            errors(&p).contains(&"E_SQL_MISSING_OUTSIDE_ROUTE".to_string()),
            "{:?}",
            errors(&p)
        );
    }

    fn with_cleaner() -> Project {
        let mut p = with_token();
        sample::add_cleaner_job(&mut p);
        p
    }

    #[test]
    fn a_job_runs_steps_without_a_request() {
        let mut p = with_cleaner();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        // There is no `input`, and nobody to answer.
        p.services[0].jobs[0].steps.insert(
            0,
            Step::Log {
                id: "x".into(),
                level: LogLevel::Info,
                message: "{input.order_id}".into(),
            },
        );
        assert!(
            !errors(&p).is_empty(),
            "a job has no input: {:?}",
            validate(&p)
        );
        p.services[0].jobs[0].steps.remove(0);
        for step in [
            Step::Respond {
                id: "r".into(),
                fields: vec![],
                status: None,
                cookies: vec![],
            },
            fail(400, None),
            Step::RateLimit {
                id: "l".into(),
                key: Expr::Literal { value: "k".into() },
                max: 1,
                window_secs: 60,
            },
        ] {
            p.services[0].jobs[0].steps.push(step);
        }
        let found = errors(&p);
        for code in [
            "E_RESPOND_IN_CONSUMER",
            "E_FAIL_OUTSIDE_ROUTE",
            "E_RATE_LIMIT_OUTSIDE_ROUTE",
        ] {
            assert!(found.contains(&code.to_string()), "{code}: {found:?}");
        }
    }

    #[test]
    fn jobs_are_named_and_scheduled_sensibly() {
        for (code, change) in [
            (
                "E_JOB_NAME",
                &(|j: &mut Job| j.name = "Cleaner Job".into()) as &dyn Fn(&mut Job),
            ),
            ("E_JOB_INTERVAL", &|j| j.every_secs = 0),
            ("E_JOB_INTERVAL", &|j| j.every_secs = 2_592_001),
        ] {
            let mut p = with_cleaner();
            change(&mut p.services[0].jobs[0]);
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
        let mut p = with_cleaner();
        let twin = p.services[0].jobs[0].clone();
        p.services[0].jobs.push(twin);
        assert!(errors(&p).contains(&"E_DUPLICATE_JOB".to_string()));
        // A service with only a job is not empty.
        let mut p = with_cleaner();
        let svc = &mut p.services[0];
        svc.rpcs.clear();
        svc.routes.clear();
        svc.consumers.clear();
        assert!(!validate(&p).iter().any(|d| d.code == "W_EMPTY_SERVICE"));
    }

    #[test]
    fn an_idempotent_consumer_names_the_field_that_identifies_its_event() {
        let set = |p: &mut Project, field: Option<&str>| {
            p.services[1].consumers[0].idempotent_on = field.map(Into::into);
        };
        let mut p = sample::orders();
        set(&mut p, Some("order_id"));
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        // A field the event does not have, and one that cannot be a key (a list, an optional, a number of no
        // integer type, a schema).
        set(&mut p, Some("nope"));
        assert!(
            errors(&p).contains(&"E_IDEMPOTENT_FIELD".to_string()),
            "{:?}",
            errors(&p)
        );
        for (name, change) in [
            (
                "items",
                &(|f: &mut Field| f.repeated = true) as &dyn Fn(&mut Field),
            ),
            ("order_id", &|f| f.optional = true),
            ("order_id", &|f| f.ty = FieldType::Double),
            ("order_id", &|f| {
                f.ty = FieldType::Message {
                    name: "Customer".into(),
                }
            }),
        ] {
            let mut p = sample::orders();
            set(&mut p, Some(name));
            let topic = p.services[1].consumers[0].topic.clone();
            let message = p
                .topics
                .iter()
                .find(|t| t.name == topic)
                .unwrap()
                .message
                .clone();
            let msg = p.messages.iter_mut().find(|m| m.name == message).unwrap();
            if let Some(f) = msg.fields.iter_mut().find(|f| f.name == name) {
                change(f);
            }
            assert!(
                errors(&p).contains(&"E_IDEMPOTENT_FIELD".to_string()),
                "{name}: {:?}",
                errors(&p)
            );
        }
        set(&mut p, None);
        assert!(errors(&p).is_empty());
    }

    fn with_webhook() -> Project {
        let mut p = sample::orders();
        sample::add_stripe_webhook(&mut p);
        p
    }

    fn webhook_step(p: &mut Project) -> &mut Step {
        &mut p.services[0].routes[2].steps[0]
    }

    #[test]
    fn a_signed_webhook_is_valid_and_checked() {
        let p = with_webhook();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        type Case<'a> = (&'a str, &'a dyn Fn(&mut Step));
        let cases: [Case; 5] = [
            ("E_HMAC_HEADER", &|s| {
                if let Step::VerifyHmac { header, .. } = s {
                    *header = "Bad Header".into()
                }
            }),
            ("E_HMAC_SECRET_ENV", &|s| {
                if let Step::VerifyHmac { secret_env, .. } = s {
                    *secret_env = "stripe-secret".into()
                }
            }),
            ("E_HMAC_SECRET_ENV", &|s| {
                if let Step::VerifyHmac { secret_env, .. } = s {
                    *secret_env = "".into()
                }
            }),
            ("E_HMAC_TOLERANCE", &|s| {
                if let Step::VerifyHmac { tolerance_secs, .. } = s {
                    *tolerance_secs = 0
                }
            }),
            ("E_HMAC_TOLERANCE", &|s| {
                if let Step::VerifyHmac { tolerance_secs, .. } = s {
                    *tolerance_secs = 86_401
                }
            }),
        ];
        for (code, change) in cases {
            let mut p = with_webhook();
            change(webhook_step(&mut p));
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
    }

    #[test]
    fn the_signature_is_checked_first_and_once_on_a_route_with_a_body() {
        // Not first: something would run on an unauthenticated request.
        let mut p = with_webhook();
        p.services[0].routes[2].steps.swap(0, 1);
        assert!(
            errors(&p).contains(&"E_HMAC_POSITION".to_string()),
            "{:?}",
            errors(&p)
        );
        // Not inside a condition either: it would be skipped.
        let mut p = with_webhook();
        let step = p.services[0].routes[2].steps.remove(0);
        p.services[0].routes[2].steps.insert(
            0,
            Step::Branch {
                id: "b".into(),
                condition: Condition::Raw {
                    code: "true".into(),
                },
                then: vec![step],
                otherwise: vec![],
            },
        );
        assert!(
            errors(&p).contains(&"E_HMAC_POSITION".to_string()),
            "{:?}",
            errors(&p)
        );
        // Once.
        let mut p = with_webhook();
        let again = p.services[0].routes[2].steps[0].clone();
        p.services[0].routes[2].steps.insert(1, again);
        assert!(errors(&p).contains(&"E_HMAC_DUPLICATE".to_string()));
        // A body to sign: not a GET.
        let mut p = with_webhook();
        p.services[0].routes[2].method = HttpMethod::Get;
        p.services[0].routes[2].path = "/webhook/stripe".into();
        assert!(
            errors(&p).contains(&"E_HMAC_METHOD".to_string()),
            "{:?}",
            errors(&p)
        );
        // HTTP only.
        let mut p = with_webhook();
        let step = webhook_step(&mut p).clone();
        p.services[0].rpcs[0].steps.insert(0, step);
        assert!(
            errors(&p).contains(&"E_HMAC_OUTSIDE_ROUTE".to_string()),
            "{:?}",
            errors(&p)
        );
    }

    fn source_of<'a>(p: &'a mut Project, message: &str) -> &'a mut Field {
        p.messages
            .iter_mut()
            .find(|m| m.name == message)
            .and_then(|m| m.fields.first_mut())
            .unwrap()
    }

    #[test]
    fn a_field_can_be_read_from_a_header_or_a_cookie() {
        let p = with_jwt();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        // The fields that carry the token are filled by the route, not by the client.
        assert!(matches!(
            &p.message("WhoamiRequest").unwrap().fields[0].source,
            Some(RequestSource::Header { name }) if name == "Authorization"
        ));
        assert!(matches!(
            &p.message("SessionRequest").unwrap().fields[0].source,
            Some(RequestSource::Cookie { name }) if name == "access_token"
        ));
    }

    #[test]
    fn where_a_field_is_read_from_is_checked() {
        let mut p = with_jwt();
        source_of(&mut p, "WhoamiRequest").ty = FieldType::Int32;
        assert!(errors(&p).contains(&"E_SOURCE_TYPE".to_string()));
        let mut p = with_jwt();
        source_of(&mut p, "WhoamiRequest").optional = true;
        assert!(errors(&p).contains(&"E_SOURCE_TYPE".to_string()));
        let mut p = with_jwt();
        source_of(&mut p, "WhoamiRequest").repeated = true;
        assert!(errors(&p).contains(&"E_SOURCE_TYPE".to_string()));
        // Only an API schema comes from a request.
        let mut p = with_jwt();
        p.messages
            .iter_mut()
            .find(|m| m.name == "WhoamiRequest")
            .unwrap()
            .kind = MessageKind::Event;
        assert!(errors(&p).contains(&"E_SOURCE_KIND".to_string()));
        // A name that cannot travel in a header or a cookie.
        for (source, ok) in [
            (
                RequestSource::Header {
                    name: "X-Request-Id".into(),
                },
                true,
            ),
            (
                RequestSource::Header {
                    name: "X Request".into(),
                },
                false,
            ),
            (
                RequestSource::Header {
                    name: "X:Id".into(),
                },
                false,
            ),
            (RequestSource::Header { name: "a_b".into() }, false),
            (
                RequestSource::Header {
                    name: String::new(),
                },
                false,
            ),
            (
                RequestSource::Cookie {
                    name: "access_token".into(),
                },
                true,
            ),
            (RequestSource::Cookie { name: "a=b".into() }, false),
            (RequestSource::Cookie { name: "a;b".into() }, false),
            (RequestSource::Cookie { name: "é".into() }, false),
            (
                RequestSource::Cookie {
                    name: "x".repeat(101),
                },
                false,
            ),
        ] {
            let mut p = with_jwt();
            source_of(&mut p, "SessionRequest").source = Some(source.clone());
            assert_eq!(
                !errors(&p).contains(&"E_SOURCE_NAME".to_string()),
                ok,
                "{source:?}"
            );
        }
    }

    #[test]
    fn a_path_parameter_and_a_header_cannot_fill_the_same_field() {
        let mut p = with_jwt();
        p.services[0].routes[2].path = "/whoami/{token}".into();
        assert!(errors(&p).contains(&"E_SOURCE_PARAM".to_string()));
    }

    #[test]
    fn a_field_read_from_the_request_that_no_route_receives_is_a_warning() {
        let mut p = with_jwt();
        let warned = |p: &Project| {
            validate(p)
                .into_iter()
                .filter(|d| d.code == "W_SOURCE_UNUSED")
                .count()
        };
        assert_eq!(warned(&p), 0);
        // The route is gone: nothing fills the header any more.
        p.services[0].routes.retain(|r| r.name != "session");
        assert_eq!(warned(&p), 1);
    }

    fn with_seal() -> Project {
        let mut p = sample::orders();
        sample::add_sealed_text(&mut p);
        p
    }

    #[test]
    fn encrypt_and_decrypt_bind_a_text_and_name_their_key() {
        let p = with_seal();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        let steps = |p: &mut Project| p.services[0].routes[2].steps.clone();
        for key in ["notes_key", "NOTES-KEY", "1KEY", ""] {
            let mut p = with_seal();
            for step in &mut p.services[0].routes[2].steps {
                if let Step::Encrypt { key_env, .. } | Step::Decrypt { key_env, .. } = step {
                    *key_env = key.into();
                }
            }
            assert!(
                errors(&p).contains(&"E_KEY_ENV".to_string()),
                "{key:?}: {:?}",
                errors(&p)
            );
        }
        // What goes in is a text; the result is one, which the next steps can read.
        let mut p = with_seal();
        if let Step::Encrypt { input, .. } = &mut p.services[0].routes[2].steps[0] {
            *input = Expr::Path {
                path: "nope".into(),
            };
        }
        assert!(errors(&p).contains(&"E_UNKNOWN_PATH".to_string()));
        let mut p = with_seal();
        // `opened` is bound by the second step: reading it from the first is refused.
        if let Step::Encrypt { input, .. } = &mut p.services[0].routes[2].steps[0] {
            *input = Expr::Path {
                path: "opened".into(),
            };
        }
        assert!(errors(&p).contains(&"E_UNKNOWN_PATH".to_string()));
        let mut p = with_seal();
        if let Step::Decrypt { bind, .. } = &mut p.services[0].routes[2].steps[1] {
            *bind = "tx".into();
        }
        assert!(errors(&p).contains(&"E_RESERVED_NAME".to_string()));
        assert_eq!(steps(&mut with_seal()).len(), 3);
    }

    fn with_signup() -> Project {
        let mut p = sample::orders();
        sample::add_signup(&mut p);
        p
    }

    fn field_of<'a>(p: &'a mut Project, message: &str, name: &str) -> &'a mut Field {
        p.messages
            .iter_mut()
            .find(|m| m.name == message)
            .unwrap()
            .fields
            .iter_mut()
            .find(|f| f.name == name)
            .unwrap()
    }

    #[test]
    fn rules_are_checked() {
        let p = with_signup();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));

        // Only the text of an API schema.
        let mut p = with_signup();
        field_of(&mut p, "SignupBody", "email").ty = FieldType::Int64;
        assert!(
            errors(&p).contains(&"E_RULE_TYPE".to_string()),
            "{:?}",
            errors(&p)
        );
        let mut p = with_signup();
        p.messages
            .iter_mut()
            .find(|m| m.name == "SignupBody")
            .unwrap()
            .kind = MessageKind::Proto;
        assert!(
            errors(&p).contains(&"E_RULE_KIND".to_string()),
            "{:?}",
            errors(&p)
        );
        // Each kind once, and a length that someone can satisfy.
        let mut p = with_signup();
        field_of(&mut p, "SignupBody", "email").rules = vec![Rule::Email, Rule::Email];
        assert!(errors(&p).contains(&"E_RULE_DUPLICATE".to_string()));
        for (min, max) in [(5, 4), (0, 0), (10, 9)] {
            let mut p = with_signup();
            field_of(&mut p, "SignupBody", "password").rules = vec![Rule::Length { min, max }];
            assert!(
                errors(&p).contains(&"E_RULE_LENGTH".to_string()),
                "{min}..{max}"
            );
        }
        // 0..=n is fine: an empty text allowed, a long one not.
        let mut p = with_signup();
        field_of(&mut p, "SignupBody", "password").rules = vec![Rule::Length { min: 0, max: 5 }];
        assert!(errors(&p).is_empty());
    }

    #[test]
    fn rules_that_no_route_checks_are_a_warning() {
        let mut p = with_signup();
        assert!(!validate(&p).iter().any(|d| d.code == "W_RULE_UNUSED"));
        // The nested schema is received through the request: its rules are checked.
        let received = p.received_schemas();
        assert!(
            received.contains("SignupBody") && received.contains("SignupCompany"),
            "{received:?}"
        );
        // Responses are not received: nothing checks them.
        assert!(!received.contains("OrderView") && !received.contains("SignupAck"));
        assert!(p.has_rules("SignupBody") && p.has_rules("SignupCompany"));
        assert!(!p.has_rules("OrderView"));
        // Take the route away: nobody receives them any more.
        p.services[0].routes.pop();
        let warned: Vec<_> = validate(&p)
            .into_iter()
            .filter(|d| d.code == "W_RULE_UNUSED")
            .collect();
        assert_eq!(warned.len(), 2, "{warned:?}");
        assert!(warned[0].message.contains("never checked"));
    }

    fn with_jwt() -> Project {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        p
    }

    /// The `whoami` route is the third; its `verifyJwt` is the first step, the `jwks` route is the fourth.
    fn jwt_step(p: &mut Project, route: usize) -> &mut Step {
        &mut p.services[0].routes[route].steps[0]
    }

    #[test]
    fn a_token_can_be_signed_checked_and_its_keys_published() {
        let p = with_jwt();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        // A signed claim keeps its type: the verifying side reads `claims.roles` as a list of texts, as the schema says.
        let mut p = with_jwt();
        if let Step::Respond { fields, .. } = &mut p.services[0].routes[2].steps[1] {
            fields[1].value = Expr::Path {
                path: "claims.sub".into(),
            };
        }
        assert!(
            errors(&p).contains(&"E_TYPE_MISMATCH".to_string()),
            "{:?}",
            errors(&p)
        );
    }

    #[test]
    fn the_claims_of_a_token_are_checked() {
        // (expected code, how to break the step)
        type Case<'a> = (&'a str, &'a dyn Fn(&mut Step));
        fn sign(step: &mut Step) -> &mut Vec<Assignment> {
            match step {
                Step::SignJwt { claims, .. } => claims,
                _ => panic!("not a signJwt"),
            }
        }
        let cases: [Case; 7] = [
            ("E_JWT_CLAIM_RESERVED", &|s| {
                sign(s).push(Assignment {
                    field: "exp".into(),
                    value: Expr::Literal { value: 1.into() },
                })
            }),
            ("E_JWT_CLAIM_RESERVED", &|s| sign(s)[0].field = "iss".into()),
            ("E_JWT_CLAIM_NAME", &|s| {
                sign(s)[0].field = "Sub Ject".into()
            }),
            ("E_JWT_CLAIM_DUPLICATE", &|s| {
                let again = sign(s)[0].clone();
                sign(s).push(again)
            }),
            ("E_JWT_NO_SUB", &|s| sign(s)[0].field = "uid".into()),
            ("E_JWT_CLAIM_TYPE", &|s| {
                sign(s)[0].value = Expr::Path {
                    path: "input".into(),
                }
            }),
            ("E_UNKNOWN_PATH", &|s| {
                sign(s)[0].value = Expr::Path {
                    path: "nope".into(),
                }
            }),
        ];
        for (code, change) in cases {
            let mut p = with_jwt();
            // Add the step to a route of the sample.
            let mut step = Step::SignJwt {
                id: "j".into(),
                key_env: "JWT_SIGNING_KEY".into(),
                issuer: "orders".into(),
                ttl_secs: 300,
                claims: vec![Assignment {
                    field: "sub".into(),
                    value: Expr::Path {
                        path: "input.order_id".into(),
                    },
                }],
                bind: "token".into(),
            };
            change(&mut step);
            p.services[0].routes[1].steps.insert(0, step);
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
        // Lifetime, issuer and key.
        for (code, change) in [
            (
                "E_JWT_TTL",
                &(|s: &mut Step| {
                    if let Step::SignJwt { ttl_secs, .. } = s {
                        *ttl_secs = 0
                    }
                }) as &dyn Fn(&mut Step),
            ),
            ("E_JWT_TTL", &|s| {
                if let Step::SignJwt { ttl_secs, .. } = s {
                    *ttl_secs = 2_592_001
                }
            }),
            ("E_JWT_ISSUER", &|s| {
                if let Step::SignJwt { issuer, .. } = s {
                    issuer.clear()
                }
            }),
            ("E_KEY_ENV", &|s| {
                if let Step::SignJwt { key_env, .. } = s {
                    *key_env = "signing-key".into()
                }
            }),
        ] {
            let mut p = with_jwt();
            let mut step = Step::SignJwt {
                id: "j".into(),
                key_env: "JWT_SIGNING_KEY".into(),
                issuer: "orders".into(),
                ttl_secs: 300,
                claims: vec![Assignment {
                    field: "sub".into(),
                    value: Expr::Path {
                        path: "input.order_id".into(),
                    },
                }],
                bind: "token".into(),
            };
            change(&mut step);
            p.services[0].routes[1].steps.insert(0, step);
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
    }

    #[test]
    fn verifying_a_token_and_publishing_keys_are_checked() {
        type Case<'a> = (&'a str, usize, &'a dyn Fn(&mut Step));
        let cases: [Case; 8] = [
            ("E_JWT_SCHEMA", 2, &|s| {
                if let Step::VerifyJwt { schema, .. } = s {
                    *schema = "Nope".into()
                }
            }),
            ("E_JWT_SCHEMA", 2, &|s| {
                if let Step::VerifyJwt { schema, .. } = s {
                    *schema = "Customer".into()
                }
            }),
            ("E_JWT_KEYS", 2, &|s| {
                if let Step::VerifyJwt { source, .. } = s {
                    *source = JwksSource::Keys { key_envs: vec![] }
                }
            }),
            ("E_KEY_ENV", 2, &|s| {
                if let Step::VerifyJwt { source, .. } = s {
                    *source = JwksSource::Url {
                        url_env: "auth url".into(),
                    }
                }
            }),
            ("E_JWT_ISSUER", 2, &|s| {
                if let Step::VerifyJwt { issuer, .. } = s {
                    issuer.clear()
                }
            }),
            ("E_TYPE_MISMATCH", 2, &|s| {
                if let Step::VerifyJwt { token, .. } = s {
                    *token = Expr::Path {
                        path: "input".into(),
                    }
                }
            }),
            ("E_JWT_KEYS", 3, &|s| {
                if let Step::Jwks { key_envs, .. } = s {
                    key_envs.clear()
                }
            }),
            ("E_KEY_ENV", 3, &|s| {
                if let Step::Jwks { key_envs, .. } = s {
                    key_envs[0] = "lower".into()
                }
            }),
        ];
        for (code, route, change) in cases {
            let mut p = with_jwt();
            change(jwt_step(&mut p, route));
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
        // The JWKS needs its two schemas, in the shape of RFC 7517.
        let mut p = with_jwt();
        p.messages.retain(|m| m.name != "Jwk");
        assert!(
            errors(&p).contains(&"E_JWKS_SCHEMA".to_string()),
            "{:?}",
            errors(&p)
        );
        let mut p = with_jwt();
        p.messages
            .iter_mut()
            .find(|m| m.name == "Jwk")
            .unwrap()
            .fields
            .retain(|f| f.name != "kid");
        assert!(errors(&p).contains(&"E_JWKS_SCHEMA".to_string()));
        // Refusing a token is a 401: HTTP only.
        let mut p = with_jwt();
        let step = jwt_step(&mut p, 2).clone();
        p.services[0].rpcs[0].steps.insert(0, step);
        assert!(
            errors(&p).contains(&"E_JWT_OUTSIDE_ROUTE".to_string()),
            "{:?}",
            errors(&p)
        );
    }

    #[test]
    fn a_table_without_a_primary_key_warns() {
        let mut p = sample::orders();
        p.services[0].tables[0].columns[0].primary_key = false;
        assert!(validate(&p).iter().any(|d| d.code == "W_TABLE_NO_PK"));
    }

    #[test]
    fn a_select_types_the_variable_after_the_ticked_columns() {
        let mut p = sample::orders();
        // `get_order` reads `order_id` and `status`: the response can read both, and only both.
        assert!(errors(&p).is_empty());
        if let Step::Sql { columns, .. } = sql_step(&mut p, 1, 0) {
            columns.retain(|c| c != "status");
        }
        assert!(
            errors(&p).contains(&"E_UNKNOWN_PATH".into()),
            "`order.status` is no longer read"
        );
    }

    #[test]
    fn sql_steps_check_tables_columns_and_values() {
        let mut p = sample::orders();
        if let Step::Sql { table, .. } = sql_step(&mut p, 1, 0) {
            *table = "nope".into();
        }
        assert!(errors(&p).contains(&"E_UNKNOWN_TABLE".into()));

        let mut p = sample::orders();
        if let Step::Sql {
            conditions,
            columns,
            ..
        } = sql_step(&mut p, 1, 0)
        {
            conditions[0].column = "nope".into();
            columns.push("also_nope".into());
        }
        let e = errors(&p);
        assert!(
            e.iter().filter(|c| *c == "E_UNKNOWN_COLUMN").count() >= 2,
            "{e:?}"
        );

        // A condition is typed by its column: an int64 column against a string.
        let mut p = sample::orders();
        if let Step::Sql { conditions, .. } = sql_step(&mut p, 1, 0) {
            conditions[0].column = "amount".into();
        }
        assert!(errors(&p).contains(&"E_TYPE_MISMATCH".into()));
    }

    #[test]
    fn an_insert_gives_every_column_that_has_no_default() {
        let mut p = sample::orders();
        if let Step::Sql { values, .. } = sql_step(&mut p, 0, 3) {
            values.retain(|a| a.field != "amount");
        }
        assert!(errors(&p).contains(&"E_SQL_MISSING_VALUE".into()));
    }

    #[test]
    fn an_update_or_a_delete_needs_a_condition() {
        for op in [SqlOp::Update, SqlOp::Delete] {
            let mut p = sample::orders();
            if let Step::Sql {
                op: o,
                conditions,
                values,
                ..
            } = sql_step(&mut p, 0, 3)
            {
                *o = op;
                conditions.clear();
                values.truncate(1);
            }
            assert!(errors(&p).contains(&"E_SQL_NO_WHERE".into()), "{op:?}");
        }
    }

    #[test]
    fn a_list_of_rows_is_converted_into_a_schema() {
        let mut p = sample::orders();
        if let Step::Sql { many, .. } = sql_step(&mut p, 1, 0) {
            *many = true;
        }
        assert!(errors(&p).contains(&"E_SQL_MANY_INTO".into()));
        // `OrderView` is filled by `order_id` and `status`, both ticked: that works.
        if let Step::Sql { into, .. } = sql_step(&mut p, 1, 0) {
            *into = Some("OrderView".into());
        }
        assert!(!errors(&p).contains(&"E_SQL_MANY_INTO".into()));
        assert!(
            !errors(&p).contains(&"E_SQL_INTO_FIELD".into()),
            "{:?}",
            errors(&p)
        );
        // A field the columns cannot fill.
        p.messages
            .iter_mut()
            .find(|m| m.name == "OrderView")
            .unwrap()
            .fields
            .push(Field::new("total", FieldType::Int64));
        assert!(errors(&p).contains(&"E_SQL_INTO_FIELD".into()));
    }

    #[test]
    fn tx_is_a_reserved_variable_name() {
        let mut p = sample::orders();
        p.services[1].consumers[0].steps.push(Step::Let {
            id: "x".into(),
            name: "tx".into(),
            value_type: FieldType::String,
            optional: false,
            repeated: false,
            value: Expr::Literal { value: "x".into() },
        });
        assert!(errors(&p).contains(&"E_RESERVED_NAME".into()));
    }

    #[test]
    fn respond_in_consumer_is_an_error() {
        let mut p = sample::orders();
        p.services[1].consumers[0].steps.push(Step::Respond {
            id: "x".into(),
            fields: vec![],
            status: None,
            cookies: vec![],
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
                optional: false,
                repeated: false,
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
    fn publishing_requires_being_a_declared_producer() {
        let mut p = sample::orders();
        assert!(!errors(&p).contains(&"E_NOT_PRODUCER".into()));
        // Payments publishes payments.completed; take that declaration away.
        p.topics[1].producers.clear();
        assert!(errors(&p).contains(&"E_NOT_PRODUCER".into()));
        // Declaring another service is not enough.
        p.topics[1].producers.push("Notifications".into());
        assert!(errors(&p).contains(&"E_NOT_PRODUCER".into()));
        p.topics[1].producers.push("Payments".into());
        assert!(!errors(&p).contains(&"E_NOT_PRODUCER".into()));
    }

    #[test]
    fn producers_must_exist_and_be_unique() {
        let mut p = sample::orders();
        p.topics[0].producers.push("Ghost".into());
        assert!(errors(&p).contains(&"E_UNKNOWN_SERVICE".into()));
        p.topics[0].producers = vec!["Orders".into(), "Orders".into()];
        assert!(errors(&p).contains(&"E_DUPLICATE_PRODUCER".into()));
    }

    #[test]
    fn a_producer_without_a_publish_step_warns() {
        let mut p = sample::orders();
        p.topics[0].producers.push("Notifications".into());
        let d = validate(&p);
        assert!(
            d.iter().any(
                |d| d.code == "W_PRODUCER_NEVER_PUBLISHES" && d.path == "/topics/0/producers/1"
            )
        );
        assert!(!has_errors(&d));
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
            producers: vec![],
            position: Position::default(),
        });
        let c = codes(&p);
        assert!(c.contains(&"W_TOPIC_NO_PRODUCER".into()));
        assert!(c.contains(&"W_TOPIC_NO_CONSUMER".into()));
    }
}
