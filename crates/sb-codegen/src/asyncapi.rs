//! The AsyncAPI 3.0 document of the topics: what a team that listens to the events (or a client generator, or a catalogue) reads
//! instead of the project, the event counterpart of the OpenAPI document of the HTTP routes. It is derived from the same model as
//! the handlers and the outbox, so the two cannot drift apart:
//!
//! * a **channel** is a topic (its Kafka name, its partitions and replicas, the field that is the record key, and the dead-letter
//!   topic the outbox moves an event to after the attempts the project allows);
//! * a **message** is the event of the topic, protobuf on the wire (`application/x-protobuf`), described by the schema of its
//!   fields (the same schemas as the OpenAPI documents: the types and their names are the JSON forms of the fields);
//! * an **operation** is a service that `send`s on a topic (it owns the event, or a flow publishes it) or `receive`s from it (a
//!   consumer, with its Kafka group and whether it skips the events it already handled).
//!
//! One document for the project (the topics are shared by the services), written as `asyncapi.yaml` next to the workspace.

use sb_model::{Project, Service, Step};
use serde_json::{Map, Value, json};

use crate::openapi::Schemas;

/// The version of the Kafka bindings the document uses.
const KAFKA_BINDING: &str = "0.5.0";

/// The document as the file written with the export, or `None` when the project has no topic.
pub fn asyncapi_yaml(project: &Project) -> Option<String> {
    asyncapi(project).map(|doc| yaml(&doc))
}

/// The document, or `None` when the project has no topic.
pub fn asyncapi(project: &Project) -> Option<Value> {
    if project.topics.is_empty() {
        return None;
    }
    let mut schemas = Schemas::default();
    let mut channels = Map::new();
    let mut operations = Map::new();

    for topic in &project.topics {
        let key = channel_key(&topic.name);
        let message = &topic.message;
        let mut message_doc = json!({
            "name": message,
            "title": message,
            "contentType": "application/x-protobuf",
            "payload": schemas.reference(message),
            "bindings": { "kafka": { "bindingVersion": KAFKA_BINDING } },
        });
        if let Some(field) = &topic.key_field {
            // The record key keeps the events of one thing in order: the value of this field, as text.
            message_doc["bindings"]["kafka"]["key"] = json!({
                "type": "string",
                "description": format!("The value of `{field}` of the event: the events that share it are in order."),
            });
        }
        let description = format!(
            "The event `{message}`, {} partition{} and {} replica{}.{} After {} failed attempts to publish it, the outbox moves an \
             event to `{}.dlq`.",
            topic.partitions,
            if topic.partitions == 1 { "" } else { "s" },
            topic.replication_factor,
            if topic.replication_factor == 1 {
                ""
            } else {
                "s"
            },
            match &topic.key_field {
                Some(field) => format!(" The record key is `{field}`."),
                None => String::new(),
            },
            project.outbox.max_attempts,
            topic.name,
        );
        channels.insert(
            key.clone(),
            json!({
                "address": topic.name,
                "description": description,
                "messages": { message.clone(): message_doc },
                "bindings": { "kafka": {
                    "partitions": topic.partitions,
                    "replicas": topic.replication_factor,
                    "bindingVersion": KAFKA_BINDING,
                } },
            }),
        );

        let channel_ref = json!({ "$ref": format!("#/channels/{}", pointer(&key)) });
        let message_ref = json!([{ "$ref": format!("#/channels/{}/messages/{}", pointer(&key), pointer(message)) }]);

        // Who sends: the services that own the event, and the ones a flow publishes it from.
        for service in &project.services {
            if !publishes(service, &topic.name) && !topic.producers.contains(&service.name) {
                continue;
            }
            operations.insert(
                operation_id(&service.name, "send", &topic.name),
                json!({
                    "action": "send",
                    "channel": channel_ref,
                    "title": format!("{} publishes `{}`", service.name, topic.name),
                    "summary": format!(
                        "Written to the outbox in the transaction that makes the change, then published to `{}` by the relay of {}.",
                        topic.name, service.name
                    ),
                    "messages": message_ref,
                    "bindings": { "kafka": { "bindingVersion": KAFKA_BINDING } },
                }),
            );
        }
        // Who receives: a consumer per service and flow.
        for service in &project.services {
            for consumer in service.consumers.iter().filter(|c| c.topic == topic.name) {
                let group = crate::service::group_id(service, consumer);
                let mut summary = format!(
                    "Handled by the flow `{}` of {}. Kafka delivers at least once.",
                    consumer.name, service.name
                );
                if let Some(field) = &consumer.idempotent_on {
                    summary.push_str(&format!(
                        " An event whose `{field}` was already handled is skipped, in the transaction of the first handling."
                    ));
                }
                operations.insert(
                    operation_id(&service.name, &consumer.name, &topic.name),
                    json!({
                        "action": "receive",
                        "channel": channel_ref,
                        "title": format!("{} consumes `{}`", service.name, topic.name),
                        "summary": summary,
                        "messages": message_ref,
                        "bindings": { "kafka": {
                            "groupId": { "type": "string", "enum": [group] },
                            "bindingVersion": KAFKA_BINDING,
                        } },
                    }),
                );
            }
        }
    }

    let schemas = schemas.used(project);
    Some(json!({
        "asyncapi": "3.0.0",
        "info": {
            "title": format!("{} - events", project.name),
            "version": "1.0.0",
            "description": format!(
                "The topics of `{}`, generated by Service Builder: who publishes each event and who consumes it. Events are protobuf \
                 messages on Kafka; the schemas describe their fields. Every event goes through the transactional outbox of its \
                 service, so it is published if and only if the change that caused it is committed.",
                project.name
            ),
        },
        "servers": {
            "kafka": {
                "host": "localhost:9092",
                "protocol": "kafka",
                "description": "The Kafka brokers (`KAFKA_BROKERS`).",
            }
        },
        "channels": channels,
        "operations": operations,
        "components": { "schemas": schemas.into_iter().collect::<Map<String, Value>>() },
    }))
}

