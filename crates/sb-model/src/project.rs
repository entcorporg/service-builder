//! Serializable description of a project. This is the document the visual editor
//! edits, the file saved on disk, and the input of the code generator.
//!
//! The JSON representation is camelCase so the TypeScript side can use it as is.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub name: String,
    #[serde(default)]
    pub broker: BrokerConfig,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub topics: Vec<Topic>,
    #[serde(default)]
    pub services: Vec<Service>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// A protobuf message. Used both as Kafka event payload and as gRPC request/response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub name: String,
    #[serde(default)]
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    Message { name: String },
}

impl FieldType {
    pub fn is_message(&self) -> bool {
        matches!(self, FieldType::Message { .. })
    }
}

/// A Kafka topic carrying exactly one message type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    #[serde(default)]
    pub position: Position,
}

fn default_partitions() -> i32 {
    3
}

fn default_replication() -> i16 {
    1
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub name: String,
    pub port: u16,
    #[serde(default)]
    pub rpcs: Vec<Rpc>,
    #[serde(default)]
    pub consumers: Vec<Consumer>,
    #[serde(default)]
    pub position: Position,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rpc {
    pub name: String,
    pub request: String,
    pub response: String,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Consumer {
    pub name: String,
    pub topic: String,
    /// Kafka consumer group. Defaults to `<service>-<consumer>`.
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// One block of a handler, the equivalent of an Elementor widget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Return the RPC response (RPC handlers only).
    #[serde(rename_all = "camelCase")]
    Respond {
        id: String,
        #[serde(default)]
        fields: Vec<Assignment>,
    },
    /// Escape hatch: raw Rust statements inserted verbatim.
    #[serde(rename_all = "camelCase")]
    Code { id: String, code: String },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assignment {
    pub field: String,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

impl Service {
    pub fn rpc(&self, name: &str) -> Option<&Rpc> {
        self.rpcs.iter().find(|r| r.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(names: &[&str]) -> Message {
        Message {
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
