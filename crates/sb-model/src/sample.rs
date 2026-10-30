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
        kind: MessageKind::Proto,
        fields,
    }
}

fn api(name: &str, fields: Vec<Field>) -> Message {
    Message {
        kind: MessageKind::Api,
        ..message(name, fields)
    }
}

fn event(name: &str, fields: Vec<Field>) -> Message {
    Message {
        kind: MessageKind::Event,
        ..message(name, fields)
    }
}

fn topic(name: &str, message: &str, key: &str, producer: &str, x: f64, y: f64) -> Topic {
    Topic {
        name: name.into(),
        message: message.into(),
        partitions: 3,
        replication_factor: 1,
        key_field: Some(key.into()),
        producers: vec![producer.into()],
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

fn orders_table() -> Table {
    let mut order_id = Column::new("order_id", ColumnType::String);
    order_id.primary_key = true;
    let mut status = Column::new("status", ColumnType::String);
    status.default = Some("'PENDING'".into());
    let mut created_at = Column::new("created_at", ColumnType::Timestamp);
    created_at.default = Some("now()".into());
    Table::new(
        "orders",
        vec![
            order_id,
            Column::new("amount", ColumnType::Int64),
            status,
            created_at,
        ],
    )
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
        event(
            "OrderCreated",
            vec![
                field("order_id", string()),
                msg_field("customer", "Customer"),
                field("amount", FieldType::Int64),
                items,
            ],
        ),
        event(
            "PaymentCompleted",
            vec![field("order_id", string()), field("reference", string())],
        ),
        event(
            "PaymentFailed",
            vec![field("order_id", string()), field("reason", string())],
        ),
        api(
            "PlaceOrderBody",
            vec![field("amount", FieldType::Int64), {
                let mut f = field("items", string());
                f.repeated = true;
                f
            }],
        ),
        api("GetOrderRequest", vec![field("order_id", string())]),
        api(
            "OrderView",
            vec![field("order_id", string()), field("status", string())],
        ),
    ];
    p.topics = vec![
        topic(
            "orders.created",
            "OrderCreated",
            "order_id",
            "Orders",
            230.0,
            120.0,
        ),
        topic(
            "payments.completed",
            "PaymentCompleted",
            "order_id",
            "Payments",
            690.0,
            30.0,
        ),
        topic(
            "payments.failed",
            "PaymentFailed",
            "order_id",
            "Payments",
            690.0,
            200.0,
        ),
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
                        optional: false,
                        repeated: false,
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
                        status: None,
                        cookies: vec![],
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
                        status: None,
                        cookies: vec![],
                    },
                ],
            },
        ],
        http_port: Some(8081),
        tables: vec![orders_table()],
        routes: vec![
            Route {
                name: "place_order".into(),
                method: HttpMethod::Post,
                path: "/orders".into(),
                request: "PlaceOrderBody".into(),
                response: "OrderView".into(),
                status: 201,
                steps: vec![
                    // At most 100 orders a minute for the whole service.
                    Step::RateLimit {
                        id: "h8".into(),
                        key: lit("place_order"),
                        max: 100,
                        window_secs: 60,
                    },
                    Step::Branch {
                        id: "h0".into(),
                        condition: Condition::Compare {
                            left: Expr::Path {
                                path: "input.amount".into(),
                            },
                            op: CmpOp::Le,
                            right: lit(0),
                        },
                        then: vec![Step::Fail {
                            id: "h7".into(),
                            status: 400,
                            message: "the amount must be positive".into(),
                            retry_after: None,
                        }],
                        otherwise: vec![],
                    },
                    Step::Let {
                        id: "h1".into(),
                        name: "order_id".into(),
                        value_type: FieldType::String,
                        optional: false,
                        repeated: false,
                        value: raw("eda_runtime::new_id()"),
                    },
                    Step::Sql {
                        id: "h5".into(),
                        op: SqlOp::Insert,
                        table: "orders".into(),
                        bind: None,
                        columns: vec![],
                        many: false,
                        into: None,
                        conditions: vec![],
                        values: vec![
                            set("order_id", path("order_id")),
                            set("amount", path("input.amount")),
                        ],
                        order_by: None,
                        descending: false,
                        limit: None,
                        missing: None,
                    },
                    Step::Publish {
                        id: "h2".into(),
                        topic: "orders.created".into(),
                        payload: vec![
                            set("order_id", path("order_id")),
                            set("amount", path("input.amount")),
                            set("items", path("input.items")),
                        ],
                    },
                    Step::Respond {
                        id: "h3".into(),
                        fields: vec![
                            set("order_id", path("order_id")),
                            set("status", lit("PENDING")),
                        ],
                        status: Some(201),
                        // Remembers the last order for the front: not readable by scripts.
                        cookies: vec![{
                            let mut c = Cookie::new("last_order", path("order_id"));
                            c.max_age_secs = Some(3600);
                            c
                        }],
                    },
                ],
            },
            Route {
                name: "get_order".into(),
                method: HttpMethod::Get,
                path: "/orders/{order_id}".into(),
                request: "GetOrderRequest".into(),
                response: "OrderView".into(),
                status: 200,
                steps: vec![
                    Step::Sql {
                        id: "h6".into(),
                        op: SqlOp::Select,
                        table: "orders".into(),
                        bind: Some("order".into()),
                        columns: vec!["order_id".into(), "status".into()],
                        many: false,
                        into: None,
                        conditions: vec![SqlCondition {
                            column: "order_id".into(),
                            op: CmpOp::Eq,
                            value: path("input.order_id"),
                        }],
                        values: vec![],
                        order_by: None,
                        descending: false,
                        limit: None,
                        missing: None,
                    },
                    Step::Respond {
                        id: "h4".into(),
                        fields: vec![
                            set("order_id", path("order.order_id")),
                            set("status", path("order.status")),
                        ],
                        status: None,
                        cookies: vec![],
                    },
                ],
            },
        ],
        consumers: vec![],
        jobs: vec![Job {
            name: "heartbeat".into(),
            every_secs: 3600,
            steps: vec![Step::Log {
                id: "j1".into(),
                level: LogLevel::Info,
                message: "orders service is running".into(),
            }],
        }],
    };

    let payments = Service {
        name: "Payments".into(),
        port: 50052,
        jobs: vec![],
        position: Position { x: 460.0, y: 120.0 },
        rpcs: vec![],
        http_port: None,
        routes: vec![],
        tables: vec![],
        consumers: vec![Consumer {
            name: "on_order_created".into(),
            topic: "orders.created".into(),
            group_id: None,
            // Kafka delivers at least once: an order is approved once, however many times it arrives.
            idempotent_on: Some("order_id".into()),
            steps: vec![
                Step::Let {
                    id: "p1".into(),
                    name: "approved".into(),
                    value_type: FieldType::Bool,
                    optional: false,
                    repeated: false,
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
        jobs: vec![],
        position: Position { x: 920.0, y: 120.0 },
        rpcs: vec![],
        http_port: None,
        routes: vec![],
        tables: vec![],
        consumers: vec![
            Consumer {
                name: "on_payment_completed".into(),
                topic: "payments.completed".into(),
                group_id: None,
                idempotent_on: None,
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
                idempotent_on: None,
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

/// Adds to the first service the consumption of a single-use token, the way e-mail verification and
/// password reset do it: the hash of the token is looked up and the row is marked used **in one statement**
/// (`UPDATE ... RETURNING`), so two requests racing with the same token cannot both win.
pub fn add_single_use_token(p: &mut Project) {
    let string = || FieldType::String;
    p.messages
        .push(api("VerifyBody", vec![field("token", string())]));
    p.messages
        .push(api("VerifiedView", vec![field("account_id", string())]));

    let mut token_hash = Column::new("token_hash", ColumnType::String);
    token_hash.primary_key = true;
    let mut used = Column::new("used", ColumnType::Bool);
    used.default = Some("false".into());
    let mut new_email = Column::new("new_email", ColumnType::String);
    new_email.nullable = true;
    let service = &mut p.services[0];
    service.tables.push(Table::new(
        "auth_tokens",
        vec![
            token_hash,
            Column::new("kind", ColumnType::String),
            used,
            Column::new("expires_at", ColumnType::Timestamp),
            Column::new("account_id", ColumnType::String),
            new_email,
        ],
    ));
    service.routes.push(Route {
        name: "verify_email".into(),
        method: HttpMethod::Post,
        path: "/verify-email".into(),
        request: "VerifyBody".into(),
        response: "VerifiedView".into(),
        status: 200,
        steps: vec![
            Step::Sha256 {
                id: "t1".into(),
                input: path("input.token"),
                bind: "token_hash".into(),
            },
            // Unknown, expired or already used: the same answer, whichever it is.
            Step::Sql {
                id: "t2".into(),
                op: SqlOp::Update,
                table: "auth_tokens".into(),
                bind: Some("consumed".into()),
                columns: vec!["account_id".into(), "new_email".into()],
                many: false,
                into: None,
                conditions: vec![
                    SqlCondition {
                        column: "token_hash".into(),
                        op: CmpOp::Eq,
                        value: path("token_hash"),
                    },
                    SqlCondition {
                        column: "kind".into(),
                        op: CmpOp::Eq,
                        value: lit("email"),
                    },
                    SqlCondition {
                        column: "used".into(),
                        op: CmpOp::Eq,
                        value: lit(false),
                    },
                    SqlCondition {
                        column: "expires_at".into(),
                        op: CmpOp::Gt,
                        value: Expr::Now,
                    },
                ],
                values: vec![set("used", lit(true))],
                order_by: None,
                descending: false,
                limit: None,
                missing: Some(SqlMissing {
                    status: 400,
                    message: "invalid or expired token".into(),
                }),
            },
            Step::Respond {
                id: "t3".into(),
                fields: vec![set("account_id", path("consumed.account_id"))],
                status: None,
                cookies: vec![],
            },
        ],
    });
}

/// Adds to the first service the job that purges the tokens that expired, every 7 minutes. It needs the
/// `auth_tokens` table of [`add_single_use_token`]. A `DELETE` of what expired is safe to run on every instance.
pub fn add_cleaner_job(p: &mut Project) {
    p.services[0].jobs.push(Job {
        name: "cleaner".into(),
        every_secs: 420,
        steps: vec![
            Step::Log {
                id: "c1".into(),
                level: LogLevel::Debug,
                message: "purging the expired tokens".into(),
            },
            Step::Sql {
                id: "c2".into(),
                op: SqlOp::Delete,
                table: "auth_tokens".into(),
                bind: None,
                columns: vec![],
                many: false,
                into: None,
                conditions: vec![SqlCondition {
                    column: "expires_at".into(),
                    op: CmpOp::Le,
                    value: Expr::Now,
                }],
                values: vec![],
                order_by: None,
                descending: false,
                limit: None,
                missing: None,
            },
        ],
    });
}

/// Adds to the first service a webhook signed like Stripe's: no session, authenticated by the HMAC of the
/// raw body in `Stripe-Signature`, with a 5 minute clock tolerance against replays.
pub fn add_stripe_webhook(p: &mut Project) {
    p.messages.push(api(
        "StripeEvent",
        vec![
            field("id", FieldType::String),
            field("event_type", FieldType::String),
        ],
    ));
    p.messages
        .push(api("WebhookAck", vec![field("received", FieldType::Bool)]));
    p.services[0].routes.push(Route {
        name: "stripe_webhook".into(),
        method: HttpMethod::Post,
        path: "/webhook/stripe".into(),
        request: "StripeEvent".into(),
        response: "WebhookAck".into(),
        status: 200,
        steps: vec![
            Step::VerifyHmac {
                id: "w1".into(),
                header: "Stripe-Signature".into(),
                secret_env: "STRIPE_WEBHOOK_SECRET".into(),
                tolerance_secs: 300,
            },
            Step::Log {
                id: "w2".into(),
                level: LogLevel::Info,
                message: "stripe event {input.event_type} ({input.id})".into(),
            },
            Step::Respond {
                id: "w3".into(),
                fields: vec![set("received", lit(true))],
                status: None,
                cookies: vec![],
            },
        ],
    });
}

/// Adds to the first service a route that seals a text with AES-256-GCM and opens it again, with a key of the
/// environment (`NOTES_KEY`): what a TOTP secret goes through before it is stored.
pub fn add_sealed_text(p: &mut Project) {
    p.messages
        .push(api("SealBody", vec![field("text", FieldType::String)]));
    p.messages.push(api(
        "SealedView",
        vec![
            field("sealed", FieldType::String),
            field("opened", FieldType::String),
        ],
    ));
    p.services[0].routes.push(Route {
        name: "seal_text".into(),
        method: HttpMethod::Post,
        path: "/notes/seal".into(),
        request: "SealBody".into(),
        response: "SealedView".into(),
        status: 200,
        steps: vec![
            Step::Encrypt {
                id: "e1".into(),
                key_env: "NOTES_KEY".into(),
                input: path("input.text"),
                bind: "sealed".into(),
            },
            Step::Decrypt {
                id: "e2".into(),
                key_env: "NOTES_KEY".into(),
                input: path("sealed"),
                bind: "opened".into(),
            },
            Step::Respond {
                id: "e3".into(),
                fields: vec![set("sealed", path("sealed")), set("opened", path("opened"))],
                status: None,
                cookies: vec![],
            },
        ],
    });
}

/// Adds to the first service a sign-up route whose request carries rules: an e-mail, a password of 10 to 128
/// characters (the bound that keeps a huge one away from argon2), an optional SIREN, a list of contact
/// e-mails and a nested company. The route reads none of them by hand: the generated handler checks them
/// before any step runs.
pub fn add_signup(p: &mut Project) {
    let string = || FieldType::String;
    let ruled = |name: &str, rules: Vec<Rule>| {
        let mut f = field(name, string());
        f.rules = rules;
        f
    };
    let mut siren = ruled("siren", vec![Rule::Siren]);
    siren.optional = true;
    let mut contacts = ruled("contacts", vec![Rule::Email]);
    contacts.repeated = true;
    let company = msg_field("company", "SignupCompany");
    p.messages.push(api(
        "SignupCompany",
        vec![ruled("name", vec![Rule::Length { min: 1, max: 100 }])],
    ));
    p.messages.push(api(
        "SignupBody",
        vec![
            ruled("email", vec![Rule::Email]),
            ruled("password", vec![Rule::Length { min: 10, max: 128 }]),
            siren,
            contacts,
            company,
        ],
    ));
    p.messages
        .push(api("SignupAck", vec![field("email", string())]));
    p.services[0].routes.push(Route {
        name: "signup".into(),
        method: HttpMethod::Post,
        path: "/signup".into(),
        request: "SignupBody".into(),
        response: "SignupAck".into(),
        status: 201,
        steps: vec![
            Step::HashPassword {
                id: "s1".into(),
                input: path("input.password"),
                bind: "password_hash".into(),
            },
            Step::Respond {
                id: "s2".into(),
                fields: vec![set("email", path("input.email"))],
                status: None,
                cookies: vec![],
            },
        ],
    });
}

/// Adds to the first service the tables of an order's lines and shipments: a composite primary key, a composite
/// foreign key with a `CASCADE`, a `UNIQUE` constraint, an index and a `jsonb` column.
pub fn add_shipments(p: &mut Project) {
    let key = |name: &str, ty| {
        let mut c = Column::new(name, ty);
        c.primary_key = true;
        c
    };
    let lines = Table::new(
        "order_lines",
        vec![
            key("order_id", ColumnType::String),
            key("line_no", ColumnType::Int32),
            Column::new("sku", ColumnType::String),
        ],
    );
    let mut id = key("id", ColumnType::Uuid);
    id.default = Some("gen_random_uuid()".into());
    let mut payload = Column::new("payload", ColumnType::Jsonb);
    payload.default = Some("'{}'::jsonb".into());
    let mut shipments = Table::new(
        "shipments",
        vec![
            id,
            Column::new("order_id", ColumnType::String),
            Column::new("line_no", ColumnType::Int32),
            payload,
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
    shipments.uniques.push(UniqueConstraint {
        name: "shipments_line_uq".into(),
        columns: vec!["order_id".into(), "line_no".into()],
    });
    shipments.indexes.push(Index {
        name: "shipments_order_idx".into(),
        columns: vec!["order_id".into()],
        unique: false,
    });
    p.services[0].tables.extend([lines, shipments]);
}

/// The sample with every feature added by the helpers above, to copy the JSON of each from: signed webhook,
/// single-use token, scheduled job, encryption, request rules and composite keys. Valid, with no warning.
pub fn showcase() -> Project {
    let mut p = orders();
    add_single_use_token(&mut p);
    add_cleaner_job(&mut p);
    add_stripe_webhook(&mut p);
    add_sealed_text(&mut p);
    add_signup(&mut p);
    add_login(&mut p);
    add_shipments(&mut p);
    p
}

/// Adds to the first service a login: a rate limit per e-mail, the stored hash read with a query of its own (absent
/// for an unknown account, which is checked all the same against a dummy hash), the same `401` whichever of the
/// e-mail or the password is wrong, then a random token and its hash, and the session cookie.
pub fn add_login(p: &mut Project) {
    let string = || FieldType::String;
    let mut email = field("email", string());
    email.rules = vec![Rule::Email];
    p.messages.push(api(
        "LoginBody",
        vec![email, {
            let mut f = field("password", string());
            f.rules = vec![Rule::Length { min: 1, max: 128 }];
            f
        }],
    ));
    p.messages
        .push(api("LoginAck", vec![field("email", string())]));
    let mut password_hash = Column::new("password_hash", ColumnType::String);
    password_hash.nullable = false;
    let mut email_column = Column::new("email", ColumnType::String);
    email_column.primary_key = true;
    p.services[0]
        .tables
        .push(Table::new("accounts", vec![email_column, password_hash]));
    let mut session = Cookie::new("session", path("session_token"));
    session.max_age_secs = Some(900);
    p.services[0].routes.push(Route {
        name: "login".into(),
        method: HttpMethod::Post,
        path: "/login".into(),
        request: "LoginBody".into(),
        response: "LoginAck".into(),
        status: 200,
        steps: vec![
            Step::RateLimit {
                id: "g1".into(),
                key: path("input.email"),
                max: 5,
                window_secs: 900,
            },
            // None for an unknown account: the check below then runs against a dummy hash.
            Step::Let {
                id: "g2".into(),
                name: "stored".into(),
                value_type: FieldType::String,
                optional: true,
                repeated: false,
                value: raw(
                    "sqlx::query_scalar::<_, String>(\"SELECT password_hash FROM accounts WHERE email = $1\").bind(input.email.clone()).fetch_optional(&mut *tx).await?",
                ),
            },
            Step::VerifyPassword {
                id: "g3".into(),
                password: path("input.password"),
                stored: path("stored"),
                bind: "password_ok".into(),
            },
            // The same answer whether the e-mail or the password was wrong.
            Step::Branch {
                id: "g4".into(),
                condition: Condition::Compare {
                    left: path("password_ok"),
                    op: CmpOp::Eq,
                    right: lit(false),
                },
                then: vec![Step::Fail {
                    id: "g5".into(),
                    status: 401,
                    message: "invalid credentials".into(),
                    retry_after: None,
                }],
                otherwise: vec![],
            },
            Step::RandomToken {
                id: "g6".into(),
                bind: "session_token".into(),
            },
            // What a session table would store: the hash of the token, never the token.
            Step::Sha256 {
                id: "g7".into(),
                input: path("session_token"),
                bind: "session_hash".into(),
            },
            Step::Code {
                id: "g8".into(),
                code: "tracing::info!(session = %session_hash, \"login\");".into(),
            },
            Step::Respond {
                id: "g9".into(),
                fields: vec![set("email", path("input.email"))],
                status: None,
                cookies: vec![session],
            },
        ],
    });
}
