//! Project validation. Everything the generator relies on is checked here, so a project
//! without errors is guaranteed to produce code that type-checks (barring raw Rust blocks).
//!
//! Diagnostics point at the faulty element with a JSON pointer into the project
//! document, e.g. `/services/0/rpcs/1/steps/2/then/0`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::names::{
    is_pascal_ident, is_snake_ident, is_topic_name, route_params, route_shape, snake, topic_const,
};
mod http_call;

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
    let expanded = project.with_fragments();
    let typed = expanded.with_sql_rows();
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
    c.pods();
    c.openapi();
    // The steps of an include are checked where it plays them: what is wrong in one is shown on the `include`.
    let mut out: Vec<Diagnostic> = c
        .out
        .into_iter()
        .map(|mut d| {
            d.path = crate::fragments::original_path(&expanded, &d.path);
            d
        })
        .collect();
    out.dedup();
    out.extend(crate::fragments::diagnostics(project));
    out
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

/// What the meter and quota steps keep in the database of the service: the table of the uses and the view that adds them up.
pub const USAGE_TABLES: [&str; 2] = ["usage_events", "usage_daily"];

/// What the API key steps keep in the database of the service: the keys.
pub const API_KEYS_TABLE: &str = "api_keys";

/// What the audit steps keep in the database of the service: the journal.
pub const AUDIT_TABLE: &str = "audit_events";

/// What the flag steps keep in the database of the service: the flags.
pub const FLAGS_TABLE: &str = "feature_flags";

/// What `nextNumber` keeps in the database of the service: the last number of each sequence.
pub const SEQUENCES_TABLE: &str = "number_sequences";

/// What `verifyChallenge` keeps in the database of the service: the solutions already accepted.
pub const CHALLENGE_TABLE: &str = "challenge_uses";

/// What a circuit and a bulkhead can be set to: wide enough for any real dependency, narrow enough to be a mistake when exceeded.
pub const MAX_CIRCUIT_FAILURES: u32 = 1_000;
pub const MAX_CIRCUIT_SECS: u32 = 3_600;
pub const MAX_CONCURRENT_CALLS: u32 = 10_000;

/// The most attempts a `rateLimit` counted in the shared store can allow: the store keeps one entry per attempt of the window.
pub const SHARED_RATE_LIMIT_MAX: u32 = 10_000;

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

/// The largest object that goes through a step: 10 MiB, which is also the runtime's hard limit.
const MAX_OBJECT_BYTES: u32 = 10 * 1024 * 1024;

