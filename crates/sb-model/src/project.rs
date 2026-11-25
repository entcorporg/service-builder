//! Serializable description of a project. This is the document the visual editor
//! edits, the file saved on disk, and the input of the code generator.
//!
//! The JSON representation is camelCase so the TypeScript side can use it as is.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 2: topics declare their `producers` (before, a producer was whatever had a Publish step).
/// 3: schemas have a `kind` (event, proto or api).
/// 4: a field of an API schema is required unless it is `lenient` (before, an absent field took its default).
pub const SCHEMA_VERSION: u32 = 4;

/// The JSON Schema of a project file, derived from the types themselves so that it cannot drift
/// from what the editor and the generator accept. Doc comments become descriptions.
pub fn json_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(Project)).expect("a schema is JSON")
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub name: String,
    #[serde(default)]
    pub outbox: OutboxConfig,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub topics: Vec<Topic>,
    #[serde(default)]
    pub services: Vec<Service>,
    /// How the project runs in containers from the Infra page: resources, admin interfaces, reverse proxy.
    #[serde(default, skip_serializing_if = "crate::pods::PodsConfig::is_default")]
    pub pods: crate::pods::PodsConfig,
    /// Whether the services answer with their own OpenAPI document, and where.
    #[serde(default, skip_serializing_if = "OpenApiConfig::is_default")]
    pub openapi: OpenApiConfig,
}

/// A challenge in the manner of [ALTCHA](https://altcha.org): a proof of work, with no third-party service and no tracking. The
/// service answers `GET <path>` with a new challenge (`algorithm`, `challenge`, `maxnumber`, `salt`, `signature`: what the ALTCHA
/// widget reads); the widget finds the secret number whose SHA-256 with the salt is the challenge, which costs the visitor a moment
/// of computing and a robot the same moment, at the scale of its volume; the form is sent with the solution, and a
/// `verifyChallenge` step of the route checks it before anything else runs. A challenge is signed (HMAC-SHA256, key in the
/// environment variable `key_env`, the same on every replica), expires, and is accepted once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeConfig {
    /// Where the challenge is served: a path of the HTTP API that no route uses.
    #[serde(default = "default_challenge_path")]
    pub path: String,
    /// The environment variable that holds the key the challenges are signed with (at least 16 characters; in production the
    /// service does not start without it).
    #[serde(default = "default_challenge_key_env")]
    pub key_env: String,
    /// The secret number is drawn in `0..=max_number`: the work asked of the visitor is about `max_number / 2` hashes.
    #[serde(default = "default_challenge_max_number")]
    pub max_number: u32,
    /// How long a challenge can be solved and sent, in seconds.
    #[serde(default = "default_challenge_ttl")]
    pub ttl_secs: u32,
}

pub const DEFAULT_CHALLENGE_PATH: &str = "/challenge";
pub const DEFAULT_CHALLENGE_KEY_ENV: &str = "CHALLENGE_HMAC_KEY";
pub const DEFAULT_CHALLENGE_MAX_NUMBER: u32 = 1_000_000;
pub const DEFAULT_CHALLENGE_TTL_SECS: u32 = 300;
/// The most work a challenge can ask for: a few seconds in a browser, more on a phone.
pub const MAX_CHALLENGE_NUMBER: u32 = 10_000_000;

fn default_challenge_path() -> String {
    DEFAULT_CHALLENGE_PATH.to_string()
}
fn default_challenge_key_env() -> String {
    DEFAULT_CHALLENGE_KEY_ENV.to_string()
}
fn default_challenge_max_number() -> u32 {
    DEFAULT_CHALLENGE_MAX_NUMBER
}
fn default_challenge_ttl() -> u32 {
    DEFAULT_CHALLENGE_TTL_SECS
}

impl Default for ChallengeConfig {
    fn default() -> Self {
        Self {
            path: default_challenge_path(),
            key_env: default_challenge_key_env(),
            max_number: default_challenge_max_number(),
            ttl_secs: default_challenge_ttl(),
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The OpenAPI 3.1 document of each service is always written with the export; this decides whether the service
/// also serves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OpenApiConfig {
    /// The service answers `GET <path>` with its OpenAPI document (no authentication: it describes the API, not the data).
    #[serde(default)]
    pub serve: bool,
    /// Where it is served, a path of the HTTP API that no route uses.
    #[serde(default = "default_openapi_path")]
    pub path: String,
}

pub const DEFAULT_OPENAPI_PATH: &str = "/openapi.json";

fn default_openapi_path() -> String {
    DEFAULT_OPENAPI_PATH.to_string()
}

impl Default for OpenApiConfig {
    fn default() -> Self {
        Self {
            serve: false,
            path: default_openapi_path(),
        }
    }
}

impl OpenApiConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// How the services publish their events: every event goes through a transactional outbox, and
/// is moved to `<topic>.dlq` once it could not be published after `max_attempts` attempts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutboxConfig {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
}

pub const DEFAULT_OUTBOX_ATTEMPTS: u32 = 5;
pub const MAX_OUTBOX_ATTEMPTS: u32 = 100;

fn default_max_attempts() -> u32 {
    DEFAULT_OUTBOX_ATTEMPTS
}

impl Default for OutboxConfig {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_OUTBOX_ATTEMPTS,
        }
    }
}

/// What a schema is for. All of them are compiled into the `contracts` crate (protobuf messages);
/// API schemas also get a JSON (de)serialization, for the HTTP routes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MessageKind {
    /// Payload of a Kafka topic.
    Event,
    /// gRPC request/response, and the building blocks (value objects) shared by events and gRPC.
    #[default]
    Proto,
    /// Body, parameters and response of an HTTP route.
    Api,
    /// The row a SQL step reads: derived from the table and the columns it ticks, never stored in
    /// a project file, only added by [`Project::with_sql_rows`] so that the row can be typed.
    Row,
}

/// A schema: a named set of fields, see [`MessageKind`] for what it can be used for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub name: String,
    #[serde(default)]
    pub kind: MessageKind,
    #[serde(default)]
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: FieldType,
    #[serde(default)]
    pub repeated: bool,
    /// Proto3 `optional` (presence tracking). Only meaningful for scalar fields.
    #[serde(default)]
    pub optional: bool,
    /// Protobuf field number. Must never change once events exist on a topic, so the
    /// editor assigns it once. Untagged fields are numbered by the generator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<u32>,
    /// What a request has to satisfy, checked by the HTTP routes before any step runs. Only on text fields of
    /// API schemas; the first rule that fails is a 400.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// Where an HTTP route reads this field instead of the body or the query string: a request header or a cookie.
    /// Only on text fields of API schemas. The route always overwrites it (empty when the request lacks it), so
    /// that what a client sends in the body can never stand for a header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<RequestSource>,
    /// API schemas, in a request: an absent field takes its default value (`0`, empty text...) instead of being
    /// refused with a 400 that names it. Off by default: a field is required unless it is `optional` or lenient.
    /// Empty text is present; `null` counts as absent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lenient: bool,
    /// API schemas: the key of this field in the JSON of a request or a response, when it is not the field name
    /// (a front-end contract in camelCase, a key that is a Rust keyword such as `type`). The field keeps its name
    /// everywhere else (steps, SQL, protobuf).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_name: Option<String>,
}

impl Field {
    /// The key of the field in JSON.
    pub fn json_key(&self) -> &str {
        self.json_name.as_deref().unwrap_or(&self.name)
    }

    pub fn new(name: impl Into<String>, ty: FieldType) -> Self {
        Self {
            name: name.into(),
            ty,
            repeated: false,
            optional: false,
            tag: None,
            rules: vec![],
            source: None,
            lenient: false,
            json_name: None,
        }
    }
}

/// A part of an HTTP request other than its body, that fills a field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "from", rename_all = "camelCase")]
pub enum RequestSource {
    /// A request header, by name (`Authorization`, `X-Request-Id`...), whatever its case.
    Header { name: String },
    /// A cookie of the `Cookie` header, by name, percent-decoded.
    Cookie { name: String },
    /// The exact bytes of the request body, in base64 (standard alphabet), read before anything parses them: what a
    /// signed webhook was signed over, to archive byte for byte (`objectPut` takes base64; so does a text column).
    /// Routes with a body only; one field per schema.
    RawBody,
}

/// An image type `imageSniff` accepts. There is no SVG: it can carry a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ImageKind {
    Png,
    Jpeg,
}

impl ImageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ImageKind::Png => "png",
            ImageKind::Jpeg => "jpeg",
        }
    }
}

/// A rule on a text field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "rule", rename_all = "camelCase")]
pub enum Rule {
    /// Between `min` and `max` **characters** (not bytes), once trimmed. The upper bound is what stops a huge
    /// password from reaching argon2.
    Length { min: u32, max: u32 },
    /// Looks like an e-mail address: `local@domain.tld`, 254 characters at most.
    Email,
    /// A French company number: 9 digits, with a valid Luhn check digit.
    Siren,
}

impl Rule {
    /// The same kind of rule, whatever its parameters: a field has each kind once.
    pub fn kind(&self) -> &'static str {
        match self {
            Rule::Length { .. } => "length",
            Rule::Email => "email",
            Rule::Siren => "siren",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FieldType {
    String,
    Bool,
    Int32,
    Int64,
    Float,
    Double,
    Bytes,
    Timestamp,
    /// A UUID in its text form: a `string` on the wire and in the generated Rust, but a type of its own
    /// in the editor, which checks the literals. Paths convert to and from `string` freely.
    Uuid,
    Message {
        name: String,
    },
}

impl FieldType {
    pub fn is_message(&self) -> bool {
        matches!(self, FieldType::Message { .. })
    }

    /// Whether a value of type `from` can fill a `to`: the same type, or `uuid` and `string`, which are both text.
    pub fn assignable_from(&self, from: &FieldType) -> bool {
        self == from
            || matches!(
                (self, from),
                (FieldType::Uuid, FieldType::String) | (FieldType::String, FieldType::Uuid)
            )
    }
}

/// A Kafka topic carrying exactly one message type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Topic {
    pub name: String,
    pub message: String,
    #[serde(default = "default_partitions")]
    pub partitions: i32,
    #[serde(default = "default_replication")]
    pub replication_factor: i16,
    /// Field of `message` used as the Kafka record key (keeps related events ordered).
    #[serde(default)]
    pub key_field: Option<String>,
    /// Services allowed to publish on this topic: the owners of the event. A Publish step in any
    /// other service is an error.
    #[serde(default)]
    pub producers: Vec<String>,
    #[serde(default)]
    pub position: Position,
}

fn default_partitions() -> i32 {
    3
}

fn default_replication() -> i16 {
    1
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub name: String,
    pub port: u16,
    #[serde(default)]
    pub rpcs: Vec<Rpc>,
    /// Port of the HTTP API (the `routes`). Required as soon as there is a route.
    #[serde(default)]
    pub http_port: Option<u16>,
    #[serde(default)]
    pub routes: Vec<Route>,
    /// The tables of the service's own database.
    #[serde(default)]
    pub tables: Vec<Table>,
    /// Tables removed from the design. The migration drops them, with their rows, from a database that still has them: an
    /// explicit list, like `Table::dropped_columns`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped_tables: Vec<String>,
    #[serde(default)]
    pub consumers: Vec<Consumer>,
    /// Tasks run on a schedule, with no request and no event: purging what expired, for instance.
    #[serde(default)]
    pub jobs: Vec<Job>,
    #[serde(default)]
    pub position: Position,
    /// What the service is. A `mta` is a ready-made mail service: its consumers are the events it mails (each with a
    /// `mail` rule instead of steps), and its `templates` are the mails. Everything else in it is fixed code.
    #[serde(default, skip_serializing_if = "ServiceKind::is_standard")]
    pub kind: ServiceKind,
    /// The mails of a `mta`: one per event it listens to. Seeded into the database by a migration, and editable there
    /// afterwards (the worker reads them on every cycle).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<MailTemplate>,
    /// The extra addresses a `mta` can send from (a rule picks one with `mail.sender`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub senders: Vec<MailSender>,
    /// The settings of a `realtime` service: the channel its events come on, the path of the stream and how the access
    /// token is read. Required for that kind, refused for the others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realtime: Option<RealtimeConfig>,
    /// A proof-of-work challenge the service hands out (`GET <path>`) for its `verifyChallenge` steps to check, against the
    /// robots that fill in a form: see [`ChallengeConfig`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge: Option<ChallengeConfig>,
    /// Compress the answers of the HTTP API (gzip, brotli or zstd, whichever the client's `Accept-Encoding` prefers): JSON
    /// shrinks to a fraction. Only answers of 256 bytes or more are, never an event stream, a gRPC message or an image (already
    /// compressed). `Vary: Accept-Encoding` tells the caches. A service with no route has nothing to compress.
    #[serde(default, skip_serializing_if = "is_false")]
    pub compression: bool,
    /// Describe the service's tables as a GraphQL schema (`contracts/graphql/<service>.graphql`): one type per table, read
    /// queries by key and as paginated connections, and the relations its foreign keys draw. When the service also has routes
    /// and a `verifyJwt` step, it serves `POST /graphql` (read only): the cookie `access_token` is checked like that step does,
    /// the tenant is the claim `tenantId` (or `tenant_id`), errors carry a machine code in `errors[].extensions.code`
    /// (`NON_AUTHENTIFIE` without a valid cookie), depth and complexity are limited (`GRAPHQL_MAX_DEPTH`,
    /// `GRAPHQL_MAX_COMPLEXITY`) and `GRAPHQL_INTROSPECTION=off` turns introspection off. Each `POST`/`PUT`/`PATCH`/`DELETE` route of the service
    /// is also a mutation (`type Mutation { <routeName>(input: JSON): JSON }`): it plays the route over the loopback with the
    /// caller's cookie, so validation, tenant and `{code, message}` errors are the route's; `input` holds the body fields and
    /// the `{params}` of the path.
    #[serde(default, skip_serializing_if = "is_false")]
    pub graphql: bool,
    /// The sources of a GraphQL gateway: a service with `graphql`, no table, routes and a `verifyJwt` step reads other
    /// services' REST APIs instead of a database, and serves them as one graph on `POST /graphql` (see [`GraphqlSource`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graphql_sources: Vec<GraphqlSource>,
    /// The path the GraphQL endpoint is served on (default `/graphql`): it must start with `/`, with no `{param}` and no
    /// other route of the service on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphql_path: Option<String>,
    /// The claim of the token that names the tenant, for the tables with a tenant column (default: `tenantId`, then
    /// `tenant_id`). Set it when the tokens of the project carry another name (for instance `tid`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphql_tenant_claim: Option<String>,
    /// Headers of the client's request copied to every call of a gateway to the services behind (`X-Frontend-ID`, say), by name,
    /// when the client sent them. The cookie `access_token` always goes along.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graphql_forward_headers: Vec<String>,
    /// The query fields of a gateway that take arguments and answer several blocks at once (see [`GraphqlRoot`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graphql_roots: Vec<GraphqlRoot>,
    /// The roles allowed to call a mutation, by route name (the mutation of a route is named after it). The check runs before
    /// the route is played, and a refusal is `ACCES_REFUSE`; a route that is not listed (or listed empty) is open to any
    /// signed-in caller, the route still doing its own checks.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub graphql_mutation_roles: std::collections::BTreeMap<String, Vec<String>>,
    /// Named sequences of steps, written once and played by an `include` step in any handler of the service.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragments: Vec<Fragment>,
}

/// A type of a gateway's graph, read from a REST API of another service. `GET <path>` answers the rows (a JSON array, or an
/// object whose `items` is one); `GET <path>/<key>` answers one. The cookie `access_token` of the caller is sent along, so the
/// service behind applies its own checks and its own tenant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphqlSource {
    /// The GraphQL type (PascalCase), unique in the service.
    pub name: String,
    /// The environment variable holding the base URL of the service behind (never a value of a query).
    pub base_url_env: String,
    /// The path of the collection, starting with `/` (`/articles`).
    pub path: String,
    /// The fields of a row (the JSON properties of the answer): the others are not exposed.
    #[serde(default)]
    pub fields: Vec<GraphqlSourceField>,
    /// The field that identifies a row: it adds the query `<type>(<key>: ID!)`, read at `GET <path>/<key>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Fields that read other sources, a nested query being one call per relation however many rows it has.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<GraphqlRelation>,
    /// Fields computed from the row (or from the rows of a relation), declared rather than coded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub computed: Vec<GraphqlComputed>,
    /// The roles allowed to read this type (claims `roles` / `role` of the JWT); empty: any signed-in caller.
    /// A caller without one of them gets the field `null` and an `ACCES_REFUSE` error at its path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
}

