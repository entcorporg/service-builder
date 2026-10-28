//! The demo project loaded by "New project" and used as the main test fixture.
//!
//! Flow: `Orders.CreateOrder` publishes `orders.created`; `Payments` consumes it and
//! publishes `payments.completed` or `payments.failed`; `Notifications` reacts and, on
//! failure, calls `Orders.CancelOrder` over gRPC.

use serde_json::Value;

use crate::project::*;

fn field(name: &str, ty: FieldType) -> Field {
    Field::new(name, ty)
}

fn msg_field(name: &str, message: &str) -> Field {
    field(
        name,
        FieldType::Message {
            name: message.into(),
        },
    )
}

fn message(name: &str, fields: Vec<Field>) -> Message {
    Message {
        name: name.into(),
        fields,
    }
}

fn topic(name: &str, message: &str, key: &str, x: f64, y: f64) -> Topic {
    Topic {
        name: name.into(),
        message: message.into(),
        partitions: 3,
        replication_factor: 1,
        key_field: Some(key.into()),
        position: Position { x, y },
    }
}

fn path(p: &str) -> Expr {
    Expr::Path { path: p.into() }
}

fn lit(v: impl Into<Value>) -> Expr {
    Expr::Literal { value: v.into() }
}

fn raw(code: &str) -> Expr {
    Expr::Raw { code: code.into() }
}

fn set(field: &str, value: Expr) -> Assignment {
    Assignment {
        field: field.into(),
        value,
    }
}

pub fn orders() -> Project {
    let string = || FieldType::String;
    let mut items = field("items", string());
    items.repeated = true;
    let mut note = field("note", string());
    note.optional = true;

    let mut p = Project::new("Orders Demo");
    p.messages = vec![
        message(
            "Customer",
            vec![field("id", string()), field("name", string())],
        ),
        message(
            "CreateOrderRequest",
            vec![
                msg_field("customer", "Customer"),
                field("amount", FieldType::Int64),
                items.clone(),
            ],
        ),
        message(
            "CreateOrderResponse",
            vec![field("order_id", string()), field("status", string()), note],
        ),
        message(
            "CancelOrderRequest",
            vec![field("order_id", string()), field("reason", string())],
        ),
        message("CancelOrderResponse", vec![field("status", string())]),
        message(
            "OrderCreated",
            vec![
                field("order_id", string()),
                msg_field("customer", "Customer"),
                field("amount", FieldType::Int64),
                items,
            ],
        ),
        message(
            "PaymentCompleted",
            vec![field("order_id", string()), field("reference", string())],
        ),
        message(
            "PaymentFailed",
            vec![field("order_id", string()), field("reason", string())],
        ),
    ];
    p.topics = vec![
        topic("orders.created", "OrderCreated", "order_id", 230.0, 120.0),
        topic(
            "payments.completed",
            "PaymentCompleted",
            "order_id",
            690.0,
            30.0,
        ),
        topic("payments.failed", "PaymentFailed", "order_id", 690.0, 200.0),
    ];

    let orders = Service {
        name: "Orders".into(),
        port: 50051,
        position: Position { x: 0.0, y: 120.0 },
        rpcs: vec![
            Rpc {
                name: "CreateOrder".into(),
                request: "CreateOrderRequest".into(),
                response: "CreateOrderResponse".into(),
                steps: vec![
                    Step::Let {
                        id: "o1".into(),
                        name: "order_id".into(),
                        value_type: FieldType::String,
                        value: raw("eda_runtime::new_id()"),
                    },
                    Step::Log {
                        id: "o2".into(),
                        level: LogLevel::Info,
                        message: "creating order {order_id} for {input.customer.name}".into(),
                    },
                    Step::Publish {
                        id: "o3".into(),
                        topic: "orders.created".into(),
                        payload: vec![
                            set("order_id", path("order_id")),
                            set("customer", path("input.customer")),
                            set("amount", path("input.amount")),
                            set("items", path("input.items")),
                        ],
                    },
                    Step::Respond {
                        id: "o4".into(),
                        fields: vec![
                            set("order_id", path("order_id")),
                            set("status", lit("PENDING")),
                            set("note", path("input.customer.name")),
                        ],
                    },
                ],
            },
            Rpc {
                name: "CancelOrder".into(),
                request: "CancelOrderRequest".into(),
                response: "CancelOrderResponse".into(),
                steps: vec![
                    Step::Log {
                        id: "c1".into(),
                        level: LogLevel::Warn,
                        message: "cancelling order {input.order_id}: {input.reason}".into(),
                    },
                    Step::Respond {
                        id: "c2".into(),
                        fields: vec![set("status", lit("CANCELLED"))],
                    },
                ],
            },
        ],
        consumers: vec![],
    };

    let payments = Service {
        name: "Payments".into(),
        port: 50052,
        position: Position { x: 460.0, y: 120.0 },
        rpcs: vec![],
        consumers: vec![Consumer {
            name: "on_order_created".into(),
            topic: "orders.created".into(),
            group_id: None,
            steps: vec![
                Step::Let {
                    id: "p1".into(),
                    name: "approved".into(),
                    value_type: FieldType::Bool,
                    value: raw("input.amount < 100_000"),
                },
                Step::Branch {
                    id: "p2".into(),
                    condition: Condition::Compare {
                        left: path("approved"),
                        op: CmpOp::Eq,
                        right: lit(true),
                    },
                    then: vec![Step::Publish {
                        id: "p3".into(),
                        topic: "payments.completed".into(),
                        payload: vec![
                            set("order_id", path("input.order_id")),
                            set("reference", raw("eda_runtime::new_id()")),
                        ],
                    }],
                    otherwise: vec![Step::Publish {
                        id: "p4".into(),
                        topic: "payments.failed".into(),
                        payload: vec![
                            set("order_id", path("input.order_id")),
                            set("reason", lit("amount over the card limit")),
                        ],
                    }],
                },
            ],
        }],
    };

    let notifications = Service {
        name: "Notifications".into(),
        port: 50053,
        position: Position { x: 920.0, y: 120.0 },
        rpcs: vec![],
        consumers: vec![
            Consumer {
                name: "on_payment_completed".into(),
                topic: "payments.completed".into(),
                group_id: None,
                steps: vec![Step::Log {
                    id: "n1".into(),
                    level: LogLevel::Info,
                    message: "order {input.order_id} paid (ref {input.reference})".into(),
                }],
            },
            Consumer {
                name: "on_payment_failed".into(),
                topic: "payments.failed".into(),
                group_id: None,
                steps: vec![
                    Step::CallRpc {
                        id: "n2".into(),
                        service: "Orders".into(),
                        method: "CancelOrder".into(),
                        request: vec![
                            set("order_id", path("input.order_id")),
                            set("reason", path("input.reason")),
                        ],
                        bind: Some("cancelled".into()),
                    },
                    Step::Log {
                        id: "n3".into(),
                        level: LogLevel::Warn,
                        message: "order {input.order_id} is now {cancelled.status}".into(),
                    },
                ],
            },
        ],
    };

    p.services = vec![orders, payments, notifications];
    p.freeze_field_numbers();
    p
}