/// Whether a flow of the service publishes on the topic.
fn publishes(service: &Service, topic: &str) -> bool {
    let mut found = false;
    for step in service.handler_steps() {
        step.walk(&mut |s| {
            if let Step::Publish { topic: t, .. } = s {
                found |= t == topic;
            }
        });
    }
    found
}

/// The key of a channel: the name of the topic, which is unique.
fn channel_key(topic: &str) -> String {
    topic.to_string()
}

/// A name as one segment of a JSON pointer (`~` and `/` are escaped).
fn pointer(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}

/// The id of an operation: the service, what it does, the topic. Letters, digits and `_`, so that tools can name a function after it.
fn operation_id(service: &str, what: &str, topic: &str) -> String {
    let clean = |text: &str| -> String {
        text.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    };
    format!("{}_{}_{}", clean(service), clean(what), clean(topic))
}

// ---- YAML ---------------------------------------------------------------------------------------

/// A JSON value as YAML: block style, two spaces, keys in the order of the value, every text in double quotes (a JSON string is a
/// YAML double-quoted scalar, so no text can be read as a number, a boolean or a date).
pub fn yaml(value: &Value) -> String {
    let mut out = String::new();
    match value {
        Value::Object(map) if !map.is_empty() => object(map, 0, &mut out),
        Value::Array(list) if !list.is_empty() => array(list, 0, &mut out),
        other => {
            out.push_str(&scalar(other));
            out.push('\n');
        }
    }
    out
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => serde_json::to_string(s).expect("a text is JSON"),
        Value::Array(_) => "[]".into(),
        Value::Object(_) => "{}".into(),
    }
}

fn key(name: &str) -> String {
    let plain = !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(
            name,
            "null" | "true" | "false" | "yes" | "no" | "on" | "off" | "y" | "n" | "~"
        );
    if plain {
        name.to_string()
    } else {
        serde_json::to_string(name).expect("a key is JSON")
    }
}

fn is_block(value: &Value) -> bool {
    matches!(value, Value::Object(m) if !m.is_empty())
        || matches!(value, Value::Array(a) if !a.is_empty())
}

fn object(map: &Map<String, Value>, indent: usize, out: &mut String) {
    for (name, value) in map {
        out.push_str(&" ".repeat(indent));
        out.push_str(&key(name));
        out.push(':');
        if is_block(value) {
            out.push('\n');
            match value {
                Value::Object(inner) => object(inner, indent + 2, out),
                Value::Array(list) => array(list, indent + 2, out),
                _ => unreachable!("a block is an object or a list"),
            }
        } else {
            out.push(' ');
            out.push_str(&scalar(value));
            out.push('\n');
        }
    }
}