/// A field computed from integers of a row. A missing operand makes the field `null`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphqlComputed {
    /// Name of the field (camelCase), not used by a field or a relation of the source.
    pub name: String,
    #[serde(flatten)]
    pub kind: GraphqlComputedKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GraphqlComputedKind {
    /// The product of integer fields of the row (`Long`).
    Product { fields: Vec<String> },
    /// `numerator / denominator`, two integer fields of the row (`Float`, `null` when the denominator is 0).
    Ratio {
        numerator: String,
        denominator: String,
    },
    /// The sum, over the rows of a `many` relation of the source, of an integer field of the related rows, or of its product
    /// with `times` (`Long`): `valeurStock = sum(quantite * prixCentimes)`.
    Sum {
        relation: String,
        field: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        times: Option<String>,
    },
    /// The whole days from today (UTC) to the date held by `field` (a `string` or `timestamp` starting with `YYYY-MM-DD`):
    /// negative once past, `null` when the field is absent or not a date (`Long`): `joursRestants` of a `dlc`.
    DaysUntil { field: String },
}

/// A query field of a gateway: `<name>(<args>): <Name>!`, an object whose blocks each read a source. A block that fails is
/// `null` with its own error (the code and message of the service behind), the others are answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphqlRoot {
    /// The query field (camelCase), unique among the roots and the queries of the sources.
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<GraphqlArg>,
    /// Read before anything else: a refusal or a `404` of this route stops the whole request, with the code of the answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<GraphqlGuard>,
    #[serde(default)]
    pub blocks: Vec<GraphqlBlock>,
    /// Totals over the rows of a `many` block of the root (`Long`): the root type gets one field per entry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sums: Vec<GraphqlRootSum>,
}

/// `<name> = sum(field [* times])` over the rows of the block `block` of a root. Operands are numeric fields of the block's
/// source; a `double` operand makes the term rounded to the nearest integer (a row without the operand counts for nothing).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphqlRootSum {
    /// Name of the field (camelCase), not used by a block of the root.
    pub name: String,
    /// The `field` of a block of the root, which must be `many`.
    pub block: String,
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub times: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphqlArg {
    /// camelCase.
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ColumnType,
    /// Without a default, the argument is `Type!`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// `GET <path>` on the service of `source`, `{arg}` standing for the value of an argument of the root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphqlGuard {
    /// The `name` of the source whose base URL is used.
    pub source: String,
    pub path: String,
}

/// A field of a root that reads `source` (`GET <path>?<param>=<value>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphqlBlock {
    /// Name of the field (camelCase).
    pub field: String,
    pub source: String,
    /// A list of rows (`[Source!]`) rather than the first one (`Source`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub many: bool,
    /// Query parameters of the call: parameter name -> argument of the root that gives its value.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub params: std::collections::BTreeMap<String, String>,
    /// Query parameters with a constant value (`archive` -> `tous`); an argument of `params` wins on the same name.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub fixed_params: std::collections::BTreeMap<String, String>,
    /// Path of the call instead of the source's own, `{arg}` standing for an argument of the root (`/entrepots/{id}/capacite`).
    /// The answer is then one object (`Source`, with `many` false) rather than a list of rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphqlSourceField {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ColumnType,
    #[serde(default, skip_serializing_if = "is_false")]
    pub nullable: bool,
    /// For a `string` field: upstream value -> name of the GraphQL enumeration value (`entree` -> `ENTREE`). The field then
    /// has the enumeration type `<Source><Field>`; a value the map lacks is an error of the field.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub values: std::collections::BTreeMap<String, String>,
}

/// A field of a source that reads another source: the rows of `target` whose `target_field` equals the `via` field of the row.
/// The lookups of all the rows of a response are grouped (a dataloader): one call `GET <target path>?<target_field>=<value>`
/// per row, or, with `batch_param`, a single `GET <target path>?<batch_param>=<v1>,<v2>,...` for the whole batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphqlRelation {
    /// Name of the field (camelCase).
    pub field: String,
    /// The `name` of the source it reads.
    pub target: String,
    /// Field of this source holding the value to look for.
    pub via: String,
    /// Field of the target that must equal it.
    pub target_field: String,
    /// A list of rows (`[Target!]!`) rather than at most one (`Target`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub many: bool,
    /// Query parameter of the target taking a comma-separated list of values: one call for the whole batch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_param: Option<String>,
    /// Query parameters with a constant value added to every call of the relation (`archive` -> `tous`). The filter on
    /// `targetField` (or `batchParam`) wins on the same name.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub fixed_params: std::collections::BTreeMap<String, String>,
}

/// A named sequence of steps shared by the handlers of a service (verify the token, require a tenant, build the event
/// envelope...). An `include` step stands for exactly these steps, in place: what they bind is visible after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Fragment {
    /// Lower case letters, digits and `_`, starting with a letter; unique in the service.
    pub name: String,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ServiceKind {
    /// Built from the rpcs, routes, consumers and jobs the author draws.
    #[default]
    Standard,
    /// A ready-made mail transfer agent: queue, worker, DKIM, SMTP delivery, bounces, health and metrics.
    Mta,
    /// A ready-made service that pushes events to browsers over SSE (server-sent events): the API publishes an event on a
    /// channel of DragonflyDB (or Valkey, or Redis) and this service hands it to the browsers it is for. Built to hold
    /// tens of thousands of open streams on one instance in a bounded amount of memory.
    Realtime,
}

impl ServiceKind {
    pub fn is_standard(&self) -> bool {
        *self == ServiceKind::Standard
    }
}

/// What the project decides about a `realtime` service. The rest of it (the index of the streams, the limits, the sessions,
/// the health and the metrics) is fixed code, set by the environment of the deployment.
///
/// The access token is a JWT signed with EdDSA, whose public key is read from the `JWKS_URL` of the environment. Its claims
/// are `sub` (the account), `sid` (the session), `exp`, and optionally `tid` (the company) and `permissions` (the rights of
/// the account, `module.view` or `module.manage`). A system with no companies leaves `tid` out; one with no permissions
/// leaves `permissions` out and addresses accounts.
///
/// The events are published by the API on the channel as JSON: `{"type": "contact.updated", "tenantId": "…",
/// "permission": "contacts.view", "data": {"id": "…"}}`. An event is for the streams that have a permission (`permission`),
/// for named accounts (`accountIds`), or for every stream of an account (`accountId`). `tenantId` narrows the first two to
/// one company; without it they are for the streams that have no company, and a scope never leaks into the other. The
/// service does not know what a type means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeConfig {
    /// The pub/sub channel the events are published on.
    #[serde(default = "default_realtime_channel")]
    pub channel: String,
    /// The path the browsers open the stream on (`GET`).
    #[serde(default = "default_realtime_path")]
    pub path: String,
    /// The cookie that holds the access token.
    #[serde(default = "default_realtime_cookie")]
    pub access_cookie: String,
    /// The issuer (`iss`) the access tokens must have.
    #[serde(default = "default_realtime_issuer")]
    pub issuer: String,
    /// The prefix of the keys where the API leaves the trace of each session: `<prefix>:<session id>` holds
    /// `{"accountId", "tenantId", "exp"}` and expires with the access token it was written for. A stream lives while its
    /// session does.
    #[serde(default = "default_realtime_session_prefix")]
    pub session_key_prefix: String,
}

fn default_realtime_channel() -> String {
    "live-events".into()
}
fn default_realtime_path() -> String {
    "/v1/events".into()
}
fn default_realtime_cookie() -> String {
    "access_token".into()
}
fn default_realtime_issuer() -> String {
    "api".into()
}
fn default_realtime_session_prefix() -> String {
    "sess".into()
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            channel: default_realtime_channel(),
            path: default_realtime_path(),
            access_cookie: default_realtime_cookie(),
            issuer: default_realtime_issuer(),
            session_key_prefix: default_realtime_session_prefix(),
        }
    }
}

/// One mail of a `mta`, rendered with [Tera](https://keats.github.io/tera/) from the fields of the event (their names
/// as declared in the schema): `{{ name }}`. The HTML is escaped, the subject and the text are not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MailTemplate {
    /// The topic it is the mail of.
    pub event_type: String,
    pub subject: String,
    /// The HTML written by hand. Ignored when the mail has a `design`.
    #[serde(default)]
    pub html: String,
    /// The plain text written by hand. Ignored when the mail has a `design`.
    #[serde(default)]
    pub text: String,
    /// The mail composed as blocks: its HTML and its text are derived from it (see [`MailTemplate::bodies`]). Absent for a
    /// mail written by hand, which keeps working as it always did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design: Option<crate::maildesign::MailDesign>,
}

impl MailTemplate {
    /// The HTML and the plain text the mail is sent with: derived from the design when there is one, as written otherwise.
    pub fn bodies(&self) -> (String, String) {
        match &self.design {
            Some(d) => (
                crate::maildesign::render_html(d),
                crate::maildesign::render_text(d),
            ),
            None => (self.html.clone(), self.text.clone()),
        }
    }
}

/// What a `mta` does with an event it listens to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MailRule {
    /// The text field of the event that holds the recipient's address.
    pub recipient: String,
    /// An event without an address is skipped instead of being an error.
    #[serde(default)]
    pub optional_recipient: bool,
    /// The text field that identifies the event: the same value is mailed once, whatever Kafka redelivers.
    pub dedup_field: String,
    /// What identifies it when that field is empty.
    #[serde(default)]
    pub dedup_fallback: DedupFallback,
    /// A text field with a company name: the sender shows as `<company> via <SMTP_FROM_NAME>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_company_via: Option<String>,
    /// A text field with the address replies go to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// The `id` of one of the service's `senders`: the mail leaves from that address and is signed with its key. Without
    /// it, the service's own address (`SMTP_FROM_EMAIL`) is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
}

/// An address a `mta` can send from besides its own, with its own DKIM identity. The private key is not in the project:
/// it is read from `SENDER_<ID>_DKIM_PRIVATE_KEY_PATH` (or `..._PEM`) when the service starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MailSender {
    /// Name a rule refers to (`[a-z][a-z0-9_]*`).
    pub id: String,
    /// The `From` address, e.g. `achats@mon-entreprise.fr`.
    pub email: String,
    /// The display name (the service's `SMTP_FROM_NAME` when empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// DKIM selector published in the DNS of `dkimDomain`.
    pub dkim_selector: String,
    /// DKIM signing domain: the domain of `email`, for the mail to be aligned (DMARC).
    pub dkim_domain: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DedupFallback {
    /// One mail per address.
    Recipient,
    /// One mail per topic and address.
    #[default]
    TopicAndRecipient,
    /// The event is refused: no mail without an identity.
    Reject,
}

/// A task a service runs every `every_secs` seconds, from its start-up and in the transaction of its steps. It has
/// no `input`, and no response to give. Every instance of the service runs it: its steps should be safe to repeat
/// (a `DELETE` of what expired is).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub name: String,
    pub every_secs: u32,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Rpc {
    pub name: String,
    pub request: String,
    pub response: String,
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// A column type. `Timestamp` and `Uuid` are stored natively but read and written as text
/// (RFC 3339, canonical UUID) by the flows, so that they are plain strings everywhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ColumnType {
    String,
    Bool,
    Int32,
    Int64,
    Float,
    Double,
    Timestamp,
    Uuid,
    Jsonb,
}

impl ColumnType {
    pub const ALL: [ColumnType; 9] = [
        ColumnType::String,
        ColumnType::Bool,
        ColumnType::Int32,
        ColumnType::Int64,
        ColumnType::Float,
        ColumnType::Double,
        ColumnType::Timestamp,
        ColumnType::Uuid,
        ColumnType::Jsonb,
    ];

    /// The PostgreSQL type.
    pub fn sql(self) -> &'static str {
        match self {
            ColumnType::String => "TEXT",
            ColumnType::Bool => "BOOLEAN",
            ColumnType::Int32 => "INTEGER",
            ColumnType::Int64 => "BIGINT",
            ColumnType::Float => "REAL",
            ColumnType::Double => "DOUBLE PRECISION",
            ColumnType::Timestamp => "TIMESTAMPTZ",
            ColumnType::Uuid => "UUID",
            ColumnType::Jsonb => "JSONB",
        }
    }

    fn is_numeric(self) -> bool {
        matches!(
            self,
            ColumnType::Int32 | ColumnType::Int64 | ColumnType::Float | ColumnType::Double
        )
    }

    /// Whether a column of this type can be converted to `to` by `USING col::to`, with the meaning kept: numbers among
    /// themselves, `bool` and `int32`, anything to text, and text to anything (which fails, with nothing changed, on a
    /// value that is not of the target type). Other pairs (a uuid to a number, a timestamp to a bool) have no sound cast.
    pub fn converts_to(self, to: ColumnType) -> bool {
        self != to
            && (self.is_numeric() && to.is_numeric()
                || to == ColumnType::String
                || self == ColumnType::String
                || matches!(
                    (self, to),
                    (ColumnType::Bool, ColumnType::Int32) | (ColumnType::Int32, ColumnType::Bool)
                ))
    }

    /// Whether the conversion to `to` can fail or lose precision on existing values.
    pub fn conversion_is_lossy(self, to: ColumnType) -> bool {
        self == ColumnType::String && to != ColumnType::String
            || matches!(
                (self, to),
                (ColumnType::Int64, ColumnType::Int32)
                    | (ColumnType::Double, ColumnType::Float)
                    | (
                        ColumnType::Float | ColumnType::Double,
                        ColumnType::Int32 | ColumnType::Int64
                    )
                    | (ColumnType::Int64, ColumnType::Float)
                    | (ColumnType::Int32 | ColumnType::Int64, ColumnType::Bool)
            )
    }

    /// The type a flow sees.
    pub fn field_type(self) -> FieldType {
        match self {
            ColumnType::String | ColumnType::Timestamp | ColumnType::Jsonb => FieldType::String,
            ColumnType::Uuid => FieldType::Uuid,
            ColumnType::Bool => FieldType::Bool,
            ColumnType::Int32 => FieldType::Int32,
            ColumnType::Int64 => FieldType::Int64,
            ColumnType::Float => FieldType::Float,
            ColumnType::Double => FieldType::Double,
        }
    }

    /// Whether the database type differs from the text a flow reads and writes (it needs a cast).
    /// A `jsonb` column is the JSON document as text.
    pub fn is_text_cast(self) -> bool {
        matches!(
            self,
            ColumnType::Timestamp | ColumnType::Uuid | ColumnType::Jsonb
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ColumnType,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub primary_key: bool,
    /// SQL expression used when an insert does not give the column a value (`now()`, `0`, `'new'`).
    #[serde(default)]
    pub default: Option<String>,
    /// The names this column had before, oldest first. The snapshot of the schema has no history, so a rename is
    /// recorded here: the migration renames the column of a database that still has an old name (and only then), which
    /// keeps its data. A name stays listed for as long as a database may still carry it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renamed_from: Vec<String>,
    /// The type before a change of type. The migration converts the column (`USING col::type`) when the database does not
    /// have the current type yet; the conversion must be a sound one (`E_RETYPE_UNSUPPORTED`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retyped_from: Option<ColumnType>,
    /// Whether the column was nullable before a change of nullability. Making a column required fills its nulls with the
    /// default first (and fails when there is none and a row holds a null: nothing is lost, the start-up stops).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nullable_from: Option<bool>,
}

impl Column {
    pub fn new(name: impl Into<String>, ty: ColumnType) -> Self {
        Self {
            name: name.into(),
            ty,
            nullable: false,
            primary_key: false,
            default: None,
            renamed_from: vec![],
            retyped_from: None,
            nullable_from: None,
        }
    }

    /// An insert must give a value to the columns that cannot be left out.
    pub fn is_required_on_insert(&self) -> bool {
        !self.nullable && self.default.as_deref().is_none_or(|d| d.trim().is_empty())
    }
}

/// A table of the database of a service. Identifiers are snake_case, so they can be quoted safely.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    pub name: String,
    #[serde(default)]
    pub columns: Vec<Column>,
    /// `UNIQUE (a, b)`: the columns together identify at most one row.
    #[serde(default)]
    pub uniques: Vec<UniqueConstraint>,
    #[serde(default)]
    pub indexes: Vec<Index>,
    /// Foreign keys, composite when they name several columns.
    #[serde(default)]
    pub foreign_keys: Vec<ForeignKey>,
    /// The column that says which tenant a row belongs to. Every SQL step on the table is filtered by it and every insert
    /// sets it, from the variable of the same name that the handler has bound (for example from a verified token): an
    /// author cannot forget it, and cannot read or write another tenant's rows with these steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_column: Option<String>,
    /// Rows are never removed: a `delete` sets `deleted_at` instead, and `select`, `update` and `delete` skip the rows that
    /// have it. The table needs a nullable timestamp column `deleted_at`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub soft_delete: bool,
    /// The names this table had before, oldest first (see `Column::renamed_from`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renamed_from: Vec<String>,
    /// Columns removed from the design. The migration drops them from a database that still has them, with their data:
    /// this is destructive, so it is an explicit list and not the mere absence of the column.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped_columns: Vec<String>,
    /// The roles allowed to read this table through the GraphQL endpoint (the token's claim `roles`, a list, or `role`, a
    /// string). Empty: any signed-in caller. The check runs before any row is read, for a root query as well as for a
    /// relation that reaches the table, and a refusal is `ACCES_REFUSE`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graphql_roles: Vec<String>,
}