/// Recovery codes issued at once: a sheet to print, not a stock.
const MAX_RECOVERY_CODES: u32 = 20;

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
        if !(1..=MAX_OUTBOX_ATTEMPTS).contains(&p.outbox.max_attempts) {
            self.error(
                "E_OUTBOX_ATTEMPTS",
                "/outbox/maxAttempts",
                format!("an event gets between 1 and {MAX_OUTBOX_ATTEMPTS} publish attempts before it is moved to the DLQ"),
            );
        }
    }

    // ---- pods ------------------------------------------------------------------------

    /// The settings of the Pods page: they end up in a compose file and in the configuration of Traefik, so
    /// what would not be valid there is refused here.
    fn pods(&mut self) {
        use crate::pods::*;
        let pods = &self.project.pods;
        if !pods.cpuset.is_empty() && !is_cpuset(&pods.cpuset) {
            self.error(
                "E_PODS_CPUSET",
                "/pods/cpuset",
                "the CPUs are numbers and ranges separated by commas, like `0-5` or `0,2,4`",
            );
        }
        if !RESTART_POLICIES.contains(&pods.restart.as_str()) {
            self.error(
                "E_PODS_RESTART",
                "/pods/restart",
                format!(
                    "the restart policy is one of {}",
                    RESTART_POLICIES.join(", ")
                ),
            );
        }
        for (key, value) in [
            ("serviceMemory", &pods.service_memory),
            ("kafkaMemory", &pods.kafka_memory),
            ("postgresMemory", &pods.postgres_memory),
        ] {
            if !is_memory(value) {
                self.error(
                    "E_PODS_MEMORY",
                    format!("/pods/{key}"),
                    "a memory amount is a number and a unit, like `160m` or `1g`",
                );
            }
        }
        if pods.env_file.contains(['\n', '"']) {
            self.error(
                "E_PODS_ENV_FILE",
                "/pods/envFile",
                "the environment file is the path of one file",
            );
        }
        let t = &pods.traefik;
        if !t.enabled {
            return;
        }
        let domain = t.domain.trim();
        if !domain.is_empty() && (!is_host(domain) || !domain.contains('.')) {
            self.error(
                "E_PODS_DOMAIN",
                "/pods/traefik/domain",
                "the domain is a host name in lower case, like `example.com`",
            );
        }
        if t.tls() && !t.acme_email.contains('@') {
            self.error(
                "E_PODS_ACME_EMAIL",
                "/pods/traefik/acmeEmail",
                "Let's Encrypt needs an e-mail address to issue the certificates of the domain",
            );
        }
        if t.acme_email
            .contains(|c: char| c.is_whitespace() || c == '"')
        {
            self.error(
                "E_PODS_ACME_EMAIL",
                "/pods/traefik/acmeEmail",
                "the e-mail address has a space or a quote",
            );
        }
        if t.http_port == 0 || t.https_port == 0 || (t.tls() && t.http_port == t.https_port) {
            self.error(
                "E_PODS_PORT",
                "/pods/traefik",
                "the HTTP and HTTPS ports of the proxy are two different ports of the host",
            );
        }
        if t.rate_average == 0 || t.rate_burst == 0 {
            self.error(
                "E_PODS_RATE_LIMIT",
                "/pods/traefik",
                "the rate limit of the API is at least 1 request per second, and so is its burst",
            );
        }
        if !t.admin_auth.is_empty() && !is_htpasswd(&t.admin_auth) {
            self.error(
                "E_PODS_ADMIN_AUTH",
                "/pods/traefik/adminAuth",
                "the authentication is one `user:hash` line, as written by `htpasswd -nB user`",
            );
        }

        // Every host the proxy serves, with where it is set: two of them cannot share a name.
        let mta = self
            .project
            .services
            .iter()
            .any(|s| s.kind == ServiceKind::Mta);
        let mut hosts: Vec<(String, String)> = vec![("api".into(), t.hosts.api.clone())];
        for (on, key, value) in [
            (t.dashboard, "dashboard", &t.hosts.dashboard),
            (pods.adminer, "adminer", &t.hosts.adminer),
            (pods.kafka_ui, "kafkaUi", &t.hosts.kafka_ui),
            (mta, "mailpit", &t.hosts.mailpit),
            (self.project.uses_garage(), "s3", &t.hosts.s3),
        ] {
            if on {
                hosts.push((key.into(), value.clone()));
            }
        }
        for (key, value) in &hosts {
            if !is_host(value) {
                self.error(
                    "E_PODS_HOST",
                    format!("/pods/traefik/hosts/{key}"),
                    "a sub-domain is made of lower case letters, digits and `-`",
                );
            }
        }
        let mut hosts: Vec<(String, String)> = hosts
            .into_iter()
            .map(|(key, value)| (format!("/pods/traefik/hosts/{key}"), value))
            .collect();

        let mut names = HashSet::new();
        for (i, f) in t.frontends.iter().enumerate() {
            let at = format!("/pods/traefik/frontends/{i}");
            if !is_frontend_name(&f.name) {
                self.error(
                    "E_PODS_FRONTEND_NAME",
                    format!("{at}/name"),
                    "the name of a frontend starts with a lower case letter and holds letters, digits and `-`",
                );
            } else if !names.insert(f.name.as_str()) {
                self.error(
                    "E_PODS_FRONTEND_NAME",
                    format!("{at}/name"),
                    format!("two frontends are named `{}`", f.name),
                );
            }
            if !f.host.is_empty() && !is_host(&f.host) {
                self.error(
                    "E_PODS_HOST",
                    format!("{at}/host"),
                    "a sub-domain is made of lower case letters, digits and `-`",
                );
            }
            hosts.push((format!("{at}/host"), f.host.clone()));
            if f.port == 0 {
                self.error(
                    "E_PODS_PORT",
                    format!("{at}/port"),
                    "the port of the dev server of the frontend is missing",
                );
            }
        }
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for (at, host) in &hosts {
            if let Some(first) = seen.insert(host.as_str(), at.as_str()) {
                self.error(
                    "E_PODS_HOST_DUPLICATE",
                    at.clone(),
                    format!("`{}` is already the host of {first}", t.fqdn(host)),
                );
            }
        }

        let exposed = t.dashboard || pods.adminer || pods.kafka_ui || mta;
        if t.tls() && exposed && t.admin_auth.is_empty() {
            self.warn(
                "W_PODS_ADMIN_OPEN",
                "/pods/traefik/adminAuth",
                "the admin interfaces are served on a public domain without authentication: anyone can open them",
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
            let mut json_keys = HashSet::new();
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
                if let Some(j) = &f.json_name
                    && (j.is_empty() || j.chars().any(|c| c.is_whitespace() || c.is_control()))
                {
                    self.error(
                        "E_JSON_NAME",
                        format!("{fp}/jsonName"),
                        format!(
                            "jsonName `{j}` of field `{}` must be a non-empty key without spaces",
                            f.name
                        ),
                    );
                }
                if !json_keys.insert(f.json_key()) {
                    self.error(
                        "E_DUPLICATE_JSON_NAME",
                        format!("{fp}/name"),
                        format!(
                            "two fields of `{}` have the JSON key `{}`",
                            m.name,
                            f.json_key()
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
                if f.lenient && m.kind != MessageKind::Api {
                    self.error(
                        "E_LENIENT_KIND",
                        format!("{fp}/lenient"),
                        format!(
                            "field `{}` is lenient, but `{}` is {}: only API schemas are read from a request",
                            f.name,
                            m.name,
                            kind_label(m.kind)
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
            if !s.routes.is_empty() || s.kind != ServiceKind::Standard {
                match s.http_port {
                    None | Some(0) => self.error(
                        "E_HTTP_PORT",
                        format!("{base}/httpPort"),
                        format!(
                            "`{}` has HTTP routes or serves health and metrics: give it an HTTP port (1 to 65535)",
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
                && s.kind == ServiceKind::Standard
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
            self.mta_service(s, &base);
            self.realtime_service(s, &base);
            self.challenge_service(s, &base);
            if s.compression && (s.kind != ServiceKind::Standard || s.routes.is_empty()) {
                self.warn(
                    "W_COMPRESSION_UNUSED",
                    format!("{base}/compression"),
                    "only the HTTP API of a service with routes is compressed: this setting changes nothing here",
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
                if s.kind == ServiceKind::Mta {
                    self.mail_rule(c, &cp, &topic.message);
                    continue;
                }
                if c.mail.is_some() {
                    self.error(
                        "E_MAIL_OUTSIDE_MTA",
                        format!("{cp}/mail"),
                        format!(
                            "`{}` is not a mail service: a mail rule only belongs to a consumer of a service of kind `mta`",
                            s.name
                        ),
                    );
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
                    "field `{}` is read from the request: it must be a single required string or uuid (a 400 when the request lacks it, empty when lenient)",
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
            if !matches!(r.status, 200..=202 | 204) {
                self.error(
                    "E_ROUTE_STATUS",
                    format!("{rp}/status"),
                    "the success status must be 200, 201, 202 or 204",
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
                                    | FieldType::Uuid
                                    | FieldType::Bool
                                    | FieldType::Int32
                                    | FieldType::Int64
                            ) =>
                    {
                        self.error(
                            "E_ROUTE_PARAM",
                            format!("{rp}/path"),
                            format!(
                                "field `{param}` of `{}` fills `{{{param}}}`: it must be a required string, uuid, bool, int32 or int64",
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

    /// A mail service is fixed code with two inputs, the events it mails and the mails: it has nothing of its own
    /// to draw, and each event has exactly one mail.
    fn mta_service(&mut self, s: &Service, base: &str) {
        if s.kind != ServiceKind::Mta {
            if !s.templates.is_empty() {
                self.error(
                    "E_TEMPLATE_OUTSIDE_MTA",
                    format!("{base}/templates"),
                    format!("`{}` is not a mail service: mail templates belong to a service of kind `mta`", s.name),
                );
            }
            return;
        }
        for (what, empty) in [
            ("RPCs", s.rpcs.is_empty()),
            ("HTTP routes", s.routes.is_empty()),
            ("tables", s.tables.is_empty()),
            ("jobs", s.jobs.is_empty()),
        ] {
            if !empty {
                self.error(
                    "E_MTA_CONTENT",
                    base,
                    format!(
                        "`{}` is a mail service (fixed code, with its own queue, bounce and template tables): it has no {what} to draw",
                        s.name
                    ),
                );
            }
        }
        let mut topics = HashSet::new();
        for (j, c) in s.consumers.iter().enumerate() {
            if !topics.insert(c.topic.as_str()) {
                self.error(
                    "E_MTA_DUPLICATE_EVENT",
                    format!("{base}/consumers/{j}/topic"),
                    format!(
                        "`{}` already mails the events of `{}`: a topic has one mail",
                        s.name, c.topic
                    ),
                );
            }
        }
        let mut seen = HashSet::new();
        for (j, t) in s.templates.iter().enumerate() {
            let tp = format!("{base}/templates/{j}");
            if !seen.insert(t.event_type.as_str()) {
                self.error(
                    "E_MTA_TEMPLATE_DUPLICATE",
                    format!("{tp}/eventType"),
                    format!("`{}` has two mails", t.event_type),
                );
            }
            if !topics.contains(t.event_type.as_str()) {
                self.error(
                    "E_MTA_TEMPLATE_UNKNOWN",
                    format!("{tp}/eventType"),
                    format!(
                        "`{}` mails no event on `{}`: add the event or remove the mail",
                        s.name, t.event_type
                    ),
                );
            }
            if t.subject.trim().is_empty() {
                self.error(
                    "E_MTA_TEMPLATE_SUBJECT",
                    format!("{tp}/subject"),
                    "a mail needs a subject",
                );
            }
            let (html, text) = t.bodies();
            self.mail_design(t, &tp);
            if html.trim().is_empty() && text.trim().is_empty() {
                self.error(
                    "E_MTA_TEMPLATE_BODY",
                    &tp,
                    "a mail needs a body: HTML, text, or both",
                );
            }
            // The mails are seeded by a migration that quotes them with this delimiter.
            for (field, text) in [
                ("eventType", &t.event_type),
                ("subject", &t.subject),
                ("html", &html),
                ("text", &text),
            ] {
                if text.contains("$tpl$") {
                    self.error(
                        "E_MTA_TEMPLATE_DELIMITER",
                        format!("{tp}/{field}"),
                        "`$tpl$` is the delimiter of the migration that seeds the mails: it cannot appear in one",
                    );
                }
            }
        }
        for (j, c) in s.consumers.iter().enumerate() {
            if !seen.contains(c.topic.as_str()) {
                self.error(
                    "E_MTA_TEMPLATE_MISSING",
                    format!("{base}/consumers/{j}/topic"),
                    format!("the event `{}` has no mail: add its template", c.topic),
                );
            }
        }
    }

    /// A mail composed as blocks: its colours, sizes, links and the variables of its texts, against the fields of the
    /// event it is the mail of. Written by hand, a mail has nothing to check here.
    fn mail_design(&mut self, t: &MailTemplate, tp: &str) {
        let Some(design) = &t.design else { return };
        if !t.html.trim().is_empty() || !t.text.trim().is_empty() {
            self.warn(
                "W_MAIL_HTML_IGNORED",
                tp,
                "this mail is composed as blocks: its hand-written HTML and text are ignored",
            );
        }
        let project = self.project;
        let fields: Vec<&str> = project
            .topic(&t.event_type)
            .and_then(|topic| project.message(&topic.message))
            .map(|m| {
                m.fields
                    .iter()
                    .filter(|f| !f.repeated && !matches!(f.ty, FieldType::Message { .. }))
                    .map(|f| f.name.as_str())
                    .collect()
            })
            .unwrap_or_default();
        for issue in crate::maildesign::check(design, &fields, &format!("{tp}/design")) {
            if issue.error {
                self.error(issue.code, issue.path, issue.message);
            } else {
                self.warn(issue.code, issue.path, issue.message);
            }
        }
    }

    /// A real-time service is fixed code with five settings: it has nothing of its own to draw, and what it reads (a
    /// channel, the keys of the sessions, a cookie) is a name that must not break the places it ends up in.
    fn realtime_service(&mut self, s: &Service, base: &str) {
        if s.kind != ServiceKind::Realtime {
            if s.realtime.is_some() {
                self.error(
                    "E_REALTIME_OUTSIDE",
                    format!("{base}/realtime"),
                    format!("`{}` is not a real-time service: these settings belong to a service of kind `realtime`", s.name),
                );
            }
            return;
        }
        for (what, empty) in [
            ("RPCs", s.rpcs.is_empty()),
            ("HTTP routes", s.routes.is_empty()),
            ("tables", s.tables.is_empty()),
            ("jobs", s.jobs.is_empty()),
            ("consumers", s.consumers.is_empty()),
            ("mail templates", s.templates.is_empty()),
        ] {
            if !empty {
                self.error(
                    "E_RT_CONTENT",
                    base,
                    format!(
                        "`{}` is a real-time service (fixed code, with no database): it has no {what} to draw",
                        s.name
                    ),
                );
            }
        }
        let Some(config) = &s.realtime else {
            self.error(
                "E_RT_CONFIG",
                format!("{base}/realtime"),
                format!("`{}` is a real-time service: give it its settings (channel, path, cookie, issuer, session key prefix)", s.name),
            );
            return;
        };
        let rp = format!("{base}/realtime");
        // Names that go into a Redis command, a key, a route and a cookie header: a short, plain alphabet.
        let word = |text: &str, extra: &str| {
            !text.is_empty()
                && text.len() <= 128
                && text
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
        };
        if !word(&config.channel, "-_.:") {
            self.error(
                "E_RT_CHANNEL",
                format!("{rp}/channel"),
                "the channel is 1 to 128 letters, digits, `-`, `_`, `.` or `:`",
            );
        }
        if !word(&config.session_key_prefix, "-_.:") {
            self.error(
                "E_RT_PREFIX",
                format!("{rp}/sessionKeyPrefix"),
                "the key prefix is 1 to 128 letters, digits, `-`, `_`, `.` or `:`",
            );
        }
        if !word(&config.access_cookie, "-_.") {
            self.error(
                "E_RT_COOKIE",
                format!("{rp}/accessCookie"),
                "the cookie name is 1 to 128 letters, digits, `-`, `_` or `.`",
            );
        }
        if config.issuer.trim().is_empty()
            || config.issuer.len() > 256
            || config.issuer.chars().any(char::is_control)
        {
            self.error(
                "E_RT_ISSUER",
                format!("{rp}/issuer"),
                "the issuer is the `iss` the access tokens carry: 1 to 256 characters, no control character",
            );
        }
        let path = &config.path;
        let plain = path.starts_with('/')
            && path.len() <= 256
            && !path.contains("//")
            && path
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "/-_.".contains(c));
        if !plain {
            self.error(
                "E_RT_PATH",
                format!("{rp}/path"),
                "the path starts with `/` and holds letters, digits, `/`, `-`, `_` and `.` (no parameter)",
            );
        } else if ["/livez", "/readyz", "/metrics"].contains(&path.trim_end_matches('/')) {
            self.error(
                "E_RT_PATH",
                format!("{rp}/path"),
                format!("`{path}` is taken by the health and metrics endpoints of the service"),
            );
        }
    }

    /// The mail an event sends: where to read the address, the identity and the sender from, in a schema the JSON of
    /// which the mails are rendered from.
    fn mail_rule(&mut self, c: &Consumer, cp: &str, message: &str) {
        if !c.steps.is_empty() {
            self.error(
                "E_MTA_STEPS",
                format!("{cp}/steps"),
                "an event of a mail service sends its mail: it has no steps",
            );
        }
        if c.idempotent_on.is_some() {
            self.error(
                "E_MTA_STEPS",
                format!("{cp}/idempotentOn"),
                "a mail service deduplicates on the field of its mail rule",
            );
        }
        let Some(rule) = &c.mail else {
            self.error(
                "E_MTA_RULE",
                format!("{cp}/mail"),
                format!(
                    "event `{}` of a mail service needs a mail rule: where its address is",
                    c.topic
                ),
            );
            return;
        };
        let Some(msg) = self.project.message(message) else {
            return;
        };
        let text = |name: &str, path: String, what: &str, this: &mut Self| match msg.field(name) {
            Some(f) if !f.repeated && matches!(f.ty, FieldType::String | FieldType::Uuid) => {}
            Some(_) => this.error(
                "E_MTA_FIELD_TYPE",
                path,
                format!("`{name}` ({what}) must be a single text (string or uuid) of `{message}`"),
            ),
            None => this.error(
                "E_MTA_FIELD",
                path,
                format!("`{message}` has no field `{name}` ({what})"),
            ),
        };
        text(
            &rule.recipient,
            format!("{cp}/mail/recipient"),
            "the address",
            self,
        );
        text(
            &rule.dedup_field,
            format!("{cp}/mail/dedupField"),
            "the identity of the event",
            self,
        );
        if let Some(f) = &rule.sender_company_via {
            text(
                f,
                format!("{cp}/mail/senderCompanyVia"),
                "the company shown as sender",
                self,
            );
        }
        if let Some(f) = &rule.reply_to {
            text(
                f,
                format!("{cp}/mail/replyTo"),
                "the Reply-To address",
                self,
            );
        }
        // The event is rendered from its JSON: a timestamp has none yet.
        let mut todo = vec![message];
        let mut seen = HashSet::new();
        while let Some(name) = todo.pop() {
            if !seen.insert(name) {
                continue;
            }
            let Some(m) = self.project.message(name) else {
                continue;
            };
            for f in &m.fields {
                match &f.ty {
                    FieldType::Timestamp => {
                        self.error(
                            "E_MTA_EVENT_FIELD",
                            format!("{cp}/topic"),
                            format!("`{}.{}` is a timestamp, which has no JSON form yet: a mail is rendered from the JSON of its event", m.name, f.name),
                        );
                    }
                    FieldType::Message { name } => todo.push(name),
                    _ => {}
                }
            }
        }
    }

    /// The columns the tenant filter and the soft delete lean on must exist, and be what they are used as.
    fn tenant_and_soft_delete(&mut self, t: &Table, tp: &str) {
        if let Some(name) = &t.tenant_column {
            let ok = t.column(name).is_some_and(|c| {
                !c.nullable && matches!(c.ty, ColumnType::String | ColumnType::Uuid)
            });
            if !ok {
                self.error(
                    "E_TENANT_COLUMN",
                    format!("{tp}/tenantColumn"),
                    format!(
                        "`{name}` must be a column of `{}` that is required (not nullable) text or uuid: a row without a tenant would belong to nobody",
                        t.name
                    ),
                );
            }
        }
        if t.soft_delete {
            let ok = t
                .column(DELETED_AT)
                .is_some_and(|c| c.nullable && c.ty == ColumnType::Timestamp);
            if !ok {
                self.error(
                    "E_SOFT_DELETE_COLUMN",
                    format!("{tp}/softDelete"),
                    format!(
                        "a soft delete needs a nullable timestamp column `{DELETED_AT}` in `{}`",
                        t.name
                    ),
                );
            }
        }
    }

    /// The journal of changes of a table: the old names, the dropped columns and the changes of type and nullability.
    /// The generated migration trusts it, so it must be unambiguous: an old name is never a current one, and a name is
    /// never both dropped and kept.
    fn table_journal(&mut self, s: &Service, t: &Table, tp: &str) {
        let others = || s.tables.iter().filter(|o| o.name != t.name);
        for (k, old) in t.renamed_from.iter().enumerate() {
            let at = format!("{tp}/renamedFrom/{k}");
            let problem = if !is_snake_ident(old) {
                Some("must be snake_case")
            } else if *old == t.name || s.tables.iter().any(|o| o.name == *old) {
                Some("is the name of a table of the service")
            } else if others().any(|o| o.renamed_from.contains(old)) {
                Some("is also the previous name of another table")
            } else if s.dropped_tables.contains(old) {
                Some("is also listed in droppedTables")
            } else if t.renamed_from[..k].contains(old) {
                Some("is listed twice")
            } else {
                None
            };
            if let Some(problem) = problem {
                self.error(
                    "E_RENAMED_FROM",
                    at,
                    format!("previous name `{old}` of table `{}` {problem}", t.name),
                );
            }
        }
        let renamed_cols: Vec<&String> = t.columns.iter().flat_map(|c| &c.renamed_from).collect();
        for (k, name) in t.dropped_columns.iter().enumerate() {
            let at = format!("{tp}/droppedColumns/{k}");
            let problem = if !is_snake_ident(name) {
                Some("must be snake_case")
            } else if t.column(name).is_some() {
                Some("is still a column of the table: remove it from the columns first")
            } else if renamed_cols.contains(&name) {
                Some("is also the previous name of a column")
            } else if t.dropped_columns[..k].contains(name) {
                Some("is listed twice")
            } else {
                None
            };
            if let Some(problem) = problem {
                self.error(
                    "E_DROPPED_NAME",
                    at,
                    format!("dropped column `{name}` of `{}` {problem}", t.name),
                );
            }
        }
        for (j, c) in t.columns.iter().enumerate() {
            let cp = format!("{tp}/columns/{j}");
            for (k, old) in c.renamed_from.iter().enumerate() {
                let problem = if !is_snake_ident(old) {
                    Some("must be snake_case")
                } else if t.column(old).is_some() {
                    Some("is the name of a column of the table")
                } else if t
                    .columns
                    .iter()
                    .enumerate()
                    .any(|(i, o)| i != j && o.renamed_from.contains(old))
                {
                    Some("is also the previous name of another column")
                } else if c.renamed_from[..k].contains(old) {
                    Some("is listed twice")
                } else {
                    None
                };
                if let Some(problem) = problem {
                    self.error(
                        "E_RENAMED_FROM",
                        format!("{cp}/renamedFrom/{k}"),
                        format!("previous name `{old}` of column `{}` {problem}", c.name),
                    );
                }
            }
            if let Some(from) = c.retyped_from {
                if !from.converts_to(c.ty) {
                    self.error(
                        "E_RETYPE_UNSUPPORTED",
                        format!("{cp}/retypedFrom"),
                        format!(
                            "`{}` cannot be converted from {} to {}: no sound cast exists (convert through text, or add a new column and drop this one)",
                            c.name,
                            from.sql(),
                            c.ty.sql()
                        ),
                    );
                } else if from.conversion_is_lossy(c.ty) {
                    self.warn(
                        "W_RETYPE_LOSSY",
                        format!("{cp}/retypedFrom"),
                        format!(
                            "converting `{}` from {} to {} can lose precision or fail on values that do not fit (the start-up then stops, nothing is changed)",
                            c.name,
                            from.sql(),
                            c.ty.sql()
                        ),
                    );
                }
                // Postgres cannot change the type of one side of a foreign key on its own.
                let in_fk = t.foreign_keys.iter().any(|f| f.columns.contains(&c.name))
                    || s.tables.iter().any(|o| {
                        o.foreign_keys
                            .iter()
                            .any(|f| f.ref_table == t.name && f.ref_columns.contains(&c.name))
                    });
                if in_fk {
                    self.error(
                        "E_RETYPE_FK",
                        format!("{cp}/retypedFrom"),
                        format!(
                            "`{}` is part of a foreign key: its type cannot change alone (both sides would have to change at once)",
                            c.name
                        ),
                    );
                }
            }
            if let Some(was) = c.nullable_from
                && was
                && !c.nullable
                && c.default.as_deref().is_none_or(|d| d.trim().is_empty())
            {
                self.warn(
                    "W_REQUIRED_NO_DEFAULT",
                    format!("{cp}/nullableFrom"),
                    format!(
                        "`{}` becomes required with no default: the start-up stops if a row still holds a null",
                        c.name
                    ),
                );
            }
        }
    }

    /// Names of the tables dropped from a service: not kept, not ambiguous.
    fn dropped_tables(&mut self, s: &Service, base: &str) {
        for (k, name) in s.dropped_tables.iter().enumerate() {
            let problem = if !is_snake_ident(name) {
                Some("must be snake_case")
            } else if s.tables.iter().any(|t| t.name == *name) {
                Some("is still a table of the service: remove it from the tables first")
            } else if s.dropped_tables[..k].contains(name) {
                Some("is listed twice")
            } else {
                None
            };
            if let Some(problem) = problem {
                self.error(
                    "E_DROPPED_NAME",
                    format!("{base}/droppedTables/{k}"),
                    format!("dropped table `{name}` {problem}"),
                );
            }
        }
    }

    fn tables(&mut self, s: &Service, base: &str) {
        self.dropped_tables(s, base);
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
            if (s.uses_usage() && USAGE_TABLES.contains(&t.name.as_str()))
                || (s.uses_challenge_check() && t.name == CHALLENGE_TABLE)
                || (s.uses_api_keys() && t.name == API_KEYS_TABLE)
                || (s.uses_sequences() && t.name == SEQUENCES_TABLE)
                || (s.uses_audit() && t.name == AUDIT_TABLE)
                || (s.uses_flags() && t.name == FLAGS_TABLE)
            {
                self.error(
                    "E_TABLE_RESERVED",
                    format!("{tp}/name"),
                    format!(
                        "`{}` is kept by the meter, quota, challenge, API key, numbering, audit or flag steps of this service, pick another name",
                        t.name
                    ),
                );
            }
            if !names.insert(t.name.as_str()) {
                self.error(
                    "E_DUPLICATE_TABLE",
                    format!("{tp}/name"),
                    format!("table `{}` is defined twice in `{}`", t.name, s.name),
                );
            }
            self.tenant_and_soft_delete(t, &tp);
            self.table_journal(s, t, &tp);
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
            joins,
            into,
            limit,
            conflict,
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
            self.sql_where(env, scope, table, joins, c, &format!("{path}/where/{i}"));
        }
        self.tenant_in_sql(t, scope, path, step);
        if !joins.is_empty() {
            self.sql_joins(env, scope, path, step);
        }
        self.sql_aliases(env, path, step);
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
                    && env.service.sql_column(table, joins, o).is_none()
                {
                    self.error(
                        "E_UNKNOWN_COLUMN",
                        format!("{path}/orderBy"),
                        Self::unknown_sql_column(table, joins, o),
                    );
                }
                self.sql_page(env, scope, path, step);
                self.sql_result(env, path, step, t, scope);
            }
            SqlOp::Insert | SqlOp::Upsert | SqlOp::Update => {
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
                    self.sql_result(env, path, step, t, scope);
                }
                if *op == SqlOp::Upsert {
                    self.sql_upsert(path, t, conflict, values, conditions.is_empty());
                }
                if matches!(op, SqlOp::Insert | SqlOp::Upsert) {
                    for col in t.columns.iter().filter(|c| {
                        c.is_required_on_insert()
                            && t.tenant_column.as_deref() != Some(c.name.as_str())
                    }) {
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
                    self.sql_result(env, path, step, t, scope);
                }
            }
            SqlOp::Count => {
                if !columns.is_empty()
                    || into.is_some()
                    || *many
                    || order_by.is_some()
                    || limit.is_some()
                {
                    self.error(
                        "E_SQL_COUNT_ROWS",
                        path,
                        "a count reads no row: leave out columns, `into`, `many`, the sort and the limit",
                    );
                }
                match bind {
                    Some(var) => {
                        self.check_var_name(var, &format!("{path}/bind"));
                        scope.bind(var.clone(), TypeRef::scalar(FieldType::Int64));
                    }
                    None => self.error(
                        "E_SQL_BIND",
                        format!("{path}/bind"),
                        "name the variable that receives the number of rows",
                    ),
                }
            }
        }
        if *op != SqlOp::Upsert && !conflict.is_empty() {
            self.error(
                "E_SQL_CONFLICT_OP",
                format!("{path}/conflict"),
                "the conflict key only exists in an upsert",
            );
        }
        if !matches!(op, SqlOp::Select | SqlOp::Count)
            && (!joins.is_empty() || Self::sql_read_only_fields(step))
        {
            self.error(
                "E_SQL_JOIN_OP",
                path,
                "joins, extra sort keys and pagination only exist in a select",
            );
        }

        if *op == SqlOp::Count && Self::sql_read_only_fields(step) {
            self.error(
                "E_SQL_JOIN_OP",
                path,
                "extra sort keys and pagination only exist in a select that reads rows",
            );
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

    /// The variable that scopes a table to its tenant is bound, and fits the column.
    fn tenant_scope(&mut self, t: &Table, scope: &Scope, path: &str) {
        let Some(name) = &t.tenant_column else {
            return;
        };
        let Some(col) = t.column(name) else { return };
        let want = Self::column_type(col);
        match resolve_path(self.project, scope, name) {
            Ok(found) if found.adapt_to(&want).is_some() => {}
            Ok(_) => self.error(
                "E_TENANT_SCOPE",
                path,
                format!("the variable `{name}` that scopes `{}` to its tenant is not a {}", t.name, col.ty.sql()),
            ),
            Err(_) => self.error(
                "E_TENANT_SCOPE",
                path,
                format!(
                    "table `{}` is filtered by its tenant: bind a variable `{name}` first (a `let` from the verified token)",
                    t.name
                ),
            ),
        }
    }

    /// A step on a table with a tenant: the variable of that name must be bound, as the column is, and the author does not
    /// write the column nor `deleted_at`: the generator does.
    fn tenant_in_sql(&mut self, t: &Table, scope: &Scope, path: &str, step: &Step) {
        let Step::Sql {
            op,
            conditions,
            values,
            ..
        } = step
        else {
            return;
        };
        if let Some(name) = &t.tenant_column
            && t.column(name).is_some()
        {
            self.tenant_scope(t, scope, path);
            let written = values.iter().any(|a| &a.field == name);
            let compared = conditions
                .iter()
                .flat_map(|c| c.all())
                .any(|c| &c.column == name);
            if written || compared {
                self.error(
                    "E_TENANT_MANUAL",
                    format!("{path}/{}", if written { "values" } else { "where" }),
                    format!("`{name}` is set and filtered on by the generator: leave it out"),
                );
            }
        }
        if t.soft_delete
            && matches!(op, SqlOp::Insert | SqlOp::Upsert | SqlOp::Update)
            && values.iter().any(|a| a.field == DELETED_AT)
        {
            self.error(
                "E_SOFT_DELETE_MANUAL",
                format!("{path}/values"),
                format!("`{DELETED_AT}` is set by a delete: leave it out"),
            );
        }
    }

    /// An upsert: a key to conflict on, made of columns of the table that the step writes, on a table whose rows are not
    /// soft-deleted (the conflicting row could be a deleted one, which an update would not bring back).
    fn sql_upsert(
        &mut self,
        path: &str,
        t: &Table,
        conflict: &[String],
        values: &[Assignment],
        no_where: bool,
    ) {
        let cp = format!("{path}/conflict");
        if !no_where {
            self.error(
                "E_SQL_UPSERT_WHERE",
                format!("{path}/where"),
                "an upsert has no `where`: its key is `conflict`",
            );
        }
        if t.soft_delete {
            self.error(
                "E_SQL_UPSERT_SOFT_DELETE",
                path,
                format!(
                    "`{}` keeps its deleted rows: an upsert could update a deleted one. Use a select, then an insert or an update",
                    t.name
                ),
            );
        }
        if conflict.is_empty() {
            self.error(
                "E_SQL_NO_CONFLICT",
                &cp,
                "an upsert needs the columns of the unique key to conflict on",
            );
        }
        let mut seen = HashSet::new();
        for (i, name) in conflict.iter().enumerate() {
            let ip = format!("{cp}/{i}");
            if !seen.insert(name.as_str()) {
                self.error(
                    "E_SQL_DUPLICATE_COLUMN",
                    &ip,
                    format!("`{name}` is listed twice"),
                );
            }
            if t.column(name).is_none() {
                self.error(
                    "E_UNKNOWN_COLUMN",
                    &ip,
                    format!("table `{}` has no column `{name}`", t.name),
                );
            } else if t.tenant_column.as_deref() == Some(name.as_str()) {
                self.error(
                    "E_TENANT_MANUAL",
                    &ip,
                    format!("`{name}` is the tenant: the generator already keeps the key inside it, leave it out"),
                );
            } else if !values.iter().any(|a| &a.field == name) {
                self.error(
                    "E_SQL_CONFLICT_UNSET",
                    &ip,
                    format!("`{name}` is part of the key: give it a value"),
                );
            }
        }
    }

    /// `columnAs`: each key is a ticked column, each name a valid field name, and no two columns give the same field.
    fn sql_aliases(&mut self, env: &Env, path: &str, step: &Step) {
        let Step::Sql {
            table,
            joins,
            columns,
            column_as,
            ..
        } = step
        else {
            return;
        };
        if column_as.is_empty() {
            return;
        }
        for (k, a) in column_as {
            let ap = format!("{path}/columnAs/{k}");
            if !columns.contains(k) {
                self.error(
                    "E_SQL_ALIAS_COLUMN",
                    &ap,
                    format!("`{k}` is not one of the columns the step reads"),
                );
            }
            if !is_snake_ident(a) {
                self.error(
                    "E_SQL_ALIAS_NAME",
                    &ap,
                    format!("`{a}` must be a snake_case field name that is not a Rust keyword"),
                );
            }
        }
        // With joins, the clash of two fields is already reported by the joins.
        if joins.is_empty() {
            let mut fields: HashSet<String> = HashSet::new();
            for (i, o) in env
                .service
                .sql_out_columns(table, joins, columns, column_as)
                .iter()
                .enumerate()
            {
                if !fields.insert(o.field.clone()) {
                    self.error(
                        "E_SQL_COLUMN_CLASH",
                        format!("{path}/columns/{i}"),
                        format!("two columns give the field `{}` of the row", o.field),
                    );
                }
            }
        }
    }

    fn unknown_sql_column(table: &str, joins: &[SqlJoin], name: &str) -> String {
        match name.split_once('.') {
            Some((t, _)) if t != table && !joins.iter().any(|j| j.table == t) => {
                format!("`{t}` is not in the query: join it first (`{name}`)")
            }
            Some((t, c)) => format!("table `{t}` has no column `{c}`"),
            None => format!("table `{table}` has no column `{name}`"),
        }
    }

    /// Whether a step carries a field that only a select understands.
    fn sql_read_only_fields(step: &Step) -> bool {
        let Step::Sql {
            then_by,
            limit_from,
            offset,
            cursor,
            total_bind,
            more_bind,
            last_bind,
            ..
        } = step
        else {
            return false;
        };
        !then_by.is_empty()
            || limit_from.is_some()
            || offset.is_some()
            || !cursor.is_empty()
            || total_bind.is_some()
            || more_bind.is_some()
            || last_bind.is_some()
    }

    /// One condition of a `where` and its alternatives (`or`), each checked like a condition of its own.
    fn sql_where(
        &mut self,
        env: &Env,
        scope: &Scope,
        table: &str,
        joins: &[SqlJoin],
        c: &SqlCondition,
        cp: &str,
    ) {
        match env.service.sql_column(table, joins, &c.column) {
            None => self.error(
                "E_UNKNOWN_COLUMN",
                format!("{cp}/column"),
                Self::unknown_sql_column(table, joins, &c.column),
            ),
            Some((ct, col, left)) => {
                self.sql_test(scope, c, col, left, cp);
                if ct.name != *table && ct.tenant_column.as_deref() == Some(col.name.as_str()) {
                    self.error(
                        "E_TENANT_MANUAL",
                        format!("{cp}/column"),
                        format!(
                            "`{}` is filtered on by the generator for `{}`: leave it out",
                            col.name, ct.name
                        ),
                    );
                }
            }
        }
        for (i, alt) in c.or.iter().enumerate() {
            self.sql_where(env, scope, table, joins, alt, &format!("{cp}/or/{i}"));
        }
    }

    /// A condition of a `where`: its value is typed after the column (or a list of them, or a text), by what is tested.
    fn sql_test(&mut self, scope: &Scope, c: &SqlCondition, col: &Column, left: bool, path: &str) {
        let value = format!("{path}/value");
        match c.test {
            SqlTest::Compare => self.sql_value(scope, &c.value, col, &value),
            SqlTest::In => {
                if matches!(c.value, Expr::Now) {
                    self.error("E_SQL_TEST_VALUE", &value, "`now()` is not a list");
                    return;
                }
                let list = TypeRef {
                    repeated: true,
                    ..TypeRef::scalar(col.ty.field_type())
                };
                self.check_expr(scope, &c.value, Some(&list), &value);
            }
            SqlTest::Contains | SqlTest::StartsWith => {
                if col.ty != ColumnType::String {
                    self.error(
                        "E_SQL_TEST_VALUE",
                        format!("{path}/test"),
                        format!(
                            "a text search needs a text column: `{}` is {}",
                            col.name,
                            col.ty.sql()
                        ),
                    );
                }
                if matches!(c.value, Expr::Now) {
                    self.error("E_SQL_TEST_VALUE", &value, "`now()` is not a text");
                    return;
                }
                self.check_expr(
                    scope,
                    &c.value,
                    Some(&TypeRef::scalar(FieldType::String)),
                    &value,
                );
            }
            SqlTest::IsNull | SqlTest::IsNotNull => {
                if !col.nullable && !left {
                    self.warn(
                        "W_SQL_NEVER_NULL",
                        format!("{path}/test"),
                        format!(
                            "`{}` is never null: the condition is decided already",
                            col.name
                        ),
                    );
                }
            }
        }
    }

    /// The joins of a select: tables of the service, once each, matched by columns of the same type; the rules of each
    /// joined table (tenant) need their variable, as the table of the step does.
    fn sql_joins(&mut self, env: &Env, scope: &Scope, path: &str, step: &Step) {
        let Step::Sql { table, joins, .. } = step else {
            return;
        };
        let mut seen: Vec<&str> = vec![table.as_str()];
        for (i, j) in joins.iter().enumerate() {
            let jp = format!("{path}/joins/{i}");
            let Some(jt) = env.service.table(&j.table) else {
                self.error(
                    "E_SQL_JOIN_TABLE",
                    format!("{jp}/table"),
                    format!("`{}` has no table `{}`", env.service.name, j.table),
                );
                continue;
            };
            if seen.contains(&j.table.as_str()) {
                self.error(
                    "E_SQL_JOIN_DUPLICATE",
                    format!("{jp}/table"),
                    format!(
                        "table `{}` is already in the query: a table is joined once",
                        j.table
                    ),
                );
            }
            if j.on.is_empty() {
                self.error(
                    "E_SQL_JOIN_ON",
                    format!("{jp}/on"),
                    "say which columns match (a join without a condition pairs every row with every row)",
                );
            }
            for (k, on) in j.on.iter().enumerate() {
                let op = format!("{jp}/on/{k}");
                // The left side can only be a table that comes before.
                let before: Vec<SqlJoin> = joins[..i].to_vec();
                let left = env.service.sql_column(table, &before, &on.column);
                let right = jt.column(&on.ref_column);
                if left.is_none() {
                    self.error(
                        "E_SQL_JOIN_ON",
                        format!("{op}/column"),
                        Self::unknown_sql_column(table, &before, &on.column),
                    );
                }
                if right.is_none() {
                    self.error(
                        "E_SQL_JOIN_ON",
                        format!("{op}/refColumn"),
                        format!("table `{}` has no column `{}`", j.table, on.ref_column),
                    );
                }
                if let (Some((_, l, _)), Some(r)) = (left, right)
                    && l.ty != r.ty
                {
                    self.error(
                        "E_SQL_JOIN_TYPE",
                        &op,
                        format!(
                            "`{}` is {} and `{}.{}` is {}: only columns of the same type match",
                            on.column,
                            l.ty.sql(),
                            j.table,
                            r.name,
                            r.ty.sql()
                        ),
                    );
                }
            }
            seen.push(&j.table);
            self.tenant_scope(jt, scope, &jp);
        }
        // Two columns that would give the same field of the row.
        let mut fields: HashSet<String> = HashSet::new();
        let Step::Sql {
            columns, column_as, ..
        } = step
        else {
            return;
        };
        for (i, o) in env
            .service
            .sql_out_columns(table, joins, columns, column_as)
            .iter()
            .enumerate()
        {
            if !fields.insert(o.field.clone()) {
                self.error(
                    "E_SQL_COLUMN_CLASH",
                    format!("{path}/columns/{i}"),
                    format!("two columns give the field `{}` of the row", o.field),
                );
            }
        }
    }

    /// The pagination of a select, and the sort keys it relies on.
    fn sql_page(&mut self, env: &Env, scope: &Scope, path: &str, step: &Step) {
        let Step::Sql {
            table,
            many,
            into,
            order_by,
            then_by,
            limit,
            limit_from,
            offset,
            cursor,
            total_bind,
            more_bind,
            last_bind,
            joins,
            ..
        } = step
        else {
            return;
        };
        for (i, k) in then_by.iter().enumerate() {
            if env.service.sql_column(table, joins, &k.column).is_none() {
                self.error(
                    "E_UNKNOWN_COLUMN",
                    format!("{path}/thenBy/{i}/column"),
                    Self::unknown_sql_column(table, joins, &k.column),
                );
            }
        }
        if let Some(e) = limit_from {
            self.sql_int(scope, e, &format!("{path}/limitFrom"));
        }
        if let Some(e) = offset {
            self.sql_int(scope, e, &format!("{path}/offset"));
        }
        let paged = limit_from.is_some()
            || offset.is_some()
            || !cursor.is_empty()
            || more_bind.is_some()
            || last_bind.is_some();
        if paged && (!*many || limit.is_none()) {
            self.error(
                "E_SQL_PAGE_NEEDS_LIMIT",
                format!("{path}/limit"),
                "a page needs a list of rows (`many`) and a `limit`, the largest page",
            );
        }
        if offset.is_some() && !cursor.is_empty() {
            self.error(
                "E_SQL_PAGE_CONFLICT",
                format!("{path}/offset"),
                "an offset and a cursor are two ways to pick the page: keep one",
            );
        }
        if (offset.is_some() || !cursor.is_empty()) && order_by.is_none() {
            self.warn(
                "W_SQL_PAGE_NO_ORDER",
                format!("{path}/orderBy"),
                "without a sort, the database may order the rows differently from one page to the next",
            );
        }
        if last_bind.is_some() && into.is_none() {
            self.error(
                "E_SQL_LAST_INTO",
                format!("{path}/lastBind"),
                "the last row is typed by the schema the rows are converted into: choose one (`into`)",
            );
        }
        if !cursor.is_empty() {
            let keys: Vec<&str> = order_by
                .iter()
                .map(String::as_str)
                .chain(then_by.iter().map(|k| k.column.as_str()))
                .collect();
            if cursor.len() != keys.len() {
                self.error(
                    "E_SQL_CURSOR_ARITY",
                    format!("{path}/cursor"),
                    format!(
                        "the cursor has {} value(s) and the sort has {} key(s): one value per key",
                        cursor.len(),
                        keys.len()
                    ),
                );
            }
            let mut unique_per_table: HashMap<&str, Vec<&str>> = HashMap::new();
            for (i, (k, e)) in keys.iter().zip(cursor).enumerate() {
                let Some((kt, kc, left)) = env.service.sql_column(table, joins, k) else {
                    continue;
                };
                if kc.nullable || left {
                    self.error(
                        "E_SQL_CURSOR_NULLABLE",
                        format!("{path}/cursor/{i}"),
                        format!(
                            "`{k}` can be null: a null sorts at an end and cannot be compared, so it cannot carry a cursor"
                        ),
                    );
                }
                if matches!(e, Expr::Now) {
                    self.error(
                        "E_SQL_TEST_VALUE",
                        format!("{path}/cursor/{i}"),
                        "`now()` is not a cursor value",
                    );
                } else {
                    let mut want = Self::column_type(kc);
                    want.optional = true;
                    self.check_expr(scope, e, Some(&want), &format!("{path}/cursor/{i}"));
                }
                unique_per_table.entry(&kt.name).or_default().push(&kc.name);
            }
            // The last key must break every tie, or a row can be skipped or shown twice between two pages.
            let mut tables: Vec<(&str, Option<&SqlJoin>)> = vec![(table.as_str(), None)];
            tables.extend(joins.iter().map(|j| (j.table.as_str(), Some(j))));
            let total = tables.iter().all(|(name, join)| {
                let Some(t) = env.service.table(name) else {
                    return true;
                };
                let identifies = |have: &[&str]| {
                    let covers = |cols: &[String]| {
                        !cols.is_empty() && cols.iter().all(|c| have.contains(&c.as_str()))
                    };
                    let pk: Vec<String> = t.primary_key().iter().map(|c| c.to_string()).collect();
                    covers(&pk)
                        || t.uniques.iter().any(|u| covers(&u.columns))
                        || t.indexes.iter().any(|x| x.unique && covers(&x.columns))
                };
                // A joined table matched on a whole unique key gives at most one row per row: it cannot create a tie.
                let matched: Vec<&str> = join
                    .map(|j| j.on.iter().map(|o| o.ref_column.as_str()).collect())
                    .unwrap_or_default();
                identifies(&unique_per_table.get(name).cloned().unwrap_or_default())
                    || (join.is_some() && identifies(&matched))
            });
            if !total {
                self.warn(
                    "W_SQL_CURSOR_NOT_UNIQUE",
                    format!("{path}/cursor"),
                    "the sort does not identify a row (add the primary key, or a unique column, as the last key of every table): rows that tie can be skipped or repeated between pages",
                );
            }
        }
        for (name, p) in [(total_bind, "totalBind"), (more_bind, "moreBind")] {
            if let Some(v) = name {
                if !*many {
                    self.error(
                        "E_SQL_PAGE_NEEDS_LIMIT",
                        format!("{path}/{p}"),
                        "a total and a next page only exist for a list of rows (`many`)",
                    );
                }
                self.check_var_name(v, &format!("{path}/{p}"));
            }
        }
        if let Some(v) = last_bind {
            self.check_var_name(v, &format!("{path}/lastBind"));
        }
    }

    /// A page size or an offset: a whole number from the flow, or written in the step.
    fn sql_int(&mut self, scope: &Scope, e: &Expr, path: &str) {
        match e {
            Expr::Literal { value } if value.as_u64().is_some() => {}
            Expr::Literal { .. } | Expr::Now => {
                self.error("E_SQL_PAGE_VALUE", path, "a whole number, zero or more")
            }
            Expr::Raw { .. } => {
                self.check_expr(scope, e, None, path);
            }
            // A path, a text or an operation: typed by the same table as everywhere else.
            Expr::Path { .. } | Expr::Template { .. } | Expr::Call { .. } => {
                if let Some(ty) = self.check_expr(scope, e, None, path)
                    && (ty.repeated || !matches!(ty.ty, FieldType::Int32 | FieldType::Int64))
                {
                    self.error(
                        "E_SQL_PAGE_VALUE",
                        path,
                        format!("this is {ty}: a page size or an offset is an int32 or an int64"),
                    );
                }
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
    fn sql_result(&mut self, env: &Env, path: &str, step: &Step, _t: &Table, scope: &mut Scope) {
        let Step::Sql {
            id,
            table,
            bind,
            columns,
            many,
            into,
            joins,
            column_as,
            total_bind,
            more_bind,
            last_bind,
            ..
        } = step
        else {
            return;
        };
        for (i, name) in columns.iter().enumerate() {
            if env.service.sql_column(table, joins, name).is_none() {
                self.error(
                    "E_UNKNOWN_COLUMN",
                    format!("{path}/columns/{i}"),
                    Self::unknown_sql_column(table, joins, name),
                );
            }
        }
        let outs = env
            .service
            .sql_out_columns(table, joins, columns, column_as);
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
                self.sql_into(path, table, &outs, schema);
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
        // What a page adds: the number of rows that match, whether a page follows, the last row.
        if let Some(v) = total_bind {
            scope.bind(v.clone(), TypeRef::scalar(FieldType::Int64));
        }
        if let Some(v) = more_bind {
            scope.bind(v.clone(), TypeRef::scalar(FieldType::Bool));
        }
        if let (Some(v), Some(schema)) = (last_bind, into) {
            scope.bind(v.clone(), TypeRef::message(schema));
        }
    }

    /// `into`: every field of the schema is filled by the column of the same name, with the same type.
    fn sql_into(&mut self, path: &str, table: &str, outs: &[SqlOutColumn], schema: &str) {
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
            let ok = match outs.iter().find(|o| o.field == f.name) {
                None => Err(format!("no ticked column `{}` fills it", f.name)),
                Some(o) if f.repeated || f.ty != o.column.ty.field_type() => Err(format!(
                    "`{}` is a {} column, the field is {}",
                    o.field,
                    o.column.ty.sql(),
                    TypeRef::of_field(f)
                )),
                Some(o) if o.optional && !f.optional => {
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
            self.http_call_after_write(&path, &steps[..i], step);
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
                mutable,
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
                if *mutable {
                    scope.bind_mut(name.clone(), ty);
                } else {
                    scope.bind(name.clone(), ty);
                }
                false
            }
            Step::Assign { name, value, .. } => {
                match scope.get(name).cloned() {
                    None => self.error(
                        "E_UNKNOWN_VAR",
                        format!("{path}/name"),
                        format!("unknown variable `{name}`"),
                    ),
                    Some(_) if !scope.is_mutable(name) => self.error(
                        "E_ASSIGN_IMMUTABLE",
                        format!("{path}/name"),
                        format!("`{name}` cannot be assigned: tick `mutable` on its `let` step"),
                    ),
                    Some(ty) => {
                        self.check_expr(scope, value, Some(&ty), &format!("{path}/value"));
                    }
                }
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
            Step::SessionTrace {
                service,
                session_id,
                account_id,
                tenant_id,
                ttl_seconds,
                ..
            } => {
                self.realtime_target(path, service);
                let id = TypeRef::scalar(FieldType::Uuid);
                self.check_expr(scope, session_id, Some(&id), &format!("{path}/sessionId"));
                self.check_expr(scope, account_id, Some(&id), &format!("{path}/accountId"));
                if let Some(e) = tenant_id {
                    self.check_expr(scope, e, Some(&id), &format!("{path}/tenantId"));
                }
                let ttl = TypeRef::scalar(FieldType::Int64);
                self.check_expr(
                    scope,
                    ttl_seconds,
                    Some(&ttl),
                    &format!("{path}/ttlSeconds"),
                );
                false
            }
            Step::SessionRevoke {
                service,
                session_id,
                ..
            } => {
                self.realtime_target(path, service);
                let id = TypeRef::scalar(FieldType::Uuid);
                self.check_expr(scope, session_id, Some(&id), &format!("{path}/sessionId"));
                false
            }
            Step::PublishLive {
                service,
                event,
                account_id,
                tenant_id,
                permission,
                account_ids,
                origin,
                data,
                ..
            } => {
                self.publish_live(
                    scope,
                    path,
                    service,
                    event,
                    permission.as_deref(),
                    [account_id, tenant_id, origin],
                    account_ids,
                    data,
                );
                false
            }
            Step::LiveGrants {
                service,
                account_id,
                tenant_id,
                permissions,
                ..
            } => {
                self.live_grants(scope, path, service, account_id, tenant_id, permissions);
                false
            }
            Step::CallRpc {
                service,
                method,
                request,
                bind,
                circuit,
                max_concurrent,
                ..
            } => {
                self.resilience(path, circuit, max_concurrent);
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
            Step::ForEach {
                item, list, steps, ..
            } => {
                self.check_var_name(item, &format!("{path}/item"));
                let at = format!("{path}/list");
                let mut inner = scope.clone();
                match self.check_expr(scope, list, None, &at) {
                    Some(t) if t.repeated => {
                        inner.bind(
                            item.clone(),
                            TypeRef {
                                repeated: false,
                                ..t
                            },
                        );
                    }
                    Some(t) => self.error(
                        "E_FOREACH_LIST",
                        &at,
                        format!("a loop runs over a list, found a {t}"),
                    ),
                    None => {
                        if matches!(list, Expr::Raw { .. }) {
                            self.error(
                                "E_FOREACH_LIST",
                                &at,
                                "the editor cannot tell the type of Rust code: loop over a list variable",
                            );
                        }
                    }
                }
                if scope.get(item).is_some() {
                    self.error(
                        "E_VAR_SHADOW",
                        format!("{path}/item"),
                        format!("`{item}` already names a variable of the flow"),
                    );
                }
                let mut responds = false;
                for s in steps {
                    s.walk(&mut |st| responds |= matches!(st, Step::Respond { .. }));
                }
                if responds {
                    self.error(
                        "E_RESPOND_IN_LOOP",
                        format!("{path}/steps"),
                        "a `respond` inside a loop would answer once per element: answer after the loop",
                    );
                }
                self.steps(env, &format!("{path}/steps"), steps, &mut inner);
                false
            }
            Step::Respond {
                fields,
                status,
                cookies,
                headers,
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
                        if !headers.is_empty() {
                            self.error(
                                "E_HEADER_OUTSIDE_ROUTE",
                                format!("{path}/headers"),
                                "response headers are HTTP: they only make sense in an HTTP route",
                            );
                        }
                    }
                    Handler::Route { response } => {
                        if let Some(msg) = p.message(response) {
                            self.assignments(msg, fields, scope, &format!("{path}/fields"));
                        }
                        self.respond_extras(path, scope, *status, cookies);
                        self.respond_headers(path, scope, headers);
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
                let for_realtime = self.project.services.iter().any(|s| {
                    s.kind == ServiceKind::Realtime
                        && s.realtime.as_ref().is_some_and(|r| r.issuer == *issuer)
                });
                if !for_realtime
                    && claims.iter().any(|c| c.field == "sid")
                    && let Some(r) = self.project.services.iter().find_map(|s| {
                        s.realtime
                            .as_ref()
                            .filter(|_| s.kind == ServiceKind::Realtime)
                    })
                {
                    self.warn(
                        "W_RT_ISSUER",
                        format!("{path}/issuer"),
                        format!(
                            "this token carries a session id (`sid`) but its issuer is `{issuer}`: the real-time service only accepts `{}` and answers 401",
                            r.issuer
                        ),
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
                    let claim_type =
                        self.check_expr(scope, &claim.value, None, &format!("{cp}/value"));
                    if let Some(t) = &claim_type
                        && (t.is_message()
                            || matches!(t.ty, FieldType::Bytes | FieldType::Timestamp))
                    {
                        self.error(
                            "E_JWT_CLAIM_TYPE",
                            format!("{cp}/value"),
                            format!("claim `{name}` is a {t}: a claim is a text, a number, a boolean, or a list of those"),
                        );
                    }
                    if for_realtime
                        && matches!(name, "sub" | "tid" | "sid")
                        && let Some(t) = &claim_type
                        && (t.repeated || !matches!(t.ty, FieldType::Uuid | FieldType::String))
                    {
                        self.warn(
                            "W_RT_CLAIM_UUID",
                            format!("{cp}/value"),
                            format!("the real-time service reads `{name}` as a UUID: this claim is a {t}"),
                        );
                    }
                }
                if for_realtime {
                    self.realtime_claims(path, claims.iter().map(|c| c.field.as_str()));
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
            Step::TotpGenerate {
                key_env,
                issuer,
                account,
                bind,
                ..
            } => {
                self.check_var_name(bind, &format!("{path}/bind"));
                self.jwt_key_env(&format!("{path}/keyEnv"), key_env);
                // The issuer is a part of the label of the `otpauth://` URI, which a colon would split.
                if issuer.trim().is_empty()
                    || issuer.chars().count() > 64
                    || issuer.contains(':')
                    || issuer.chars().any(char::is_control)
                {
                    self.error(
                        "E_TOTP_ISSUER",
                        format!("{path}/issuer"),
                        "the issuer is the name the authenticator app shows: 1 to 64 characters, no colon",
                    );
                }
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, account, Some(&text), &format!("{path}/account"));
                self.totp_schema(path);
                scope.bind(bind.clone(), TypeRef::message("TotpSetup"));
                false
            }
            Step::TotpVerify {
                key_env,
                sealed,
                code,
                account,
                ..
            } => {
                self.totp_in_route(env, path, "a refused code");
                self.jwt_key_env(&format!("{path}/keyEnv"), key_env);
                let text = TypeRef::scalar(FieldType::String);
                for (name, e) in [("sealed", sealed), ("code", code), ("account", account)] {
                    self.check_expr(scope, e, Some(&text), &format!("{path}/{name}"));
                }
                false
            }
            Step::RecoveryCodes {
                account,
                count,
                bind,
                ..
            } => {
                self.check_var_name(bind, &format!("{path}/bind"));
                if !(1..=MAX_RECOVERY_CODES).contains(count) {
                    self.error(
                        "E_RECOVERY_COUNT",
                        format!("{path}/count"),
                        format!("issue 1 to {MAX_RECOVERY_CODES} recovery codes"),
                    );
                }
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, account, Some(&text), &format!("{path}/account"));
                scope.bind(
                    bind.clone(),
                    TypeRef {
                        repeated: true,
                        ..text
                    },
                );
                false
            }
            Step::RecoveryCodeUse { account, code, .. } => {
                self.totp_in_route(env, path, "a refused recovery code");
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, account, Some(&text), &format!("{path}/account"));
                self.check_expr(scope, code, Some(&text), &format!("{path}/code"));
                false
            }
            Step::PasskeyRegisterStart {
                account,
                name,
                app_url,
                bind,
                ..
            } => {
                self.passkey_in_route(env, path);
                self.check_var_name(bind, &format!("{path}/bind"));
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, account, Some(&text), &format!("{path}/account"));
                self.check_expr(scope, name, Some(&text), &format!("{path}/name"));
                self.check_app_url(scope, app_url, path);
                self.passkey_schema(path);
                scope.bind(bind.clone(), TypeRef::message("PasskeyChallenge"));
                false
            }
            Step::PasskeyRegisterFinish {
                state_id,
                credential,
                account,
                app_url,
                ..
            } => {
                self.passkey_in_route(env, path);
                let text = TypeRef::scalar(FieldType::String);
                for (name, e) in [
                    ("stateId", state_id),
                    ("credential", credential),
                    ("account", account),
                ] {
                    self.check_expr(scope, e, Some(&text), &format!("{path}/{name}"));
                }
                self.check_app_url(scope, app_url, path);
                false
            }
            Step::PasskeyLoginStart {
                account,
                app_url,
                bind,
                ..
            } => {
                self.passkey_in_route(env, path);
                self.check_var_name(bind, &format!("{path}/bind"));
                let text = TypeRef::scalar(FieldType::String);
                if let Some(account) = account {
                    self.check_expr(scope, account, Some(&text), &format!("{path}/account"));
                }
                self.check_app_url(scope, app_url, path);
                self.passkey_schema(path);
                scope.bind(bind.clone(), TypeRef::message("PasskeyChallenge"));
                false
            }
            Step::PasskeyLoginFinish {
                state_id,
                credential,
                app_url,
                bind,
                ..
            } => {
                self.passkey_in_route(env, path);
                self.check_var_name(bind, &format!("{path}/bind"));
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, state_id, Some(&text), &format!("{path}/stateId"));
                self.check_expr(
                    scope,
                    credential,
                    Some(&text),
                    &format!("{path}/credential"),
                );
                self.check_app_url(scope, app_url, path);
                scope.bind(bind.clone(), text);
                false
            }
            Step::ImageSniff {
                input,
                allow,
                max_bytes,
                bind,
                ..
            } => {
                if !matches!(env.handler, Handler::Route { .. }) {
                    self.error(
                        "E_IMAGE_OUTSIDE_ROUTE",
                        path,
                        "a refused image is a 400: checking one only makes sense in an HTTP route",
                    );
                }
                self.check_var_name(bind, &format!("{path}/bind"));
                if allow.is_empty() {
                    self.error(
                        "E_IMAGE_ALLOW",
                        format!("{path}/allow"),
                        "accept at least one image type (png, jpeg)",
                    );
                }
                let mut seen = HashSet::new();
                for (i, kind) in allow.iter().enumerate() {
                    if !seen.insert(*kind) {
                        self.error(
                            "E_IMAGE_ALLOW",
                            format!("{path}/allow/{i}"),
                            format!("`{}` is listed twice", kind.as_str()),
                        );
                    }
                }
                if !(1..=MAX_OBJECT_BYTES).contains(max_bytes) {
                    self.error(
                        "E_IMAGE_SIZE",
                        format!("{path}/maxBytes"),
                        format!(
                            "the largest image is between 1 byte and {MAX_OBJECT_BYTES} (10 MiB)"
                        ),
                    );
                }
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, input, Some(&text), &format!("{path}/input"));
                scope.bind(bind.clone(), text);
                false
            }
            Step::ObjectPut {
                bucket_env,
                key,
                body,
                content_type,
                ..
            } => {
                self.bucket_env(&format!("{path}/bucketEnv"), bucket_env);
                let text = TypeRef::scalar(FieldType::String);
                for (name, e) in [("key", key), ("body", body), ("contentType", content_type)] {
                    self.check_expr(scope, e, Some(&text), &format!("{path}/{name}"));
                }
                false
            }
            Step::ObjectGet {
                bucket_env,
                key,
                bind,
                ..
            } => {
                self.bucket_env(&format!("{path}/bucketEnv"), bucket_env);
                self.check_var_name(bind, &format!("{path}/bind"));
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, key, Some(&text), &format!("{path}/key"));
                scope.bind(
                    bind.clone(),
                    TypeRef {
                        optional: true,
                        ..text
                    },
                );
                false
            }
            Step::ObjectPresignGet {
                bucket_env,
                key,
                expires_secs,
                bind,
                ..
            } => {
                self.bucket_env(&format!("{path}/bucketEnv"), bucket_env);
                self.check_var_name(bind, &format!("{path}/bind"));
                self.presign_expiry(&format!("{path}/expiresSecs"), *expires_secs);
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, key, Some(&text), &format!("{path}/key"));
                scope.bind(bind.clone(), text);
                false
            }
            Step::ObjectPresignPut {
                bucket_env,
                key,
                content_type,
                size,
                max_bytes,
                expires_secs,
                bind,
                ..
            } => {
                self.bucket_env(&format!("{path}/bucketEnv"), bucket_env);
                self.check_var_name(bind, &format!("{path}/bind"));
                self.presign_expiry(&format!("{path}/expiresSecs"), *expires_secs);
                if !(1..=MAX_OBJECT_BYTES).contains(max_bytes) {
                    self.error(
                        "E_IMAGE_SIZE",
                        format!("{path}/maxBytes"),
                        format!(
                            "the largest object is between 1 byte and {MAX_OBJECT_BYTES} (10 MiB)"
                        ),
                    );
                }
                let text = TypeRef::scalar(FieldType::String);
                for (name, e) in [("key", key), ("contentType", content_type)] {
                    self.check_expr(scope, e, Some(&text), &format!("{path}/{name}"));
                }
                let int = TypeRef::scalar(FieldType::Int64);
                self.check_expr(scope, size, Some(&int), &format!("{path}/size"));
                scope.bind(bind.clone(), text);
                false
            }
            Step::ObjectDelete {
                bucket_env, key, ..
            } => {
                self.bucket_env(&format!("{path}/bucketEnv"), bucket_env);
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, key, Some(&text), &format!("{path}/key"));
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
                shared,
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
                if *shared && *max > SHARED_RATE_LIMIT_MAX {
                    self.error(
                        "E_RATE_LIMIT_SHARED_MAX",
                        format!("{path}/max"),
                        format!(
                            "a limit counted in the shared store keeps one entry per attempt of the window: {SHARED_RATE_LIMIT_MAX} attempts at most"
                        ),
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
            Step::ApiKeyCreate {
                owner,
                name,
                scopes,
                expires_in_secs,
                bind,
                key_id_bind,
                ..
            } => {
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, owner, Some(&text), &format!("{path}/owner"));
                self.check_expr(scope, name, Some(&text), &format!("{path}/name"));
                for (i, name) in scopes.iter().enumerate() {
                    self.check_api_scope(name, &format!("{path}/scopes/{i}"));
                }
                if let Some(seconds) = expires_in_secs {
                    let seconds_type = TypeRef::scalar(FieldType::Int64);
                    self.check_expr(
                        scope,
                        seconds,
                        Some(&seconds_type),
                        &format!("{path}/expiresInSecs"),
                    );
                }
                self.check_var_name(bind, &format!("{path}/bind"));
                scope.bind(bind.clone(), text.clone());
                if let Some(kid) = key_id_bind {
                    self.check_var_name(kid, &format!("{path}/keyIdBind"));
                    if kid == bind {
                        self.error(
                            "E_API_KEY_BIND",
                            format!("{path}/keyIdBind"),
                            "the key and its id are two variables",
                        );
                    }
                    scope.bind(kid.clone(), text);
                }
                false
            }
            Step::ApiKeyVerify {
                key,
                scope: needed,
                bind,
                key_id_bind,
                ..
            } => {
                if !matches!(env.handler, Handler::Route { .. }) {
                    self.error(
                        "E_API_KEY_OUTSIDE_ROUTE",
                        path,
                        "a bad key is answered with a 401: it only makes sense in an HTTP route",
                    );
                }
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, key, Some(&text), &format!("{path}/key"));
                if let Some(needed) = needed {
                    self.check_api_scope(needed, &format!("{path}/scope"));
                }
                self.check_var_name(bind, &format!("{path}/bind"));
                scope.bind(bind.clone(), text.clone());
                if let Some(kid) = key_id_bind {
                    self.check_var_name(kid, &format!("{path}/keyIdBind"));
                    if kid == bind {
                        self.error(
                            "E_API_KEY_BIND",
                            format!("{path}/keyIdBind"),
                            "the owner and the id of the key are two variables",
                        );
                    }
                    scope.bind(kid.clone(), text);
                }
                false
            }
            Step::ApiKeyRevoke { key_id, owner, .. } => {
                if !matches!(env.handler, Handler::Route { .. }) {
                    self.error(
                        "E_API_KEY_OUTSIDE_ROUTE",
                        path,
                        "an unknown key is answered with a 404: it only makes sense in an HTTP route",
                    );
                }
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, key_id, Some(&text), &format!("{path}/keyId"));
                if let Some(owner) = owner {
                    self.check_expr(scope, owner, Some(&text), &format!("{path}/owner"));
                }
                false
            }
            Step::Flag {
                key,
                subject,
                properties,
                rollout,
                bind,
                variant_bind,
                ..
            } => {
                self.check_name_of(
                    key,
                    "E_FLAG_NAME",
                    "a flag",
                    "new_checkout`, `beta.search",
                    &format!("{path}/key"),
                );
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, subject, Some(&text), &format!("{path}/subject"));
                self.plain_values(
                    scope,
                    properties,
                    &format!("{path}/properties"),
                    "properties",
                );
                if *rollout > 100 {
                    self.error(
                        "E_FLAG_ROLLOUT",
                        format!("{path}/rollout"),
                        "the share of subjects a flag is on for is 0 to 100 (percent)",
                    );
                }
                self.check_var_name(bind, &format!("{path}/bind"));
                scope.bind(bind.clone(), TypeRef::scalar(FieldType::Bool));
                if let Some(variant) = variant_bind {
                    self.check_var_name(variant, &format!("{path}/variantBind"));
                    if variant == bind {
                        self.error(
                            "E_FLAG_BIND",
                            format!("{path}/variantBind"),
                            "the answer and the variant are two variables",
                        );
                    }
                    scope.bind(variant.clone(), text);
                }
                false
            }
            Step::Audit {
                action,
                target_type,
                target,
                actor,
                tenant,
                before,
                after,
                ..
            } => {
                self.check_name_of(
                    action,
                    "E_AUDIT_ACTION",
                    "an action",
                    "invoice.issued`, `member.removed",
                    &format!("{path}/action"),
                );
                self.check_name_of(
                    target_type,
                    "E_AUDIT_TARGET_TYPE",
                    "a kind of target",
                    "invoice`, `member",
                    &format!("{path}/targetType"),
                );
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, target, Some(&text), &format!("{path}/target"));
                self.check_expr(scope, actor, Some(&text), &format!("{path}/actor"));
                if let Some(tenant) = tenant {
                    self.check_expr(scope, tenant, Some(&text), &format!("{path}/tenant"));
                }
                self.plain_values(scope, before, &format!("{path}/before"), "before");
                self.plain_values(scope, after, &format!("{path}/after"), "after");
                false
            }
            Step::AuditRead {
                tenant,
                target_type,
                target,
                action,
                cursor,
                limit,
                bind,
                next_cursor_bind,
                ..
            } => {
                self.audit_schema(&format!("{path}/bind"));
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, tenant, Some(&text), &format!("{path}/tenant"));
                for (value, name) in [
                    (target_type, "targetType"),
                    (target, "target"),
                    (action, "action"),
                ] {
                    if let Some(value) = value {
                        self.check_expr(scope, value, Some(&text), &format!("{path}/{name}"));
                    }
                }
                if let Some(cursor) = cursor {
                    let id = TypeRef::scalar(FieldType::Int64);
                    self.check_expr(scope, cursor, Some(&id), &format!("{path}/cursor"));
                }
                if !(1..=MAX_AUDIT_PAGE).contains(limit) {
                    self.error(
                        "E_AUDIT_PAGE",
                        format!("{path}/limit"),
                        format!("a page of the journal is 1 to {MAX_AUDIT_PAGE} entries"),
                    );
                }
                self.check_var_name(bind, &format!("{path}/bind"));
                scope.bind(
                    bind.clone(),
                    TypeRef {
                        ty: FieldType::Message {
                            name: "AuditEntry".into(),
                        },
                        repeated: true,
                        optional: false,
                    },
                );
                if let Some(next) = next_cursor_bind {
                    self.check_var_name(next, &format!("{path}/nextCursorBind"));
                    if next == bind {
                        self.error(
                            "E_AUDIT_BIND",
                            format!("{path}/nextCursorBind"),
                            "the entries and the cursor of the next page are two variables",
                        );
                    }
                    scope.bind(
                        next.clone(),
                        TypeRef {
                            ty: FieldType::Int64,
                            repeated: false,
                            optional: true,
                        },
                    );
                }
                false
            }
            Step::NextNumber {
                sequence,
                scope: kept_apart,
                start,
                bind,
                ..
            } => {
                self.check_sequence_name(sequence, &format!("{path}/sequence"));
                if let Some(by) = kept_apart {
                    let text = TypeRef::scalar(FieldType::String);
                    self.check_expr(scope, by, Some(&text), &format!("{path}/scope"));
                }
                if *start < 0 {
                    self.error(
                        "E_SEQUENCE_START",
                        format!("{path}/start"),
                        "a sequence starts at 0 or more",
                    );
                }
                self.check_var_name(bind, &format!("{path}/bind"));
                scope.bind(bind.clone(), TypeRef::scalar(FieldType::Int64));
                false
            }
            Step::PasswordStrength {
                password,
                min_score,
                inputs,
                bind,
                ..
            } => {
                if !matches!(env.handler, Handler::Route { .. }) {
                    self.error(
                        "E_PASSWORD_OUTSIDE_ROUTE",
                        path,
                        "a weak password is answered with a 400: it only makes sense in an HTTP route",
                    );
                }
                if *min_score > 4 {
                    self.error(
                        "E_PASSWORD_SCORE",
                        format!("{path}/minScore"),
                        "the score goes from 0 to 4",
                    );
                }
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, password, Some(&text), &format!("{path}/password"));
                for (i, input) in inputs.iter().enumerate() {
                    self.check_expr(scope, input, Some(&text), &format!("{path}/inputs/{i}"));
                }
                if let Some(bind) = bind {
                    self.check_var_name(bind, &format!("{path}/bind"));
                    scope.bind(bind.clone(), TypeRef::scalar(FieldType::Int32));
                }
                false
            }
            Step::VerifyChallenge { payload, .. } => {
                if !matches!(env.handler, Handler::Route { .. }) {
                    self.error(
                        "E_CHALLENGE_OUTSIDE_ROUTE",
                        path,
                        "a challenge answers 400 to the visitor: it only makes sense in an HTTP route",
                    );
                }
                if env.service.challenge.is_none() {
                    self.error(
                        "E_CHALLENGE_NOT_SERVED",
                        path,
                        "this service hands out no challenge: turn it on in the service (`challenge`), with the key and the path",
                    );
                }
                let text = TypeRef::scalar(FieldType::String);
                self.check_expr(scope, payload, Some(&text), &format!("{path}/payload"));
                false
            }
            Step::Meter {
                key,
                tenant,
                amount,
                idempotency_key,
                ..
            } => {
                self.check_meter_name(key, &format!("{path}/key"));
                let text = TypeRef::scalar(FieldType::String);
                let units = TypeRef::scalar(FieldType::Int64);
                self.check_expr(scope, tenant, Some(&text), &format!("{path}/tenant"));
                self.check_expr(scope, amount, Some(&units), &format!("{path}/amount"));
                if let Some(once) = idempotency_key {
                    self.check_expr(scope, once, Some(&text), &format!("{path}/idempotencyKey"));
                }
                false
            }
            Step::CheckQuota {
                key,
                tenant,
                limit,
                wanted,
                bind,
                ..
            } => {
                if !matches!(env.handler, Handler::Route { .. }) {
                    self.error(
                        "E_QUOTA_OUTSIDE_ROUTE",
                        path,
                        "a quota answers 429 and `Retry-After`: it only makes sense in an HTTP route",
                    );
                }
                self.check_meter_name(key, &format!("{path}/key"));
                let text = TypeRef::scalar(FieldType::String);
                let units = TypeRef::scalar(FieldType::Int64);
                self.check_expr(scope, tenant, Some(&text), &format!("{path}/tenant"));
                self.check_expr(scope, limit, Some(&units), &format!("{path}/limit"));
                if let Some(wanted) = wanted {
                    self.check_expr(scope, wanted, Some(&units), &format!("{path}/wanted"));
                }
                if let Some(bind) = bind {
                    self.check_var_name(bind, &format!("{path}/bind"));
                    scope.bind(bind.clone(), units);
                }
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
            Step::RequirePermission {
                claims,
                permission,
                status,
                message,
                ..
            } => {
                if !matches!(env.handler, Handler::Route { .. }) {
                    self.error(
                        "E_FAIL_OUTSIDE_ROUTE",
                        path,
                        "RequirePermission answers an HTTP status: it only makes sense in an HTTP route",
                    );
                }
                if !(400..=599).contains(status) || matches!(status, 429 | 503) {
                    self.error(
                        "E_FAIL_STATUS",
                        format!("{path}/status"),
                        format!("`{status}` is not a refusal status (400 to 599, not 429 or 503)"),
                    );
                }
                if message.trim().is_empty() {
                    self.error(
                        "E_FAIL_MESSAGE",
                        format!("{path}/message"),
                        "say what the client is told",
                    );
                }
                if permission.trim().is_empty() {
                    self.error(
                        "E_REQUIRE_PERMISSION",
                        format!("{path}/permission"),
                        "name the permission the caller must hold (e.g. `crm.manage`)",
                    );
                }
                let list = TypeRef {
                    repeated: true,
                    ..TypeRef::scalar(FieldType::String)
                };
                self.check_expr(scope, claims, Some(&list), &format!("{path}/claims"));
                false
            }
            Step::HttpCall { .. } => {
                self.http_call(scope, path, step);
                false
            }
            // Played before the check: what the fragment holds is checked where it is included.
            Step::Include { .. } => false,
            Step::Code { code, .. } => {
                if code.trim().is_empty() {
                    self.warn("W_EMPTY_CODE", format!("{path}/code"), "empty Rust block");
                }
                false
            }
        }
    }

    /// The circuit and the bulkhead of a call to a dependency: counts, windows and delays that mean something.
    fn resilience(
        &mut self,
        path: &str,
        circuit: &Option<CircuitConfig>,
        max_concurrent: &Option<u32>,
    ) {
        if let Some(c) = circuit {
            if !(1..=MAX_CIRCUIT_FAILURES).contains(&c.failures) {
                self.error(
                    "E_CIRCUIT_FAILURES",
                    format!("{path}/circuit/failures"),
                    format!("the circuit opens after 1 to {MAX_CIRCUIT_FAILURES} failures"),
                );
            }
            if !(1..=MAX_CIRCUIT_SECS).contains(&c.window_secs) {
                self.error(
                    "E_CIRCUIT_WINDOW",
                    format!("{path}/circuit/windowSecs"),
                    format!("the window the failures are counted in is 1 second to 1 hour ({MAX_CIRCUIT_SECS})"),
                );
            }
            if !(1..=MAX_CIRCUIT_SECS).contains(&c.open_secs) {
                self.error(
                    "E_CIRCUIT_OPEN",
                    format!("{path}/circuit/openSecs"),
                    format!("the circuit stays open for 1 second to 1 hour ({MAX_CIRCUIT_SECS})"),
                );
            }
        }
        if let Some(n) = max_concurrent
            && !(1..=MAX_CONCURRENT_CALLS).contains(n)
        {
            self.error(
                "E_MAX_CONCURRENT",
                format!("{path}/maxConcurrent"),
                format!("a bulkhead lets 1 to {MAX_CONCURRENT_CALLS} calls through at a time"),
            );
        }
    }

    /// The name of what an API key may do: the same spelling in the key and in the route that asks for it.
    fn check_api_scope(&mut self, name: &str, path: &str) {
        let ok = !name.is_empty()
            && name.len() <= 64
            && name.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | ':' | '-')
            });
        if !ok {
            self.error(
                "E_API_KEY_SCOPE",
                path,
                "a scope is named with lowercase letters, digits and `_.:-` (64 characters at most): `orders:read`",
            );
        }
    }

    /// The name of a meter: what the uses are added up under, so it is spelled the same everywhere.
    fn check_meter_name(&mut self, key: &str, path: &str) {
        self.check_name_of(key, "E_METER_NAME", "a meter", "api_calls`, `seats", path);
    }

    /// The name of a sequence: spelled the same everywhere it is used.
    fn check_sequence_name(&mut self, key: &str, path: &str) {
        self.check_name_of(
            key,
            "E_SEQUENCE_NAME",
            "a sequence",
            "invoices`, `credit_notes",
            path,
        );
    }

    fn check_name_of(&mut self, key: &str, code: &str, what: &str, examples: &str, path: &str) {
        let ok = !key.is_empty()
            && key.len() <= 64
            && key.starts_with(|c: char| c.is_ascii_lowercase())
            && key.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-')
            });
        if !ok {
            self.error(
                code,
                path,
                format!(
                    "{what} is named with lowercase letters, digits, `_`, `.` and `-`, starting with a letter (64 characters at most): `{examples}`"
                ),
            );
        }
    }

    /// The name of an environment variable.
    /// The environment variable that holds the name of a bucket.
    fn bucket_env(&mut self, path: &str, name: &str) {
        if !is_env_name(name) {
            self.error(
                "E_BUCKET_ENV",
                path,
                format!("`{name}` is not an environment variable name (e.g. `AVATAR_BUCKET`)"),
            );
        }
    }

    /// A signed URL lives from a second to a week (the longest SigV4 allows).
    fn presign_expiry(&mut self, path: &str, secs: u32) {
        if !(1..=604_800).contains(&secs) {
            self.error(
                "E_PRESIGN_EXPIRY",
                path,
                "a signed URL lasts between 1 second and 7 days (604800)",
            );
        }
    }

    /// A refused code is a 401: only an HTTP route answers one.
    fn totp_in_route(&mut self, env: &Env, path: &str, what: &str) {
        if !matches!(env.handler, Handler::Route { .. }) {
            self.error(
                "E_TOTP_OUTSIDE_ROUTE",
                path,
                format!("{what} is a 401: this step only makes sense in an HTTP route"),
            );
        }
    }

    /// The optional `app_url` of a passkey step: a text.
    fn check_app_url(&mut self, scope: &Scope, app_url: &Option<Expr>, path: &str) {
        if let Some(app_url) = app_url {
            let text = TypeRef::scalar(FieldType::String);
            self.check_expr(scope, app_url, Some(&text), &format!("{path}/appUrl"));
        }
    }

    /// A refused passkey is a 400 or a 401, and the browser's answers are HTTP requests: routes only.
    fn passkey_in_route(&mut self, env: &Env, path: &str) {
        if !matches!(env.handler, Handler::Route { .. }) {
            self.error(
                "E_PASSKEY_OUTSIDE_ROUTE",
                path,
                "a passkey is registered and used by the browser, request after request: this step only makes sense in an HTTP route",
            );
        }
    }

    /// What the `start` steps hand over: the API schema `PasskeyChallenge`, two required texts.
    fn passkey_schema(&mut self, path: &str) {
        let ok = self.project.message("PasskeyChallenge").is_some_and(|m| {
            m.kind == MessageKind::Api
                && ["state_id", "options"].iter().all(|n| {
                    m.field(n)
                        .is_some_and(|f| f.ty == FieldType::String && !f.repeated && !f.optional)
                })
        });
        if !ok {
            self.error(
                "E_PASSKEY_SCHEMA",
                path,
                "starting a passkey step needs the API schema `PasskeyChallenge` (`state_id`, `options`: texts)",
            );
        }
    }

    /// Named plain values (the values of an audit entry, the properties of a flag): snake_case names, once each, and values that are
    /// text, numbers, yes/no or uuids, never a message, a list, bytes or a timestamp.
    fn plain_values(&mut self, scope: &Scope, list: &[Assignment], path: &str, what: &str) {
        let mut seen = HashSet::new();
        for (i, a) in list.iter().enumerate() {
            let ap = format!("{path}/{i}");
            if !crate::names::is_snake_ident(&a.field) || a.field.len() > 64 {
                self.error(
                    "E_PLAIN_FIELD",
                    format!("{ap}/field"),
                    "a value is named in snake_case (64 characters at most): `status`, `total_cents`",
                );
            } else if !seen.insert(a.field.as_str()) {
                self.error(
                    "E_PLAIN_FIELD",
                    format!("{ap}/field"),
                    format!("`{}` is given twice in `{what}`", a.field),
                );
            }
            self.check_expr(scope, &a.value, None, &format!("{ap}/value"));
            if let Ok(Some(ty)) = crate::typing::expr_type(self.project, scope, &a.value, None)
                && (ty.repeated
                    || matches!(
                        ty.ty,
                        FieldType::Message { .. } | FieldType::Bytes | FieldType::Timestamp
                    ))
            {
                self.error(
                    "E_PLAIN_VALUE",
                    format!("{ap}/value"),
                    format!("`{what}` keeps plain values (text, number, yes/no, uuid), and `{}` is a {ty}", a.field),
                );
            }
        }
    }

    /// What `auditRead` hands over: the API schema `AuditEntry`, with the fields the journal has.
    fn audit_schema(&mut self, path: &str) {
        let texts = [
            "at",
            "actor",
            "action",
            "target_type",
            "target_id",
            "before",
            "after",
        ];
        let ok = self.project.message("AuditEntry").is_some_and(|m| {
            m.kind == MessageKind::Api
                && texts.iter().all(|n| {
                    m.field(n)
                        .is_some_and(|f| f.ty == FieldType::String && !f.repeated && !f.optional)
                })
                && m.field("id")
                    .is_some_and(|f| f.ty == FieldType::Int64 && !f.repeated && !f.optional)
                && m.field("changed")
                    .is_some_and(|f| f.ty == FieldType::String && f.repeated)
        });
        if !ok {
            self.error(
                "E_AUDIT_SCHEMA",
                path,
                "reading the journal needs the API schema `AuditEntry`: `id` (int64), `at`, `actor`, `action`, `target_type`, `target_id`, `before`, `after` (texts) and `changed` (a list of texts)",
            );
        }
    }

    /// What `totpGenerate` hands over: the API schema `TotpSetup`, three required texts.
    fn totp_schema(&mut self, path: &str) {
        let ok = self.project.message("TotpSetup").is_some_and(|m| {
            m.kind == MessageKind::Api
                && ["sealed", "otpauth_url", "secret"].iter().all(|n| {
                    m.field(n)
                        .is_some_and(|f| f.ty == FieldType::String && !f.repeated && !f.optional)
                })
        });
        if !ok {
            self.error(
                "E_TOTP_SCHEMA",
                path,
                "enrolling a TOTP needs the API schema `TotpSetup` (`sealed`, `otpauth_url`, `secret`: texts)",
            );
        }
    }

    fn jwt_key_env(&mut self, path: &str, name: &str) {
        if !is_env_name(name) {
            self.error(
                "E_KEY_ENV",
                path,
                format!("`{name}` is not an environment variable name (e.g. `JWT_SIGNING_KEY`)"),
            );
        }
    }

    /// A token the real-time service accepts names its session (`sid`); a missing `sid` is a 401 at runtime, so say it here.
    fn realtime_claims<'n>(&mut self, path: &str, names: impl Iterator<Item = &'n str>) {
        let names: Vec<&str> = names.collect();
        if !names.contains(&"sid") {
            self.warn(
                "W_RT_NO_SID",
                format!("{path}/claims"),
                "this token is for the real-time service (same issuer): add a `sid` claim, the session id it looks up, or the stream answers 401",
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

    /// The challenge a service hands out: where, with which key, how much work and for how long. Checked wherever it is set,
    /// served or not.
    fn challenge_service(&mut self, s: &Service, base: &str) {
        let Some(cfg) = &s.challenge else {
            return;
        };
        let path = cfg.path.as_str();
        let plain = path.starts_with('/')
            && path.len() > 1
            && path.len() <= 100
            && path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b))
            && !path.contains("//");
        if !plain {
            self.error(
                "E_CHALLENGE_PATH",
                format!("{base}/challenge/path"),
                "the path of the challenge starts with `/` and holds letters, digits, `/`, `-`, `_` and `.` only (e.g. `/challenge`)",
            );
        } else if let Some(r) = s
            .routes
            .iter()
            .find(|r| r.method == HttpMethod::Get && r.path == path)
        {
            self.error(
                "E_CHALLENGE_PATH",
                format!("{base}/challenge/path"),
                format!("`{path}` is already the route `{}` of `{}`", r.name, s.name),
            );
        } else if self.project.openapi.serve && self.project.openapi.path == path {
            self.error(
                "E_CHALLENGE_PATH",
                format!("{base}/challenge/path"),
                format!("`{path}` is where the OpenAPI document is served"),
            );
        }
        if !is_env_name(&cfg.key_env) {
            self.error(
                "E_CHALLENGE_KEY_ENV",
                format!("{base}/challenge/keyEnv"),
                "the key of the challenge is read from an environment variable: capital letters, digits and `_`, not starting with a digit",
            );
        }
        if !(1..=MAX_CHALLENGE_NUMBER).contains(&cfg.max_number) {
            self.error(
                "E_CHALLENGE_WORK",
                format!("{base}/challenge/maxNumber"),
                format!(
                    "the work asked of the visitor is between 1 and {MAX_CHALLENGE_NUMBER} (about half of it in hashes: a second or two in a browser at the top)"
                ),
            );
        }
        if !(10..=86_400).contains(&cfg.ttl_secs) {
            self.error(
                "E_CHALLENGE_TTL",
                format!("{base}/challenge/ttlSecs"),
                "a challenge stays valid for 10 seconds to 24 hours (86400)",
            );
        }
        if s.routes.is_empty() {
            self.error(
                "E_CHALLENGE_NO_HTTP",
                format!("{base}/challenge"),
                "a challenge is served by the HTTP API of the service, and this service has no route",
            );
        }
    }

    /// Where the services serve their OpenAPI document: a plain path that no `GET` route of a service already uses.
    fn openapi(&mut self) {
        let cfg = &self.project.openapi;
        if !cfg.serve {
            return;
        }
        let path = cfg.path.as_str();
        let plain = path.starts_with('/')
            && path.len() > 1
            && path.len() <= 100
            && path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b))
            && !path.contains("//");
        if !plain {
            self.error(
                "E_OPENAPI_PATH",
                "/openapi/path",
                "the OpenAPI path starts with `/` and holds letters, digits, `/`, `-`, `_` and `.` only (e.g. `/openapi.json`)",
            );
            return;
        }
        for s in &self.project.services {
            if let Some(r) = s
                .routes
                .iter()
                .find(|r| r.method == HttpMethod::Get && r.path == path)
            {
                let at = format!("/services/{}/routes/{}", s.name, r.name);
                self.error(
                    "E_OPENAPI_PATH",
                    "/openapi/path",
                    format!(
                        "`{path}` is already the route `{}` of `{}` ({at})",
                        r.name, s.name
                    ),
                );
            }
        }
    }

    /// The free headers of a route response: a valid name the runtime does not already use, once, and a value that
    /// cannot break out of the header.
    fn respond_headers(&mut self, path: &str, scope: &Scope, headers: &[ResponseHeader]) {
        let text = TypeRef::scalar(FieldType::String);
        let mut names = HashSet::new();
        for (i, h) in headers.iter().enumerate() {
            let hp = format!("{path}/headers/{i}");
            // An HTTP token (RFC 9110): what can travel as a header name.
            let token = !h.name.is_empty()
                && h.name.len() <= 100
                && h.name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
            let lower = h.name.to_ascii_lowercase();
            if !token {
                self.error(
                    "E_HEADER_NAME",
                    format!("{hp}/name"),
                    format!(
                        "`{}` is not a header name: letters, digits and ``!#$%&'*+-.^_`|~`` only, 100 characters at most",
                        h.name
                    ),
                );
            } else if RESERVED_RESPONSE_HEADERS.contains(&lower.as_str())
                || lower.starts_with("access-control-")
            {
                self.error(
                    "E_HEADER_RESERVED",
                    format!("{hp}/name"),
                    format!(
                        "`{}` is set by the runtime or by cookies (`Set-Cookie`), the error step (`Retry-After`) or the CORS layer: it cannot be set here",
                        h.name
                    ),
                );
            } else if !names.insert(lower) {
                self.error(
                    "E_HEADER_DUPLICATE",
                    format!("{hp}/name"),
                    format!(
                        "header `{}` is set twice (names are not case sensitive)",
                        h.name
                    ),
                );
            }
            // A literal is known now: nothing but visible ASCII and spaces, so no line break can end the header.
            if let Expr::Literal { value } = &h.value
                && let Some(text) = value.as_str()
                && (text
                    .bytes()
                    .any(|b| !(b == b' ' || b == b'\t' || (0x21..=0x7e).contains(&b)))
                    || text.starts_with([' ', '\t'])
                    || text.ends_with([' ', '\t']))
            {
                self.error(
                    "E_HEADER_VALUE",
                    format!("{hp}/value"),
                    "a header value is visible ASCII (and inner spaces), with no line break or space around it",
                );
            }
            self.check_expr(scope, &h.value, Some(&text), &format!("{hp}/value"));
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

    /// The steps that write in the store of a real-time service: the service must be one.
    fn realtime_target(&mut self, path: &str, service: &str) {
        match self.project.service(service) {
            Some(s) if s.kind == ServiceKind::Realtime => {}
            Some(_) => self.error(
                "E_LIVE_SERVICE",
                format!("{path}/service"),
                format!("`{service}` is not a real-time service"),
            ),
            None => self.error(
                "E_LIVE_SERVICE",
                format!("{path}/service"),
                format!("unknown service `{service}`: pick a real-time service"),
            ),
        }
    }

    /// The step `liveGrants`: the service must be a real-time one; the account and the company are ids, the permissions a
    /// list of text (the service itself refuses a permission that is not `module.view|manage`).
    fn live_grants(
        &mut self,
        scope: &Scope,
        path: &str,
        service: &str,
        account_id: &Expr,
        tenant_id: &Option<Expr>,
        permissions: &Option<Expr>,
    ) {
        self.realtime_target(path, service);
        let id = TypeRef::scalar(FieldType::Uuid);
        self.check_expr(scope, account_id, Some(&id), &format!("{path}/accountId"));
        if let Some(e) = tenant_id {
            self.check_expr(scope, e, Some(&id), &format!("{path}/tenantId"));
        }
        if let Some(e) = permissions {
            let list = TypeRef {
                repeated: true,
                ..TypeRef::scalar(FieldType::String)
            };
            self.check_expr(scope, e, Some(&list), &format!("{path}/permissions"));
        }
    }

    /// The step `publishLive`: the service must be a real-time one, and the event something that service accepts (the same
    /// rules as `events.rs` of its template, so that a refusal is seen here and not in the logs of a running service).
    #[allow(clippy::too_many_arguments)]
    fn publish_live(
        &mut self,
        scope: &Scope,
        path: &str,
        service: &str,
        event: &str,
        permission: Option<&str>,
        [account_id, tenant_id, origin]: [&Option<Expr>; 3],
        account_ids: &Option<Expr>,
        data: &[Assignment],
    ) {
        match self.project.service(service) {
            Some(s) if s.kind == ServiceKind::Realtime => {}
            Some(_) => self.error(
                "E_LIVE_SERVICE",
                format!("{path}/service"),
                format!("`{service}` is not a real-time service"),
            ),
            None => self.error(
                "E_LIVE_SERVICE",
                format!("{path}/service"),
                format!("unknown service `{service}`: pick a real-time service"),
            ),
        }
        let segments = || event.split('.');
        let plain =
            |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
        if event.len() > 64 || segments().count() < 2 || !segments().all(plain) {
            self.error(
                "E_LIVE_EVENT",
                format!("{path}/event"),
                "the event is `domain.action`: two segments or more of lowercase letters and `_`, separated by dots (64 characters at most)",
            );
        } else if event == "grants.updated" || event == "resync" {
            self.error(
                "E_LIVE_EVENT",
                format!("{path}/event"),
                format!("`{event}` is a control event of the real-time service: pick another name"),
            );
        }
        if let Some(permission) = permission {
            let ok = permission
                .split_once('.')
                .is_some_and(|(module, level)| plain(module) && matches!(level, "view" | "manage"));
            if !ok {
                self.error(
                    "E_LIVE_PERMISSION",
                    format!("{path}/permission"),
                    "a permission is `module.view` or `module.manage`",
                );
            }
        }
        if account_id.is_some() {
            if tenant_id.is_some() || permission.is_some() || account_ids.is_some() {
                self.error(
                    "E_LIVE_AUDIENCE",
                    path,
                    "`accountId` is for every stream of one account: leave out the company, the permission and the list of accounts",
                );
            }
        } else if permission.is_none() && account_ids.is_none() {
            self.error(
                "E_LIVE_AUDIENCE",
                path,
                "an event needs an audience: an account, a permission, or a list of accounts",
            );
        }
        let id = TypeRef::scalar(FieldType::Uuid);
        for (name, e) in [
            ("accountId", account_id),
            ("tenantId", tenant_id),
            ("origin", origin),
        ] {
            if let Some(e) = e {
                self.check_expr(scope, e, Some(&id), &format!("{path}/{name}"));
            }
        }
        if let Some(e) = account_ids {
            let list = TypeRef {
                repeated: true,
                ..id
            };
            self.check_expr(scope, e, Some(&list), &format!("{path}/accountIds"));
        }
        if data.len() > 4 {
            self.error(
                "E_LIVE_DATA",
                format!("{path}/data"),
                "an event carries 4 entries at most: a few identifiers, not content",
            );
        }
        let text = TypeRef::scalar(FieldType::String);
        let mut seen = HashSet::new();
        for (i, a) in data.iter().enumerate() {
            let at = format!("{path}/data/{i}");
            if a.field.is_empty()
                || a.field.len() > 64
                || !a.field.bytes().all(|b| b.is_ascii_alphanumeric())
            {
                self.error(
                    "E_LIVE_DATA",
                    &at,
                    format!("`{}` is not a key: 1 to 64 letters or digits", a.field),
                );
            } else if !seen.insert(a.field.as_str()) {
                self.error(
                    "E_LIVE_DATA",
                    &at,
                    format!("key `{}` is set twice", a.field),
                );
            }
            self.check_expr(scope, &a.value, Some(&text), &format!("{at}/value"));
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
            Expr::Template { .. } | Expr::Call { .. } => {
                self.check_typed_expr(scope, expr, expected, path)
            }
        }
    }

    /// Templates and operations: typed as a whole by `typing::expr_type`, then each argument is walked so that an
    /// error points to the argument at fault.
    fn check_typed_expr(
        &mut self,
        scope: &Scope,
        expr: &Expr,
        expected: Option<&TypeRef>,
        path: &str,
    ) -> Option<TypeRef> {
        if let Expr::Call { args, .. } = expr {
            for (i, a) in args.iter().enumerate() {
                if let Expr::Raw { code } = a
                    && code.trim().is_empty()
                {
                    self.error(
                        "E_EMPTY_EXPR",
                        format!("{path}/args/{i}"),
                        "empty Rust expression",
                    );
                }
                if matches!(a, Expr::Now) {
                    self.error(
                        "E_NOW_OUTSIDE_SQL",
                        format!("{path}/args/{i}"),
                        "`now()` is the database clock: it only fills a timestamp column of an SQL step",
                    );
                    return None;
                }
                if matches!(a, Expr::Call { .. } | Expr::Template { .. }) {
                    // Nested: reported at its own path.
                    let before = self.out.len();
                    self.check_typed_expr(scope, a, None, &format!("{path}/args/{i}"));
                    if self.out.len() > before {
                        return None;
                    }
                }
            }
        }
        match crate::typing::expr_type(self.project, scope, expr, expected) {
            Err(e) => {
                let code = if matches!(expr, Expr::Template { .. }) {
                    "E_TEMPLATE"
                } else {
                    "E_EXPR_TYPE"
                };
                self.error(code, path, e);
                None
            }
            Ok(ty) => {
                if let (Some(want), Some(got)) = (expected, &ty)
                    && !want.accepts(got)
                    && !(!got.optional
                        && want.optional
                        && want.ty.assignable_from(&got.ty)
                        && !want.repeated
                        && !got.repeated)
                {
                    self.error(
                        "E_TYPE_MISMATCH",
                        path,
                        format!("this gives a {got} but a {want} is expected"),
                    );
                }
                ty
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
            Condition::And { conditions } | Condition::Or { conditions } => {
                let key = if matches!(cond, Condition::And { .. }) {
                    "and"
                } else {
                    "or"
                };
                if conditions.is_empty() {
                    self.warn(
                        "W_EMPTY_LOGIC",
                        path,
                        format!("an empty `{key}` is always {}", key == "and"),
                    );
                }
                for (i, c) in conditions.iter().enumerate() {
                    self.condition(scope, c, &format!("{path}/conditions/{i}"));
                }
            }
            Condition::Not { condition } => {
                self.condition(scope, condition, &format!("{path}/condition"));
            }
            Condition::Present { value } | Condition::Absent { value } => {
                let at = format!("{path}/value");
                if let Some(t) = self.check_expr(scope, value, None, &at) {
                    // A field of a schema is present or absent as well; a bare schema variable always is.
                    let has_presence = t.optional
                        || (t.is_message()
                            && !t.repeated
                            && matches!(value, Expr::Path { path } if path.contains('.')));
                    if !has_presence {
                        self.error(
                            "E_PRESENCE_TYPE",
                            at,
                            format!("a {t} is never absent: only an optional value, or a field of a schema, can be"),
                        );
                    }
                }
            }
            Condition::Empty { value } => {
                let at = format!("{path}/value");
                if let Some(t) = self.check_expr(scope, value, None, &at)
                    && !(t.repeated
                        || (!t.optional && matches!(t.ty, FieldType::String | FieldType::Uuid)))
                {
                    self.error(
                        "E_EMPTY_TYPE",
                        at,
                        format!("only a text or a list can be empty, found a {t}"),
                    );
                }
            }
            Condition::Test { value } => {
                let want = TypeRef::scalar(FieldType::Bool);
                self.check_expr(scope, value, Some(&want), &format!("{path}/value"));
            }
            Condition::In { value, values } => {
                let at = format!("{path}/value");
                let t = match value {
                    Expr::Literal { .. } => None,
                    _ => self.check_expr(scope, value, None, &at),
                };
                if let Some(t) = &t
                    && !t.is_simple_scalar()
                {
                    self.error(
                        "E_COMPARE_TYPE",
                        &at,
                        format!("only plain scalars can be compared, found {t}"),
                    );
                    return;
                }
                if values.is_empty() {
                    self.error(
                        "E_IN_EMPTY",
                        format!("{path}/values"),
                        "give at least one value",
                    );
                }
                for (i, v) in values.iter().enumerate() {
                    let vat = format!("{path}/values/{i}");
                    match (&t, v) {
                        (Some(t), v) => {
                            self.check_expr(scope, v, Some(t), &vat);
                        }
                        (None, Expr::Literal { .. }) => {}
                        (None, v) => {
                            self.check_expr(scope, v, None, &vat);
                        }
                    }
                }
            }
            Condition::Compare { left, right, .. } => {
                // Comparisons need both sides to be exactly the same plain scalar type.
                let path_type = |this: &mut Self, e: &Expr, at: &str| -> Option<TypeRef> {
                    match e {
                        Expr::Path { .. } | Expr::Call { .. } | Expr::Template { .. } => {
                            this.check_expr(scope, e, None, at)
                        }
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

    /// A service that only reads a table it shares by name with another service, which is the one that writes it:
    /// each service has its own database, so the copy read here is never fed.
    fn foreign_tables(&mut self) {
        let p = self.project;
        // (service, table) pairs written by an insert, update or delete step.
        let mut written: BTreeSet<(&str, &str)> = BTreeSet::new();
        let mut raw: BTreeSet<&str> = BTreeSet::new();
        for s in &p.services {
            for step in s.handler_steps() {
                step.walk(&mut |st| match st {
                    Step::Sql { op, table, .. } if *op != SqlOp::Select => {
                        written.insert((s.name.as_str(), table.as_str()));
                    }
                    Step::Code { .. } => {
                        raw.insert(s.name.as_str());
                    }
                    _ => {}
                });
            }
        }
        for (i, s) in p.services.iter().enumerate() {
            if raw.contains(s.name.as_str()) {
                continue;
            }
            let mut seen = BTreeSet::new();
            for step in s.handler_steps() {
                step.walk(&mut |st| {
                    let Step::Sql { op: SqlOp::Select, table, .. } = st else {
                        return;
                    };
                    let t = table.as_str();
                    if !seen.insert(t) || written.contains(&(s.name.as_str(), t)) || s.table(t).is_none() {
                        return;
                    }
                    let Some(owner) = p
                        .services
                        .iter()
                        .find(|o| o.name != s.name && o.table(t).is_some() && written.contains(&(o.name.as_str(), t)))
                    else {
                        return;
                    };
                    self.warn(
                        "W_TABLE_FOREIGN",
                        format!("/services/{i}/tables"),
                        format!(
                            "`{}` reads its table `{t}` but never writes it, while `{}` writes a table of the same name in its own database: feed it from events, or ask `{}` over RPC",
                            s.name, owner.name, owner.name
                        ),
                    );
                });
            }
        }
    }

    fn graph_warnings(&mut self) {
        self.foreign_tables();
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

    /// The orders sample with the steps of the first RPC replaced by `steps` (the request has `amount`: int64, `customer`).
    fn with_steps(steps: Vec<Step>) -> Project {
        let mut p = sample::orders();
        p.services[0].rpcs[0].steps = steps;
        p
    }

    fn lit(v: impl Into<serde_json::Value>) -> Expr {
        Expr::Literal { value: v.into() }
    }

    fn call(func: Func, args: Vec<Expr>) -> Expr {
        Expr::Call { func, args }
    }

    fn let_step(name: &str, ty: FieldType, optional: bool, value: Expr) -> Step {
        Step::Let {
            id: format!("t_{name}"),
            name: name.into(),
            value_type: ty,
            optional,
            repeated: false,
            mutable: false,
            value,
        }
    }

    fn branch(condition: Condition) -> Step {
        Step::Branch {
            id: "t_branch".into(),
            condition,
            then: vec![],
            otherwise: vec![],
        }
    }

    #[test]
    fn operations_are_typed() {
        let good = |ty: FieldType, optional: bool, value: Expr| {
            let p = with_steps(vec![let_step("x", ty, optional, value)]);
            assert_eq!(errors(&p), Vec::<String>::new());
        };
        let bad = |code: &str, ty: FieldType, optional: bool, value: Expr| {
            let p = with_steps(vec![let_step("x", ty, optional, value)]);
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                validate(&p)
            );
        };
        // `input.amount` is an int64: a literal follows it, a text does not.
        good(
            FieldType::Int64,
            false,
            call(Func::Add, vec![path("input.amount"), lit(1)]),
        );
        good(
            FieldType::Int64,
            false,
            call(
                Func::Mul,
                vec![path("input.amount"), call(Func::ToInt64, vec![lit(2)])],
            ),
        );
        bad(
            "E_EXPR_TYPE",
            FieldType::Int64,
            false,
            call(Func::Add, vec![path("input.amount"), lit("a")]),
        );
        bad(
            "E_EXPR_TYPE",
            FieldType::Int64,
            false,
            call(Func::Add, vec![lit(1)]),
        );
        bad(
            "E_EXPR_TYPE",
            FieldType::String,
            false,
            call(Func::Upper, vec![path("input.amount")]),
        );
        bad(
            "E_TYPE_MISMATCH",
            FieldType::String,
            false,
            call(Func::Add, vec![path("input.amount"), lit(1)]),
        );
        // A conversion is explicit, and a text is read into an optional number.
        good(
            FieldType::Int32,
            false,
            call(Func::ToInt32, vec![path("input.amount")]),
        );
        good(
            FieldType::Int64,
            true,
            call(Func::ParseInt64, vec![lit("12")]),
        );
        bad(
            "E_TYPE_MISMATCH",
            FieldType::Int64,
            false,
            call(Func::ParseInt64, vec![lit("12")]),
        );
        good(
            FieldType::Int64,
            false,
            call(
                Func::Default,
                vec![call(Func::ParseInt64, vec![lit("12")]), lit(0)],
            ),
        );
        // `default` wants an optional first argument; `rem` an integer.
        bad(
            "E_EXPR_TYPE",
            FieldType::Int64,
            false,
            call(Func::Default, vec![path("input.amount"), lit(0)]),
        );
        bad(
            "E_EXPR_TYPE",
            FieldType::Double,
            false,
            call(Func::Rem, vec![lit(1.5), lit(2.0)]),
        );
        good(FieldType::Uuid, false, call(Func::NewUuid, vec![]));
        bad(
            "E_EXPR_TYPE",
            FieldType::Uuid,
            false,
            call(Func::NewUuid, vec![lit(1)]),
        );
        // Templates read plain values only.
        good(
            FieldType::String,
            false,
            Expr::Template {
                template: "n {input.amount}".into(),
            },
        );
        bad(
            "E_TEMPLATE",
            FieldType::String,
            false,
            Expr::Template {
                template: "{input.customer}".into(),
            },
        );
        bad(
            "E_TEMPLATE",
            FieldType::String,
            false,
            Expr::Template {
                template: "{nope}".into(),
            },
        );
        bad(
            "E_TEMPLATE",
            FieldType::String,
            false,
            Expr::Template {
                template: "{open".into(),
            },
        );
        // The error points to the nested operation at fault.
        let p = with_steps(vec![let_step(
            "x",
            FieldType::String,
            false,
            call(
                Func::Concat,
                vec![lit("a"), call(Func::Upper, vec![path("input.amount")])],
            ),
        )]);
        assert!(
            validate(&p)
                .iter()
                .any(|d| d.code == "E_EXPR_TYPE" && d.path.ends_with("/steps/0/value/args/1")),
            "{:?}",
            validate(&p)
        );
    }

    #[test]
    fn conditions_are_checked_all_the_way_down() {
        let ok = |c: Condition| {
            let p = with_steps(vec![branch(c)]);
            assert_eq!(errors(&p), Vec::<String>::new());
        };
        let bad = |code: &str, c: Condition| {
            let p = with_steps(vec![branch(c)]);
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                validate(&p)
            );
        };
        let gt = |v: i64| Condition::Compare {
            left: path("input.amount"),
            op: CmpOp::Gt,
            right: lit(v),
        };
        ok(Condition::And {
            conditions: vec![
                gt(1),
                Condition::Not {
                    condition: Box::new(Condition::Or {
                        conditions: vec![
                            gt(5),
                            Condition::Empty {
                                value: path("input.customer.name"),
                            },
                        ],
                    }),
                },
            ],
        });
        ok(Condition::In {
            value: path("input.amount"),
            values: vec![lit(1), lit(2)],
        });
        ok(Condition::Test {
            value: call(
                Func::StartsWith,
                vec![path("input.customer.name"), lit("a")],
            ),
        });
        // A comparison takes operations on both sides.
        ok(Condition::Compare {
            left: call(Func::Length, vec![path("input.customer.name")]),
            op: CmpOp::Ge,
            right: lit(3),
        });
        // An error deep inside a group keeps its own path.
        let p = with_steps(vec![branch(Condition::And {
            conditions: vec![
                gt(1),
                Condition::Present {
                    value: path("input.amount"),
                },
            ],
        })]);
        assert!(
            validate(&p).iter().any(|d| d.code == "E_PRESENCE_TYPE"
                && d.path.ends_with("/condition/conditions/1/value")),
            "{:?}",
            validate(&p)
        );
        bad(
            "E_EMPTY_TYPE",
            Condition::Empty {
                value: path("input.amount"),
            },
        );
        bad(
            "E_TYPE_MISMATCH",
            Condition::Test {
                value: path("input.amount"),
            },
        );
        bad(
            "E_IN_EMPTY",
            Condition::In {
                value: path("input.amount"),
                values: vec![],
            },
        );
        bad(
            "E_TYPE_MISMATCH",
            Condition::In {
                value: path("input.amount"),
                values: vec![lit("a")],
            },
        );
        bad(
            "E_UNKNOWN_PATH",
            Condition::Not {
                condition: Box::new(Condition::Absent {
                    value: path("input.nope"),
                }),
            },
        );
    }

    #[test]
    fn a_loop_binds_its_element_and_keeps_the_flow_honest() {
        let mut p = sample::showcase();
        // The loop of the showcase runs over a list of lines; its element is typed after the list.
        assert_eq!(errors(&p), Vec::<String>::new());
        let route = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "add_lines")
            .unwrap();
        let Step::ForEach { steps, .. } = &mut route.steps[3] else {
            panic!("sample layout changed")
        };
        // `respond` in a loop would answer once per element.
        steps.push(Step::Respond {
            id: "r".into(),
            fields: vec![],
            status: None,
            cookies: vec![],
            headers: vec![],
        });
        assert!(errors(&p).contains(&"E_RESPOND_IN_LOOP".to_string()));
        let route = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "add_lines")
            .unwrap();
        let Step::ForEach { steps, list, .. } = &mut route.steps[3] else {
            unreachable!()
        };
        steps.pop();
        // The element is not visible after the loop, and cannot hide a variable of the flow.
        *list = path("input.order_id");
        assert!(errors(&p).contains(&"E_FOREACH_LIST".to_string()));
        let route = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "add_lines")
            .unwrap();
        let Step::ForEach { list, item, .. } = &mut route.steps[3] else {
            unreachable!()
        };
        *list = path("input.lines");
        *item = "input".into();
        assert!(errors(&p).contains(&"E_VAR_SHADOW".to_string()));
        let route = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "add_lines")
            .unwrap();
        let Step::ForEach { item, .. } = &mut route.steps[3] else {
            unreachable!()
        };
        *item = "line".into();
        route.steps.insert(
            3,
            Step::Log {
                id: "after".into(),
                level: LogLevel::Info,
                message: "{line.sku}".into(),
            },
        );
        assert!(errors(&p).contains(&"E_UNKNOWN_PATH".to_string()));
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
    fn json_names_are_keys_and_do_not_collide() {
        let mut p = Project::new("shop");
        p.messages.push(Message {
            kind: Default::default(),
            name: "Item".into(),
            fields: vec![
                Field::new("a_b", FieldType::String),
                Field::new("ab", FieldType::String),
            ],
        });
        p.messages[0].fields[0].json_name = Some("ab".into());
        assert!(errors(&p).contains(&"E_DUPLICATE_JSON_NAME".into()));
        p.messages[0].fields[0].json_name = Some("two words".into());
        assert!(errors(&p).contains(&"E_JSON_NAME".into()));
        p.messages[0].fields[0].json_name = Some("aB".into());
        let e = errors(&p);
        assert!(!e.contains(&"E_JSON_NAME".into()) && !e.contains(&"E_DUPLICATE_JSON_NAME".into()));
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

    fn has_warning(p: &Project, code: &str) -> bool {
        validate(p)
            .into_iter()
            .any(|d| d.severity == Severity::Warning && d.code == code)
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

        route_of(&mut p).status = 204;
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
    fn api_schemas_carry_bytes_and_timestamps_as_json() {
        let mut p = sample::orders();
        let view = p
            .messages
            .iter_mut()
            .find(|m| m.name == "OrderView")
            .unwrap();
        view.fields
            .push(Field::new("created_at", FieldType::Timestamp));
        view.fields.push(Field::new("receipt", FieldType::Bytes));
        assert!(!has_errors(&validate(&p)), "{:?}", validate(&p));
    }

    #[test]
    fn only_api_schemas_can_be_lenient() {
        let mut p = sample::orders();
        let m = p
            .messages
            .iter_mut()
            .find(|m| m.name == "Customer")
            .unwrap();
        m.fields[0].lenient = true;
        assert!(errors(&p).contains(&"E_LENIENT_KIND".into()));
    }

    fn respond_headers(p: &mut Project) -> &mut Vec<ResponseHeader> {
        let route = &mut p.services[0].routes[0];
        let Some(Step::Respond { headers, .. }) = route.steps.last_mut() else {
            panic!("sample layout changed")
        };
        headers
    }

    #[test]
    fn response_headers_are_checked() {
        let header = |name: &str, value: Expr| ResponseHeader {
            name: name.into(),
            value,
        };
        let lit = |t: &str| Expr::Literal { value: t.into() };
        let ok = |p: &Project| !has_errors(&validate(p));
        let mut p = sample::orders();
        respond_headers(&mut p).push(header("Cache-Control", lit("no-store")));
        respond_headers(&mut p).push(header(
            "X-Order",
            Expr::Path {
                path: "order_id".into(),
            },
        ));
        assert!(ok(&p), "{:?}", validate(&p));
        for (code, name, value) in [
            ("E_HEADER_NAME", "bad name", "x"),
            ("E_HEADER_NAME", "", "x"),
            ("E_HEADER_NAME", "X-A\r\nB", "x"),
            ("E_HEADER_RESERVED", "Set-Cookie", "x"),
            ("E_HEADER_RESERVED", "content-TYPE", "x"),
            ("E_HEADER_RESERVED", "Retry-After", "x"),
            ("E_HEADER_RESERVED", "Access-Control-Allow-Origin", "x"),
            ("E_HEADER_DUPLICATE", "cache-control", "x"),
            ("E_HEADER_VALUE", "X-Evil", "a\r\nSet-Cookie: s=1"),
            ("E_HEADER_VALUE", "X-Evil", "caf\u{e9}"),
            ("E_HEADER_VALUE", "X-Evil", " padded"),
        ] {
            let mut p = p.clone();
            respond_headers(&mut p).push(header(name, lit(value)));
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code} for {name:?}"
            );
        }
    }

    #[test]
    fn response_headers_only_belong_to_routes() {
        let mut p = sample::orders();
        let Some(Step::Respond { headers, .. }) = p.services[0].rpcs[0].steps.last_mut() else {
            panic!("sample layout changed")
        };
        headers.push(ResponseHeader {
            name: "X-A".into(),
            value: Expr::Literal { value: "a".into() },
        });
        assert!(errors(&p).contains(&"E_HEADER_OUTSIDE_ROUTE".into()));
    }

    #[test]
    fn the_openapi_path_is_checked_only_when_it_is_served() {
        let mut p = sample::orders();
        p.openapi.path = "no-slash".into();
        assert!(!errors(&p).contains(&"E_OPENAPI_PATH".into()));
        p.openapi.serve = true;
        assert!(errors(&p).contains(&"E_OPENAPI_PATH".into()));
        p.openapi.path = "/orders/{id}".into();
        assert!(errors(&p).contains(&"E_OPENAPI_PATH".into()));
        // A path a GET route already answers.
        p.openapi.path = "/orders".into();
        p.services[0].routes[0].method = HttpMethod::Get;
        p.services[0].routes[0].request = "GetOrderRequest".into();
        assert!(errors(&p).contains(&"E_OPENAPI_PATH".into()));
        p.openapi.path = "/openapi.json".into();
        assert!(!errors(&p).contains(&"E_OPENAPI_PATH".into()));
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
            mutable: false,
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
            mutable: false,
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
                mutable: false,
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
                headers: vec![],
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
            headers: vec![],
        };
        p.services[0].rpcs[0].steps.push(step);
        assert!(
            errors(&p).contains(&"E_RESPOND_ROUTE_ONLY".to_string()),
            "{:?}",
            errors(&p)
        );
    }

    /// The showcase, which has a metered route, and where that route is.
    fn with_usage() -> Project {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_usage(&mut p);
        p
    }

    fn usage_route(p: &mut Project) -> &mut Route {
        p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "usage_report")
            .unwrap()
    }

    #[test]
    fn a_metered_route_is_valid_and_its_quota_binds_what_is_left() {
        let mut p = with_usage();
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        // What is left is a number the route can answer; a text is not what the response field holds.
        if let Step::Respond { fields, .. } = usage_route(&mut p).steps.last_mut().unwrap() {
            fields[0].value = Expr::Literal {
                value: "many".into(),
            };
        }
        assert!(!errors(&p).is_empty());
    }

    #[test]
    fn the_steps_of_a_meter_and_a_quota_are_checked() {
        let lit = |v: serde_json::Value| Expr::Literal { value: v };
        let meter = |key: &str, tenant: Expr, amount: Expr| Step::Meter {
            id: "m".into(),
            key: key.into(),
            tenant,
            amount,
            idempotency_key: None,
        };
        let quota = |limit: Expr, wanted: Option<Expr>| Step::CheckQuota {
            id: "q".into(),
            key: "calls".into(),
            tenant: lit("t".into()),
            limit,
            period: UsagePeriod::Month,
            wanted,
            bind: None,
        };
        let mut p = sample::orders();
        let mut check = |step: Step, code: Option<&str>| {
            route_of(&mut p).steps.insert(0, step);
            let found = errors(&p);
            route_of(&mut p).steps.remove(0);
            match code {
                None => assert!(found.is_empty(), "{found:?}"),
                Some(c) => assert!(found.contains(&c.to_string()), "{c}: {found:?}"),
            }
        };
        check(meter("calls", lit("t".into()), lit(1.into())), None);
        check(quota(lit(10.into()), Some(lit(2.into()))), None);
        // A meter is spelled the same everywhere: lowercase, digits and `_.-`, starting with a letter.
        for bad in ["", "Calls", "1calls", "api calls", "a/b", &"x".repeat(65)] {
            check(
                meter(bad, lit("t".into()), lit(1.into())),
                Some("E_METER_NAME"),
            );
        }
        for good in ["calls", "api_calls", "storage.mb", "seats-pro"] {
            check(meter(good, lit("t".into()), lit(1.into())), None);
        }
        // The tenant is text, the amount, the limit and the wanted units are whole numbers.
        check(
            meter("calls", lit(1.into()), lit(1.into())),
            Some("E_TYPE_MISMATCH"),
        );
        check(
            meter("calls", lit("t".into()), lit("one".into())),
            Some("E_TYPE_MISMATCH"),
        );
        check(quota(lit("ten".into()), None), Some("E_TYPE_MISMATCH"));
        check(
            quota(lit(10.into()), Some(lit("two".into()))),
            Some("E_TYPE_MISMATCH"),
        );
    }

    #[test]
    fn a_quota_answers_429_so_it_belongs_to_a_route_and_the_usage_tables_are_the_generator_s() {
        let mut p = with_usage();
        // In a job there is no one to answer: a meter is fine, a quota is not.
        let mut quota = usage_route(&mut p).steps[1].clone();
        let mut meter = usage_route(&mut p).steps[2].clone();
        // The claims are the route's own: the job names the tenant itself.
        let tenant = Expr::Literal {
            value: "acme".into(),
        };
        match (&mut quota, &mut meter) {
            (
                Step::CheckQuota {
                    tenant: t, bind, ..
                },
                Step::Meter { tenant: m, .. },
            ) => {
                (*t, *m, *bind) = (tenant.clone(), tenant, None);
            }
            _ => unreachable!(),
        }
        p.services[0].jobs[0].steps.push(meter);
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        p.services[0].jobs[0].steps.push(quota);
        assert!(
            errors(&p).contains(&"E_QUOTA_OUTSIDE_ROUTE".to_string()),
            "{:?}",
            errors(&p)
        );
        // The service keeps `usage_events` and `usage_daily`: a table of the author cannot take their names.
        let mut p = with_usage();
        for name in USAGE_TABLES {
            let mut table = Table::new(name, vec![Column::new("id", ColumnType::String)]);
            table.columns[0].primary_key = true;
            p.services[0].tables.push(table);
        }
        assert_eq!(
            errors(&p)
                .iter()
                .filter(|c| *c == "E_TABLE_RESERVED")
                .count(),
            2,
            "{:?}",
            errors(&p)
        );
        // Without a meter, the names are free.
        let mut p = sample::orders();
        let mut table = Table::new("usage_events", vec![Column::new("id", ColumnType::String)]);
        table.columns[0].primary_key = true;
        p.services[0].tables.push(table);
        assert!(!errors(&p).contains(&"E_TABLE_RESERVED".to_string()));
    }

    #[test]
    fn a_challenge_is_served_by_a_service_and_checked_by_a_route_of_it() {
        let mut p = sample::orders();
        sample::add_challenge(&mut p);
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        // A route that checks a challenge in a service that hands none out.
        let verify = Step::VerifyChallenge {
            id: "v".into(),
            payload: Expr::Literal { value: "x".into() },
        };
        let mut q = sample::orders();
        route_of(&mut q).steps.insert(0, verify.clone());
        assert!(
            errors(&q).contains(&"E_CHALLENGE_NOT_SERVED".to_string()),
            "{:?}",
            errors(&q)
        );
        // The solution is text.
        let mut q = p.clone();
        if let Step::VerifyChallenge { payload, .. } =
            &mut q.services[0].routes.last_mut().unwrap().steps[0]
        {
            *payload = Expr::Literal { value: 5.into() };
        }
        assert!(
            errors(&q).contains(&"E_TYPE_MISMATCH".to_string()),
            "{:?}",
            errors(&q)
        );
        // Only a route answers 400 to a visitor.
        let mut q = p.clone();
        q.services[0].jobs[0].steps.push(verify);
        assert!(
            errors(&q).contains(&"E_CHALLENGE_OUTSIDE_ROUTE".to_string()),
            "{:?}",
            errors(&q)
        );
        // The table of the solutions already accepted is the generator's.
        let mut q = p.clone();
        let mut table = Table::new(CHALLENGE_TABLE, vec![Column::new("id", ColumnType::String)]);
        table.columns[0].primary_key = true;
        q.services[0].tables.push(table);
        assert!(
            errors(&q).contains(&"E_TABLE_RESERVED".to_string()),
            "{:?}",
            errors(&q)
        );
    }

    #[test]
    fn the_settings_of_a_challenge_are_checked() {
        let mut p = sample::orders();
        sample::add_challenge(&mut p);
        let existing_route = p.services[0].routes[0].path.clone();
        let existing_method = p.services[0].routes[0].method;
        type Change = Box<dyn Fn(&mut ChallengeConfig)>;
        let cases: Vec<(&str, Change)> = vec![
            (
                "E_CHALLENGE_PATH",
                Box::new(|c| c.path = "challenge".into()),
            ),
            ("E_CHALLENGE_PATH", Box::new(|c| c.path = "/".into())),
            ("E_CHALLENGE_PATH", Box::new(|c| c.path = "/a b".into())),
            ("E_CHALLENGE_PATH", Box::new(|c| c.path = "/a//b".into())),
            (
                "E_CHALLENGE_KEY_ENV",
                Box::new(|c| c.key_env = "lower".into()),
            ),
            (
                "E_CHALLENGE_KEY_ENV",
                Box::new(|c| c.key_env = "1KEY".into()),
            ),
            ("E_CHALLENGE_WORK", Box::new(|c| c.max_number = 0)),
            (
                "E_CHALLENGE_WORK",
                Box::new(|c| c.max_number = MAX_CHALLENGE_NUMBER + 1),
            ),
            ("E_CHALLENGE_TTL", Box::new(|c| c.ttl_secs = 9)),
            ("E_CHALLENGE_TTL", Box::new(|c| c.ttl_secs = 86_401)),
        ];
        for (code, change) in cases {
            let mut q = p.clone();
            change(q.services[0].challenge.as_mut().unwrap());
            assert!(
                errors(&q).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&q)
            );
        }
        // The edges are fine.
        for change in [
            (|c: &mut ChallengeConfig| c.max_number = MAX_CHALLENGE_NUMBER)
                as fn(&mut ChallengeConfig),
            |c| c.max_number = 1,
            |c| c.ttl_secs = 10,
            |c| c.ttl_secs = 86_400,
            |c| c.path = "/api/v1/altcha.json".into(),
        ] {
            let mut q = p.clone();
            change(q.services[0].challenge.as_mut().unwrap());
            assert!(errors(&q).is_empty(), "{:?}", errors(&q));
        }
        // A path a route already answers (a `GET`: a `POST` of the same path is another thing).
        if existing_method == HttpMethod::Get {
            let mut q = p.clone();
            q.services[0].challenge.as_mut().unwrap().path = existing_route;
            assert!(errors(&q).contains(&"E_CHALLENGE_PATH".to_string()));
        }
        let mut q = p.clone();
        q.openapi.serve = true;
        q.openapi.path = "/challenge".into();
        assert!(
            errors(&q).contains(&"E_CHALLENGE_PATH".to_string()),
            "{:?}",
            errors(&q)
        );
        // A service with no route serves no HTTP: the challenge would never be handed out.
        let mut q = sample::orders();
        q.services[1].routes.clear();
        q.services[1].challenge = Some(ChallengeConfig::default());
        assert!(
            errors(&q).contains(&"E_CHALLENGE_NO_HTTP".to_string()),
            "{:?}",
            errors(&q)
        );
    }

    #[test]
    fn a_password_is_judged_in_a_route_on_a_text_with_a_score_from_0_to_4() {
        let strength = |min_score: u8, password: Expr, bind: Option<&str>| Step::PasswordStrength {
            id: "ps".into(),
            password,
            min_score,
            inputs: vec![],
            refuse_breached: false,
            bind: bind.map(String::from),
        };
        let lit = |v: &str| Expr::Literal { value: v.into() };
        let mut p = sample::orders();
        let mut check = |step: Step, code: Option<&str>| {
            route_of(&mut p).steps.insert(0, step);
            let found = errors(&p);
            route_of(&mut p).steps.remove(0);
            match code {
                None => assert!(found.is_empty(), "{found:?}"),
                Some(c) => assert!(found.contains(&c.to_string()), "{c}: {found:?}"),
            }
        };
        check(
            strength(DEFAULT_MIN_PASSWORD_SCORE, lit("x"), Some("score")),
            None,
        );
        check(strength(0, lit("x"), None), None);
        check(strength(4, lit("x"), None), None);
        check(strength(5, lit("x"), None), Some("E_PASSWORD_SCORE"));
        check(
            strength(3, Expr::Literal { value: 5.into() }, None),
            Some("E_TYPE_MISMATCH"),
        );
        check(
            strength(3, lit("x"), Some("input")),
            Some("E_RESERVED_NAME"),
        );
        // The inputs are text too.
        let mut with_input = strength(3, lit("x"), None);
        if let Step::PasswordStrength { inputs, .. } = &mut with_input {
            inputs.push(Expr::Literal { value: 1.into() });
        }
        check(with_input, Some("E_TYPE_MISMATCH"));
        // Only a route answers 400 to a visitor.
        let mut q = sample::orders();
        q.services[0].jobs[0]
            .steps
            .push(strength(3, lit("x"), None));
        assert!(
            errors(&q).contains(&"E_PASSWORD_OUTSIDE_ROUTE".to_string()),
            "{:?}",
            errors(&q)
        );
    }

    fn with_api_keys() -> Project {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_api_keys(&mut p);
        p
    }

    fn api_route<'a>(p: &'a mut Project, name: &str) -> &'a mut Route {
        p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == name)
            .unwrap()
    }

    #[test]
    fn the_api_key_steps_are_valid_and_checked() {
        let mut p = with_api_keys();
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        // A scope is spelled the same in the key and in the route that asks for it.
        for bad in ["", "Orders", "orders read", "a/b", &"x".repeat(65)] {
            let mut q = p.clone();
            if let Step::ApiKeyVerify { scope, .. } =
                &mut api_route(&mut q, "machine_orders").steps[0]
            {
                *scope = Some(bad.into());
            }
            assert!(
                errors(&q).contains(&"E_API_KEY_SCOPE".to_string()),
                "{bad:?}: {:?}",
                errors(&q)
            );
            let mut q = p.clone();
            if let Step::ApiKeyCreate { scopes, .. } =
                &mut api_route(&mut q, "issue_api_key").steps[1]
            {
                scopes.push(bad.into());
            }
            assert!(
                errors(&q).contains(&"E_API_KEY_SCOPE".to_string()),
                "{bad:?}: {:?}",
                errors(&q)
            );
        }
        // The key and its id are two variables, and neither is a reserved name.
        let mut q = p.clone();
        if let Step::ApiKeyCreate {
            bind, key_id_bind, ..
        } = &mut api_route(&mut q, "issue_api_key").steps[1]
        {
            *key_id_bind = Some(bind.clone());
        }
        assert!(
            errors(&q).contains(&"E_API_KEY_BIND".to_string()),
            "{:?}",
            errors(&q)
        );
        let mut q = p.clone();
        if let Step::ApiKeyVerify { bind, .. } = &mut api_route(&mut q, "machine_orders").steps[0] {
            *bind = "input".into();
        }
        assert!(
            errors(&q).contains(&"E_RESERVED_NAME".to_string()),
            "{:?}",
            errors(&q)
        );
        // The owner, the name and the key are text; the lifetime is a whole number.
        let mut q = p.clone();
        if let Step::ApiKeyCreate {
            expires_in_secs,
            name,
            ..
        } = &mut api_route(&mut q, "issue_api_key").steps[1]
        {
            *expires_in_secs = Some(Expr::Literal {
                value: "soon".into(),
            });
            *name = Expr::Literal { value: 1.into() };
        }
        assert_eq!(
            errors(&q)
                .iter()
                .filter(|c| *c == "E_TYPE_MISMATCH")
                .count(),
            2,
            "{:?}",
            errors(&q)
        );
        let mut q = p.clone();
        if let Step::ApiKeyVerify { key, .. } = &mut api_route(&mut q, "machine_orders").steps[0] {
            *key = Expr::Literal { value: 1.into() };
        }
        assert!(
            errors(&q).contains(&"E_TYPE_MISMATCH".to_string()),
            "{:?}",
            errors(&q)
        );
        // What the key step binds can be read after it.
        if let Step::Respond { fields, .. } = api_route(&mut p, "machine_orders")
            .steps
            .last_mut()
            .unwrap()
        {
            fields[0].value = Expr::Path {
                path: "nobody".into(),
            };
        }
        assert!(
            errors(&p).contains(&"E_UNKNOWN_PATH".to_string()),
            "{:?}",
            errors(&p)
        );
    }

    #[test]
    fn a_bad_key_is_answered_so_the_check_and_the_revocation_belong_to_a_route_and_the_table_is_reserved()
     {
        let p = with_api_keys();
        // A job has no one to answer 401 or 404: issuing a key is fine, checking or revoking one is not.
        let mut q = p.clone();
        let create = api_route(&mut q, "issue_api_key").steps[1].clone();
        let verify = api_route(&mut q, "machine_orders").steps[0].clone();
        let revoke = api_route(&mut q, "revoke_api_key").steps[1].clone();
        let lit = |v: &str| Expr::Literal { value: v.into() };
        let mut create = create;
        if let Step::ApiKeyCreate {
            owner,
            name,
            key_id_bind,
            ..
        } = &mut create
        {
            (*owner, *name, *key_id_bind) = (lit("acme"), lit("robot"), None);
        }
        q.services[0].jobs[0].steps.push(create);
        assert!(errors(&q).is_empty(), "{:?}", errors(&q));
        let (mut verify, mut revoke) = (verify, revoke);
        if let Step::ApiKeyVerify { key, .. } = &mut verify {
            *key = lit("sbk");
        }
        if let Step::ApiKeyRevoke { key_id, owner, .. } = &mut revoke {
            (*key_id, *owner) = (lit("x"), None);
        }
        q.services[0].jobs[0].steps.extend([verify, revoke]);
        assert_eq!(
            errors(&q)
                .iter()
                .filter(|c| *c == "E_API_KEY_OUTSIDE_ROUTE")
                .count(),
            2,
            "{:?}",
            errors(&q)
        );
        // The service keeps `api_keys`.
        let mut q = p.clone();
        let mut table = Table::new(API_KEYS_TABLE, vec![Column::new("id", ColumnType::String)]);
        table.columns[0].primary_key = true;
        q.services[0].tables.push(table);
        assert!(
            errors(&q).contains(&"E_TABLE_RESERVED".to_string()),
            "{:?}",
            errors(&q)
        );
    }

    #[test]
    fn compression_is_a_setting_of_a_service_that_has_routes() {
        let mut p = sample::orders();
        p.services[0].compression = true;
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        assert!(!has_warning(&p, "W_COMPRESSION_UNUSED"));
        // A service with no HTTP API has nothing to compress.
        p.services[1].routes.clear();
        p.services[1].compression = true;
        assert!(has_warning(&p, "W_COMPRESSION_UNUSED"));
        assert!(
            errors(&p).is_empty(),
            "a warning, not an error: {:?}",
            errors(&p)
        );
        // Left out, it is not written: a project without it is the same file as before.
        let mut q = sample::orders();
        q.services[0].compression = false;
        assert!(!serde_json::to_string(&q).unwrap().contains("compression"));
        q.services[0].compression = true;
        assert!(
            serde_json::to_string(&q)
                .unwrap()
                .contains("\"compression\":true")
        );
    }

    #[test]
    fn cleaning_html_takes_one_text_and_gives_one() {
        let clean = |arg: Expr| call(Func::SanitizeHtml, vec![arg]);
        let mut p = sample::orders();
        let mut check = |value: Expr, ok: bool| {
            route_of(&mut p).steps.insert(
                0,
                Step::Let {
                    id: "h".into(),
                    name: "cleaned".into(),
                    value_type: FieldType::String,
                    optional: false,
                    repeated: false,
                    mutable: false,
                    value,
                },
            );
            let found = errors(&p);
            route_of(&mut p).steps.remove(0);
            assert_eq!(found.is_empty(), ok, "{found:?}");
        };
        check(
            clean(Expr::Literal {
                value: "<b>x</b>".into(),
            }),
            true,
        );
        // A number is not text; two texts are one too many; none is one too few.
        check(clean(Expr::Literal { value: 5.into() }), false);
        check(call(Func::SanitizeHtml, vec![]), false);
        check(
            call(
                Func::SanitizeHtml,
                vec![
                    Expr::Literal { value: "a".into() },
                    Expr::Literal { value: "b".into() },
                ],
            ),
            false,
        );
        // Its name is the one the compact syntax and the JSON use.
        assert_eq!(Func::SanitizeHtml.name(), "sanitizeHtml");
        assert_eq!(Func::from_name("sanitizeHtml"), Some(Func::SanitizeHtml));
        // A project that does not clean HTML says nothing of it.
        assert!(!sample::orders().services[0].uses_html_sanitizer());
    }

    #[test]
    fn a_circuit_and_a_bulkhead_keep_to_values_that_mean_something() {
        let mut p = sample::showcase();
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        // The same settings on a call to a service of the project (`callRpc`) and on an outbound call (`httpCall`).
        let set = |p: &mut Project, circuit: Option<CircuitConfig>, max: Option<u32>| {
            let mut touched = 0;
            for s in &mut p.services {
                for c in &mut s.consumers {
                    for step in &mut c.steps {
                        if let Step::CallRpc {
                            circuit: ci,
                            max_concurrent,
                            ..
                        } = step
                        {
                            (*ci, *max_concurrent) = (circuit, max);
                            touched += 1;
                        }
                    }
                }
                for r in &mut s.routes {
                    for step in &mut r.steps {
                        if let Step::HttpCall {
                            circuit: ci,
                            max_concurrent,
                            ..
                        } = step
                        {
                            (*ci, *max_concurrent) = (circuit, max);
                            touched += 1;
                        }
                    }
                }
            }
            assert!(
                touched >= 3,
                "the showcase has two outbound calls and a call to a service"
            );
        };
        let ok = CircuitConfig {
            failures: 1,
            window_secs: 1,
            open_secs: 1,
        };
        set(&mut p, Some(ok), Some(1));
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        let top = CircuitConfig {
            failures: MAX_CIRCUIT_FAILURES,
            window_secs: MAX_CIRCUIT_SECS,
            open_secs: MAX_CIRCUIT_SECS,
        };
        set(&mut p, Some(top), Some(MAX_CONCURRENT_CALLS));
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        for (code, circuit, max) in [
            (
                "E_CIRCUIT_FAILURES",
                CircuitConfig { failures: 0, ..ok },
                None,
            ),
            (
                "E_CIRCUIT_FAILURES",
                CircuitConfig {
                    failures: MAX_CIRCUIT_FAILURES + 1,
                    ..ok
                },
                None,
            ),
            (
                "E_CIRCUIT_WINDOW",
                CircuitConfig {
                    window_secs: 0,
                    ..ok
                },
                None,
            ),
            (
                "E_CIRCUIT_WINDOW",
                CircuitConfig {
                    window_secs: MAX_CIRCUIT_SECS + 1,
                    ..ok
                },
                None,
            ),
            ("E_CIRCUIT_OPEN", CircuitConfig { open_secs: 0, ..ok }, None),
            (
                "E_CIRCUIT_OPEN",
                CircuitConfig {
                    open_secs: MAX_CIRCUIT_SECS + 1,
                    ..ok
                },
                None,
            ),
        ] {
            let mut q = p.clone();
            set(&mut q, Some(circuit), max);
            assert!(
                errors(&q).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&q)
            );
        }
        for bad in [0, MAX_CONCURRENT_CALLS + 1] {
            let mut q = p.clone();
            set(&mut q, None, Some(bad));
            assert!(
                errors(&q).contains(&"E_MAX_CONCURRENT".to_string()),
                "{bad}: {:?}",
                errors(&q)
            );
        }
        // Left out, they are not written: a project without them is the same file as before.
        let mut q = sample::orders();
        set_none(&mut q);
        assert!(!serde_json::to_string(&q).unwrap().contains("circuit"));
        assert!(!serde_json::to_string(&q).unwrap().contains("maxConcurrent"));
    }

    /// Takes the circuits and the bulkheads off every call of a project.
    fn set_none(p: &mut Project) {
        for s in &mut p.services {
            for step in s.consumers.iter_mut().flat_map(|c| c.steps.iter_mut()) {
                if let Step::CallRpc {
                    circuit,
                    max_concurrent,
                    ..
                } = step
                {
                    (*circuit, *max_concurrent) = (None, None);
                }
            }
        }
    }

    #[test]
    fn a_sequence_is_numbered_in_a_transaction_and_its_table_is_the_generator_s() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_invoices(&mut p);
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        fn route(p: &Project) -> usize {
            p.services[0]
                .routes
                .iter()
                .position(|r| r.name == "issue_invoice")
                .unwrap()
        }
        fn next(p: &mut Project) -> &mut Step {
            let r = route(p);
            &mut p.services[0].routes[r].steps[2]
        }
        // The name is spelled the same everywhere; the first number is zero or more; the scope is text.
        for bad in ["", "Invoices", "credit notes", "1a", &"x".repeat(65)] {
            let mut q = p.clone();
            if let Step::NextNumber { sequence, .. } = next(&mut q) {
                *sequence = bad.into();
            }
            assert!(
                errors(&q).contains(&"E_SEQUENCE_NAME".to_string()),
                "{bad:?}: {:?}",
                errors(&q)
            );
        }
        for good in ["invoices", "credit_notes", "a.b-c"] {
            let mut q = p.clone();
            if let Step::NextNumber { sequence, .. } = next(&mut q) {
                *sequence = good.into();
            }
            assert!(errors(&q).is_empty(), "{good:?}: {:?}", errors(&q));
        }
        let mut q = p.clone();
        if let Step::NextNumber { start, .. } = next(&mut q) {
            *start = -1;
        }
        assert!(
            errors(&q).contains(&"E_SEQUENCE_START".to_string()),
            "{:?}",
            errors(&q)
        );
        let mut q = p.clone();
        if let Step::NextNumber { scope, .. } = next(&mut q) {
            *scope = Some(Expr::Literal { value: 5.into() });
        }
        assert!(
            errors(&q).contains(&"E_TYPE_MISMATCH".to_string()),
            "{:?}",
            errors(&q)
        );
        // The number is an int64 the steps after it can read, not before.
        let mut q = p.clone();
        let r = route(&q);
        q.services[0].routes[r].steps.swap(2, 3);
        assert!(
            errors(&q)
                .iter()
                .any(|c| c == "E_UNKNOWN_PATH" || c == "E_EXPR_TYPE"),
            "{:?}",
            errors(&q)
        );
        // `number_sequences` is the generator's.
        let mut q = p.clone();
        let mut table = Table::new(SEQUENCES_TABLE, vec![Column::new("id", ColumnType::String)]);
        table.columns[0].primary_key = true;
        q.services[0].tables.push(table);
        assert!(
            errors(&q).contains(&"E_TABLE_RESERVED".to_string()),
            "{:?}",
            errors(&q)
        );
    }

    #[test]
    fn the_identifier_functions_take_and_give_the_types_they_say() {
        let mut p = sample::orders();
        let mut check = |value_type: FieldType, optional: bool, value: Expr, ok: bool| {
            route_of(&mut p).steps.insert(
                0,
                Step::Let {
                    id: "i".into(),
                    name: "made".into(),
                    value_type,
                    optional,
                    repeated: false,
                    mutable: false,
                    value,
                },
            );
            let found = errors(&p);
            route_of(&mut p).steps.remove(0);
            assert_eq!(found.is_empty(), ok, "{found:?}");
        };
        let int = |n: i64| Expr::Literal { value: n.into() };
        let text = |t: &str| Expr::Literal { value: t.into() };
        check(FieldType::Uuid, false, call(Func::NewUuidV7, vec![]), true);
        check(
            FieldType::Uuid,
            false,
            call(Func::NewUuidV7, vec![int(1)]),
            false,
        );
        check(
            FieldType::String,
            false,
            call(Func::ShortId, vec![int(5)]),
            true,
        );
        check(
            FieldType::String,
            false,
            call(Func::ShortId, vec![text("5")]),
            false,
        );
        check(
            FieldType::Int64,
            true,
            call(Func::FromShortId, vec![text("UkLWZg")]),
            true,
        );
        // A short text may be nothing, so the number is optional; it cannot fill a required one.
        check(
            FieldType::Int64,
            false,
            call(Func::FromShortId, vec![text("UkLWZg")]),
            false,
        );
        check(
            FieldType::Int64,
            true,
            call(Func::FromShortId, vec![int(1)]),
            false,
        );
        // Their names are the ones the compact syntax and the JSON use.
        for (func, name) in [
            (Func::NewUuidV7, "newUuidV7"),
            (Func::ShortId, "shortId"),
            (Func::FromShortId, "fromShortId"),
        ] {
            assert_eq!(func.name(), name);
            assert_eq!(Func::from_name(name), Some(func));
        }
        assert!(!sample::orders().services[0].uses_short_ids());
    }

    #[test]
    fn the_calendar_functions_take_instants_and_zones_and_a_zone_written_in_the_flow_must_exist() {
        let int = |n: i64| Expr::Literal { value: n.into() };
        let text = |t: &str| Expr::Literal { value: t.into() };
        let mut p = sample::orders();
        let mut check = |value_type: FieldType, optional: bool, value: Expr, ok: bool| {
            route_of(&mut p).steps.insert(
                0,
                Step::Let {
                    id: "c".into(),
                    name: "made".into(),
                    value_type,
                    optional,
                    repeated: false,
                    mutable: false,
                    value,
                },
            );
            let found = errors(&p);
            route_of(&mut p).steps.remove(0);
            assert_eq!(found.is_empty(), ok, "{found:?}");
        };
        let paris = || text("Europe/Paris");
        check(
            FieldType::Int64,
            false,
            call(Func::NowSeconds, vec![]),
            true,
        );
        check(
            FieldType::Int64,
            false,
            call(Func::NowSeconds, vec![int(1)]),
            false,
        );
        check(
            FieldType::String,
            false,
            call(Func::LocalDate, vec![int(0), paris()]),
            true,
        );
        check(
            FieldType::String,
            false,
            call(Func::LocalTime, vec![int(0), text("UTC")]),
            true,
        );
        check(
            FieldType::Int64,
            false,
            call(Func::StartOfDay, vec![int(0), paris()]),
            true,
        );
        check(
            FieldType::Int64,
            false,
            call(Func::AddDays, vec![int(0), int(7), paris()]),
            true,
        );
        check(
            FieldType::Int64,
            false,
            call(Func::AddMonths, vec![int(0), int(-1), paris()]),
            true,
        );
        // A text that may not be a date: the instant is optional.
        check(
            FieldType::Int64,
            true,
            call(Func::ParseLocal, vec![text("2026-03-29T10:00"), paris()]),
            true,
        );
        check(
            FieldType::Int64,
            false,
            call(Func::ParseLocal, vec![text("2026-03-29T10:00"), paris()]),
            false,
        );
        // The types: seconds first (not a text), the zone last (not a number), days and months whole numbers.
        check(
            FieldType::String,
            false,
            call(Func::LocalDate, vec![text("0"), paris()]),
            false,
        );
        check(
            FieldType::String,
            false,
            call(Func::LocalDate, vec![int(0), int(1)]),
            false,
        );
        check(
            FieldType::Int64,
            false,
            call(Func::AddDays, vec![int(0), text("7"), paris()]),
            false,
        );
        check(
            FieldType::Int64,
            false,
            call(Func::AddDays, vec![int(0), paris()]),
            false,
        );
        // A zone written in the flow must be in the IANA database; one that comes from the data is the runtime's to check.
        check(
            FieldType::String,
            false,
            call(Func::LocalDate, vec![int(0), text("Europe/Pariss")]),
            false,
        );
        check(
            FieldType::String,
            false,
            call(Func::LocalDate, vec![int(0), text("")]),
            false,
        );
        check(
            FieldType::String,
            false,
            call(Func::LocalDate, vec![int(0), text("America/New_York")]),
            true,
        );
        check(
            FieldType::String,
            false,
            call(Func::LocalDate, vec![int(0), text("UTC")]),
            true,
        );
        check(
            FieldType::Int64,
            true,
            call(Func::ParseLocal, vec![text("x"), text("Mars/Olympus")]),
            false,
        );
        // Their names are the ones the compact syntax and the JSON use.
        for (func, name) in [
            (Func::NowSeconds, "nowSeconds"),
            (Func::LocalDate, "localDate"),
            (Func::LocalTime, "localTime"),
            (Func::StartOfDay, "startOfDay"),
            (Func::AddDays, "addDays"),
            (Func::AddMonths, "addMonths"),
            (Func::ParseLocal, "parseLocal"),
        ] {
            assert_eq!(func.name(), name);
            assert_eq!(Func::from_name(name), Some(func));
        }
        assert!(!sample::orders().services[0].uses_time_zones());
        assert!(
            crate::typing::is_time_zone("Europe/Paris")
                && !crate::typing::is_time_zone("Europe/Pariss")
        );
    }

    fn with_audit() -> Project {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_audit(&mut p);
        p
    }

    fn audit_step<'a>(p: &'a mut Project, route: &str, index: usize) -> &'a mut Step {
        let r = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == route)
            .unwrap();
        &mut r.steps[index]
    }

    #[test]
    fn the_audit_steps_are_checked_and_the_journal_schema_is_required() {
        let p = with_audit();
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        // The name of an action and of a kind of target are spelled the same everywhere.
        for bad in ["", "Invoice.Sent", "invoice sent", "1a", &"x".repeat(65)] {
            let mut q = p.clone();
            if let Step::Audit {
                action,
                target_type,
                ..
            } = audit_step(&mut q, "send_invoice", 1)
            {
                (*action, *target_type) = (bad.into(), bad.into());
            }
            let found = errors(&q);
            assert!(
                found.contains(&"E_AUDIT_ACTION".to_string())
                    && found.contains(&"E_AUDIT_TARGET_TYPE".to_string()),
                "{bad:?}: {found:?}"
            );
        }
        // The values are named, once each, and plain.
        let lit = |v: &str| Expr::Literal { value: v.into() };
        let mut q = p.clone();
        if let Step::Audit { before, .. } = audit_step(&mut q, "send_invoice", 1) {
            before.push(Assignment {
                field: "Bad Name".into(),
                value: lit("x"),
            });
            before.push(Assignment {
                field: "status".into(),
                value: lit("again"),
            });
        }
        assert_eq!(
            errors(&q).iter().filter(|c| *c == "E_PLAIN_FIELD").count(),
            2,
            "{:?}",
            errors(&q)
        );
        // A whole message, a list and a timestamp are not plain values.
        let mut q = p.clone();
        if let Step::Audit { after, .. } = audit_step(&mut q, "send_invoice", 1) {
            after.push(Assignment {
                field: "claims".into(),
                value: Expr::Path {
                    path: "claims".into(),
                },
            });
            after.push(Assignment {
                field: "roles".into(),
                value: Expr::Path {
                    path: "claims.roles".into(),
                },
            });
        }
        assert_eq!(
            errors(&q).iter().filter(|c| *c == "E_PLAIN_VALUE").count(),
            2,
            "{:?}",
            errors(&q)
        );
        // The target, the actor and the company are text.
        let mut q = p.clone();
        if let Step::Audit {
            target,
            actor,
            tenant,
            ..
        } = audit_step(&mut q, "send_invoice", 1)
        {
            (*target, *actor, *tenant) = (
                Expr::Literal { value: 1.into() },
                Expr::Literal { value: 2.into() },
                Some(Expr::Literal { value: 3.into() }),
            );
        }
        assert_eq!(
            errors(&q)
                .iter()
                .filter(|c| *c == "E_TYPE_MISMATCH")
                .count(),
            3,
            "{:?}",
            errors(&q)
        );
        // Reading: a page of 1 to 200, a cursor that is a number, and the schema `AuditEntry` as the journal has it.
        for bad in [0, MAX_AUDIT_PAGE + 1] {
            let mut q = p.clone();
            if let Step::AuditRead { limit, .. } = audit_step(&mut q, "audit_journal", 1) {
                *limit = bad;
            }
            assert!(
                errors(&q).contains(&"E_AUDIT_PAGE".to_string()),
                "{bad}: {:?}",
                errors(&q)
            );
        }
        let mut q = p.clone();
        if let Step::AuditRead {
            cursor,
            tenant,
            next_cursor_bind,
            bind,
            ..
        } = audit_step(&mut q, "audit_journal", 1)
        {
            *cursor = Some(lit("x"));
            *tenant = Expr::Literal { value: 1.into() };
            *next_cursor_bind = Some(bind.clone());
        }
        let found = errors(&q);
        assert!(
            found.contains(&"E_AUDIT_BIND".to_string())
                && found.iter().filter(|c| *c == "E_TYPE_MISMATCH").count() >= 2,
            "{found:?}"
        );
        let mut q = p.clone();
        q.messages.retain(|m| m.name != "AuditEntry");
        assert!(
            errors(&q)
                .iter()
                .any(|c| c == "E_AUDIT_SCHEMA" || c == "E_UNKNOWN_MESSAGE"),
            "{:?}",
            errors(&q)
        );
        let mut q = p.clone();
        q.messages
            .iter_mut()
            .find(|m| m.name == "AuditEntry")
            .unwrap()
            .fields
            .retain(|f| f.name != "changed");
        assert!(
            errors(&q).contains(&"E_AUDIT_SCHEMA".to_string()),
            "{:?}",
            errors(&q)
        );
        // The entries and the next cursor can be read after the step, not before; the journal table is the generator's.
        let mut q = p.clone();
        let r = q.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "audit_journal")
            .unwrap();
        r.steps.swap(1, 2);
        assert!(!errors(&q).is_empty());
        let mut q = p.clone();
        let mut table = Table::new(AUDIT_TABLE, vec![Column::new("id", ColumnType::String)]);
        table.columns[0].primary_key = true;
        q.services[0].tables.push(table);
        assert!(
            errors(&q).contains(&"E_TABLE_RESERVED".to_string()),
            "{:?}",
            errors(&q)
        );
    }

    #[test]
    fn a_flag_names_its_subject_its_properties_and_what_it_starts_from() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_flags(&mut p);
        assert!(errors(&p).is_empty(), "{:?}", errors(&p));
        fn flag(p: &mut Project) -> &mut Step {
            let r = p.services[0]
                .routes
                .iter_mut()
                .find(|r| r.name == "checkout_flag")
                .unwrap();
            &mut r.steps[1]
        }
        for bad in ["", "New Checkout", "1a", &"x".repeat(65)] {
            let mut q = p.clone();
            if let Step::Flag { key, .. } = flag(&mut q) {
                *key = bad.into();
            }
            assert!(
                errors(&q).contains(&"E_FLAG_NAME".to_string()),
                "{bad:?}: {:?}",
                errors(&q)
            );
        }
        for good in ["new_checkout", "beta.search", "a-b"] {
            let mut q = p.clone();
            if let Step::Flag { key, .. } = flag(&mut q) {
                *key = good.into();
            }
            assert!(errors(&q).is_empty(), "{good:?}: {:?}", errors(&q));
        }
        let mut q = p.clone();
        if let Step::Flag { rollout, .. } = flag(&mut q) {
            *rollout = 101;
        }
        assert!(
            errors(&q).contains(&"E_FLAG_ROLLOUT".to_string()),
            "{:?}",
            errors(&q)
        );
        for edge in [0u8, 100] {
            let mut q = p.clone();
            if let Step::Flag { rollout, .. } = flag(&mut q) {
                *rollout = edge;
            }
            assert!(errors(&q).is_empty(), "{edge}: {:?}", errors(&q));
        }
        // The subject is text; the properties are named plain values, once each.
        let mut q = p.clone();
        if let Step::Flag {
            subject,
            properties,
            ..
        } = flag(&mut q)
        {
            *subject = Expr::Literal { value: 1.into() };
            properties.push(Assignment {
                field: "Bad Name".into(),
                value: Expr::Literal { value: "x".into() },
            });
            properties.push(Assignment {
                field: "tenant".into(),
                value: Expr::Literal {
                    value: "again".into(),
                },
            });
            properties.push(Assignment {
                field: "roles".into(),
                value: Expr::Path {
                    path: "claims.roles".into(),
                },
            });
        }
        let found = errors(&q);
        assert!(found.contains(&"E_TYPE_MISMATCH".to_string()), "{found:?}");
        assert_eq!(
            found.iter().filter(|c| *c == "E_PLAIN_FIELD").count(),
            2,
            "{found:?}"
        );
        assert_eq!(
            found.iter().filter(|c| *c == "E_PLAIN_VALUE").count(),
            1,
            "{found:?}"
        );
        // The answer is a yes/no and the variant a text, two variables that the steps after can read, not before.
        let mut q = p.clone();
        if let Step::Flag {
            variant_bind, bind, ..
        } = flag(&mut q)
        {
            *variant_bind = Some(bind.clone());
        }
        assert!(
            errors(&q).contains(&"E_FLAG_BIND".to_string()),
            "{:?}",
            errors(&q)
        );
        let mut q = p.clone();
        q.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "checkout_flag")
            .unwrap()
            .steps
            .swap(1, 2);
        assert!(!errors(&q).is_empty());
        // The table of the flags is the generator's.
        let mut q = p.clone();
        let mut table = Table::new(FLAGS_TABLE, vec![Column::new("id", ColumnType::String)]);
        table.columns[0].primary_key = true;
        q.services[0].tables.push(table);
        assert!(
            errors(&q).contains(&"E_TABLE_RESERVED".to_string()),
            "{:?}",
            errors(&q)
        );
    }

    #[test]
    fn a_shared_rate_limit_keeps_to_what_the_store_can_count() {
        let mut p = sample::orders();
        let limit = |max: u32, shared: bool| Step::RateLimit {
            id: "rl".into(),
            key: Expr::Literal { value: "k".into() },
            max,
            window_secs: 60,
            shared,
        };
        let mut check = |step: Step, code: Option<&str>| {
            route_of(&mut p).steps.insert(0, step);
            let found = errors(&p);
            route_of(&mut p).steps.remove(0);
            match code {
                None => assert!(found.is_empty(), "{found:?}"),
                Some(c) => assert!(found.contains(&c.to_string()), "{c}: {found:?}"),
            }
        };
        check(limit(SHARED_RATE_LIMIT_MAX, true), None);
        check(
            limit(SHARED_RATE_LIMIT_MAX + 1, true),
            Some("E_RATE_LIMIT_SHARED_MAX"),
        );
        // In memory the entries are cheap: no such ceiling.
        check(limit(SHARED_RATE_LIMIT_MAX + 1, false), None);
    }

    #[test]
    fn a_rate_limit_is_checked() {
        let mut p = sample::orders();
        let limit = |max: u32, window_secs: u32, key: Expr| Step::RateLimit {
            id: "rl".into(),
            key,
            max,
            window_secs,
            shared: false,
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
                headers: vec![],
            },
            fail(400, None),
            Step::RateLimit {
                id: "l".into(),
                key: Expr::Literal { value: "k".into() },
                max: 1,
                window_secs: 60,
                shared: false,
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

    fn with_totp() -> Project {
        let mut p = sample::orders();
        sample::add_totp(&mut p);
        p
    }

    /// The routes of `add_totp` follow the two of the orders: enrolment, verification, recovery.
    fn totp_step(p: &mut Project, route: usize, at: usize) -> &mut Step {
        &mut p.services[0].routes[route].steps[at]
    }

    #[test]
    fn a_second_factor_can_be_enrolled_checked_and_recovered() {
        let p = with_totp();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        // The codes are a list of texts the response can carry.
        let mut p = with_totp();
        if let Step::Respond { fields, .. } = totp_step(&mut p, 2, 3) {
            fields[2].value = Expr::Path {
                path: "setup.secret".into(),
            };
        }
        assert!(
            errors(&p).contains(&"E_TYPE_MISMATCH".to_string()),
            "{:?}",
            errors(&p)
        );
    }

    #[test]
    fn the_steps_of_a_second_factor_are_checked() {
        // (expected code, how to break it)
        type Case<'a> = (&'a str, &'a dyn Fn(&mut Project));
        let cases: [Case; 9] = [
            ("E_KEY_ENV", &|p| {
                if let Step::TotpGenerate { key_env, .. } = totp_step(p, 2, 0) {
                    *key_env = "totp-key".into();
                }
            }),
            ("E_KEY_ENV", &|p| {
                if let Step::TotpVerify { key_env, .. } = totp_step(p, 3, 2) {
                    *key_env = String::new();
                }
            }),
            ("E_TOTP_ISSUER", &|p| {
                if let Step::TotpGenerate { issuer, .. } = totp_step(p, 2, 0) {
                    *issuer = "Acme: Pro".into();
                }
            }),
            ("E_TOTP_ISSUER", &|p| {
                if let Step::TotpGenerate { issuer, .. } = totp_step(p, 2, 0) {
                    *issuer = " ".into();
                }
            }),
            ("E_RECOVERY_COUNT", &|p| {
                if let Step::RecoveryCodes { count, .. } = totp_step(p, 2, 2) {
                    *count = 0;
                }
            }),
            ("E_RECOVERY_COUNT", &|p| {
                if let Step::RecoveryCodes { count, .. } = totp_step(p, 2, 2) {
                    *count = 21;
                }
            }),
            ("E_TOTP_SCHEMA", &|p| {
                p.messages.retain(|m| m.name != "TotpSetup");
            }),
            ("E_UNKNOWN_PATH", &|p| {
                if let Step::TotpVerify { sealed, .. } = totp_step(p, 3, 2) {
                    *sealed = Expr::Path {
                        path: "factor.nope".into(),
                    };
                }
            }),
            ("E_RESERVED_NAME", &|p| {
                if let Step::RecoveryCodes { bind, .. } = totp_step(p, 2, 2) {
                    *bind = "tx".into();
                }
            }),
        ];
        for (code, break_it) in cases {
            let mut p = with_totp();
            break_it(&mut p);
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
        // The schema must hold texts, not any shape.
        let mut p = with_totp();
        p.messages
            .iter_mut()
            .find(|m| m.name == "TotpSetup")
            .unwrap()
            .fields[0]
            .repeated = true;
        assert!(errors(&p).contains(&"E_TOTP_SCHEMA".to_string()));
    }

    #[test]
    fn a_refused_code_is_a_401_so_only_routes_check_one() {
        for (route, at) in [(3, 2), (4, 1)] {
            let mut p = with_totp();
            let step = totp_step(&mut p, route, at).clone();
            p.services[0].rpcs[0].steps.insert(0, step);
            assert!(
                errors(&p).contains(&"E_TOTP_OUTSIDE_ROUTE".to_string()),
                "{route}: {:?}",
                errors(&p)
            );
        }
        // Enrolling and issuing codes answer nothing: they work anywhere.
        let mut p = with_totp();
        let step = totp_step(&mut p, 2, 2).clone();
        p.services[0].rpcs[0].steps.insert(0, step);
        assert!(!errors(&p).contains(&"E_TOTP_OUTSIDE_ROUTE".to_string()));
    }

    fn with_passkeys() -> Project {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_passkeys(&mut p);
        p
    }

    /// The four requests of `add_passkeys` come after `place_order`, `get_order`, `whoami`, `jwks` and `session`.
    fn passkey_step(p: &mut Project, route: usize, at: usize) -> &mut Step {
        &mut p.services[0].routes[5 + route].steps[at]
    }

    #[test]
    fn a_passkey_is_registered_and_used_in_two_requests_each() {
        let p = with_passkeys();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        // The account the login gives back is a text the next steps can sign.
        let mut p = with_passkeys();
        if let Step::SignJwt { claims, .. } = passkey_step(&mut p, 3, 1) {
            claims[0].value = Expr::Path {
                path: "nope".into(),
            };
        }
        assert!(errors(&p).contains(&"E_UNKNOWN_PATH".to_string()));
    }

    #[test]
    fn a_passkey_login_may_leave_the_account_out_but_not_get_it_wrong() {
        // The showcase has both: with an account (route 2) and without (the last route).
        let p = with_passkeys();
        let routes = &p.services[0].routes;
        let account_of = |r: &crate::project::Route| {
            r.steps.iter().find_map(|s| match s {
                Step::PasskeyLoginStart { account, .. } => Some(account.is_some()),
                _ => None,
            })
        };
        assert_eq!(account_of(&routes[5 + 2]), Some(true));
        assert_eq!(account_of(routes.last().unwrap()), Some(false));
        // An account that is there is still checked.
        let mut p = with_passkeys();
        if let Step::PasskeyLoginStart { account, .. } = passkey_step(&mut p, 2, 1) {
            *account = Some(Expr::Path {
                path: "input.nope".into(),
            });
        }
        assert!(errors(&p).contains(&"E_UNKNOWN_PATH".to_string()));
        // In the JSON, no `account` is a discoverable login, and an old project with one reads as before.
        let step: Step =
            serde_json::from_str(r#"{"type":"passkeyLoginStart","id":"a","bind":"c"}"#).unwrap();
        assert!(matches!(
            step,
            Step::PasskeyLoginStart { account: None, .. }
        ));
        assert!(!serde_json::to_string(&step).unwrap().contains("account"));
        let step: Step = serde_json::from_str(
            r#"{"type":"passkeyLoginStart","id":"a","bind":"c","account":{"kind":"path","path":"input.account"}}"#,
        )
        .unwrap();
        assert!(matches!(
            step,
            Step::PasskeyLoginStart {
                account: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn the_steps_of_a_passkey_are_checked() {
        type Case<'a> = (&'a str, &'a dyn Fn(&mut Project));
        let cases: [Case; 6] = [
            ("E_PASSKEY_SCHEMA", &|p| {
                p.messages
                    .iter_mut()
                    .find(|m| m.name == "PasskeyChallenge")
                    .unwrap()
                    .fields[0]
                    .repeated = true;
            }),
            ("E_PASSKEY_SCHEMA", &|p| {
                p.messages
                    .iter_mut()
                    .find(|m| m.name == "PasskeyChallenge")
                    .unwrap()
                    .fields
                    .retain(|f| f.name != "options");
            }),
            ("E_UNKNOWN_PATH", &|p| {
                if let Step::PasskeyRegisterStart { account, .. } = passkey_step(p, 0, 1) {
                    *account = Expr::Path {
                        path: "claims.nope".into(),
                    };
                }
            }),
            ("E_UNKNOWN_PATH", &|p| {
                if let Step::PasskeyRegisterFinish { credential, .. } = passkey_step(p, 1, 1) {
                    *credential = Expr::Path {
                        path: "input.nope".into(),
                    };
                }
            }),
            ("E_RESERVED_NAME", &|p| {
                if let Step::PasskeyLoginFinish { bind, .. } = passkey_step(p, 3, 0) {
                    *bind = "tx".into();
                }
            }),
            ("E_RESERVED_NAME", &|p| {
                if let Step::PasskeyLoginStart { bind, .. } = passkey_step(p, 2, 1) {
                    *bind = "ctx".into();
                }
            }),
        ];
        for (code, break_it) in cases {
            let mut p = with_passkeys();
            break_it(&mut p);
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
    }

    #[test]
    fn passkeys_only_make_sense_in_a_route() {
        for (route, at) in [(0, 1), (1, 1), (2, 1), (3, 0)] {
            let mut p = with_passkeys();
            let step = passkey_step(&mut p, route, at).clone();
            p.services[0].rpcs[0].steps.insert(0, step);
            assert!(
                errors(&p).contains(&"E_PASSKEY_OUTSIDE_ROUTE".to_string()),
                "{route}: {:?}",
                errors(&p)
            );
        }
    }

    fn with_avatars() -> Project {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_avatars(&mut p);
        p
    }

    /// The three routes of `add_avatars` come after the five of the orders and the JWT.
    fn avatar_step(p: &mut Project, route: usize, at: usize) -> &mut Step {
        &mut p.services[0].routes[5 + route].steps[at]
    }

    #[test]
    fn an_image_is_checked_stored_read_and_deleted() {
        let p = with_avatars();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        // What is read back may be absent: the answer has to allow it.
        let mut p = with_avatars();
        p.messages
            .iter_mut()
            .find(|m| m.name == "AvatarView")
            .unwrap()
            .fields[0]
            .optional = false;
        assert!(
            errors(&p).contains(&"E_TYPE_MISMATCH".to_string()),
            "{:?}",
            errors(&p)
        );
    }

    #[test]
    fn the_steps_of_the_object_storage_are_checked() {
        type Case<'a> = (&'a str, &'a dyn Fn(&mut Project));
        let cases: [Case; 13] = [
            ("E_PRESIGN_EXPIRY", &|p| {
                if let Step::ObjectPresignGet { expires_secs, .. } = avatar_step(p, 4, 1) {
                    *expires_secs = 0;
                }
            }),
            ("E_PRESIGN_EXPIRY", &|p| {
                if let Step::ObjectPresignPut { expires_secs, .. } = avatar_step(p, 3, 1) {
                    *expires_secs = 604_801;
                }
            }),
            ("E_IMAGE_SIZE", &|p| {
                if let Step::ObjectPresignPut { max_bytes, .. } = avatar_step(p, 3, 1) {
                    *max_bytes = 0;
                }
            }),
            ("E_TYPE_MISMATCH", &|p| {
                if let Step::ObjectPresignPut { size, .. } = avatar_step(p, 3, 1) {
                    *size = Expr::Path {
                        path: "input.content_type".into(),
                    };
                }
            }),
            ("E_IMAGE_ALLOW", &|p| {
                if let Step::ImageSniff { allow, .. } = avatar_step(p, 0, 1) {
                    allow.clear();
                }
            }),
            ("E_IMAGE_ALLOW", &|p| {
                if let Step::ImageSniff { allow, .. } = avatar_step(p, 0, 1) {
                    allow.push(ImageKind::Png);
                }
            }),
            ("E_IMAGE_SIZE", &|p| {
                if let Step::ImageSniff { max_bytes, .. } = avatar_step(p, 0, 1) {
                    *max_bytes = 0;
                }
            }),
            ("E_IMAGE_SIZE", &|p| {
                if let Step::ImageSniff { max_bytes, .. } = avatar_step(p, 0, 1) {
                    *max_bytes = 10 * 1024 * 1024 + 1;
                }
            }),
            ("E_BUCKET_ENV", &|p| {
                if let Step::ObjectPut { bucket_env, .. } = avatar_step(p, 0, 2) {
                    *bucket_env = "avatar-bucket".into();
                }
            }),
            ("E_BUCKET_ENV", &|p| {
                if let Step::ObjectGet { bucket_env, .. } = avatar_step(p, 1, 1) {
                    *bucket_env = String::new();
                }
            }),
            ("E_UNKNOWN_PATH", &|p| {
                if let Step::ObjectPut { content_type, .. } = avatar_step(p, 0, 2) {
                    *content_type = Expr::Path {
                        path: "nope".into(),
                    };
                }
            }),
            ("E_UNKNOWN_PATH", &|p| {
                if let Step::ObjectDelete { key, .. } = avatar_step(p, 2, 1) {
                    *key = Expr::Path {
                        path: "claims.nope".into(),
                    };
                }
            }),
            ("E_RESERVED_NAME", &|p| {
                if let Step::ImageSniff { bind, .. } = avatar_step(p, 0, 1) {
                    *bind = "tx".into();
                }
            }),
        ];
        for (code, break_it) in cases {
            let mut p = with_avatars();
            break_it(&mut p);
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
    }

    #[test]
    fn a_refused_image_is_a_400_so_only_routes_check_one() {
        let mut p = with_avatars();
        let step = avatar_step(&mut p, 0, 1).clone();
        p.services[0].rpcs[0].steps.insert(0, step);
        assert!(errors(&p).contains(&"E_IMAGE_OUTSIDE_ROUTE".to_string()));
        // Storing, reading and deleting work anywhere: a consumer can clean up after an event.
        let mut p = with_avatars();
        let step = avatar_step(&mut p, 2, 1).clone();
        p.services[0].rpcs[0].steps.insert(0, step);
        assert!(!errors(&p).iter().any(|c| c.starts_with("E_IMAGE")));
    }

    fn with_notes() -> Project {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_notes(&mut p);
        p
    }

    fn notes_table(p: &mut Project) -> &mut Table {
        p.services[0]
            .tables
            .iter_mut()
            .find(|t| t.name == "notes")
            .unwrap()
    }

    /// `create_note`, `get_note` and `delete_note` are the routes 5, 6 and 7; the SQL step comes after the token, the
    /// branch and the `tenant_id` variable.
    fn note_sql(p: &mut Project, route: usize) -> &mut Step {
        let at = if route == 5 { 4 } else { 3 };
        &mut p.services[0].routes[route].steps[at]
    }

    #[test]
    fn a_table_can_be_scoped_to_its_tenant_and_keep_what_is_deleted() {
        let p = with_notes();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        let table = p.services[0]
            .tables
            .iter()
            .find(|t| t.name == "notes")
            .unwrap();
        assert_eq!(table.tenant_column.as_deref(), Some("tenant_id"));
        assert!(table.soft_delete);
    }

    #[test]
    fn the_columns_the_tenant_and_the_soft_delete_lean_on_are_checked() {
        type Case<'a> = (&'a str, &'a dyn Fn(&mut Table));
        let cases: [Case; 6] = [
            ("E_TENANT_COLUMN", &|t| {
                t.tenant_column = Some("nope".into())
            }),
            ("E_TENANT_COLUMN", &|t| {
                t.tenant_column = Some("deleted_at".into())
            }),
            ("E_TENANT_COLUMN", &|t| {
                t.columns
                    .iter_mut()
                    .find(|c| c.name == "tenant_id")
                    .unwrap()
                    .nullable = true
            }),
            ("E_TENANT_COLUMN", &|t| {
                t.columns
                    .iter_mut()
                    .find(|c| c.name == "tenant_id")
                    .unwrap()
                    .ty = ColumnType::Int64
            }),
            ("E_SOFT_DELETE_COLUMN", &|t| {
                t.columns.retain(|c| c.name != "deleted_at")
            }),
            ("E_SOFT_DELETE_COLUMN", &|t| {
                t.columns
                    .iter_mut()
                    .find(|c| c.name == "deleted_at")
                    .unwrap()
                    .nullable = false
            }),
        ];
        for (code, break_it) in cases {
            let mut p = with_notes();
            break_it(notes_table(&mut p));
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
    }

    #[test]
    fn a_step_on_a_tenant_table_needs_the_tenant_and_leaves_the_columns_to_the_generator() {
        // No variable `tenant_id`: the filter would have nothing to filter by.
        let mut p = with_notes();
        p.services[0].routes[6].steps.remove(2);
        assert!(
            errors(&p).contains(&"E_TENANT_SCOPE".to_string()),
            "{:?}",
            errors(&p)
        );
        // A variable of the wrong type is no better.
        let mut p = with_notes();
        if let Step::Let {
            value_type, value, ..
        } = &mut p.services[0].routes[6].steps[2]
        {
            *value_type = FieldType::Int64;
            *value = Expr::Literal { value: 1.into() };
        }
        assert!(
            errors(&p).contains(&"E_TENANT_SCOPE".to_string()),
            "{:?}",
            errors(&p)
        );
        // The author does not set the tenant, which would move a row to another, nor filter on it.
        let mut p = with_notes();
        if let Step::Sql { values, .. } = note_sql(&mut p, 5) {
            values.push(Assignment {
                field: "tenant_id".into(),
                value: Expr::Path {
                    path: "tenant_id".into(),
                },
            });
        }
        assert!(
            errors(&p).contains(&"E_TENANT_MANUAL".to_string()),
            "{:?}",
            errors(&p)
        );
        let mut p = with_notes();
        if let Step::Sql { conditions, .. } = note_sql(&mut p, 6) {
            conditions.push(SqlCondition::new(
                "tenant_id",
                CmpOp::Eq,
                Expr::Path {
                    path: "tenant_id".into(),
                },
            ));
        }
        assert!(
            errors(&p).contains(&"E_TENANT_MANUAL".to_string()),
            "{:?}",
            errors(&p)
        );
        // Nor does the author set `deleted_at`.
        let mut p = with_notes();
        if let Step::Sql { values, .. } = note_sql(&mut p, 5) {
            values.push(Assignment {
                field: "deleted_at".into(),
                value: Expr::Now,
            });
        }
        assert!(
            errors(&p).contains(&"E_SOFT_DELETE_MANUAL".to_string()),
            "{:?}",
            errors(&p)
        );
        // Reading the columns is fine.
        let mut p = with_notes();
        if let Step::Sql { columns, .. } = note_sql(&mut p, 6) {
            columns.extend(["tenant_id".to_string(), "deleted_at".to_string()]);
        }
        assert_eq!(errors(&p), Vec::<String>::new());
    }

    fn with_mailer() -> Project {
        let mut p = sample::orders();
        sample::add_signup(&mut p);
        sample::add_mailer(&mut p);
        p
    }

    fn mailer(p: &mut Project) -> &mut Service {
        p.services.iter_mut().find(|s| s.name == "Mailer").unwrap()
    }

    use crate::maildesign::{MailBlock, MailDesign};

    fn with_mail_design() -> Project {
        let mut p = with_mailer();
        sample::add_mail_design(&mut p);
        p
    }

    #[test]
    fn a_composed_mail_is_valid_and_its_checks_point_at_the_block() {
        let p = with_mail_design();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        let broken = |change: &dyn Fn(&mut MailDesign)| {
            let mut p = with_mail_design();
            change(mailer(&mut p).templates[0].design.as_mut().unwrap());
            validate(&p)
        };
        let has = |d: &[Diagnostic], code: &str, path: &str| {
            d.iter().any(|d| d.code == code && d.path == path)
        };
        let base = "/services/";
        let at = |tail: &str| {
            let p = with_mail_design();
            let i = p.services.iter().position(|s| s.name == "Mailer").unwrap();
            format!("{base}{i}/templates/0/design/{tail}")
        };
        let d = broken(&|d| d.blocks[1] = MailBlock::paragraph("Hi {{ nobody }}"));
        assert!(
            has(&d, "E_MAIL_VARIABLE_UNKNOWN", &at("blocks/1/text")),
            "{d:?}"
        );
        let d = broken(&|d| d.blocks[2] = MailBlock::button("Go", "ftp://x"));
        assert!(has(&d, "E_MAIL_LINK", &at("blocks/2/url")), "{d:?}");
        let d = broken(&|d| d.settings.background = "white".into());
        assert!(has(&d, "E_MAIL_COLOR", &at("settings/background")), "{d:?}");
        let d = broken(&|d| d.blocks.clear());
        assert!(has(&d, "E_MAIL_DESIGN_EMPTY", &at("blocks")), "{d:?}");
        let d = broken(&|d| d.blocks[0] = MailBlock::heading("{% if x %}"));
        assert!(has(&d, "E_MAIL_TEXT_SYNTAX", &at("blocks/0/text")), "{d:?}");
        // Warnings do not stop the generation.
        let d = broken(&|d| d.blocks.push(MailBlock::image("https://a.b/c.png", "")));
        assert!(has(&d, "W_MAIL_IMAGE_ALT", &at("blocks/5/alt")), "{d:?}");
        assert!(!has_errors(&d));
        // A design and hand-written HTML at once: the HTML is ignored, and the author is told.
        let mut p = with_mail_design();
        mailer(&mut p).templates[0].html = "<p>x</p>".into();
        assert!(validate(&p).iter().any(|d| d.code == "W_MAIL_HTML_IGNORED"));
        // A design with no block has no body, hand-written or not.
        let d = broken(&|d| d.blocks.clear());
        assert!(
            d.iter().any(|d| d.code == "E_MTA_TEMPLATE_BODY")
                || has(&d, "E_MAIL_DESIGN_EMPTY", &at("blocks"))
        );
    }

    #[test]
    fn a_hand_written_mail_still_opens_without_a_design() {
        let json = r#"{"eventType":"a.b","subject":"s","html":"<p>x</p>","text":"x"}"#;
        let t: MailTemplate = serde_json::from_str(json).unwrap();
        assert!(t.design.is_none());
        assert_eq!(t.bodies(), ("<p>x</p>".to_string(), "x".to_string()));
        assert!(!serde_json::to_string(&t).unwrap().contains("design"));
    }

    #[test]
    fn a_mail_service_mails_the_events_it_listens_to() {
        let p = with_mailer();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
    }

    fn with_realtime() -> Project {
        let mut p = sample::orders();
        sample::add_realtime(&mut p);
        p
    }

    fn realtime(p: &mut Project) -> &mut Service {
        p.services
            .iter_mut()
            .find(|s| s.kind == ServiceKind::Realtime)
            .unwrap()
    }

    #[test]
    fn a_realtime_service_is_valid_with_its_five_settings_and_nothing_else() {
        let p = with_realtime();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        // No `W_EMPTY_SERVICE`: it is fixed code, it has nothing to draw.
        assert!(validate(&p).iter().all(|d| d.code != "W_EMPTY_SERVICE"));
        // The settings are optional in the JSON (the defaults are the usual ones), but the service needs them.
        let json = serde_json::to_value(&p).unwrap();
        let service = json["services"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["kind"] == "realtime")
            .expect("a realtime service");
        assert_eq!(service["realtime"]["sessionKeyPrefix"], "sess");
        let bare: RealtimeConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(bare, RealtimeConfig::default());
    }

    #[test]
    fn what_a_realtime_service_is_made_of_is_checked() {
        type Case<'a> = (&'a str, &'a dyn Fn(&mut Project));
        let cases: [Case; 20] = [
            ("E_RT_CONFIG", &|p| realtime(p).realtime = None),
            ("E_HTTP_PORT", &|p| realtime(p).http_port = None),
            // Fixed code, with no database: nothing of its own to draw.
            ("E_RT_CONTENT", &|p| {
                realtime(p).jobs.push(Job {
                    name: "j".into(),
                    every_secs: 60,
                    steps: vec![],
                })
            }),
            ("E_RT_CONTENT", &|p| {
                realtime(p)
                    .tables
                    .push(Table::new("t", vec![Column::new("a", ColumnType::String)]))
            }),
            ("E_RT_CONTENT", &|p| {
                realtime(p).templates.push(MailTemplate {
                    event_type: "x".into(),
                    subject: "s".into(),
                    html: "h".into(),
                    text: "t".into(),
                    design: None,
                })
            }),
            // The names end up in a Redis command, a key, a route and a Cookie header.
            ("E_RT_CHANNEL", &|p| {
                realtime(p).realtime.as_mut().unwrap().channel = "".into()
            }),
            ("E_RT_CHANNEL", &|p| {
                realtime(p).realtime.as_mut().unwrap().channel = "live events".into()
            }),
            ("E_RT_CHANNEL", &|p| {
                realtime(p).realtime.as_mut().unwrap().channel = "live*".into()
            }),
            ("E_RT_CHANNEL", &|p| {
                realtime(p).realtime.as_mut().unwrap().channel = "a".repeat(129)
            }),
            ("E_RT_PREFIX", &|p| {
                realtime(p).realtime.as_mut().unwrap().session_key_prefix = "".into()
            }),
            ("E_RT_PREFIX", &|p| {
                realtime(p).realtime.as_mut().unwrap().session_key_prefix = "sess:\r\n".into()
            }),
            ("E_RT_COOKIE", &|p| {
                realtime(p).realtime.as_mut().unwrap().access_cookie = "".into()
            }),
            ("E_RT_COOKIE", &|p| {
                realtime(p).realtime.as_mut().unwrap().access_cookie = "a=b".into()
            }),
            ("E_RT_COOKIE", &|p| {
                realtime(p).realtime.as_mut().unwrap().access_cookie = "a; Path=/".into()
            }),
            ("E_RT_ISSUER", &|p| {
                realtime(p).realtime.as_mut().unwrap().issuer = "  ".into()
            }),
            ("E_RT_ISSUER", &|p| {
                realtime(p).realtime.as_mut().unwrap().issuer = "a\nb".into()
            }),
            ("E_RT_PATH", &|p| {
                realtime(p).realtime.as_mut().unwrap().path = "v1/events".into()
            }),
            ("E_RT_PATH", &|p| {
                realtime(p).realtime.as_mut().unwrap().path = "/v1/{id}".into()
            }),
            ("E_RT_PATH", &|p| {
                realtime(p).realtime.as_mut().unwrap().path = "/v1//events".into()
            }),
            // Taken by the health and metrics endpoints of the same service.
            ("E_RT_PATH", &|p| {
                realtime(p).realtime.as_mut().unwrap().path = "/metrics".into()
            }),
        ];
        for (code, change) in cases {
            let mut p = with_realtime();
            change(&mut p);
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
    }

    #[test]
    fn realtime_settings_belong_to_a_realtime_service_and_it_has_its_own_port() {
        let mut p = with_realtime();
        p.services[0].realtime = Some(RealtimeConfig::default());
        assert!(errors(&p).contains(&"E_REALTIME_OUTSIDE".to_string()));
        // Its HTTP port is checked against the others like any.
        let mut p = with_realtime();
        let taken = p.services[0].port;
        realtime(&mut p).http_port = Some(taken);
        assert!(errors(&p).contains(&"E_DUPLICATE_PORT".to_string()));
    }

    #[test]
    fn what_a_mail_service_is_made_of_is_checked() {
        type Case<'a> = (&'a str, &'a dyn Fn(&mut Project));
        let cases: [Case; 17] = [
            // Fixed code: nothing of its own to draw.
            ("E_MTA_CONTENT", &|p| {
                mailer(p).jobs.push(Job {
                    name: "j".into(),
                    every_secs: 60,
                    steps: vec![],
                })
            }),
            ("E_MTA_CONTENT", &|p| {
                mailer(p)
                    .tables
                    .push(Table::new("t", vec![Column::new("a", ColumnType::String)]))
            }),
            ("E_HTTP_PORT", &|p| mailer(p).http_port = None),
            // Each event has a rule, and no steps.
            ("E_MTA_RULE", &|p| mailer(p).consumers[0].mail = None),
            ("E_MTA_STEPS", &|p| {
                mailer(p).consumers[0].steps.push(Step::Log {
                    id: "x".into(),
                    level: LogLevel::Info,
                    message: "m".into(),
                })
            }),
            ("E_MTA_STEPS", &|p| {
                mailer(p).consumers[0].idempotent_on = Some("user_id".into())
            }),
            // The fields the rule reads are texts of the message.
            ("E_MTA_FIELD", &|p| {
                mailer(p).consumers[0].mail.as_mut().unwrap().recipient = "nope".into()
            }),
            ("E_MTA_FIELD", &|p| {
                mailer(p).consumers[0].mail.as_mut().unwrap().dedup_field = "nope".into()
            }),
            ("E_MTA_FIELD", &|p| {
                mailer(p).consumers[0].mail.as_mut().unwrap().reply_to = Some("nope".into())
            }),
            ("E_MTA_FIELD_TYPE", &|p| {
                p.messages
                    .iter_mut()
                    .find(|m| m.name == "AccountRegistered")
                    .unwrap()
                    .fields[1]
                    .repeated = true
            }),
            ("E_MTA_EVENT_FIELD", &|p| {
                p.messages
                    .iter_mut()
                    .find(|m| m.name == "AccountRegistered")
                    .unwrap()
                    .fields
                    .push(Field::new("at", FieldType::Timestamp))
            }),
            // Each event has one mail, and each mail one event.
            ("E_MTA_TEMPLATE_MISSING", &|p| mailer(p).templates.clear()),
            ("E_MTA_TEMPLATE_UNKNOWN", &|p| {
                mailer(p).templates[0].event_type = "other.topic".into()
            }),
            ("E_MTA_TEMPLATE_DUPLICATE", &|p| {
                let t = mailer(p).templates[0].clone();
                mailer(p).templates.push(t)
            }),
            ("E_MTA_TEMPLATE_SUBJECT", &|p| {
                mailer(p).templates[0].subject = " ".into()
            }),
            ("E_MTA_TEMPLATE_BODY", &|p| {
                mailer(p).templates[0].html.clear();
                mailer(p).templates[0].text.clear()
            }),
            ("E_MTA_DUPLICATE_EVENT", &|p| {
                let c = mailer(p).consumers[0].clone();
                mailer(p).consumers.push(Consumer {
                    name: "again".into(),
                    ..c
                })
            }),
        ];
        for (code, break_it) in cases {
            let mut p = with_mailer();
            break_it(&mut p);
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code}: {:?}",
                errors(&p)
            );
        }
    }

    #[test]
    fn mail_rules_and_templates_belong_to_a_mail_service() {
        let mut p = with_mailer();
        mailer(&mut p).kind = ServiceKind::Standard;
        let found = errors(&p);
        assert!(
            found.contains(&"E_MAIL_OUTSIDE_MTA".to_string()),
            "{found:?}"
        );
        assert!(
            found.contains(&"E_TEMPLATE_OUTSIDE_MTA".to_string()),
            "{found:?}"
        );
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
    fn a_token_for_the_realtime_service_is_checked_against_its_contract() {
        fn sign(issuer: &str, claims: &[(&str, &str)]) -> Step {
            Step::SignJwt {
                id: "j".into(),
                key_env: "JWT_SIGNING_KEY".into(),
                issuer: issuer.into(),
                ttl_secs: 300,
                claims: claims
                    .iter()
                    .map(|(f, v)| Assignment {
                        field: (*f).into(),
                        value: Expr::Path { path: (*v).into() },
                    })
                    .collect(),
                bind: "token".into(),
            }
        }
        let warnings = |p: &Project| -> Vec<String> {
            validate(p)
                .into_iter()
                .filter(|d| d.severity == Severity::Warning)
                .map(|d| d.code)
                .collect()
        };
        let build = |issuer: &str, claims: &[(&str, &str)]| {
            let mut p = with_jwt();
            sample::add_realtime(&mut p);
            realtime(&mut p).realtime.as_mut().unwrap().issuer = "orders".into();
            p.services[0].routes[1]
                .steps
                .insert(0, sign(issuer, claims));
            p
        };
        // Same issuer, no `sid`: the stream would answer 401.
        let p = build("orders", &[("sub", "input.order_id")]);
        assert!(
            warnings(&p).contains(&"W_RT_NO_SID".to_string()),
            "{:?}",
            warnings(&p)
        );
        // With `sid` and text claims: nothing to say.
        let p = build(
            "orders",
            &[("sub", "input.order_id"), ("sid", "input.order_id")],
        );
        assert!(
            !warnings(&p).iter().any(|c| c.starts_with("W_RT_")),
            "{:?}",
            warnings(&p)
        );
        // A token for another issuer that names a session: the issuer would not match.
        let p = build(
            "other",
            &[("sub", "input.order_id"), ("sid", "input.order_id")],
        );
        assert!(
            warnings(&p).contains(&"W_RT_ISSUER".to_string()),
            "{:?}",
            warnings(&p)
        );
        // Another issuer without `sid`: it is not meant for the stream.
        let p = build("other", &[("sub", "input.order_id")]);
        assert!(
            !warnings(&p).iter().any(|c| c.starts_with("W_RT_")),
            "{:?}",
            warnings(&p)
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
    fn an_upsert_needs_a_key_written_by_the_step() {
        let upsert = |conflict: &[&str]| {
            let mut p = sample::orders();
            if let Step::Sql {
                op, conflict: c, ..
            } = sql_step(&mut p, 0, 3)
            {
                *op = SqlOp::Upsert;
                *c = conflict.iter().map(|s| s.to_string()).collect();
            }
            errors(&p)
        };
        assert!(
            upsert(&["order_id"]).is_empty(),
            "{:?}",
            upsert(&["order_id"])
        );
        assert!(upsert(&[]).contains(&"E_SQL_NO_CONFLICT".into()));
        assert!(upsert(&["nope"]).contains(&"E_UNKNOWN_COLUMN".into()));
        assert!(upsert(&["status"]).contains(&"E_SQL_CONFLICT_UNSET".into()));
        // A key outside an upsert is refused.
        let mut p = sample::orders();
        if let Step::Sql { conflict, .. } = sql_step(&mut p, 0, 3) {
            *conflict = vec!["order_id".into()];
        }
        assert!(errors(&p).contains(&"E_SQL_CONFLICT_OP".into()));
    }

    #[test]
    fn a_table_read_but_fed_by_another_service_is_flagged() {
        let codes =
            |p: &Project| -> Vec<String> { validate(p).into_iter().map(|d| d.code).collect() };
        let mut p = sample::orders();
        assert!(!codes(&p).contains(&"W_TABLE_FOREIGN".into()));
        let mut copy = p.services[0].clone();
        copy.name = "Mirror".into();
        for step in copy.routes.iter_mut().flat_map(|r| r.steps.iter_mut()) {
            if let Step::Sql { op, .. } = step {
                *op = SqlOp::Select;
            }
        }
        p.services.push(copy);
        assert!(codes(&p).contains(&"W_TABLE_FOREIGN".into()));
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
            mutable: false,
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
            headers: vec![],
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
                mutable: false,
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
            circuit: None,
            max_concurrent: None,
        });
        p.services[2].consumers[1].steps.push(Step::CallRpc {
            id: "d".into(),
            service: "Orders".into(),
            method: "Missing".into(),
            request: vec![],
            bind: None,
            circuit: None,
            max_concurrent: None,
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

    // ---- publishLive ---------------------------------------------------------------------

    fn live_step(p: &mut Project) -> &mut Step {
        p.services
            .iter_mut()
            .find(|s| s.name == "Orders")
            .unwrap()
            .rpcs
            .iter_mut()
            .find(|r| r.name == "CreateOrder")
            .unwrap()
            .steps
            .iter_mut()
            .find(|s| matches!(s, Step::PublishLive { .. }))
            .expect("a publishLive step")
    }

    #[test]
    fn a_step_that_pushes_an_event_to_the_browsers_is_valid() {
        let mut p = sample::orders();
        sample::add_live_events(&mut p);
        assert_eq!(errors(&p), Vec::<String>::new(), "{:?}", validate(&p));
    }

    #[test]
    fn a_grants_step_needs_a_real_time_service_and_the_right_types() {
        use serde_json::json;
        let mut p = sample::orders();
        sample::add_live_events(&mut p);
        let step = |service: &str, permissions: serde_json::Value| -> Step {
            serde_json::from_value(json!({
                "type": "liveGrants", "id": "g1", "service": service,
                "accountId": {"kind": "path", "path": "order_id"},
                "permissions": permissions,
            }))
            .unwrap()
        };
        let put = |p: &mut Project, s: Step| {
            let rpc = p
                .services
                .iter_mut()
                .find(|s| s.name == "Orders")
                .unwrap()
                .rpcs
                .iter_mut()
                .find(|r| r.name == "CreateOrder")
                .unwrap();
            rpc.steps.insert(0, s);
        };
        let mut ok = p.clone();
        put(
            &mut ok,
            step(
                "Realtime",
                json!({"kind": "path", "path": "input.customer_id"}),
            ),
        );
        assert!(
            !codes(&ok).contains(&"E_LIVE_SERVICE".into()),
            "{:?}",
            validate(&ok)
        );
        let mut bad = p.clone();
        put(&mut bad, step("Orders", json!(null)));
        assert!(codes(&bad).contains(&"E_LIVE_SERVICE".into()));
        let mut gone = p;
        put(&mut gone, step("Nowhere", json!(null)));
        assert!(codes(&gone).contains(&"E_LIVE_SERVICE".into()));
    }

    #[test]
    fn session_steps_need_a_real_time_service_and_an_integer_lifetime() {
        use serde_json::json;
        let mut p = sample::orders();
        sample::add_live_events(&mut p);
        let put = |p: &mut Project, v: serde_json::Value| {
            let rpc = p
                .services
                .iter_mut()
                .find(|s| s.name == "Orders")
                .unwrap()
                .rpcs
                .iter_mut()
                .find(|r| r.name == "CreateOrder")
                .unwrap();
            rpc.steps.insert(0, serde_json::from_value(v).unwrap());
        };
        let trace = |service: &str, ttl: serde_json::Value| {
            json!({
                "type": "sessionTrace", "id": "t1", "service": service,
                "sessionId": {"kind": "path", "path": "order_id"},
                "accountId": {"kind": "path", "path": "order_id"},
                "ttlSeconds": ttl,
            })
        };
        let mut ok = p.clone();
        put(
            &mut ok,
            trace("Realtime", json!({"kind": "literal", "value": 900})),
        );
        put(
            &mut ok,
            json!({"type": "sessionRevoke", "id": "t2", "service": "Realtime", "sessionId": {"kind": "path", "path": "order_id"}}),
        );
        assert!(
            !codes(&ok).contains(&"E_LIVE_SERVICE".into()),
            "{:?}",
            validate(&ok)
        );
        let mut bad = p.clone();
        put(
            &mut bad,
            trace("Orders", json!({"kind": "literal", "value": 900})),
        );
        assert!(codes(&bad).contains(&"E_LIVE_SERVICE".into()));
        let mut revoke = p;
        put(
            &mut revoke,
            json!({"type": "sessionRevoke", "id": "t2", "service": "Nowhere", "sessionId": {"kind": "path", "path": "order_id"}}),
        );
        assert!(codes(&revoke).contains(&"E_LIVE_SERVICE".into()));
    }

    /// Changes some fields of a step, by their JSON names (`null` removes one).
    fn patch(step: &mut Step, changes: serde_json::Value) {
        let mut value = serde_json::to_value(&*step).unwrap();
        for (key, new) in changes.as_object().unwrap() {
            if new.is_null() {
                value.as_object_mut().unwrap().remove(key);
            } else {
                value[key] = new.clone();
            }
        }
        *step = serde_json::from_value(value).unwrap();
    }

    #[test]
    fn what_a_live_event_says_and_who_it_is_for_is_checked() {
        use serde_json::json;
        let uuid = "2f0b2b50-6a5c-4d52-9d7b-0a1c5c0f9a11";
        let key = |i: usize| json!({"field": format!("k{i}"), "value": {"kind": "path", "path": "order_id"}});
        let cases = [
            ("E_LIVE_SERVICE", json!({"service": "Orders"})),
            ("E_LIVE_SERVICE", json!({"service": "Nowhere"})),
            ("E_LIVE_EVENT", json!({"event": "created"})),
            ("E_LIVE_EVENT", json!({"event": "Orders.Created"})),
            ("E_LIVE_EVENT", json!({"event": "orders.created\nx"})),
            // The two names the real-time service keeps for itself.
            ("E_LIVE_EVENT", json!({"event": "grants.updated"})),
            ("E_LIVE_PERMISSION", json!({"permission": "orders"})),
            ("E_LIVE_PERMISSION", json!({"permission": "orders.edit"})),
            // An event is never "for everybody".
            ("E_LIVE_AUDIENCE", json!({"permission": null})),
            // An account is the whole audience: the service would ignore the rest.
            (
                "E_LIVE_AUDIENCE",
                json!({"accountId": {"kind": "literal", "value": uuid}}),
            ),
            (
                "E_LIVE_DATA",
                json!({"data": [key(0), key(1), key(2), key(3), key(4)]}),
            ),
            (
                "E_LIVE_DATA",
                json!({"data": [{"field": "bad key", "value": {"kind": "path", "path": "order_id"}}]}),
            ),
            ("E_LIVE_DATA", json!({"data": [key(0), key(0)]})),
            (
                "E_TYPE_MISMATCH",
                json!({"tenantId": {"kind": "literal", "value": "not a uuid"}}),
            ),
            (
                "E_UNKNOWN_PATH",
                json!({"origin": {"kind": "path", "path": "nowhere"}}),
            ),
        ];
        for (code, change) in cases {
            let mut p = sample::orders();
            sample::add_live_events(&mut p);
            patch(live_step(&mut p), change.clone());
            assert!(
                errors(&p).contains(&code.to_string()),
                "{code} {change}: {:?}",
                validate(&p)
            );
        }
        // An account alone is an audience; so are accounts of a company.
        for change in [
            json!({"permission": null, "accountId": {"kind": "literal", "value": uuid}}),
            json!({"permission": null, "tenantId": {"kind": "literal", "value": uuid}, "accountIds": {"kind": "raw", "code": "vec![]"}}),
        ] {
            let mut p = sample::orders();
            sample::add_live_events(&mut p);
            patch(live_step(&mut p), change.clone());
            assert_eq!(
                errors(&p),
                Vec::<String>::new(),
                "{change}: {:?}",
                validate(&p)
            );
        }
    }

    fn with_reads() -> Project {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_sql_reads(&mut p);
        p
    }

    /// The paged read of the sample, to break one thing at a time.
    fn reads_step(p: &mut Project) -> &mut Step {
        let route = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "list_posts")
            .unwrap();
        &mut route.steps[4]
    }

    fn reads_codes(p: &Project) -> Vec<String> {
        validate(p).into_iter().map(|d| d.code).collect()
    }

    #[test]
    fn a_paged_read_across_two_tenant_tables_is_valid() {
        let p = with_reads();
        assert_eq!(validate(&p), vec![], "{:?}", validate(&p));
        // The page binds its variables: the rows, the total, whether a page follows, the last row.
        let route = p.services[0]
            .routes
            .iter()
            .find(|r| r.name == "list_posts")
            .unwrap();
        let typed = p.with_sql_rows();
        let _ = typed;
        assert!(matches!(route.steps[5], Step::Respond { .. }));
    }

    #[test]
    fn a_project_without_joins_keeps_its_file_as_it_was() {
        // The new fields are left out of the file when they are empty, and an old file without them opens.
        let p = sample::orders();
        let text = serde_json::to_string(&p).unwrap();
        for word in [
            "\"joins\"",
            "\"thenBy\"",
            "\"cursor\"",
            "\"limitFrom\"",
            "\"totalBind\"",
        ] {
            assert!(!text.contains(word), "{word}");
        }
        let old: Step = serde_json::from_str(
            r#"{"type":"sql","id":"s","op":"select","table":"orders","bind":"o","columns":["id"],
                "where":[{"column":"id","op":"eq","value":{"kind":"path","path":"input.id"}}]}"#,
        )
        .unwrap();
        let Step::Sql {
            joins, conditions, ..
        } = old
        else {
            panic!()
        };
        assert!(joins.is_empty());
        assert_eq!(conditions[0].test, SqlTest::Compare);
    }

    #[test]
    fn column_aliases_are_checked() {
        let broken = |key: &str, name: &str| {
            let mut p = with_reads();
            if let Step::Sql { column_as, .. } = reads_step(&mut p) {
                column_as.insert(key.into(), name.into());
            }
            reads_codes(&p)
        };
        let codes = broken("authors.name", "Bad Name");
        assert!(codes.contains(&"E_SQL_ALIAS_NAME".to_string()), "{codes:?}");
        let codes = broken("authors.email", "email");
        assert!(
            codes.contains(&"E_SQL_ALIAS_COLUMN".to_string()),
            "{codes:?}"
        );
        // Two columns giving the same field.
        let codes = broken("authors.name", "title");
        assert!(
            codes.contains(&"E_SQL_COLUMN_CLASH".to_string()),
            "{codes:?}"
        );
        // A good alias has no alias error.
        let codes = broken("authors.name", "author");
        assert!(
            !codes.iter().any(|c| c.starts_with("E_SQL_ALIAS")),
            "{codes:?}"
        );
    }

    #[test]
    fn joins_are_checked() {
        let broken = |edit: &dyn Fn(&mut Step)| {
            let mut p = with_reads();
            edit(reads_step(&mut p));
            reads_codes(&p)
        };
        let codes = broken(&|s| {
            if let Step::Sql { joins, .. } = s {
                joins[0].table = "nowhere".into();
            }
        });
        assert!(codes.contains(&"E_SQL_JOIN_TABLE".to_string()), "{codes:?}");
        let codes = broken(&|s| {
            if let Step::Sql { joins, .. } = s {
                joins[0].table = "posts".into();
            }
        });
        assert!(
            codes.contains(&"E_SQL_JOIN_DUPLICATE".to_string()),
            "{codes:?}"
        );
        let codes = broken(&|s| {
            if let Step::Sql { joins, .. } = s {
                joins[0].on.clear();
            }
        });
        assert!(codes.contains(&"E_SQL_JOIN_ON".to_string()), "{codes:?}");
        let codes = broken(&|s| {
            if let Step::Sql { joins, .. } = s {
                joins[0].on[0].ref_column = "deleted_at".into();
            }
        });
        assert!(codes.contains(&"E_SQL_JOIN_TYPE".to_string()), "{codes:?}");
        // A column of a table that is not in the query, and two columns that give the same field.
        let codes = broken(&|s| {
            if let Step::Sql { columns, .. } = s {
                columns.push("orders.status".into());
            }
        });
        assert!(codes.contains(&"E_UNKNOWN_COLUMN".to_string()), "{codes:?}");
        let codes = broken(&|s| {
            if let Step::Sql { columns, .. } = s {
                columns.push("title".into());
            }
        });
        assert!(
            codes.contains(&"E_SQL_COLUMN_CLASH".to_string()),
            "{codes:?}"
        );
        // Joins only exist in a select.
        let codes = broken(&|s| {
            if let Step::Sql { op, .. } = s {
                *op = SqlOp::Delete;
            }
        });
        assert!(codes.contains(&"E_SQL_JOIN_OP".to_string()), "{codes:?}");
    }

    #[test]
    fn the_rules_of_a_joined_table_hold_too() {
        // The tenant variable is needed for the joined table even when the table of the step has no tenant.
        let mut p = with_reads();
        for t in &mut p.services[0].tables {
            if t.name == "posts" {
                t.tenant_column = None;
                t.soft_delete = false;
            }
        }
        let route = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "list_posts")
            .unwrap();
        route.steps.remove(3);
        assert!(
            reads_codes(&p).contains(&"E_TENANT_SCOPE".to_string()),
            "{:?}",
            reads_codes(&p)
        );
        // Filtering on the tenant of a joined table by hand is refused, as it is for the table of the step.
        let mut p = with_reads();
        if let Step::Sql { conditions, .. } = reads_step(&mut p) {
            conditions.push(SqlCondition::new(
                "authors.tenant_id",
                CmpOp::Eq,
                Expr::Path {
                    path: "tenant_id".into(),
                },
            ));
        }
        assert!(
            reads_codes(&p).contains(&"E_TENANT_MANUAL".to_string()),
            "{:?}",
            reads_codes(&p)
        );
    }

    #[test]
    fn pages_and_cursors_are_checked() {
        let broken = |edit: &dyn Fn(&mut Step)| {
            let mut p = with_reads();
            edit(reads_step(&mut p));
            validate(&p)
        };
        let has = |d: &[Diagnostic], code: &str| d.iter().any(|x| x.code == code);
        // A page needs its largest size.
        let d = broken(&|s| {
            if let Step::Sql { limit, .. } = s {
                *limit = None;
            }
        });
        assert!(has(&d, "E_SQL_PAGE_NEEDS_LIMIT"), "{d:?}");
        // An offset and a cursor are two ways to do one thing.
        let d = broken(&|s| {
            if let Step::Sql { offset, .. } = s {
                *offset = Some(Expr::Literal { value: 10.into() });
            }
        });
        assert!(has(&d, "E_SQL_PAGE_CONFLICT"), "{d:?}");
        // One value per sort key.
        let d = broken(&|s| {
            if let Step::Sql { cursor, .. } = s {
                cursor.pop();
            }
        });
        assert!(has(&d, "E_SQL_CURSOR_ARITY"), "{d:?}");
        // A key that can be null cannot carry a cursor: the left joined `authors.name` can.
        let d = broken(&|s| {
            if let Step::Sql { order_by, .. } = s {
                *order_by = Some("authors.name".into());
            }
        });
        assert!(has(&d, "E_SQL_CURSOR_NULLABLE"), "{d:?}");
        // A sort that does not identify a row is a warning, not an error.
        let d = broken(&|s| {
            if let Step::Sql {
                then_by, cursor, ..
            } = s
            {
                then_by.clear();
                cursor.pop();
            }
        });
        assert!(has(&d, "W_SQL_CURSOR_NOT_UNIQUE"), "{d:?}");
        assert!(!d.iter().any(|x| x.severity == Severity::Error), "{d:?}");
        // The last row is typed by the schema the rows are converted into.
        let d = broken(&|s| {
            if let Step::Sql { into, .. } = s {
                *into = None;
            }
        });
        assert!(has(&d, "E_SQL_LAST_INTO"), "{d:?}");
        // A page size is a whole number.
        let d = broken(&|s| {
            if let Step::Sql { limit_from, .. } = s {
                *limit_from = Some(Expr::Path {
                    path: "input.search".into(),
                });
            }
        });
        assert!(has(&d, "E_SQL_PAGE_VALUE"), "{d:?}");
    }

    #[test]
    fn the_tests_of_a_condition_are_typed_by_what_they_do() {
        let broken = |test: SqlTest, column: &str, value: Expr| {
            let mut p = with_reads();
            if let Step::Sql { conditions, .. } = reads_step(&mut p) {
                conditions.push(SqlCondition {
                    or: Vec::new(),
                    column: column.into(),
                    op: CmpOp::Eq,
                    value,
                    test,
                });
            }
            validate(&p)
        };
        let text = || Expr::Path {
            path: "input.search".into(),
        };
        // A list is needed after `in`; a text column after `contains`.
        let d = broken(SqlTest::In, "title", text());
        assert!(d.iter().any(|x| x.severity == Severity::Error), "{d:?}");
        let d = broken(SqlTest::Contains, "deleted_at", text());
        assert!(d.iter().any(|x| x.code == "E_SQL_TEST_VALUE"), "{d:?}");
        // Null tests take no value, and warn when the column is never null.
        let d = broken(SqlTest::IsNull, "authors.name", text());
        assert_eq!(d, vec![], "{d:?}");
        let d = broken(SqlTest::IsNotNull, "title", text());
        assert!(d.iter().any(|x| x.code == "W_SQL_NEVER_NULL"), "{d:?}");
    }
}
