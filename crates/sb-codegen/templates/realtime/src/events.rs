//! The messages of the channel and who they are for.
//!
//! The service does not know what a type means: it checks its shape (the type becomes the `event:` line of the stream)
//! and requires an explicit audience. A new event touches the publisher and the front only.

use std::collections::BTreeMap;

use bytes::Bytes;
use serde::Deserialize;
use uuid::Uuid;

/// The control message published when the rights of an account change. With `permissions`, the streams of that account
/// in that company adopt them without closing and the browser is told (`grants.updated`, so that it reads its state
/// again). Without `permissions` (the account left the company), those streams close.
pub const GRANTS_UPDATED: &str = "grants.updated";
/// Sent to every open stream after the subscription to the channel was lost and came back: events may have been missed,
/// so the browser reads its state again. The streams are not closed.
pub const RESYNC: &str = "resync";

const MAX_TYPE_LEN: usize = 64;
/// A few identifiers, never content.
const MAX_DATA_ENTRIES: usize = 4;
const MAX_DATA_LEN: usize = 64;
/// Permissions of a `grants.updated`: one per module and level.
pub const MAX_PERMISSIONS: usize = 256;

pub type Data = BTreeMap<String, String>;

/// The scope of a system with no companies (no `tid` in the tokens, no `tenantId` in the events). A stream and an event are
/// in the same scope when they name the same company, or when neither names one: a scope never leaks into another, so a stream
/// with a company never gets an event for everybody, and the reverse.
pub const NO_TENANT: Uuid = Uuid::nil();
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
    account_id: Option<Uuid>,
    tenant_id: Option<Uuid>,
    permissions: Option<Vec<String>>,
    permission: Option<String>,
    #[serde(default)]
    account_ids: Vec<Uuid>,
    origin: Option<Uuid>,
    #[serde(default)]
    data: Data,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Audience {
    /// Every stream of this account, whatever its company.
    Account(Uuid),
    /// The streams open in this company (`NO_TENANT` when the system has none) whose rights contain `permission`, and those of
    /// the `accounts` (without it). Never both empty.
    Tenant {
        tenant_id: Uuid,
        permission: Option<String>,
        accounts: Vec<Uuid>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveEvent {
    /// Written as is in the `event:` line.
    pub kind: String,
    pub audience: Audience,
    /// The tab that made the change (`X-Client-Id`), left out: it already has the answer to its write. Neither a secret
    /// nor a proof: forging it only deprives one's own tab.
    pub origin: Option<Uuid>,
    pub data: Data,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Event(LiveEvent),
    GrantsUpdated {
        tenant_id: Uuid,
        account_id: Uuid,
        permissions: Option<Vec<String>>,
    },
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    #[error("unreadable envelope: {0}")]
    Malformed(String),
    #[error("invalid event type: {0:?}")]
    InvalidType(String),
    #[error("unknown permission in {kind}: {permission:?}")]
    UnknownPermission { kind: String, permission: String },
    #[error("invalid data in {0}")]
    InvalidData(String),
    #[error("event without an audience: {0}")]
    NoAudience(String),
}

impl Message {
    pub fn parse(payload: &[u8]) -> Result<Self, Refused> {
        let envelope: Envelope =
            serde_json::from_slice(payload).map_err(|err| Refused::Malformed(err.to_string()))?;
        if !is_valid_type(&envelope.kind) {
            return Err(Refused::InvalidType(envelope.kind));
        }
        if envelope.kind == GRANTS_UPDATED {
            if let Some(permissions) = &envelope.permissions
                && (permissions.len() > MAX_PERMISSIONS || !permissions.iter().all(|p| is_permission(p)))
            {
                return Err(Refused::InvalidData(envelope.kind));
            }
            return match envelope.account_id {
                Some(account_id) => Ok(Message::GrantsUpdated {
                    tenant_id: envelope.tenant_id.unwrap_or(NO_TENANT),
                    account_id,
                    permissions: envelope.permissions,
                }),
                None => Err(Refused::NoAudience(envelope.kind)),
            };
        }
        // The name of the control event is not one an application event can take: the browser would take it for a
        // request to read its state again.
        if envelope.kind == RESYNC {
            return Err(Refused::InvalidType(envelope.kind));
        }
        if let Some(permission) = &envelope.permission
            && !is_permission(permission)
        {
            return Err(Refused::UnknownPermission {
                permission: permission.clone(),
                kind: envelope.kind,
            });
        }
        if !is_valid_data(&envelope.data) {
            return Err(Refused::InvalidData(envelope.kind));
        }
        let audience = match envelope.account_id {
            Some(account_id) => Audience::Account(account_id),
            // The audience is named: a permission, or accounts. Without a `tenantId` it is the streams that have no company.
            None if envelope.permission.is_some() || !envelope.account_ids.is_empty() => Audience::Tenant {
                tenant_id: envelope.tenant_id.unwrap_or(NO_TENANT),
                permission: envelope.permission,
                accounts: envelope.account_ids,
            },
            None => return Err(Refused::NoAudience(envelope.kind)),
        };
        Ok(Message::Event(LiveEvent {
            kind: envelope.kind,
            audience,
            origin: envelope.origin,
            data: envelope.data,
        }))
    }
}

/// `domain.action`: at least two segments of lowercase letters or underscores, separated by dots. Nothing else reaches
/// the `event:` line: no line break, so no way to inject a field into the stream.
fn is_valid_type(kind: &str) -> bool {
    kind.len() <= MAX_TYPE_LEN
        && kind.split('.').count() >= 2
        && kind
            .split('.')
            .all(|segment| !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
}

/// `module.view` or `module.manage`.
pub fn is_permission(name: &str) -> bool {
    match name.split_once('.') {
        Some((module, level)) => {
            !module.is_empty()
                && module.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                && matches!(level, "view" | "manage")
        }
        None => false,
    }
}

fn is_valid_data(data: &Data) -> bool {
    data.len() <= MAX_DATA_ENTRIES
        && data.iter().all(|(key, value)| {
            !key.is_empty()
                && key.len() <= MAX_DATA_LEN
                && key.bytes().all(|b| b.is_ascii_alphanumeric())
                && value.len() <= MAX_DATA_LEN
                && !value.chars().any(char::is_control)
        })
}

/// The bytes of an event on the wire. Made once per event and shared by every stream it goes to: the JSON is not
/// rebuilt for each of them. The data is compact JSON, which never holds a raw line break.
pub fn frame(kind: &str, data: &Data) -> Bytes {
    let json = serde_json::to_string(data).unwrap_or_else(|_| "{}".into());
    Bytes::from(format!("event: {kind}\ndata: {json}\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn parse(value: Value) -> Result<Message, Refused> {
        Message::parse(value.to_string().as_bytes())
    }

    #[test]
    fn a_tenant_event_carries_its_permission_and_ids() {
        let tenant = Uuid::new_v4();
        let message = parse(json!({
            "type": "contact.updated", "tenantId": tenant, "permission": "contacts.view",
            "data": {"id": "c1", "clientId": "k1"},
        }))
        .unwrap();
        let Message::Event(e) = message else { panic!("an event") };
        assert_eq!(e.kind, "contact.updated");
        assert_eq!(e.data.get("clientId").map(String::as_str), Some("k1"));
        assert_eq!(
            e.audience,
            Audience::Tenant { tenant_id: tenant, permission: Some("contacts.view".into()), accounts: vec![] }
        );
    }

    #[test]
    fn a_system_without_companies_publishes_without_a_tenant_id() {
        let account = Uuid::new_v4();
        let Message::Event(e) = parse(json!({"type": "news.posted", "permission": "news.view"})).unwrap() else { panic!("an event") };
        assert_eq!(e.audience, Audience::Tenant { tenant_id: NO_TENANT, permission: Some("news.view".into()), accounts: vec![] });
        let Message::Event(e) = parse(json!({"type": "news.posted", "accountIds": [account]})).unwrap() else { panic!("an event") };
        assert_eq!(e.audience, Audience::Tenant { tenant_id: NO_TENANT, permission: None, accounts: vec![account] });
        assert_eq!(
            parse(json!({"type": "grants.updated", "accountId": account, "permissions": ["news.view"]})).unwrap(),
            Message::GrantsUpdated { tenant_id: NO_TENANT, account_id: account, permissions: Some(vec!["news.view".into()]) }
        );
        // Still an explicit audience: nobody is not everybody.
        assert!(matches!(parse(json!({"type": "news.posted"})), Err(Refused::NoAudience(_))));
    }

    #[test]
    fn grants_updated_is_a_control_message() {
        let (tenant, account) = (Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(
            parse(json!({"type": "grants.updated", "tenantId": tenant, "accountId": account, "permissions": ["clients.view"]}))
                .unwrap(),
            Message::GrantsUpdated { tenant_id: tenant, account_id: account, permissions: Some(vec!["clients.view".into()]) }
        );
        assert_eq!(
            parse(json!({"type": "grants.updated", "tenantId": tenant, "accountId": account})).unwrap(),
            Message::GrantsUpdated { tenant_id: tenant, account_id: account, permissions: None }
        );
        assert!(parse(json!({"type": "grants.updated", "tenantId": tenant})).is_err(), "it needs the account");
        assert!(parse(json!({"type": "grants.updated", "tenantId": tenant, "accountId": account, "permissions": ["clients.edit"]})).is_err());
    }

    #[test]
    fn the_resync_event_cannot_be_published() {
        let tenant = Uuid::new_v4();
        assert!(parse(json!({"type": "resync", "tenantId": tenant, "permission": "a.view"})).is_err());
        // A real type is two segments at least, so a publisher cannot reach it by accident either.
        assert!(parse(json!({"type": "a.resync", "tenantId": tenant, "permission": "a.view"})).is_ok());
    }

    #[test]
    fn malformed_messages_are_refused() {
        let tenant = Uuid::new_v4();
        assert!(matches!(Message::parse(b"not json"), Err(Refused::Malformed(_))));
        for kind in ["contact", "Contact.updated", "contact.up\ndated", "contact..updated", ""] {
            assert!(
                matches!(parse(json!({"type": kind, "tenantId": tenant, "permission": "contacts.view"})), Err(Refused::InvalidType(_))),
                "{kind:?}"
            );
        }
        assert!(matches!(
            parse(json!({"type": "contact.updated", "tenantId": tenant, "permission": "contacts.read"})),
            Err(Refused::UnknownPermission { .. })
        ));
    }

    #[test]
    fn an_event_needs_an_explicit_audience() {
        let tenant = Uuid::new_v4();
        assert!(matches!(parse(json!({"type": "contact.updated", "tenantId": tenant})), Err(Refused::NoAudience(_))));
        assert!(matches!(parse(json!({"type": "contact.updated"})), Err(Refused::NoAudience(_))));
    }

    #[test]
    fn data_is_a_few_short_plain_values() {
        let tenant = Uuid::new_v4();
        let with = |data: Value| parse(json!({"type": "a.b", "tenantId": tenant, "permission": "x.view", "data": data}));
        assert!(with(json!({"id": "x".repeat(65)})).is_err());
        assert!(with(json!({"a": "1", "b": "2", "c": "3", "d": "4", "e": "5"})).is_err());
        assert!(with(json!({"id": "a\nb"})).is_err());
        assert!(with(json!({"bad key": "1"})).is_err());
        assert!(with(json!({"id": 5})).is_err());
        assert!(with(json!({"id": "ok"})).is_ok());
    }

    #[test]
    fn a_frame_is_one_event_with_compact_json_data() {
        let data = Data::from([("id".to_string(), "c1".to_string())]);
        assert_eq!(&frame("contact.updated", &data)[..], b"event: contact.updated\ndata: {\"id\":\"c1\"}\n\n");
        assert_eq!(&frame("a.b", &Data::new())[..], b"event: a.b\ndata: {}\n\n");
    }

    #[test]
    fn permission_names_are_module_dot_level() {
        for ok in ["clients.view", "invoices.manage", "knowledge_base.view"] {
            assert!(is_permission(ok), "{ok}");
        }
        for ko in ["clients", "clients.read", "Clients.view", ".view", "clients.view.x"] {
            assert!(!is_permission(ko), "{ko}");
        }
    }
}