/// The column a soft delete sets.
pub const DELETED_AT: &str = "deleted_at";

impl Table {
    pub fn new(name: impl Into<String>, columns: Vec<Column>) -> Self {
        Self {
            name: name.into(),
            columns,
            uniques: vec![],
            indexes: vec![],
            foreign_keys: vec![],
            tenant_column: None,
            soft_delete: false,
            renamed_from: vec![],
            dropped_columns: vec![],
            graphql_roles: vec![],
        }
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// The primary key, in the order of the columns.
    pub fn primary_key(&self) -> Vec<&str> {
        self.columns
            .iter()
            .filter(|c| c.primary_key)
            .map(|c| c.name.as_str())
            .collect()
    }

    /// Whether `columns` (in any order) are guaranteed unique: the primary key, a `UNIQUE`
    /// constraint or a unique index made of exactly these columns. A foreign key may only
    /// reference such a set.
    pub fn is_unique_key(&self, columns: &[String]) -> bool {
        let same = |other: &[&str]| {
            other.len() == columns.len() && columns.iter().all(|c| other.contains(&c.as_str()))
        };
        same(&self.primary_key())
            || self
                .uniques
                .iter()
                .map(|u| &u.columns)
                .chain(self.indexes.iter().filter(|i| i.unique).map(|i| &i.columns))
                .any(|cols| same(&cols.iter().map(String::as_str).collect::<Vec<_>>()))
    }
}

/// A named `UNIQUE` constraint. Postgres backs it with an index of the same name, so the name
/// is shared with the indexes of the service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UniqueConstraint {
    pub name: String,
    pub columns: Vec<String>,
}

/// A secondary index, optionally unique, on one or several columns (the order matters).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    pub name: String,
    pub columns: Vec<String>,
    #[serde(default)]
    pub unique: bool,
}

/// `FOREIGN KEY (columns) REFERENCES ref_table (ref_columns)`, within the database of the
/// service. `columns[i]` points at `ref_columns[i]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    #[serde(default)]
    pub on_delete: ReferentialAction,
    #[serde(default)]
    pub on_update: ReferentialAction,
}

/// What happens to the rows holding a foreign key when the row they point at is deleted or updated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ReferentialAction {
    #[default]
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ReferentialAction {
    pub const ALL: [ReferentialAction; 5] = [
        ReferentialAction::NoAction,
        ReferentialAction::Restrict,
        ReferentialAction::Cascade,
        ReferentialAction::SetNull,
        ReferentialAction::SetDefault,
    ];

    pub fn sql(self) -> &'static str {
        match self {
            ReferentialAction::NoAction => "NO ACTION",
            ReferentialAction::Restrict => "RESTRICT",
            ReferentialAction::Cascade => "CASCADE",
            ReferentialAction::SetNull => "SET NULL",
            ReferentialAction::SetDefault => "SET DEFAULT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    pub const ALL: [HttpMethod; 5] = [
        HttpMethod::Get,
        HttpMethod::Post,
        HttpMethod::Put,
        HttpMethod::Patch,
        HttpMethod::Delete,
    ];

    /// Upper case, as written in a request line.
    pub fn as_str(self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Delete => "DELETE",
        }
    }

    /// Whether the request schema travels in a JSON body (otherwise in the query string).
    pub fn has_body(self) -> bool {
        matches!(self, HttpMethod::Post | HttpMethod::Put | HttpMethod::Patch)
    }
}

/// An HTTP endpoint of a service, for the front. `path` can hold `{params}` that fill the
/// fields of the `request` schema with the same name; the other fields come from the JSON body
/// (POST, PUT, PATCH) or from the query string (GET, DELETE). Both schemas are API schemas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    /// Name of the handler (snake_case): the function generated in `flows::api`.
    pub name: String,
    pub method: HttpMethod,
    pub path: String,
    pub request: String,
    pub response: String,
    /// Success status code (200, 201, 202 or 204: no content, the response is not sent).
    #[serde(default = "default_status")]
    pub status: u16,
    /// How the response is written: JSON (default), or CSV for a route whose response schema is one list of
    /// messages (`text/csv`, one column per scalar field of the item, RFC 4180).
    #[serde(default, skip_serializing_if = "is_default_format")]
    pub response_format: BodyFormat,
    /// How the body is read: JSON (default), or CSV for a `POST`/`PUT`/`PATCH` whose request schema is one list
    /// of messages (`text/csv`, first line = header, one item per following line).
    #[serde(default, skip_serializing_if = "is_default_format")]
    pub body_format: BodyFormat,
    /// What a CSV body does with a line that is refused when it is read (wrong cell count, a cell that is not of its
    /// column's type): `allOrNothing` (default) refuses the whole file with a 400; `partial` keeps the valid lines and
    /// fills the `rejected` list of the response (`{ line, column, message }`) with the others.
    #[serde(default, skip_serializing_if = "is_default_csv_mode")]
    pub csv_mode: CsvMode,
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// How a CSV body treats the lines that are refused when it is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CsvMode {
    #[default]
    AllOrNothing,
    Partial,
}

fn is_default_csv_mode(m: &CsvMode) -> bool {
    *m == CsvMode::AllOrNothing
}

/// The format of an HTTP body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum BodyFormat {
    #[default]
    Json,
    Csv,
}

fn is_default_format(f: &BodyFormat) -> bool {
    *f == BodyFormat::Json
}

/// Largest CSV body a route reads, in bytes.
pub const CSV_MAX_BODY_BYTES: usize = 5 * 1024 * 1024;

/// The list that a CSV body or answer is made of: the one repeated message field of `message` that is not filled
/// by the path (`params`) or a header, and the item it holds. A CSV line is flat, so the item has only single
/// scalar fields.
pub fn csv_list<'a>(
    project: &'a Project,
    message: &'a Message,
    params: &[String],
) -> Result<(&'a Field, &'a Message), String> {
    let lists: Vec<&Field> = message
        .fields
        .iter()
        .filter(|f| f.repeated && f.source.is_none() && !params.contains(&f.name))
        .collect();
    let [list] = lists[..] else {
        return Err("it needs exactly one list field (the lines of the file)".into());
    };
    let FieldType::Message { name } = &list.ty else {
        return Err(format!(
            "the list `{}` must hold a schema (one per line), not single values",
            list.name
        ));
    };
    let Some(item) = project.message(name) else {
        return Err(format!("the list `{}` holds an unknown schema", list.name));
    };
    if let Some(f) = item.fields.iter().find(|f| f.repeated || f.ty.is_message()) {
        return Err(format!(
            "`{}` has a list or a nested schema (`{}`): a line of CSV is flat",
            item.name, f.name
        ));
    }
    if item.fields.is_empty() {
        return Err(format!(
            "`{}` has no field: nothing to write on a line",
            item.name
        ));
    }
    Ok((list, item))
}

/// Clock tolerance of a signed webhook: Stripe's own default.
fn default_tolerance_secs() -> u32 {
    300
}

fn default_status() -> u16 {
    200
}

