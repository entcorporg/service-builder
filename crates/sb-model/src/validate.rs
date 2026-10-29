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

fn kind_label(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Event => "an event schema",
        MessageKind::Proto => "a proto schema",
        MessageKind::Api => "an API schema",
        MessageKind::Row => "a SQL row",
    }
}

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
            if s.rpcs.is_empty() && s.consumers.is_empty() && s.routes.is_empty() {
                self.warn(
                    "W_EMPTY_SERVICE",
                    base.clone(),
                    format!(
                        "service `{}` has no RPC, no HTTP route and no consumer",
                        s.name
                    ),
                );
            }
            self.routes(s, &base);
            self.tables(s, &base);

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
                let env = Env {
                    service: s,
                    handler: Handler::Consumer,
                };
                let mut scope = Scope::with_input(&topic.message);
                self.steps(&env, &format!("{cp}/steps"), &c.steps, &mut scope);
            }
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

    /// The type of the values a column holds, as a flow sees it.
    fn column_type(c: &Column) -> TypeRef {
        let mut t = TypeRef::scalar(c.ty.field_type());
        t.optional = c.nullable;
        t
    }

    fn sql(&mut self, env: &Env, path: &str, step: &Step, scope: &mut Scope) {
        let Step::Sql {
            id,
            op,
            table,
            bind,
            columns,
            many,
            into,
            conditions,
            values,
            order_by,
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
                Some(col) => {
                    self.check_expr(
                        scope,
                        &c.value,
                        Some(&Self::column_type(col)),
                        &format!("{cp}/value"),
                    );
                }
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
                for (i, name) in columns.iter().enumerate() {
                    if column(name).is_none() {
                        self.error(
                            "E_UNKNOWN_COLUMN",
                            format!("{path}/columns/{i}"),
                            format!("table `{table}` has no column `{name}`"),
                        );
                    }
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
                        Some(col) => {
                            self.check_expr(
                                scope,
                                &a.value,
                                Some(&Self::column_type(col)),
                                &format!("{vp}/value"),
                            );
                        }
                    }
                }
                if *op == SqlOp::Update && values.is_empty() {
                    self.error(
                        "E_SQL_NO_VALUES",
                        format!("{path}/values"),
                        "give at least one column a new value",
                    );
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
            SqlOp::Delete => {}
        }
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
                value,
                ..
            } => {
                let ty = TypeRef::scalar(value_type.clone());
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
                        if !matches!(value, Expr::Raw { .. }) {
                            self.error(
                                "E_LET_VALUE",
                                format!("{path}/value"),
                                format!("a `{name}` variable is built with Rust code, e.g. `pb::{name} {{ .. }}`"),
                            );
                        }
                    }
                    _ if !ty.is_simple_scalar() => self.error(
                        "E_LET_TYPE",
                        format!("{path}/valueType"),
                        "variables hold a string, bool, int32, int64, float, double, or a schema built with Rust",
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
    fn a_variable_can_hold_a_schema_built_with_rust() {
        let mut p = sample::orders();
        let let_step = |value_type: FieldType, value: Expr| Step::Let {
            id: "v".into(),
            name: "view".into(),
            value_type,
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
        if let Step::Sql { values, .. } = sql_step(&mut p, 0, 1) {
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
            } = sql_step(&mut p, 0, 1)
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