fn array(list: &[Value], indent: usize, out: &mut String) {
    for value in list {
        out.push_str(&" ".repeat(indent));
        out.push('-');
        match value {
            Value::Object(inner) if !inner.is_empty() => {
                // The first key shares the line of the dash, the others are aligned under it.
                let mut text = String::new();
                object(inner, indent + 2, &mut text);
                out.push(' ');
                out.push_str(text[indent + 2..].trim_start_matches(' '));
            }
            Value::Array(inner) if !inner.is_empty() => {
                let mut text = String::new();
                array(inner, indent + 2, &mut text);
                out.push(' ');
                out.push_str(text[indent + 2..].trim_start_matches(' '));
            }
            other => {
                out.push(' ');
                out.push_str(&scalar(other));
                out.push('\n');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_model::sample;

    #[test]
    fn a_value_is_written_as_block_yaml_with_every_text_quoted() {
        let value = json!({
            "name": "orders",
            "count": 3,
            "ok": true,
            "none": null,
            "empty_list": [],
            "empty_map": {},
            "looks_like_a_number": "42",
            "looks_like_a_bool": "yes",
            "multi": "two\nlines: and \"quotes\"",
            "$ref": "#/a/b",
            "list": ["a", 1, {"x": 1, "y": [true, {"z": "w"}]}],
            "nested": {"inner": {"deep": 1.5}},
        });
        assert_eq!(
            yaml(&value),
            "\"$ref\": \"#/a/b\"\n\
             count: 3\n\
             empty_list: []\n\
             empty_map: {}\n\
             list:\n  - \"a\"\n  - 1\n  - x: 1\n    \"y\":\n      - true\n      - z: \"w\"\n\
             looks_like_a_bool: \"yes\"\n\
             looks_like_a_number: \"42\"\n\
             multi: \"two\\nlines: and \\\"quotes\\\"\"\n\
             name: \"orders\"\n\
             nested:\n  inner:\n    deep: 1.5\n\
             none: null\n\
             ok: true\n"
        );
        // A key that YAML would read as something else is quoted.
        assert_eq!(
            yaml(&json!({"true": 1, "a b": 2, "": 3, "1a": 4})),
            "\"\": 3\n\"1a\": 4\n\"a b\": 2\n\"true\": 1\n"
        );
    }

    #[test]
    fn a_project_without_a_topic_has_no_document() {
        let mut p = sample::orders();
        p.topics.clear();
        assert!(asyncapi(&p).is_none() && asyncapi_yaml(&p).is_none());
    }

    #[test]
    fn the_topics_are_channels_and_the_services_are_their_senders_and_receivers() {
        let p = sample::orders();
        let doc = asyncapi(&p).unwrap();
        assert_eq!(doc["asyncapi"], "3.0.0");
        // Every topic is a channel, with its address, its partitions and its message.
        for topic in &p.topics {
            let channel = &doc["channels"][&topic.name];
            assert_eq!(channel["address"], topic.name, "{}", topic.name);
            assert_eq!(channel["bindings"]["kafka"]["partitions"], topic.partitions);
            assert_eq!(
                channel["bindings"]["kafka"]["replicas"],
                topic.replication_factor
            );
            let message = &channel["messages"][&topic.message];
            assert_eq!(message["contentType"], "application/x-protobuf");
            assert_eq!(
                message["payload"]["$ref"],
                format!("#/components/schemas/{}", topic.message)
            );
            assert!(
                doc["components"]["schemas"][&topic.message].is_object(),
                "{}",
                topic.message
            );
            assert!(
                channel["description"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("{}.dlq", topic.name))
            );
            if let Some(field) = &topic.key_field {
                assert!(
                    message["bindings"]["kafka"]["key"]["description"]
                        .as_str()
                        .unwrap()
                        .contains(field)
                );
            } else {
                assert!(message["bindings"]["kafka"]["key"].is_null());
            }
        }
        // Every consumer is a `receive` with its Kafka group; every producer, and every flow that publishes, a `send`.
        let operations = doc["operations"].as_object().unwrap();
        let mut receives = 0;
        for service in &p.services {
            for consumer in &service.consumers {
                receives += 1;
                let group = crate::service::group_id(service, consumer);
                let found = operations.values().any(|op| {
                    op["action"] == "receive"
                        && op["channel"]["$ref"] == format!("#/channels/{}", consumer.topic)
                        && op["bindings"]["kafka"]["groupId"]["enum"][0] == group
                });
                assert!(found, "{} / {}", service.name, consumer.name);
            }
        }
        assert_eq!(
            operations
                .values()
                .filter(|o| o["action"] == "receive")
                .count(),
            receives
        );
        for topic in &p.topics {
            for producer in &topic.producers {
                assert!(
                    operations.values().any(|op| op["action"] == "send"
                        && op["channel"]["$ref"] == format!("#/channels/{}", topic.name)
                        && op["title"].as_str().unwrap().starts_with(producer)),
                    "{producer} on {}",
                    topic.name
                );
            }
        }
        // The id of an operation is a name a tool can use.
        assert!(
            operations
                .keys()
                .all(|k| k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')),
            "{:?}",
            operations.keys()
        );
    }

    #[test]
    fn every_reference_of_the_document_points_at_something_in_it() {
        let doc = asyncapi(&sample::showcase()).unwrap();
        let mut refs = vec![];
        fn collect(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Object(m) => {
                    for (k, v) in m {
                        if k == "$ref" {
                            out.push(v.as_str().unwrap().to_string());
                        } else {
                            collect(v, out);
                        }
                    }
                }
                Value::Array(a) => a.iter().for_each(|v| collect(v, out)),
                _ => {}
            }
        }
        collect(&doc, &mut refs);
        assert!(refs.len() > 10, "{}", refs.len());
        for reference in refs {
            let pointer = reference.strip_prefix('#').expect("a local reference");
            assert!(
                doc.pointer(pointer).is_some(),
                "{reference} points at nothing"
            );
        }
    }

    #[test]
    fn the_idempotent_consumers_and_the_record_key_are_told() {
        let doc = asyncapi(&sample::orders()).unwrap();
        let text = yaml(&doc);
        assert!(text.starts_with("asyncapi: \"3.0.0\"\n"), "{text}");
        assert!(
            text.contains("An event whose `order_id` was already handled is skipped"),
            "{text}"
        );
        assert!(text.contains("The record key is `order_id`."), "{text}");
        assert!(text.contains("groupId:"), "{text}");
    }
}