/// First port tried for the HTTP API of a service.
pub const DEFAULT_HTTP_PORT: u16 = 8080;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Consumer {
    pub name: String,
    pub topic: String,
    /// Kafka consumer group. Defaults to `<service>-<consumer>`.
    #[serde(default)]
    pub group_id: Option<String>,
    /// Kafka delivers at least once: the same event can arrive twice. With this field of the event, which
    /// identifies it, an event already handled is skipped: the check and the effects of the first handling
    /// are in the same transaction.
    #[serde(default)]
    pub idempotent_on: Option<String>,
    /// In a `mta`: the mail this event sends, instead of steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail: Option<MailRule>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

impl Service {
    /// Whether a consumer skips the events it already handled, which needs the `processed_events` table.
    pub fn has_idempotent_consumers(&self) -> bool {
        self.consumers.iter().any(|c| c.idempotent_on.is_some())
    }
}

/// One block of a handler, the equivalent of an Elementor widget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Step {
    /// Emit a structured log line. `{input.field}` placeholders are replaced by values.
    #[serde(rename_all = "camelCase")]
    Log {
        id: String,
        level: LogLevel,
        message: String,
    },
    /// Bind a scalar variable.
    #[serde(rename_all = "camelCase")]
    Let {
        id: String,
        name: String,
        #[serde(rename = "valueType")]
        value_type: FieldType,
        /// `Option<T>`: the value may be absent (`null` as a literal).
        #[serde(default)]
        optional: bool,
        /// `Vec<T>`: a list (a JSON array as a literal).
        #[serde(default)]
        repeated: bool,
        /// A later `assign` step (in this list, a branch or a loop) may give it another value.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        mutable: bool,
        value: Expr,
    },
    /// Give a new value to a variable declared with `mutable` on its `let` step, wherever the step sits
    /// below it (a branch, a loop).
    #[serde(rename_all = "camelCase")]
    Assign {
        id: String,
        name: String,
        value: Expr,
    },
    /// Publish an event on a topic.
    #[serde(rename_all = "camelCase")]
    Publish {
        id: String,
        topic: String,
        #[serde(default)]
        payload: Vec<Assignment>,
    },
    /// Pushes an event to the browsers through a real-time service: a JSON message on its channel, handed to the open
    /// streams it is for. Sent once the flow has committed (never for a run that fails), best effort: the database is
    /// the truth and the browser reads its state again, so a lost event never fails the request.
    ///
    /// The audience is explicit, as the real-time service requires: `accountId` (every stream of that account), or
    /// `permission` (`module.view` or `module.manage`) and/or `accountIds`, narrowed to one company by `tenantId`
    /// (without it, the streams that have no company). `data` holds a few identifiers, never content.
    #[serde(rename_all = "camelCase")]
    PublishLive {
        id: String,
        /// The real-time service whose channel the event goes to.
        service: String,
        /// `domain.action`, written as is on the `event:` line of the stream.
        event: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<Expr>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<Expr>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permission: Option<String>,
        /// A list of account ids (`Vec<String>`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_ids: Option<Expr>,
        /// The tab that made the change (the `X-Client-Id` header): it does not get the event back.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<Expr>,
        /// Up to 4 entries: the field is the key, the value a text of 64 characters at most.
        #[serde(default)]
        data: Vec<Assignment>,
    },
    /// Tells a real-time service that the rights of an account changed (its `grants.updated` control message). With
    /// `permissions` (a list of `module.view|manage`), the open streams of that account in that company adopt them and
    /// the browser reads its state again; without, those streams close (the account left the company). Sent once the
    /// flow has committed, best effort, like `publishLive`.
    #[serde(rename_all = "camelCase")]
    LiveGrants {
        id: String,
        /// The real-time service whose channel the message goes to.
        service: String,
        account_id: Expr,
        /// The company; without it, the streams that have none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<Expr>,
        /// A list of `module.view|manage` (`Vec<String>`); absent: the streams close.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permissions: Option<Expr>,
    },
    /// Leaves the trace of a session where a real-time service reads it: `<sessionKeyPrefix>:<sessionId>` =
    /// `{accountId, tenantId, exp}`, with a lifetime of `ttlSeconds` (the one of the access token it goes with). A stream
    /// of the real-time service lives while that trace does. The prefix is the one of the service's settings, so both
    /// sides cannot drift. Written once the flow has committed, best effort, like `publishLive`.
    #[serde(rename_all = "camelCase")]
    SessionTrace {
        id: String,
        /// The real-time service that reads the trace.
        service: String,
        session_id: Expr,
        account_id: Expr,
        /// The company; absent in a system without companies.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_id: Option<Expr>,
        /// Lifetime of the trace, in seconds (an integer, the one of the access token).
        ttl_seconds: Expr,
    },
    /// Deletes the trace of a session (logout, refresh token revoked): its streams close at once instead of at their next
    /// deadline. Written once the flow has committed, best effort.
    #[serde(rename_all = "camelCase")]
    SessionRevoke {
        id: String,
        /// The real-time service that reads the trace.
        service: String,
        session_id: Expr,
    },
    /// Call an RPC of another service.
    #[serde(rename_all = "camelCase")]
    CallRpc {
        id: String,
        service: String,
        method: String,
        #[serde(default)]
        request: Vec<Assignment>,
        /// Variable receiving the response message.
        #[serde(default)]
        bind: Option<String>,
        /// Cut the calls to this service while it fails: see [`CircuitConfig`]. Shared by every step that calls it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        circuit: Option<CircuitConfig>,
        /// At most this many calls of this step at the same time (a bulkhead): the next one is refused at once with a 503, not
        /// queued behind a slow service.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_concurrent: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Branch {
        id: String,
        condition: Condition,
        #[serde(default)]
        then: Vec<Step>,
        #[serde(default)]
        otherwise: Vec<Step>,
    },
    /// Return the RPC or route response. An HTTP route can also pick its success `status` and set `cookies`.
    #[serde(rename_all = "camelCase")]
    Respond {
        id: String,
        #[serde(default)]
        fields: Vec<Assignment>,
        /// Routes: the success status (200, 201 or 202), instead of the route's own.
        #[serde(default)]
        status: Option<u16>,
        /// Routes: `Set-Cookie` headers.
        #[serde(default)]
        cookies: Vec<Cookie>,
        /// Routes: free response headers (`Cache-Control`, `Location`, `X-Total-Count`...), their value read from the flow.
        #[serde(default)]
        headers: Vec<ResponseHeader>,
    },
    /// Counts this attempt under `key` and, past `max` attempts in the last `window_secs`, ends the route
    /// with a 429 and a `Retry-After`. A sliding window per key, kept in the memory of the process, or, with
    /// `shared`, in the key-value server of the project, so that the limit is the same whatever the number of replicas.
    #[serde(rename_all = "camelCase")]
    RateLimit {
        id: String,
        key: Expr,
        max: u32,
        window_secs: u32,
        /// Count in DragonflyDB (or Valkey, Redis: `DRAGONFLY_URL`) instead of in the memory of each instance. When the server
        /// cannot be reached the step counts in memory, as it would without this option: it never refuses a request for that.
        #[serde(default)]
        shared: bool,
    },
    /// Checks the solution of a proof-of-work challenge (see [`ChallengeConfig`]) before the rest of the route runs: the form
    /// the visitor filled in is read for what it is only once a browser has spent a moment computing. `payload` is what the
    /// ALTCHA widget sends (a text: base64 of a JSON). A solution that is missing, malformed, wrong, expired or already used
    /// ends the route with a 400. A solution is accepted once, for all the replicas, in the transaction of the handler.
    /// Routes only, in a service that serves a challenge.
    #[serde(rename_all = "camelCase")]
    VerifyChallenge { id: String, payload: Expr },
    /// Asks whether a **feature flag** is on for a subject (an account, a company): the way to ship code dark, turn it on for a few
    /// and then for all, and turn it off again, **without deploying**. The flag lives in the table `feature_flags` of the service's
    /// database, which an operator changes with one statement (`UPDATE feature_flags SET rollout_percent = 25 WHERE key = 'x'`); the
    /// step only says what it starts from when the table has no row for the key yet (`enabled`, `rollout`: the row is then
    /// written, and never overwritten by a deployment). A flag is **on** for a subject when it is enabled, every one of its
    /// `rules` matches the `properties` the step gives (`plan` in `pro`, `team`...), and the subject falls in the share of the
    /// rollout: the subject and the key are hashed to a number from 0 to 99, so **the same subject always gets the same
    /// answer**, and raising the percentage only adds subjects, it never takes one away. With `variants` in the row (`blue` 50,
    /// `red` 50) the subjects that are on get one of them, as stably. `bind` receives the answer (a bool), `variantBind` the
    /// variant (a text; empty when the flag is off or has none).
    #[serde(rename_all = "camelCase")]
    Flag {
        id: String,
        /// The flag: `new_checkout`, `beta.search`.
        key: String,
        /// Who is asked about (a text): what the rollout is stable for.
        subject: Expr,
        /// What the rules of the flag can look at: `plan = "pro"`, `country = "FR"`. Plain values, named.
        #[serde(default)]
        properties: Vec<Assignment>,
        /// Whether the flag is on when the table has no row for it yet (it is written then, so it can be changed).
        #[serde(default)]
        enabled: bool,
        /// The share of subjects it is on for when the table has no row yet: 0 to 100.
        #[serde(default = "default_flag_rollout")]
        rollout: u8,
        bind: String,
        #[serde(default)]
        variant_bind: Option<String>,
    },
    /// Writes what happened to the audit journal: who (`actor`) did what (`action`, `invoice.issued`) to what (`targetType` and
    /// `target`), for which company (`tenant`), with the values `before` and `after` the change. A row of the table
    /// `audit_events` of the service's database, written **in the transaction of the handler**: the change and its trace commit
    /// together, or neither does, so the journal never tells of something that did not happen nor misses something that did. The
    /// table is **append-only**: a trigger refuses an update and a delete. The values are plain (text, number, bool, uuid), named
    /// (`status = "draft"`); the fields whose value is not the same before and after are kept apart (`changed`).
    #[serde(rename_all = "camelCase")]
    Audit {
        id: String,
        /// What was done: `domain.action`, lowercase letters, digits and `_.-` (`invoice.issued`, `member.removed`).
        action: String,
        /// The kind of thing it was done to: `invoice`, `member` (the same spelling everywhere).
        target_type: String,
        /// Which one: its identifier (a text).
        target: Expr,
        /// Who did it: the account, the API key, the job (a text).
        actor: Expr,
        /// For which company; left out, the entry is not any company's.
        #[serde(default)]
        tenant: Option<Expr>,
        /// The values before the change, one `name = value` each.
        #[serde(default)]
        before: Vec<Assignment>,
        /// The values after the change, one `name = value` each.
        #[serde(default)]
        after: Vec<Assignment>,
    },
    /// Reads the journal of a company, **newest first**, a page at a time. `tenant` is required: a company reads its own entries and
    /// no one else's. Filtered by `targetType`, `target` and `action` when given. `bind` receives the entries, a list of the API
    /// schema `AuditEntry` (`id`, `at`, `actor`, `action`, `target_type`, `target_id`, `before`, `after`: texts, `before` and `after`
    /// being JSON, and `changed`: a list of texts, and `id` a number); `nextCursorBind` the `cursor` of the next page (an optional
    /// int64, absent on the last one): to be given back as `cursor` to read the entries older than the last of this page.
    #[serde(rename_all = "camelCase")]
    AuditRead {
        id: String,
        tenant: Expr,
        #[serde(default)]
        target_type: Option<Expr>,
        #[serde(default)]
        target: Option<Expr>,
        #[serde(default)]
        action: Option<Expr>,
        /// The `id` of the last entry already read (an `int64`); left out, the newest.
        #[serde(default)]
        cursor: Option<Expr>,
        /// Entries in a page: 1 to 200, 50 by default.
        #[serde(default = "default_audit_page")]
        limit: u32,
        bind: String,
        #[serde(default)]
        next_cursor_bind: Option<String>,
    },
    /// The next number of a sequence **without gaps**, for what the law asks to be numbered one after the other (invoices, credit
    /// notes): 1, 2, 3... In the table `number_sequences` of the service's database, in the transaction of the handler: the row of
    /// the sequence is locked until the transaction ends, so two runs cannot get the same number, and a run that fails gives its
    /// number back (the next one gets it), which a database sequence (`SERIAL`) never does. The price: runs that number the same
    /// sequence and scope wait for one another, so number at the **end** of the run, right before the write. `scope` makes one
    /// sequence per value (a company, a year: `2026`): each counts from `start`. `bind` receives the number (an `int64`).
    #[serde(rename_all = "camelCase")]
    NextNumber {
        id: String,
        /// The sequence: `invoices`, `credit_notes`.
        sequence: String,
        /// What the numbering is kept apart by (a text: the company, the year); left out, one for the whole service.
        #[serde(default)]
        scope: Option<Expr>,
        /// The first number of a sequence (1 by default).
        #[serde(default = "default_sequence_start")]
        start: i64,
        bind: String,
    },
    /// Records a use of a meter for a tenant: `amount` units of `key` (API calls, seats, megabytes) in the table
    /// `usage_events` of the service's database, in the transaction of the handler, so that the use and what it pays for commit
    /// together. A negative amount gives units back (a seat released). With `idempotency_key`, a second use with the same text
    /// is ignored, so that a retried request or a redelivered event counts once.
    #[serde(rename_all = "camelCase")]
    Meter {
        id: String,
        /// The meter: `api_calls`, `seats`, `storage_mb`.
        key: String,
        /// Who is metered, as text: a company, an account.
        tenant: Expr,
        /// How many units (an `int64`).
        amount: Expr,
        #[serde(default)]
        idempotency_key: Option<Expr>,
    },
    /// Refuses the request, with a 429 and a `Retry-After` (the seconds until the period ends), when the tenant's use of the
    /// meter over the period, plus `wanted`, would pass `limit`. The limit comes from the plan: a literal, or a value the flow
    /// read. Two requests of the same tenant cannot both take the last unit: the check holds a lock on the meter and the tenant
    /// until the transaction ends, so put the `meter` that follows it in the same run. Routes only.
    #[serde(rename_all = "camelCase")]
    CheckQuota {
        id: String,
        key: String,
        tenant: Expr,
        /// The ceiling over the period (an `int64`).
        limit: Expr,
        /// The period the use is added up over: calendar periods in UTC, or everything since the start.
        #[serde(default)]
        period: UsagePeriod,
        /// How many units this request is about to use (an `int64`, 1 when it is left out).
        #[serde(default)]
        wanted: Option<Expr>,
        /// Receives what is left after this request (an `int64`): for a header, or a message.
        #[serde(default)]
        bind: Option<String>,
    },
    /// A query on a table of the service's database, in the transaction of the handler. A
    /// `Select` binds `bind`, typed after the ticked columns: that is what the next steps read.
    #[serde(rename_all = "camelCase")]
    Sql {
        id: String,
        op: SqlOp,
        table: String,
        /// Select: the variable receiving the row (or, with `many`, the list of rows).
        #[serde(default)]
        bind: Option<String>,
        /// Select: the columns to read.
        #[serde(default)]
        columns: Vec<String>,
        /// Select: every matching row (a list, which needs `into`) instead of exactly one.
        #[serde(default)]
        many: bool,
        /// Select: convert the rows into this schema (same-named columns fill its fields).
        #[serde(default)]
        into: Option<String>,
        /// `WHERE`, all the conditions together.
        #[serde(default, rename = "where")]
        conditions: Vec<SqlCondition>,
        /// Insert and update: the value of each column.
        #[serde(default)]
        values: Vec<Assignment>,
        #[serde(default)]
        order_by: Option<String>,
        #[serde(default)]
        descending: bool,
        #[serde(default)]
        limit: Option<u32>,
        /// One row expected and none found: the answer, instead of the 404 of the table.
        #[serde(default)]
        missing: Option<SqlMissing>,
        /// Select: other tables of the service joined to `table`. Their columns are then written `table.column` (in
        /// `columns`, `where`, `orderBy`, `thenBy`), and the rule of each table (tenant, soft delete) applies to it too.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        joins: Vec<SqlJoin>,
        /// Select: the name of the field a column gives in the row, instead of the column name (or `table_column` for a
        /// joined one): `known_accounts.email` → `email`. The key is one of `columns`.
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        column_as: std::collections::BTreeMap<String, String>,
        /// Select: more sort keys, after `orderBy`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        then_by: Vec<SqlSort>,
        /// Select, a list: the page size asked by the flow (a request field). It is held between 1 and `limit`, which is
        /// then the largest page and the size when the flow gives none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit_from: Option<Expr>,
        /// Select, a list: how many rows to skip (a request field), at most 1 000 000. Prefer `cursor` on big tables: an
        /// offset makes the database read and drop every skipped row.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<Expr>,
        /// Select, a list: keyset pagination. One value per sort key (`orderBy` then `thenBy`), those of the last row of
        /// the previous page; only the rows after it are read. An absent (optional) value reads the first page. The sort
        /// columns must not be nullable, and the last one should make the order total (a unique column).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cursor: Vec<Expr>,
        /// Select, a list: a variable (int64) receiving the number of rows that match, whatever the page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total_bind: Option<String>,
        /// Select, a list: a variable (bool) that says whether a row follows this page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        more_bind: Option<String>,
        /// Select, a list converted `into` a schema: a variable receiving the last row of the page (empty if none), to
        /// read the values of the next cursor.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_bind: Option<String>,
        /// Upsert: the columns of the unique key (`ON CONFLICT (...)`). A row that already holds this key is updated
        /// with the other `values` instead of failing; with nothing else to set, it is left as it is.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        conflict: Vec<String>,
    },
    /// Hashes a password with argon2id, off the async runtime (it is slow on purpose). The PHC string
    /// (`$argon2id$...`) is what gets stored.
    #[serde(rename_all = "camelCase")]
    HashPassword {
        id: String,
        input: Expr,
        bind: String,
        /// Judge the password as it goes by, without refusing it: tells the visitor it is weak (`passwordStrength` refuses).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weakness: Option<Weakness>,
    },
    /// Checks a password against a stored hash into a `bool`. `stored` may be absent (an unknown
    /// account): the check then runs against a dummy hash, so that it takes as long as a real
    /// refusal and the delay does not tell which accounts exist.
    #[serde(rename_all = "camelCase")]
    VerifyPassword {
        id: String,
        password: Expr,
        stored: Expr,
        bind: String,
        /// Judge the password once it is right (a wrong one is not judged), without refusing the login: a way to tell the
        /// person to change a weak password.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weakness: Option<Weakness>,
        /// When the password is wrong: publish an audit event, then refuse (`401 invalid credentials` unless `fail` says
        /// otherwise). Left out, the step binds `false` and the flow goes on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_fail: Option<OnFail>,
    },
    /// Issues an API key to a client that is a machine, not a person: `sbk_<id>_<secret>`, a readable prefix and an id (what the
    /// client and the lists name), then 256 random bits. Only the SHA-256 of the secret is stored (it is random enough for a fast
    /// hash), in the table `api_keys` of the service's database, in the transaction of the handler, with who the key is for
    /// (`owner`: a company, an account), a name, its `scopes` and, if asked, when it expires. `bind` receives the whole key, **which
    /// cannot be found again**: it is shown once, in the answer. `keyIdBind` receives the id, to name the key afterwards.
    #[serde(rename_all = "camelCase")]
    ApiKeyCreate {
        id: String,
        owner: Expr,
        /// What the key is called: « invoicing robot ».
        name: Expr,
        /// What the key may do: names the routes that check it ask for (`orders:read`). Lowercase letters, digits, `_.:-`.
        #[serde(default)]
        scopes: Vec<String>,
        /// Seconds the key is valid from now (an `int64`); left out, it does not expire.
        #[serde(default)]
        expires_in_secs: Option<Expr>,
        bind: String,
        #[serde(default)]
        key_id_bind: Option<String>,
    },
    /// Authenticates a client by its API key (`key`: the text it sent, in a header: the field of the request reads it; a
    /// `Bearer ` in front is taken off). A key that is missing, malformed, unknown, wrong, revoked or expired is a 401, the same
    /// whichever it is (what is wrong stays in the logs); a good key without the `scope` asked is a 403. `bind` receives the
    /// owner of the key (text), what the route then acts for; `keyIdBind` the id. The time of the last use is kept, at most once
    /// a minute per key. Routes only.
    #[serde(rename_all = "camelCase")]
    ApiKeyVerify {
        id: String,
        key: Expr,
        /// What the route needs the key to be allowed: one of its scopes.
        #[serde(default)]
        scope: Option<String>,
        bind: String,
        #[serde(default)]
        key_id_bind: Option<String>,
    },
    /// Revokes a key: from then on it is a 401. A key is revoked once (a 404 for one that is unknown or already revoked). With
    /// `owner`, only a key of that owner can be: a company revokes its own keys and no one else's. Routes only.
    #[serde(rename_all = "camelCase")]
    ApiKeyRevoke {
        id: String,
        key_id: Expr,
        #[serde(default)]
        owner: Option<Expr>,
    },
    /// Refresh tokens that work once, in families. Without `token` it **starts a family**: a first refresh token for
    /// `subject` (at a login), valid `ttlSecs` seconds. With `token` (the one the client presents) it **rotates**: the token is
    /// spent, a new one of the same family is issued into `bind` (the client keeps only the new one) and the subject of the
    /// family into `subjectBind`, which the flow signs the new access token for. Only the SHA-256 of a token is stored (table
    /// `refresh_tokens`, in the transaction of the handler). A token that is unknown, expired or of a revoked family is a
    /// `401 invalid refresh token`. A token **already spent** was stolen or copied: the whole family is revoked (committed, a
    /// plain refusal would undo it), `onReplay` publishes the audit event and answers (`fail`, else the same 401). Routes only.
    #[serde(rename_all = "camelCase")]
    RefreshRotate {
        id: String,
        /// The refresh token the client presents; left out, a new family is started.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<Expr>,
        /// Whom a new family is for (a text: an account id). Only when `token` is left out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<Expr>,
        /// Seconds the new token is valid.
        ttl_secs: u32,
        /// Receives the new refresh token (a text, shown once).
        bind: String,
        /// Receives the subject of the family (rotation only); also readable in the `publish` of `onReplay`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject_bind: Option<String>,
        /// A spent token was presented again (rotation only): publish an audit event, then answer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_replay: Option<OnFail>,
    },
    /// Ends a refresh token family (a logout): the family of the presented token is revoked, so no token of it works any more.
    /// Idempotent: a token that is unknown, expired or of a family already revoked changes nothing and is not an error (a
    /// logout must always succeed). With `subject`, only a family of that subject is revoked (a user ends their own sessions).
    /// Same table as `refreshRotate` (`refresh_tokens`, in the transaction of the handler). Routes only.
    #[serde(rename_all = "camelCase")]
    RefreshRevoke {
        id: String,
        /// The refresh token the client presents.
        token: Expr,
        /// Only a family of this subject (a text: an account id) is revoked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<Expr>,
    },
    /// Judges a password before it is hashed: how easy it is to guess (zxcvbn: dictionaries, keyboard patterns, dates, repeats,
    /// l33t, and the `inputs` of this account: the e-mail, the name) and, with `refuseBreached`, whether it is in a known data
    /// breach (the first five characters of its SHA-1 are sent to the Pwned Passwords range API, never the password). Under
    /// `minScore` (0 to 4, 3 by default) or breached, the route ends with a 400. `bind` receives the score (an `int32`). Routes only.
    #[serde(rename_all = "camelCase")]
    PasswordStrength {
        id: String,
        password: Expr,
        /// The lowest score accepted: 0 too guessable, 1 very guessable, 2 somewhat, 3 safely unguessable, 4 very unguessable.
        #[serde(default = "default_min_score")]
        min_score: u8,
        /// Texts the password must not be made of: the e-mail address, the name of the account.
        #[serde(default)]
        inputs: Vec<Expr>,
        /// Refuse a password found in a known breach. If the range API cannot be reached the password is let through (and the
        /// log says so): a service must not stop signing people up because another one is down.
        #[serde(default)]
        refuse_breached: bool,
        /// Shortest and longest accepted, in characters as typed (no trim), checked before the score: a 400 with `code`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_length: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<u32>,
        /// The stable machine code of the refusals (`MOT_DE_PASSE_INVALIDE` for a length when unset; none on the others).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(default)]
        bind: Option<String>,
    },
    /// 256 random bits as 64 hexadecimal characters: a session, e-mail or reset token.
    #[serde(rename_all = "camelCase")]
    RandomToken { id: String, bind: String },
    /// SHA-256 of a text as 64 hexadecimal characters. A token is already random: only its hash is stored.
    #[serde(rename_all = "camelCase")]
    Sha256 {
        id: String,
        input: Expr,
        bind: String,
    },
    /// Signs an access token with Ed25519 (EdDSA). The claims are the author's (`sub`, `tid`, `roles`...) plus `iss`, `iat`
    /// and `exp` (now + `ttl_secs`), which cannot be overridden. The seed is 32 bytes as 64 hexadecimal characters in the
    /// environment variable `key_env`. The variable `bind` receives the token, a text.
    #[serde(rename_all = "camelCase")]
    SignJwt {
        id: String,
        key_env: String,
        issuer: String,
        ttl_secs: u32,
        claims: Vec<Assignment>,
        bind: String,
    },
    /// Checks a token: the algorithm is EdDSA only, the issuer, `exp` (5 seconds of tolerance) and `sub` are required,
    /// and the key is picked by the `kid` of the token. Anything wrong is the same 401. `bind` receives the claims as
    /// the API schema `schema`, which lists the claims the flow reads (`sub`, `roles`...): a claim the token lacks has
    /// the default value of its field.
    #[serde(rename_all = "camelCase")]
    VerifyJwt {
        id: String,
        /// The token: a text, or an `Authorization` header when `bearer` is set.
        token: Expr,
        /// The token is the value of an `Authorization: Bearer <token>` header: the prefix is taken off.
        #[serde(default)]
        bearer: bool,
        source: JwksSource,
        issuer: String,
        schema: String,
        bind: String,
        /// A token signed by the same keys for another issuer (a restricted token: password change required) is
        /// answered with this status, message and machine code instead of the generic 401. Its signature, its
        /// expiry and its `sub` are still checked: anything else stays the 401.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        other_issuer: Option<OtherIssuer>,
    },
    /// Publishes the public keys of these seeds as a JWKS (RFC 7517), into a variable of the API schema `Jwks`
    /// (`keys`: a list of `Jwk`: `kty`, `crv`, `alg`, `kid`, `x`). The first seed signs: a rotation puts the new one
    /// first and keeps the old one listed until the tokens it signed have expired. Nothing private is ever published.
    #[serde(rename_all = "camelCase")]
    Jwks {
        id: String,
        key_envs: Vec<String>,
        bind: String,
    },
    /// AES-256-GCM of a text, for what is stored at rest (a TOTP secret, an API token). The key is 32 bytes as 64
    /// hexadecimal characters in the environment variable `key_env`. The result is `hex(nonce || ciphertext)` with
    /// a random nonce, so the same text never gives the same result twice.
    #[serde(rename_all = "camelCase")]
    Encrypt {
        id: String,
        key_env: String,
        input: Expr,
        bind: String,
    },
    /// The text an `Encrypt` step sealed. A wrong key, a truncated or an altered value is an internal error (500):
    /// what is stored was not written by this service, or has been tampered with.
    #[serde(rename_all = "camelCase")]
    Decrypt {
        id: String,
        key_env: String,
        input: Expr,
        bind: String,
    },
    /// Starts the enrolment of an authenticator app (TOTP, RFC 6238: SHA-1, 6 digits, 30 seconds): a random secret,
    /// sealed with AES-256-GCM under `key_env` for storage (it must be readable again to check a code, so it is
    /// encrypted, never hashed). `bind` receives the API schema `TotpSetup`: `sealed` (what to store), `otpauth_url`
    /// (the client draws the QR code from it) and `secret` (base32, to type by hand). Neither the URL nor the secret
    /// is stored: they are shown once.
    #[serde(rename_all = "camelCase")]
    TotpGenerate {
        id: String,
        key_env: String,
        /// Shown by the authenticator app next to the account.
        issuer: String,
        /// The account label in the app, usually the e-mail address.
        account: Expr,
        bind: String,
    },
    /// Checks a TOTP code against the sealed secret. The code of the previous, current or next 30 seconds is
    /// accepted (clock drift), every one is compared in constant time, and a code is **accepted once**: it must be for
    /// a time step newer than the last one accepted for `account`, recorded atomically, so a replay (or two requests
    /// racing with the same code) is refused. A wrong, replayed or malformed code is the same `401 invalid code`.
    /// Routes only. Limit the attempts with a `rateLimit` before it.
    #[serde(rename_all = "camelCase")]
    TotpVerify {
        id: String,
        key_env: String,
        /// The text a `totpGenerate` step sealed.
        sealed: Expr,
        code: Expr,
        /// Whose last accepted time step is kept: an id, not an e-mail that can change.
        account: Expr,
        /// When the code is refused: publish an audit event (committed), then answer `fail` or the usual 401.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_fail: Option<OnFail>,
    },
    /// Issues `count` recovery codes (`XXXXX-XXXXX`, 40 random bits each) for `account`, replacing the ones it had. Only
    /// their hashes are stored; `bind` receives the codes, a list of texts, to show once.
    #[serde(rename_all = "camelCase")]
    RecoveryCodes {
        id: String,
        account: Expr,
        count: u32,
        bind: String,
    },
    /// Spends a recovery code of `account`, entered with or without the dash, in any case: it works once. Anything
    /// else is a `401 invalid code`. Routes only.
    #[serde(rename_all = "camelCase")]
    RecoveryCodeUse {
        id: String,
        account: Expr,
        code: Expr,
        /// When the code is refused: publish an audit event (committed), then answer `fail` or the usual 401.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_fail: Option<OnFail>,
    },
    /// First request of registering a passkey (WebAuthn) for `account`, who is already authenticated: a challenge for the
    /// browser. `bind` receives the API schema `PasskeyChallenge`: `state_id` (given back to `passkeyRegisterFinish`) and
    /// `options` (JSON for `navigator.credentials.create`). A device the account already has is excluded. The site
    /// address, which is the relying party, is read from the environment variable `APP_URL`, unless `app_url` names the
    /// origin of the request (see below). Routes only.
    ///
    /// **Several frontends.** `app_url`, on this step and the three others, is an optional expression for the origin
    /// the request comes from (`https://app.example.com`), evaluated per request, for example from a header or a field
    /// that names the frontend. It is used only when it is `APP_URL` or listed in the environment variable
    /// `WEBAUTHN_ALLOWED_ORIGINS` (origins separated by commas): any other value is a `400` (`401` on a login's end). The
    /// origin a challenge was started for is kept with it and the end step checks the answer against that one; an
    /// `app_url` there that names another origin is refused. To share passkeys between subdomains, set
    /// `WEBAUTHN_RP_ID` to their parent domain (`example.com`): every origin must then be that domain or one of its
    /// subdomains, and an answer from any of the listed origins is accepted. Without it, each origin is its own relying
    /// party.
    #[serde(rename_all = "camelCase")]
    PasskeyRegisterStart {
        id: String,
        /// A stable id of the account, not something that can change like an e-mail address.
        account: Expr,
        /// What the device shows for the account: an e-mail address or a user name.
        name: Expr,
        /// The origin of the request; absent: `APP_URL`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_url: Option<Expr>,
        bind: String,
    },
    /// Second request: checks the answer of the browser (`credential`, the JSON it produced, as a text) against the
    /// challenge `state_id`, which must have been started for `account`, and stores the passkey. A challenge works once
    /// and for five minutes. Anything wrong, or a device already registered, is a `400 invalid passkey`. Routes only.
    #[serde(rename_all = "camelCase")]
    PasskeyRegisterFinish {
        id: String,
        state_id: Expr,
        credential: Expr,
        account: Expr,
        /// The origin of the request; absent: the one the challenge was started for. When given, it must be that one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_url: Option<Expr>,
    },
    /// First request of logging in with a passkey, into a variable of the schema `PasskeyChallenge` (`options` is for
    /// `navigator.credentials.get`). With an `account`, the challenge is for the passkeys of that account; an account
    /// without any is a `401 invalid passkey`, which tells whether it has one. Without `account` (a discoverable login,
    /// no identifier asked), the challenge names nobody: the device offers its passkeys for the site and the account is
    /// the one that owns the passkey it signs with (`passkeyLoginFinish` gives it back); only passkeys registered with
    /// `passkeyRegisterStart` can be found so. Either way, limit the attempts with a `rateLimit` before it. Routes only.
    #[serde(rename_all = "camelCase")]
    PasskeyLoginStart {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<Expr>,
        /// The origin of the request; absent: `APP_URL`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_url: Option<Expr>,
        bind: String,
    },
    /// Second request: checks the answer of the browser against the challenge `state_id` (once, within five minutes)
    /// and gives back, as a text, the account the challenge was started for (for a discoverable login, the account that
    /// owns the passkey the device signed with); the signature counter of the passkey is
    /// kept up to date. Anything wrong is the same `401 invalid passkey`. Routes only.
    #[serde(rename_all = "camelCase")]
    PasskeyLoginFinish {
        id: String,
        state_id: Expr,
        credential: Expr,
        /// The origin of the request; absent: the one the challenge was started for. When given, it must be that one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_url: Option<Expr>,
        bind: String,
    },
    /// Recognises an image from its first bytes (never from the `Content-Type` a client sends, which decides what is
    /// served later). `input` is the image in base64, as a client sends a file in JSON; it must not be empty or above
    /// `max_bytes` once decoded, and must be one of `allow`: there is no SVG, which can carry a script. `bind` receives
    /// the media type (`image/png`, `image/jpeg`). Anything else is a `400`. Routes only; the body limit of the route is
    /// raised to fit.
    #[serde(rename_all = "camelCase")]
    ImageSniff {
        id: String,
        input: Expr,
        allow: Vec<ImageKind>,
        max_bytes: u32,
        bind: String,
    },
    /// Stores an object (`body`, in base64, at most 10 MiB) under `key` in a private bucket, named by the environment
    /// variable `bucket_env`. Garage, MinIO or AWS: with the environment variable `S3_ENDPOINT` it is a compatible store
    /// addressed by path, without it AWS; the credentials are the standard `AWS_*` ones. A store that cannot be reached
    /// is a `503`. Put the object first and the database row after (at worst an object the next write replaces).
    #[serde(rename_all = "camelCase")]
    ObjectPut {
        id: String,
        bucket_env: String,
        key: Expr,
        body: Expr,
        /// `type/subtype`, usually what `imageSniff` found.
        content_type: Expr,
    },
    /// Reads an object: `bind` receives it in base64, or nothing when there is none (an optional text). A store that cannot
    /// be reached is a `503`.
    #[serde(rename_all = "camelCase")]
    ObjectGet {
        id: String,
        bucket_env: String,
        key: Expr,
        bind: String,
    },
    /// Deletes an object; one that is not there is not an error. Delete the database row first and the object after (an
    /// object nobody points to is unreadable, a row that points to nothing is not).
    #[serde(rename_all = "camelCase")]
    ObjectDelete {
        id: String,
        bucket_env: String,
        key: Expr,
    },
    /// A time-limited URL to **read** an object, signed (SigV4) so that the client fetches it straight from the store or
    /// from a CDN in front of it: the bytes never cross the service. `bind` receives the URL (a text). The service only
    /// checks who may read and signs; it does not look whether the object exists (the client gets a 404 from the store).
    /// The host is `S3_PUBLIC_ENDPOINT` when set (the address the client reaches, which may differ from the service's
    /// own `S3_ENDPOINT`), else the store's. `expires_secs` is between 1 second and 7 days.
    #[serde(rename_all = "camelCase")]
    ObjectPresignGet {
        id: String,
        bucket_env: String,
        key: Expr,
        expires_secs: u32,
        bind: String,
    },
    /// A time-limited URL to **write** an object: the client sends it a `PUT` with the file, straight to the store. The
    /// `Content-Type` and the `Content-Length` are part of the signature, so the client must send exactly the type and the
    /// size declared here: `size` (bytes) is checked against `max_bytes` (a `400` above it, or below 1) and any other
    /// body is refused by the store. The type is the client's word, not sniffed: do not serve it back as is for anything
    /// but a download. `bind` receives the URL (a text).
    #[serde(rename_all = "camelCase")]
    ObjectPresignPut {
        id: String,
        bucket_env: String,
        key: Expr,
        content_type: Expr,
        size: Expr,
        max_bytes: u32,
        expires_secs: u32,
        bind: String,
    },
    /// Authenticates a webhook by its signature, before the body is read as JSON: an HMAC-SHA256 of
    /// `"{timestamp}.{raw body}"`, sent in `header` as `t=<unix seconds>,v1=<hex>` (Stripe's scheme). A
    /// wrong, missing or stale signature (past `tolerance_secs`, which stops a replay) is a 401. It has
    /// to be the first step of a route, and the secret is read from the environment variable `secret_env`.
    #[serde(rename_all = "camelCase")]
    VerifyHmac {
        id: String,
        header: String,
        secret_env: String,
        #[serde(default = "default_tolerance_secs")]
        tolerance_secs: u32,
    },
    /// Calls an external HTTP API (Stripe, Twilio, any REST service) and binds its answer. The base address is read from
    /// the environment variable `base_url_env` (never from a value of the flow: a request cannot be sent where a client
    /// chose); `path` is the path after it, where each `{variable}` is a value of the flow, percent-encoded as one path
    /// segment. `query`, `headers` and `body` are lists of `name = value`; the `body` is sent as `body_format` (JSON, or
    /// `application/x-www-form-urlencoded` as Stripe and Twilio take it). The secrets of `auth` are read from the
    /// environment. Redirects are not followed. The call runs inside the transaction of the flow: keep `timeout_secs` low,
    /// and call before writing when you can. A request that is not idempotent is only retried with an `idempotency_key`.
    #[serde(rename_all = "camelCase")]
    HttpCall {
        id: String,
        method: HttpMethod,
        /// Environment variable that holds the base URL (`https://api.stripe.com`).
        base_url_env: String,
        /// Starts with `/`; `{variable}` placeholders are values of the flow, encoded as a single segment.
        path: String,
        /// Query string parameters, one `name = value` each (a value that is absent is left out).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        query: Vec<Assignment>,
        /// Extra request headers, one `name = value` each.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        headers: Vec<Assignment>,
        /// How the secret is sent: a bearer token, basic authentication or a key in a header. Absent: none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<HttpAuthKind>,
        /// The environment variable that holds the token (bearer), the user name (basic) or the key (API key).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth_env: Option<String>,
        /// Basic authentication: the environment variable that holds the password.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth_password_env: Option<String>,
        /// API key: the name of the header that carries it (`X-Api-Key`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth_header: Option<String>,
        /// The fields of the request body, one `name = value` each; none: no body.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        body: Vec<Assignment>,
        #[serde(default)]
        body_format: HttpBodyFormat,
        /// Seconds before the call is given up (1 to 30), for each attempt.
        #[serde(default = "default_http_timeout_secs")]
        timeout_secs: u32,
        /// Extra attempts after a timeout, an unreachable host, a 429 or a 502, 503 or 504 (0 to 5), with a growing wait.
        /// `POST` and `PATCH` are retried only with an `idempotency_key`.
        #[serde(default)]
        retries: u8,
        /// Sent as the `Idempotency-Key` header, the same on every attempt: the provider then does the work once.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        idempotency_key: Option<Expr>,
        /// The API schema the JSON answer is read into (fields it does not name are ignored).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response: Option<String>,
        /// The variable that receives the answer, as a `response`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind: Option<String>,
        /// The variable that receives the HTTP status (an `int32`; `0` when nobody answered).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status_bind: Option<String>,
        /// What a non-2xx answer, a timeout or an unreachable host does.
        #[serde(default)]
        on_error: HttpOnError,
        /// With `on_error` = `fail`: the status the flow ends with (400..=599).
        #[serde(default = "default_http_error_status")]
        error_status: u16,
        /// With `on_error` = `fail`: the fixed text the client is told (a 5xx answers a generic one and logs this).
        #[serde(default = "default_http_error_message")]
        error_message: String,
        /// Cut the calls to this external service while it fails: see [`CircuitConfig`]. Shared by every step that calls the
        /// same base address. An open circuit answers like a call nobody answered (status `0`, so `on_error` decides).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        circuit: Option<CircuitConfig>,
        /// At most this many calls of this step at the same time (a bulkhead): the next one is refused at once, like a call
        /// nobody answered, not queued behind a slow service.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_concurrent: Option<u32>,
    },
    /// Ends an HTTP route early with an error status (400..=599) and `{ "error": message }`.
    /// The message is a fixed text: a 5xx answers a generic one and keeps this in the logs.
    #[serde(rename_all = "camelCase")]
    Fail {
        id: String,
        status: u16,
        message: String,
        /// A stable machine code (`STOCK_INSUFFISANT`): when set, the body is `{ "code", "message", "error" }`
        /// (`error` repeats the message, for the clients that read the plain form).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        /// 429 and 503: seconds written in the `Retry-After` header.
        #[serde(default)]
        retry_after: Option<Expr>,
    },
    /// Ends an HTTP route early with an error status unless `claims` (a list of strings, typically the
    /// `permissions` claim of a verified token) contains `permission`. Replaces the raw `if !claims.contains(..)`
    /// conditions every protected route repeats.
    #[serde(rename_all = "camelCase")]
    RequirePermission {
        id: String,
        /// A list of strings: `claims.permissions`.
        claims: Expr,
        /// The permission that must be in the list, e.g. `crm.manage`.
        permission: String,
        #[serde(default = "default_forbidden_status")]
        status: u16,
        #[serde(default = "default_forbidden_message")]
        message: String,
    },
    /// Runs `steps` once per element of a list: a repeated field, a `Vec` variable, the rows of a `many` SQL step.
    /// `item` is the element, visible inside only (a scalar, or a schema whose fields are read with `item.field`);
    /// the variables of the enclosing flow stay visible, the ones bound inside do not leak out. The steps share the
    /// transaction of the handler: a `fail` or an error inside ends the whole request and rolls everything back.
    /// A `respond` is not allowed inside a loop (it would answer once per element).
    #[serde(rename_all = "camelCase")]
    ForEach {
        id: String,
        /// Name of the variable holding the current element.
        item: String,
        /// The list: an expression whose value is repeated.
        list: Expr,
        #[serde(default)]
        steps: Vec<Step>,
    },
    /// Plays the steps of a fragment of the service, in place. The variables they bind stay visible after it. A fragment
    /// can include another one (not itself, directly or not).
    #[serde(rename_all = "camelCase")]
    Include { id: String, fragment: String },
    /// Escape hatch: raw Rust statements inserted verbatim.
    #[serde(rename_all = "camelCase")]
    Code { id: String, code: String },
}

