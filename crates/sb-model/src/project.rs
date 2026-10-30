//! Serializable description of a project. This is the document the visual editor
//! edits, the file saved on disk, and the input of the code generator.
//!
//! The JSON representation is camelCase so the TypeScript side can use it as is.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 2: topics declare their `producers` (before, a producer was whatever had a Publish step).
/// 3: schemas have a `kind` (event, proto or api).
pub const SCHEMA_VERSION: u32 = 3;

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
    pub broker: BrokerConfig,
    #[serde(default)]
    pub outbox: OutboxConfig,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub topics: Vec<Topic>,
    #[serde(default)]
    pub services: Vec<Service>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrokerConfig {
    pub bootstrap_servers: String,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            bootstrap_servers: "localhost:9092".into(),
        }
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
}

impl Field {
    pub fn new(name: impl Into<String>, ty: FieldType) -> Self {
        Self {
            name: name.into(),
            ty,
            repeated: false,
            optional: false,
            tag: None,
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
    #[serde(default)]
    pub consumers: Vec<Consumer>,
    /// Tasks run on a schedule, with no request and no event: purging what expired, for instance.
    #[serde(default)]
    pub jobs: Vec<Job>,
    #[serde(default)]
    pub position: Position,
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
}

impl Column {
    pub fn new(name: impl Into<String>, ty: ColumnType) -> Self {
        Self {
            name: name.into(),
            ty,
            nullable: false,
            primary_key: false,
            default: None,
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
}

impl Table {
    pub fn new(name: impl Into<String>, columns: Vec<Column>) -> Self {
        Self {
            name: name.into(),
            columns,
            uniques: vec![],
            indexes: vec![],
            foreign_keys: vec![],
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
    /// Success status code (200, 201 or 202).
    #[serde(default = "default_status")]
    pub status: u16,
    #[serde(default)]
    pub steps: Vec<Step>,
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
    },
    /// Counts this attempt under `key` and, past `max` attempts in the last `window_secs`, ends the route
    /// with a 429 and a `Retry-After`. A sliding window per key, kept in the memory of the process.
    #[serde(rename_all = "camelCase")]
    RateLimit {
        id: String,
        key: Expr,
        max: u32,
        window_secs: u32,
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
    },
    /// Hashes a password with argon2id, off the async runtime (it is slow on purpose). The PHC string
    /// (`$argon2id$...`) is what gets stored.
    #[serde(rename_all = "camelCase")]
    HashPassword {
        id: String,
        input: Expr,
        bind: String,
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
    /// Ends an HTTP route early with an error status (400..=599) and `{ "error": message }`.
    /// The message is a fixed text: a 5xx answers a generic one and keeps this in the logs.
    #[serde(rename_all = "camelCase")]
    Fail {
        id: String,
        status: u16,
        message: String,
        /// 429 and 503: seconds written in the `Retry-After` header.
        #[serde(default)]
        retry_after: Option<Expr>,
    },
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
}

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
    Update,
    Delete,
}

/// `column <op> value`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SqlCondition {
    pub column: String,
    pub op: CmpOp,
    pub value: Expr,
}

impl SqlCondition {
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

impl Step {
    pub fn id(&self) -> &str {
        match self {
            Step::Log { id, .. }
            | Step::Let { id, .. }
            | Step::Publish { id, .. }
            | Step::CallRpc { id, .. }
            | Step::Branch { id, .. }
            | Step::Respond { id, .. }
            | Step::Sql { id, .. }
            | Step::Fail { id, .. }
            | Step::RateLimit { id, .. }
            | Step::HashPassword { id, .. }
            | Step::VerifyPassword { id, .. }
            | Step::RandomToken { id, .. }
            | Step::Sha256 { id, .. }
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
            broker: BrokerConfig::default(),
            outbox: OutboxConfig::default(),
            messages: vec![],
            topics: vec![],
            services: vec![],
        }
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
        let mut out = self.clone();
        for svc in &self.services {
            for step in svc.handler_steps() {
                step.walk(&mut |s| {
                    // A select, or a step that returns the rows it wrote (`RETURNING`).
                    let Step::Sql {
                        id,
                        table,
                        columns,
                        into: None,
                        ..
                    } = s
                    else {
                        return;
                    };
                    if columns.is_empty() {
                        return;
                    }
                    let Some(t) = svc.table(table) else { return };
                    out.messages.push(Message {
                        name: sql_row_name(table, id),
                        kind: MessageKind::Row,
                        fields: columns
                            .iter()
                            .filter_map(|c| t.column(c))
                            .map(|c| {
                                let mut f = Field::new(&c.name, c.ty.field_type());
                                f.optional = c.nullable;
                                f
                            })
                            .collect(),
                    });
                });
            }
        }
        out
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