/// What a route answers when a step that expects one row finds none: a token that is unknown, expired or
/// already used, an account that does not exist. The message is fixed text, like the one of `Fail`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SqlMissing {
    pub status: u16,
    pub message: String,
    /// A stable machine code, like the one of `Fail`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// The answer to a valid token of another issuer (see `VerifyJwt.other_issuer`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OtherIssuer {
    pub issuer: String,
    pub status: u16,
    pub message: String,
    /// A stable machine code (`MDP_CHANGEMENT_REQUIS`): the body is `{ "code", "message", "error" }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// Where the public keys that check a token come from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JwksSource {
    /// The seeds of this service, in the environment: it checks what it signs.
    #[serde(rename_all = "camelCase")]
    Keys { key_envs: Vec<String> },
    /// The environment variable that holds the URL (`http://host:port/path`) of the JWKS of the service that signs. It is
    /// fetched once, and again when a token names a key that is not in it (a rotation), never more than every 10 seconds.
    #[serde(rename_all = "camelCase")]
    Url { url_env: String },
}

/// A header of a route response, whose value comes from the flow. Cookies have their own list, and the headers the
/// runtime sets itself (`Content-Type`, `Retry-After`, CORS...) are refused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResponseHeader {
    pub name: String,
    /// Text: a field, a literal or a value of the flow. A value that is not a valid header value is a 500, logged.
    pub value: Expr,
}

/// Names a response header cannot take: the runtime sets them, or they are the business of the connection.
pub const RESERVED_RESPONSE_HEADERS: &[&str] = &[
    "set-cookie",
    "content-type",
    "content-length",
    "content-encoding",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "upgrade",
    "te",
    "trailer",
    "host",
    "date",
    "retry-after",
    "vary",
];

/// A `Set-Cookie` of a route response. Safe by default: `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    pub value: Expr,
    /// Seconds before it expires; `0` deletes it. Absent: a session cookie.
    #[serde(default)]
    pub max_age_secs: Option<u32>,
    #[serde(default = "default_true")]
    pub http_only: bool,
    #[serde(default = "default_true")]
    pub secure: bool,
    #[serde(default)]
    pub same_site: SameSite,
    #[serde(default)]
    pub path: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SameSite {
    #[default]
    Lax,
    Strict,
    /// Sent cross-site: browsers then require `Secure`.
    None,
}

impl Cookie {
    pub fn new(name: impl Into<String>, value: Expr) -> Self {
        Self {
            name: name.into(),
            value,
            max_age_secs: None,
            http_only: true,
            secure: true,
            same_site: SameSite::Lax,
            path: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SqlOp {
    Select,
    Insert,
    Upsert,
    Update,
    Delete,
    // The number of rows that match (an int64 in `bind`), joins and conditions as in a select. No row is read.
    // (A plain comment: a doc comment would turn the schema of the whole enum into a `oneOf` the forms cannot read.)
    Count,
}

/// `column <op> value`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SqlCondition {
    /// A column of the table, or `table.column` for a joined one.
    pub column: String,
    #[serde(default = "default_cmp")]
    pub op: CmpOp,
    #[serde(default = "default_null")]
    pub value: Expr,
    /// What is tested: a comparison with `op` (the default), membership in a list, a text that contains or starts with the
    /// value, or null. The last two ignore `value`.
    #[serde(default, skip_serializing_if = "SqlTest::is_compare")]
    pub test: SqlTest,
    /// Alternatives: the condition holds when it does, or when any of these does (`(a OR b OR c)`). Each alternative may
    /// have its own. The conditions of the `where` stay AND-ed together.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub or: Vec<SqlCondition>,
}

fn default_cmp() -> CmpOp {
    CmpOp::Eq
}

fn default_null() -> Expr {
    Expr::Literal { value: Value::Null }
}

/// What a `where` condition tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SqlTest {
    /// `column <op> value`.
    #[default]
    Compare,
    /// The column is one of a list of values (`= ANY(list)`).
    In,
    /// A text column contains the value, ignoring case. `%` and `_` in the value are plain characters.
    Contains,
    /// A text column starts with the value, ignoring case. `%` and `_` in the value are plain characters.
    StartsWith,
    /// The column is null.
    IsNull,
    /// The column is not null.
    IsNotNull,
}

impl SqlTest {
    pub fn is_compare(&self) -> bool {
        *self == SqlTest::Compare
    }

    /// Whether the condition reads a value.
    pub fn has_value(self) -> bool {
        !matches!(self, SqlTest::IsNull | SqlTest::IsNotNull)
    }
}

/// How a joined table is matched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SqlJoinKind {
    /// Only the rows that have a match.
    #[default]
    Inner,
    /// Every row of what comes before, with null columns when nothing matches.
    Left,
}

/// A table joined to the one of the step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SqlJoin {
    #[serde(default)]
    pub kind: SqlJoinKind,
    /// A table of the service, not already in the query.
    pub table: String,
    /// The prefix of the fields this table's columns give in the row (`alias_column`); the table name when absent. A
    /// rename of the table sets it, so that the schemas the rows are converted into keep their field names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// The pairs of columns that match, all together (`ON a = b AND c = d`). A foreign key declared on one of the two
    /// tables gives them.
    pub on: Vec<SqlJoinOn>,
}

/// `column = joined.refColumn`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SqlJoinOn {
    /// A column of a table that comes before: `column` for the table of the step, `table.column` for another.
    pub column: String,
    /// A column of the joined table.
    pub ref_column: String,
}

/// A sort key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SqlSort {
    /// A column of the table, or `table.column` for a joined one.
    pub column: String,
    #[serde(default)]
    pub descending: bool,
}

/// One column a select reads, resolved against the tables of the step.
#[derive(Debug, Clone)]
pub struct SqlOutColumn<'a> {
    /// The name of the field of the row: the column name, or `table_column` for a joined table.
    pub field: String,
    pub table: &'a Table,
    pub column: &'a Column,
    /// Null is possible: the column says so, or its table is joined with a left join.
    pub optional: bool,
}

impl Service {
    /// The table and column a name of an SQL step stands for: `column` is the table of the step, `table.column` one of
    /// the tables joined to it. The bool says whether the table is the right side of a left join.
    pub fn sql_column<'a>(
        &'a self,
        base: &str,
        joins: &[SqlJoin],
        name: &str,
    ) -> Option<(&'a Table, &'a Column, bool)> {
        let (table, column) = match name.split_once('.') {
            Some((t, c)) => (t, c),
            None => (base, name),
        };
        if table != base && !joins.iter().any(|j| j.table == table) {
            return None;
        }
        let left = table != base
            && joins
                .iter()
                .any(|j| j.table == table && j.kind == SqlJoinKind::Left);
        let t = self.table(table)?;
        Some((t, t.column(column)?, left))
    }

    /// The columns a select reads, in order, with the name of their field. The ones that do not exist are left out (the
    /// validator reports them).
    pub fn sql_out_columns<'a>(
        &'a self,
        base: &str,
        joins: &[SqlJoin],
        columns: &[String],
        column_as: &std::collections::BTreeMap<String, String>,
    ) -> Vec<SqlOutColumn<'a>> {
        columns
            .iter()
            .filter_map(|n| {
                let (table, column, left) = self.sql_column(base, joins, n)?;
                let prefix = joins
                    .iter()
                    .find(|j| j.table == table.name)
                    .map(|j| j.alias.as_deref().unwrap_or(&j.table));
                Some(SqlOutColumn {
                    field: match (column_as.get(n), prefix.filter(|_| table.name != base)) {
                        (Some(a), _) => a.clone(),
                        (None, None) => column.name.clone(),
                        (None, Some(prefix)) => format!("{prefix}_{}", column.name),
                    },
                    table,
                    column,
                    optional: column.nullable || left,
                })
            })
            .collect()
    }
}

impl SqlCondition {
    pub fn new(column: impl Into<String>, op: CmpOp, value: Expr) -> Self {
        Self {
            column: column.into(),
            op,
            value,
            test: SqlTest::Compare,
            or: Vec::new(),
        }
    }

    /// This condition and every alternative under it, depth first.
    pub fn all(&self) -> Vec<&SqlCondition> {
        let mut out = vec![self];
        for alt in &self.or {
            out.extend(alt.all());
        }
        out
    }

    pub fn sql_op(&self) -> &'static str {
        match self.op {
            CmpOp::Eq => "=",
            CmpOp::Ne => "<>",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
        }
    }
}

impl HttpMethod {
    /// A method that can be sent twice with the same effect (RFC 9110): an outbound call may be retried safely.
    pub fn is_idempotent(self) -> bool {
        !matches!(self, HttpMethod::Post | HttpMethod::Patch)
    }
}

/// How an outbound call authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum HttpAuthKind {
    /// `Authorization: Bearer <token>`.
    Bearer,
    /// `Authorization: Basic` with a user name and a password.
    Basic,
    /// A key in a header of your choice.
    ApiKey,
}

/// How the body of an outbound call is encoded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum HttpBodyFormat {
    /// A JSON object (numbers and booleans stay what they are).
    Json,
    /// `application/x-www-form-urlencoded`, as Stripe and Twilio take it.
    #[default]
    Form,
}

/// What an outbound call that did not succeed (not a 2xx, no answer in time, host unreachable) does to the flow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum HttpOnError {
    /// The flow stops and the transaction is rolled back: the route answers `error_status`.
    #[default]
    Fail,
    /// The flow goes on and reads the status (`status_bind`, `0` when nobody answered) to decide, with a branch.
    Continue,
}

fn default_http_timeout_secs() -> u32 {
    10
}

fn default_http_error_status() -> u16 {
    502
}

fn default_forbidden_status() -> u16 {
    403
}

fn default_forbidden_message() -> String {
    "Forbidden".into()
}

fn default_http_error_message() -> String {
    "the external service did not answer correctly".into()
}

impl Step {
    /// The event a step publishes besides `Publish`: the audit event of an `onFail`.
    pub fn fail_publish(&self) -> Option<&FailPublish> {
        match self {
            Step::VerifyPassword { on_fail, .. }
            | Step::TotpVerify { on_fail, .. }
            | Step::RecoveryCodeUse { on_fail, .. } => {
                on_fail.as_ref().and_then(|o| o.publish.as_ref())
            }
            Step::RefreshRotate { on_replay, .. } => {
                on_replay.as_ref().and_then(|o| o.publish.as_ref())
            }
            _ => None,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Step::Log { id, .. }
            | Step::Let { id, .. }
            | Step::Assign { id, .. }
            | Step::Publish { id, .. }
            | Step::PublishLive { id, .. }
            | Step::LiveGrants { id, .. }
            | Step::SessionTrace { id, .. }
            | Step::SessionRevoke { id, .. }
            | Step::CallRpc { id, .. }
            | Step::Branch { id, .. }
            | Step::Respond { id, .. }
            | Step::Sql { id, .. }
            | Step::Fail { id, .. }
            | Step::RequirePermission { id, .. }
            | Step::RateLimit { id, .. }
            | Step::NextNumber { id, .. }
            | Step::Flag { id, .. }
            | Step::Audit { id, .. }
            | Step::AuditRead { id, .. }
            | Step::Meter { id, .. }
            | Step::CheckQuota { id, .. }
            | Step::VerifyChallenge { id, .. }
            | Step::VerifyHmac { id, .. }
            | Step::HttpCall { id, .. }
            | Step::Encrypt { id, .. }
            | Step::SignJwt { id, .. }
            | Step::VerifyJwt { id, .. }
            | Step::Jwks { id, .. }
            | Step::TotpGenerate { id, .. }
            | Step::TotpVerify { id, .. }
            | Step::RecoveryCodes { id, .. }
            | Step::RecoveryCodeUse { id, .. }
            | Step::PasskeyRegisterStart { id, .. }
            | Step::PasskeyRegisterFinish { id, .. }
            | Step::PasskeyLoginStart { id, .. }
            | Step::PasskeyLoginFinish { id, .. }
            | Step::ImageSniff { id, .. }
            | Step::ObjectPut { id, .. }
            | Step::ObjectGet { id, .. }
            | Step::ObjectDelete { id, .. }
            | Step::ObjectPresignGet { id, .. }
            | Step::ObjectPresignPut { id, .. }
            | Step::Decrypt { id, .. }
            | Step::HashPassword { id, .. }
            | Step::VerifyPassword { id, .. }
            | Step::RandomToken { id, .. }
            | Step::PasswordStrength { id, .. }
            | Step::ApiKeyCreate { id, .. }
            | Step::ApiKeyVerify { id, .. }
            | Step::ApiKeyRevoke { id, .. }
            | Step::RefreshRotate { id, .. }
            | Step::RefreshRevoke { id, .. }
            | Step::Sha256 { id, .. }
            | Step::ForEach { id, .. }
            | Step::Include { id, .. }
            | Step::Code { id, .. } => id,
        }
    }

    /// Visits this step and every nested step, depth first.
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Step)) {
        f(self);
        if let Step::Branch {
            then, otherwise, ..
        } = self
        {
            for s in then.iter().chain(otherwise) {
                s.walk(f);
            }
        }
        if let Step::ForEach { steps, .. } = self {
            for s in steps {
                s.walk(f);
            }
        }
    }
}

/// A circuit breaker on a dependency (an external service, another service of the project), in the manner of Resilience4j.
/// **Closed**: the calls go through and their failures are counted. When `failures` of them happen within `windowSecs` (a sliding
/// window), the circuit **opens**: for `openSecs` every call is refused at once, without being made (a service that is down is not
/// asked, so it is not hammered while it recovers, and the caller does not wait for a timeout). Then it is **half-open**: one
/// call is let through to try; if it succeeds the circuit closes, if it fails it opens again for another `openSecs`. A failure is
/// what means the dependency is unwell: a timeout, no answer, a 429, a 5xx (or, for gRPC, `unavailable`, `deadline exceeded`,
/// `internal`, `unknown`, `resource exhausted`); a 4xx is the caller's fault and counts for nothing. The state is kept in the
/// memory of each instance, and shared by every step that calls the same dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CircuitConfig {
    /// How many failures within the window open the circuit.
    #[serde(default = "default_circuit_failures")]
    pub failures: u32,
    /// The sliding window the failures are counted in, in seconds.
    #[serde(default = "default_circuit_window")]
    pub window_secs: u32,
    /// How long the circuit stays open before it lets one call try, in seconds.
    #[serde(default = "default_circuit_open")]
    pub open_secs: u32,
}

pub const DEFAULT_CIRCUIT_FAILURES: u32 = 5;
pub const DEFAULT_CIRCUIT_WINDOW_SECS: u32 = 30;
pub const DEFAULT_CIRCUIT_OPEN_SECS: u32 = 30;

fn default_circuit_failures() -> u32 {
    DEFAULT_CIRCUIT_FAILURES
}
fn default_circuit_window() -> u32 {
    DEFAULT_CIRCUIT_WINDOW_SECS
}
fn default_circuit_open() -> u32 {
    DEFAULT_CIRCUIT_OPEN_SECS
}

impl Default for CircuitConfig {
    fn default() -> Self {
        Self {
            failures: DEFAULT_CIRCUIT_FAILURES,
            window_secs: DEFAULT_CIRCUIT_WINDOW_SECS,
            open_secs: DEFAULT_CIRCUIT_OPEN_SECS,
        }
    }
}

/// The share of subjects a flag is on for when nothing says otherwise: all of them (when it is enabled).
pub const DEFAULT_FLAG_ROLLOUT: u8 = 100;

fn default_flag_rollout() -> u8 {
    DEFAULT_FLAG_ROLLOUT
}

/// Entries in a page of the audit journal, and the most.
pub const DEFAULT_AUDIT_PAGE: u32 = 50;
pub const MAX_AUDIT_PAGE: u32 = 200;

fn default_audit_page() -> u32 {
    DEFAULT_AUDIT_PAGE
}

/// The first number of a sequence.
pub const DEFAULT_SEQUENCE_START: i64 = 1;

fn default_sequence_start() -> i64 {
    DEFAULT_SEQUENCE_START
}

/// The score under which `passwordStrength` refuses a password: "safely unguessable".
pub const DEFAULT_MIN_PASSWORD_SCORE: u8 = 3;

/// A judgment of a clear password that refuses nothing: `bind` (a `bool`) is true when the score (zxcvbn, 0 to 4) is under
/// `below` (3 by default), so that the answer can ask the person to change it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Weakness {
    pub bind: String,
    #[serde(default = "default_min_score")]
    pub below: u8,
    /// Texts the password must not be made of: the e-mail address, the name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<Expr>,
}

/// What a refusing check does on its way out: an event for the audit trail and/or its own answer. The event is queued in the
/// outbox and the transaction is **committed** before the refusal (a plain refusal rolls everything back, the event too), so
/// what the flow wrote earlier is kept: put the check first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnFail {
    /// The event published when the check refuses: same shape as a `publish` step (topic of the service, payload).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish: Option<FailPublish>,
    /// The answer, instead of the check's own (`401 invalid code`; `401 invalid credentials` for `verifyPassword`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail: Option<FailAs>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FailPublish {
    pub topic: String,
    #[serde(default)]
    pub payload: Vec<Assignment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FailAs {
    pub status: u16,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

fn default_min_score() -> u8 {
    DEFAULT_MIN_PASSWORD_SCORE
}

/// What a quota adds the use up over.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UsagePeriod {
    Hour,
    Day,
    #[default]
    Month,
    Year,
    /// Everything since the first use: a stock (seats, storage) more than a flow.
    Forever,
}

impl UsagePeriod {
    /// The unit `date_trunc` cuts the calendar at; none for `forever`.
    pub fn unit(self) -> Option<&'static str> {
        match self {
            UsagePeriod::Hour => Some("hour"),
            UsagePeriod::Day => Some("day"),
            UsagePeriod::Month => Some("month"),
            UsagePeriod::Year => Some("year"),
            UsagePeriod::Forever => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Assignment {
    pub field: String,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Expr {
    /// A variable or a field of one: `input.order_id`.
    Path {
        path: String,
    },
    Literal {
        value: Value,
    },
    /// Rust expression, inserted verbatim and not type-checked by the editor.
    Raw {
        code: String,
    },
    /// The database clock (`now()`), for the timestamp columns of an SQL step: `expires_at > now()`.
    Now,
    /// A text with values in it: `Order {input.id} of {customer.name}`. Each `{path}` is a plain value (text, number,
    /// bool, uuid) written as text; `{{` and `}}` are literal braces.
    Template {
        template: String,
    },
    /// A typed operation on values (see [`Func`]): arithmetic, text, defaults. No Rust involved.
    Call {
        func: Func,
        args: Vec<Expr>,
    },
}

/// The typed operations of [`Expr::Call`]. The number of arguments and their types are fixed per operation (the
/// editor lists them); integer arithmetic is checked: an overflow or a division by zero fails the request with a
/// 500 instead of wrapping around silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Func {
    /// Joins any number (at least one) of texts.
    Concat,
    /// Lower case of a text.
    Lower,
    /// Upper case of a text.
    Upper,
    /// A text without the spaces around it.
    Trim,
    /// A user's HTML made safe to store and show again (or to put in a mail): the scripts, the styles, the frames, the event
    /// attributes (`onclick`) and the links that run code (`javascript:`) are removed, what is left is the formatting (`p`, `b`,
    /// `a`, lists, headings, tables, quotes, images...), and the links get `rel="noopener noreferrer"`. An allow-list (Ammonia),
    /// not a list of what is forbidden: a tag it does not know is dropped. A text, as text.
    SanitizeHtml,
    /// Number of characters of a text (int64).
    Length,
    /// Whether a text contains another one (bool).
    Contains,
    /// Whether a text starts with another one (bool).
    StartsWith,
    /// Whether a text ends with another one (bool).
    EndsWith,
    /// `replace(text, from, to)`: every occurrence of `from` replaced by `to`.
    Replace,
    /// `split(text, separator)`: the list of the pieces (a list of texts).
    Split,
    /// `join(list, separator)`: a list of texts as one text.
    Join,
    /// `a + b` on two numbers of the same type.
    Add,
    /// `a - b` on two numbers of the same type.
    Sub,
    /// `a * b` on two numbers of the same type.
    Mul,
    /// `a / b`, truncated for integers; a division by zero is a 500.
    Div,
    /// Remainder of `a / b` for integers; a division by zero is a 500.
    Rem,
    /// The smaller of two numbers of the same type.
    Min,
    /// The larger of two numbers of the same type.
    Max,
    /// A float rounded to the nearest integer (int64); halves go away from zero.
    Round,
    /// A float rounded down (int64).
    Floor,
    /// A float rounded up (int64).
    Ceil,
    /// A number as an int32; a value that does not fit is a 500.
    ToInt32,
    /// A number as an int64; a float is truncated, an out-of-range one is a 500.
    ToInt64,
    /// A number as a double.
    ToDouble,
    /// A plain value as text.
    ToText,
    /// A text as an int64: absent when it is not a number (an optional int64).
    ParseInt64,
    /// A text as a double: absent when it is not a number (an optional double).
    ParseDouble,
    /// `default(optional, fallback)`: the value when it is present, else the fallback.
    Default,
    /// A new random UUID (v4), as a uuid.
    NewUuid,
    /// A new UUID version 7, as a uuid: the first 48 bits are the time in milliseconds, so the identifiers **sort in the order
    /// they were made**. A primary key that is a v7 fills the index of its table from its end, not at random places: inserts stay
    /// fast on a large table, and a listing by identifier is a listing by date.
    NewUuidV7,
    /// `shortId(number)`: a whole number (an int64, zero or more) as a short text of letters and digits (`UkLWZg`, at least six
    /// characters), for an address a person reads or pastes: `/invoices/UkLWZg`. Sqids: the same number is always the same text,
    /// and `fromShortId` gives the number back. **It hides the order of the numbers from the eye, not from a person who wants to
    /// find it**: it is not a secret and protects nothing, access still has to be checked.
    ShortId,
    /// `fromShortId(text)`: the number a text made by `shortId` stands for, absent when the text is not one (anything else, even
    /// something that looks like one, is absent: only the exact text `shortId` makes is read).
    FromShortId,
    /// Number of elements of a list (int64).
    ListLength,
    /// The current instant, in whole seconds since 1970 (int64): what the functions below take and give.
    NowSeconds,
    /// `localDate(seconds, zone)`: the calendar day of an instant in a time zone, as `2026-03-29` (a text). A report "per day" is
    /// per day **of someone**: 23:30 in Paris is already the next day in Tokyo.
    LocalDate,
    /// `localTime(seconds, zone)`: an instant as the local date and time with the offset of the zone then, as
    /// `2026-03-29T03:30:00+02:00` (RFC 3339, a text).
    LocalTime,
    /// `startOfDay(seconds, zone)`: the instant (seconds, int64) at which the day of `seconds` began in the zone. A day is not 86400
    /// seconds: it is 82800 the day the clocks go forward and 90000 the day they go back.
    StartOfDay,
    /// `addDays(seconds, days, zone)`: the instant `days` calendar days later (or earlier, if negative) **at the same time on the
    /// clock** in the zone: a day later at 09:00 is 09:00 the next day, 23 or 25 hours after across a change of time.
    AddDays,
    /// `addMonths(seconds, months, zone)`: the same on the clock `months` calendar months later; the 31st plus a month is the last
    /// day of the next month (`January 31` gives `February 28`).
    AddMonths,
    /// `parseLocal(text, zone)`: the instant (seconds, int64) of a date and time written without offset, `2026-03-29T10:00:00` or
    /// `2026-03-29 10:00` (a date alone is its midnight), read in the zone; absent when the text is not one. A time that does not
    /// exist (the hour skipped when the clocks go forward) is moved forward by the gap, one that happens twice is the first.
    ParseLocal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Condition {
    Compare {
        left: Expr,
        op: CmpOp,
        right: Expr,
    },
    /// Rust boolean expression.
    Raw {
        code: String,
    },
    /// True when every condition is (an empty list is true).
    And {
        conditions: Vec<Condition>,
    },
    /// True when one condition is (an empty list is false).
    Or {
        conditions: Vec<Condition>,
    },
    /// The opposite of a condition.
    Not {
        condition: Box<Condition>,
    },
    /// An optional value has a value (a path to an optional field or variable, or a call).
    Present {
        value: Expr,
    },
    /// An optional value is absent.
    Absent {
        value: Expr,
    },
    /// A text has no character, or a list no element.
    Empty {
        value: Expr,
    },
    /// The value equals one of these (all of the same plain type).
    In {
        value: Expr,
        values: Vec<Expr>,
    },
    /// A bool value: a `bool` variable or field, or an operation that answers yes or no (`contains`...).
    Test {
        value: Expr,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CmpOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

impl CmpOp {
    pub fn rust(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
        }
    }
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            name: name.into(),
            outbox: OutboxConfig::default(),
            messages: vec![],
            topics: vec![],
            services: vec![],
            pods: Default::default(),
            openapi: Default::default(),
        }
    }

    /// Whether the stack holds a key-value server (DragonflyDB): there is a real-time service, or a service counts its rate
    /// limits in it.
    pub fn uses_store(&self) -> bool {
        self.services
            .iter()
            .any(|s| s.kind == ServiceKind::Realtime || s.uses_shared_limit())
    }

    /// Whether the exported stack holds Garage: it is asked for, and a service has an object step.
    pub fn uses_garage(&self) -> bool {
        self.pods.garage && self.services.iter().any(|s| !s.bucket_envs().is_empty())
    }

    /// Gives every untagged field an explicit protobuf number.
    ///
    /// Without it a field's number depends on its position, so deleting a field in the middle
    /// would renumber the following ones and break events already stored in Kafka.
    pub fn freeze_field_numbers(&mut self) {
        for m in &mut self.messages {
            let numbers = m.field_numbers();
            for (f, n) in m.fields.iter_mut().zip(numbers) {
                f.tag = Some(n);
            }
        }
    }

    /// A copy where the rows read by SQL steps are real messages, so that the validator and the
    /// generator can type `row.column` like any other path. Appended after the project's own
    /// messages: their indexes, which diagnostics refer to, do not change.
    pub fn with_sql_rows(&self) -> Project {
        // The `include` steps are played first, so that everything below sees the steps they stand for.
        let expanded = self.with_fragments();
        let mut out = expanded.clone();
        for svc in &expanded.services {
            for step in svc.handler_steps() {
                step.walk(&mut |s| {
                    // A select, or a step that returns the rows it wrote (`RETURNING`).
                    let Step::Sql {
                        id,
                        table,
                        columns,
                        joins,
                        column_as,
                        into: None,
                        ..
                    } = s
                    else {
                        return;
                    };
                    if columns.is_empty() {
                        return;
                    }
                    out.messages.push(Message {
                        name: sql_row_name(table, id),
                        kind: MessageKind::Row,
                        fields: svc
                            .sql_out_columns(table, joins, columns, column_as)
                            .iter()
                            .map(|o| {
                                let mut f = Field::new(&o.field, o.column.ty.field_type());
                                f.optional = o.optional;
                                f
                            })
                            .collect(),
                    });
                });
            }
        }
        out
    }

    /// The API schemas a route receives, directly or inside another one: the only ones whose field rules are
    /// ever checked. Sorted, so that what is generated from it is stable.
    pub fn received_schemas(&self) -> std::collections::BTreeSet<&str> {
        let mut seen = std::collections::BTreeSet::new();
        let mut todo: Vec<&str> = self
            .services
            .iter()
            .flat_map(|s| s.routes.iter().map(|r| r.request.as_str()))
            .collect();
        while let Some(name) = todo.pop() {
            let Some(m) = self.message(name) else {
                continue;
            };
            if m.kind != MessageKind::Api || !seen.insert(m.name.as_str()) {
                continue;
            }
            todo.extend(m.fields.iter().filter_map(|f| match &f.ty {
                FieldType::Message { name } => Some(name.as_str()),
                _ => None,
            }));
        }
        seen
    }

    /// Whether checking a `name` means running rules: its own, or those of a schema nested in it.
    pub fn has_rules(&self, name: &str) -> bool {
        fn walk<'a>(p: &'a Project, name: &'a str, seen: &mut Vec<&'a str>) -> bool {
            if seen.contains(&name) {
                return false;
            }
            seen.push(name);
            let Some(m) = p.message(name) else {
                return false;
            };
            m.fields.iter().any(|f| {
                !f.rules.is_empty()
                    || matches!(&f.ty, FieldType::Message { name } if walk(p, name, seen))
            })
        }
        walk(self, name, &mut Vec::new())
    }

    /// Brings a project saved by an older version up to date.
    pub fn migrate(&mut self) {
        if self.schema_version < 2 {
            // Producers used to be implicit: whoever had a Publish step on the topic.
            for t in &mut self.topics {
                for svc in &self.services {
                    let mut publishes = false;
                    for step in svc.handler_steps() {
                        step.walk(&mut |s| {
                            publishes |=
                                matches!(s, Step::Publish { topic, .. } if *topic == t.name)
                        });
                    }
                    if publishes && !t.producers.contains(&svc.name) {
                        t.producers.push(svc.name.clone());
                    }
                }
            }
        }
        if self.schema_version < 3 {
            // Before kinds, everything was a protobuf message: payloads of topics become events.
            let carried: Vec<String> = self.topics.iter().map(|t| t.message.clone()).collect();
            for m in self
                .messages
                .iter_mut()
                .filter(|m| m.kind != MessageKind::Api)
            {
                m.kind = if carried.contains(&m.name) {
                    MessageKind::Event
                } else {
                    MessageKind::Proto
                };
            }
        }
        if self.schema_version < 4 {
            // Before, an absent field of a request took its default: the fields stay as lenient as they were.
            for m in self
                .messages
                .iter_mut()
                .filter(|m| m.kind == MessageKind::Api)
            {
                for f in m.fields.iter_mut().filter(|f| !f.optional) {
                    f.lenient = true;
                }
            }
        }
        self.schema_version = SCHEMA_VERSION;
    }

    pub fn message(&self, name: &str) -> Option<&Message> {
        self.messages.iter().find(|m| m.name == name)
    }

    pub fn topic(&self, name: &str) -> Option<&Topic> {
        self.topics.iter().find(|t| t.name == name)
    }

    pub fn service(&self, name: &str) -> Option<&Service> {
        self.services.iter().find(|s| s.name == name)
    }
}

impl Message {
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Protobuf number of each field, in declaration order. Explicit tags are kept; untagged
    /// fields take the smallest free numbers (skipping the reserved 19000-19999 range).
    pub fn field_numbers(&self) -> Vec<u32> {
        let used: std::collections::BTreeSet<u32> =
            self.fields.iter().filter_map(|f| f.tag).collect();
        let mut next = 1u32;
        self.fields
            .iter()
            .map(|f| {
                f.tag.unwrap_or_else(|| {
                    while used.contains(&next) || (19_000..=19_999).contains(&next) {
                        next += 1;
                    }
                    let n = next;
                    next += 1;
                    n
                })
            })
            .collect()
    }
}

/// Name of the (derived) message holding the row of a `Select` step: `OrdersRowO1`.
pub fn sql_row_name(table: &str, step_id: &str) -> String {
    let mut id: String = step_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if let Some(first) = id.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    format!("{}Row{id}", crate::names::pascal(table))
}

impl Service {
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }

    pub fn rpc(&self, name: &str) -> Option<&Rpc> {
        self.rpcs.iter().find(|r| r.name == name)
    }

    /// The top-level steps of every handler: RPCs, HTTP routes, consumers and jobs.
    pub fn handler_steps(&self) -> impl Iterator<Item = &Step> {
        self.rpcs
            .iter()
            .flat_map(|r| &r.steps)
            .chain(self.routes.iter().flat_map(|r| &r.steps))
            .chain(self.consumers.iter().flat_map(|c| &c.steps))
            .chain(self.jobs.iter().flat_map(|j| &j.steps))
    }

    /// Whether a step of the service issues, checks or revokes an API key: its database then holds `api_keys`.
    pub fn uses_api_keys(&self) -> bool {
        let mut used = false;
        for step in self.handler_steps() {
            step.walk(&mut |s| {
                used |= matches!(
                    s,
                    Step::ApiKeyCreate { .. }
                        | Step::ApiKeyVerify { .. }
                        | Step::ApiKeyRevoke { .. }
                )
            });
        }
        used
    }

    /// Whether a step of the service issues or rotates a refresh token: its database then holds `refresh_tokens`.
    pub fn uses_refresh_tokens(&self) -> bool {
        let mut used = false;
        for step in self.handler_steps() {
            step.walk(&mut |s| {
                used |= matches!(s, Step::RefreshRotate { .. } | Step::RefreshRevoke { .. })
            });
        }
        used
    }

    /// Whether a step of the service asks about a feature flag: its database then holds `feature_flags`.
    pub fn uses_flags(&self) -> bool {
        let mut used = false;
        for step in self.handler_steps() {
            step.walk(&mut |s| used |= matches!(s, Step::Flag { .. }));
        }
        used
    }

    /// Whether a step of the service writes to or reads the audit journal: its database then holds `audit_events`.
    pub fn uses_audit(&self) -> bool {
        let mut used = false;
        for step in self.handler_steps() {
            step.walk(&mut |s| used |= matches!(s, Step::Audit { .. } | Step::AuditRead { .. }));
        }
        used
    }

    /// Whether a step of the service takes a number from a sequence: its database then holds `number_sequences`.
    pub fn uses_sequences(&self) -> bool {
        let mut used = false;
        for step in self.handler_steps() {
            step.walk(&mut |s| used |= matches!(s, Step::NextNumber { .. }));
        }
        used
    }

    /// Whether an expression of the service works in a time zone: the runtime then brings the zones in (`time-zones`).
    pub fn uses_time_zones(&self) -> bool {
        serde_json::to_string(self).is_ok_and(|json| {
            [
                "nowSeconds",
                "localDate",
                "localTime",
                "startOfDay",
                "addDays",
                "addMonths",
                "parseLocal",
            ]
            .iter()
            .any(|f| json.contains(&format!("\"func\":\"{f}\"")))
        })
    }

    /// Whether an expression of the service makes or reads a short identifier: the runtime then brings Sqids in (`short-id`).
    pub fn uses_short_ids(&self) -> bool {
        serde_json::to_string(self).is_ok_and(|json| {
            json.contains("\"func\":\"shortId\"") || json.contains("\"func\":\"fromShortId\"")
        })
    }

    /// Whether an expression of the service cleans an HTML text: the runtime then brings the sanitizer in (`html`). Looked for in
    /// the project's own JSON, which is where every expression of every step is, nested or not.
    pub fn uses_html_sanitizer(&self) -> bool {
        serde_json::to_string(self).is_ok_and(|json| json.contains("\"func\":\"sanitizeHtml\""))
    }

    /// Whether a call of the service has a circuit or a bulkhead: nothing is needed to build it (the runtime always holds it), but
    /// the generated code names the policy.
    pub fn uses_resilience(&self) -> bool {
        let mut used = false;
        for step in self.handler_steps() {
            step.walk(&mut |s| match s {
                Step::HttpCall {
                    circuit,
                    max_concurrent,
                    ..
                }
                | Step::CallRpc {
                    circuit,
                    max_concurrent,
                    ..
                } => {
                    used |= circuit.is_some() || max_concurrent.is_some();
                }
                _ => {}
            });
        }
        used
    }

    /// Whether a step of the service judges a password: the runtime then brings the estimator in (`password-strength`).
    pub fn uses_password_strength(&self) -> bool {
        let mut used = false;
        for step in self.handler_steps() {
            step.walk(&mut |s| {
                used |= match s {
                    Step::PasswordStrength { .. } => true,
                    Step::HashPassword { weakness, .. } | Step::VerifyPassword { weakness, .. } => {
                        weakness.is_some()
                    }
                    _ => false,
                }
            });
        }
        used
    }

    /// Whether a route of the service checks the solution of a challenge: its database then holds `challenge_uses`.
    pub fn uses_challenge_check(&self) -> bool {
        let mut used = false;
        for step in self.handler_steps() {
            step.walk(&mut |s| used |= matches!(s, Step::VerifyChallenge { .. }));
        }
        used
    }

    /// Whether a step of the service meters a use or checks a quota: its database then holds `usage_events`.
    pub fn uses_usage(&self) -> bool {
        let mut used = false;
        for step in self.handler_steps() {
            step.walk(&mut |s| used |= matches!(s, Step::Meter { .. } | Step::CheckQuota { .. }));
        }
        used
    }

    /// Whether a `rateLimit` of the service counts in the key-value server (`shared`): it then needs `DRAGONFLY_URL`.
    pub fn uses_shared_limit(&self) -> bool {
        let mut used = false;
        for step in self.handler_steps() {
            step.walk(&mut |s| used |= matches!(s, Step::RateLimit { shared: true, .. }));
        }
        used
    }

    /// The environment variables that name the buckets the object steps of the service use, once each.
    pub fn bucket_envs(&self) -> std::collections::BTreeSet<&str> {
        let mut envs = std::collections::BTreeSet::new();
        for step in self.handler_steps() {
            step.walk(&mut |s| {
                if let Step::ObjectPut { bucket_env, .. }
                | Step::ObjectGet { bucket_env, .. }
                | Step::ObjectDelete { bucket_env, .. }
                | Step::ObjectPresignGet { bucket_env, .. }
                | Step::ObjectPresignPut { bucket_env, .. } = s
                {
                    envs.insert(bucket_env.as_str());
                }
            });
        }
        envs
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_sample_project_conforms_to_the_derived_json_schema() {
        let schema = super::json_schema();
        let validator =
            jsonschema::validator_for(&schema).expect("the derived schema is a valid JSON Schema");
        let sample = serde_json::to_value(crate::sample::orders()).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&sample)
            .map(|e| format!("{} at {}", e, e.instance_path()))
            .collect();
        assert!(errors.is_empty(), "{errors:#?}");
        // And the schema is not vacuous: a step of an unknown type is refused.
        let mut broken = sample.clone();
        broken["services"][0]["rpcs"][0]["steps"][0]["type"] = "teleport".into();
        assert!(!validator.is_valid(&broken));
    }

    #[test]
    fn a_project_saved_before_required_fields_keeps_its_defaults() {
        let mut json = serde_json::to_value(crate::sample::orders()).unwrap();
        json["schemaVersion"] = 3.into();
        // A file of version 3 has no `lenient` at all.
        for m in json["messages"].as_array_mut().unwrap() {
            for f in m["fields"].as_array_mut().unwrap() {
                f.as_object_mut().unwrap().remove("lenient");
            }
        }
        let mut p: Project = serde_json::from_value(json).unwrap();
        assert!(
            p.message("PlaceOrderBody")
                .unwrap()
                .fields
                .iter()
                .all(|f| !f.lenient)
        );
        p.migrate();
        assert_eq!(p.schema_version, SCHEMA_VERSION);
        assert!(
            p.message("PlaceOrderBody")
                .unwrap()
                .fields
                .iter()
                .all(|f| f.lenient)
        );
        // Events are not read from a request: untouched.
        assert!(
            p.message("OrderCreated")
                .unwrap()
                .fields
                .iter()
                .all(|f| !f.lenient)
        );
        // A new field is required unless it says otherwise.
        assert!(!Field::new("x", FieldType::String).lenient);
    }

    #[test]
    fn migrating_declares_the_services_that_already_publish() {
        let mut p = crate::sample::orders();
        for t in &mut p.topics {
            t.producers.clear();
        }
        for m in p.messages.iter_mut().filter(|m| m.kind != MessageKind::Api) {
            m.kind = MessageKind::Proto;
        }
        p.schema_version = 1;
        p.migrate();
        // Version 4: the fields of the API schemas stay as lenient as they were (an absent field took its default).
        for m in p.messages.iter_mut().filter(|m| m.kind == MessageKind::Api) {
            assert!(m.fields.iter().all(|f| f.lenient || f.optional));
            for f in &mut m.fields {
                f.lenient = false;
            }
        }
        assert_eq!(p, crate::sample::orders());
        let kind = |p: &Project, n: &str| p.message(n).unwrap().kind;
        assert_eq!(kind(&p, "OrderCreated"), MessageKind::Event);
        assert_eq!(kind(&p, "CreateOrderRequest"), MessageKind::Proto);
        assert_eq!(kind(&p, "Customer"), MessageKind::Proto);
        // Migrating again changes nothing.
        let again = p.clone();
        p.migrate();
        assert_eq!(p, again);
    }

    #[test]
    fn projects_saved_before_the_outbox_get_the_default() {
        let p: Project = serde_json::from_str(r#"{"name":"Old"}"#).unwrap();
        assert_eq!(p.outbox.max_attempts, DEFAULT_OUTBOX_ATTEMPTS);
        let p: Project = serde_json::from_str(r#"{"name":"Old","outbox":{}}"#).unwrap();
        assert_eq!(p.outbox.max_attempts, DEFAULT_OUTBOX_ATTEMPTS);
    }

    use super::*;

    fn message(names: &[&str]) -> Message {
        Message {
            kind: Default::default(),
            name: "M".into(),
            fields: names
                .iter()
                .map(|n| Field::new(*n, FieldType::String))
                .collect(),
        }
    }

    #[test]
    fn frozen_numbers_survive_deleting_a_field() {
        let mut p = Project::new("p");
        p.messages.push(message(&["a", "b", "c"]));
        p.freeze_field_numbers();
        assert_eq!(p.messages[0].field_numbers(), vec![1, 2, 3]);

        // Removing `b` must not renumber `c`.
        p.messages[0].fields.remove(1);
        assert_eq!(p.messages[0].field_numbers(), vec![1, 3]);
    }

    #[test]
    fn untagged_fields_without_freezing_would_shift() {
        let mut m = message(&["a", "b", "c"]);
        m.fields.remove(1);
        assert_eq!(
            m.field_numbers(),
            vec![1, 2],
            "this is why numbers are frozen"
        );
    }

    #[test]
    fn freezing_skips_the_reserved_range_and_keeps_explicit_tags() {
        let mut m = message(&["a", "b"]);
        m.fields[0].tag = Some(2);
        assert_eq!(m.field_numbers(), vec![2, 1]);
    }
}
