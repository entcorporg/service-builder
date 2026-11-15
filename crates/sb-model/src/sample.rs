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

/// A new random UUID, the typed operation (no Rust).
fn new_uuid() -> Expr {
    call(Func::NewUuid, vec![])
}

fn call(func: Func, args: Vec<Expr>) -> Expr {
    Expr::Call { func, args }
}

fn template(text: &str) -> Expr {
    Expr::Template {
        template: text.into(),
    }
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
        kind: ServiceKind::Standard,
        templates: vec![],
        realtime: None,
        challenge: None,
        compression: false,
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
                        value: new_uuid(),
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
                        headers: vec![],
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
                        headers: vec![],
                    },
                ],
            },
        ],
        http_port: Some(8081),
        tables: vec![orders_table()],
        dropped_tables: vec![],
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
                        shared: false,
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
                        value: new_uuid(),
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
                        joins: vec![],
                        then_by: vec![],
                        limit_from: None,
                        offset: None,
                        cursor: vec![],
                        total_bind: None,
                        more_bind: None,
                        last_bind: None,
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
                        headers: vec![],
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
                            test: Default::default(),
                        }],
                        values: vec![],
                        order_by: None,
                        descending: false,
                        limit: None,
                        missing: None,
                        joins: vec![],
                        then_by: vec![],
                        limit_from: None,
                        offset: None,
                        cursor: vec![],
                        total_bind: None,
                        more_bind: None,
                        last_bind: None,
                    },
                    Step::Respond {
                        id: "h4".into(),
                        fields: vec![
                            set("order_id", path("order.order_id")),
                            set("status", path("order.status")),
                        ],
                        status: None,
                        cookies: vec![],
                        headers: vec![],
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
        kind: ServiceKind::Standard,
        templates: vec![],
        realtime: None,
        challenge: None,
        compression: false,
        rpcs: vec![],
        http_port: None,
        routes: vec![],
        tables: vec![],
        dropped_tables: vec![],
        consumers: vec![Consumer {
            name: "on_order_created".into(),
            topic: "orders.created".into(),
            group_id: None,
            // Kafka delivers at least once: an order is approved once, however many times it arrives.
            idempotent_on: Some("order_id".into()),
            mail: None,
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
                            set("reference", new_uuid()),
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
        kind: ServiceKind::Standard,
        templates: vec![],
        realtime: None,
        challenge: None,
        compression: false,
        rpcs: vec![],
        http_port: None,
        routes: vec![],
        tables: vec![],
        dropped_tables: vec![],
        consumers: vec![
            Consumer {
                name: "on_payment_completed".into(),
                topic: "payments.completed".into(),
                group_id: None,
                idempotent_on: None,
                mail: None,
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
                mail: None,
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
                        // Orders is asked to cancel; while it fails, the event waits (and is retried) instead of piling calls on it.
                        circuit: Some(CircuitConfig::default()),
                        max_concurrent: None,
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
                        test: Default::default(),
                    },
                    SqlCondition {
                        column: "kind".into(),
                        op: CmpOp::Eq,
                        value: lit("email"),
                        test: Default::default(),
                    },
                    SqlCondition {
                        column: "used".into(),
                        op: CmpOp::Eq,
                        value: lit(false),
                        test: Default::default(),
                    },
                    SqlCondition {
                        column: "expires_at".into(),
                        op: CmpOp::Gt,
                        value: Expr::Now,
                        test: Default::default(),
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
                joins: vec![],
                then_by: vec![],
                limit_from: None,
                offset: None,
                cursor: vec![],
                total_bind: None,
                more_bind: None,
                last_bind: None,
            },
            Step::Respond {
                id: "t3".into(),
                fields: vec![set("account_id", path("consumed.account_id"))],
                status: None,
                cookies: vec![],
                headers: vec![],
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
                    test: Default::default(),
                }],
                values: vec![],
                order_by: None,
                descending: false,
                limit: None,
                missing: None,
                joins: vec![],
                then_by: vec![],
                limit_from: None,
                offset: None,
                cursor: vec![],
                total_bind: None,
                more_bind: None,
                last_bind: None,
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
                headers: vec![],
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
                headers: vec![],
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
            // Before the hash: a password that a dictionary or a breach already knows is not worth hashing.
            Step::PasswordStrength {
                id: "s0".into(),
                password: path("input.password"),
                min_score: DEFAULT_MIN_PASSWORD_SCORE,
                inputs: vec![path("input.email")],
                refuse_breached: true,
                bind: None,
            },
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
                headers: vec![],
            },
        ],
    });
}

/// Adds to the first service a proof-of-work challenge (`GET /challenge`) and a contact form that checks it before it reads
/// anything else: the visitor's browser computes for a moment, a robot does the same for every form it sends. The solution
/// comes with the form, in its `challenge` field.
pub fn add_challenge(p: &mut Project) {
    let string = || FieldType::String;
    let mut email = field("email", string());
    email.rules = vec![Rule::Email];
    let mut text = field("message", string());
    text.rules = vec![Rule::Length { min: 1, max: 2000 }];
    p.messages.push(api(
        "ContactBody",
        vec![email, text, field("challenge", string())],
    ));
    p.messages
        .push(api("ContactAck", vec![field("received", FieldType::Bool)]));
    p.services[0].challenge = Some(ChallengeConfig::default());
    p.services[0].routes.push(Route {
        name: "contact".into(),
        method: HttpMethod::Post,
        path: "/contact".into(),
        request: "ContactBody".into(),
        response: "ContactAck".into(),
        status: 202,
        steps: vec![
            // First: nothing is read, nothing is stored, until a browser has spent its moment.
            Step::VerifyChallenge {
                id: "c1".into(),
                payload: path("input.challenge"),
            },
            Step::Respond {
                id: "c2".into(),
                fields: vec![set("received", lit(true))],
                status: None,
                cookies: vec![],
                headers: vec![],
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
    add_challenge(&mut p);
    add_login(&mut p);
    add_jwt(&mut p);
    add_shipments(&mut p);
    add_totp(&mut p);
    add_passkeys(&mut p);
    add_avatars(&mut p);
    add_notes(&mut p);
    add_usage(&mut p);
    add_api_keys(&mut p);
    add_invoices(&mut p);
    add_calendar(&mut p);
    add_sql_reads(&mut p);
    add_mailer(&mut p);
    add_mail_design(&mut p);
    add_live_events(&mut p);
    add_basket_lines(&mut p);
    add_schema_changes(&mut p);
    add_receipts(&mut p);
    add_http_calls(&mut p);
    // The JSON of the API goes out compressed to the clients that accept it.
    p.services[0].compression = true;
    p
}

/// The journal of changes of the first service's tables, as a project that lived a while keeps it: `orders.amount` was
/// called `total` and was an `int32` (renamed, then widened), `orders` had a `legacy_flag` column and the service had a
/// table `order_drafts`, both dropped. A database migrated before any of this keeps its rows through all four.
pub fn add_schema_changes(p: &mut Project) {
    let svc = &mut p.services[0];
    if let Some(orders) = svc.tables.iter_mut().find(|t| t.name == "orders")
        && let Some(amount) = orders.columns.iter_mut().find(|c| c.name == "amount")
    {
        amount.renamed_from = vec!["total".into()];
        amount.retyped_from = Some(ColumnType::Int32);
        orders.dropped_columns.push("legacy_flag".into());
    }
    svc.dropped_tables.push("order_drafts".into());
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
    p.messages.push(api(
        "LoginAck",
        vec![field("email", string()), field("access_token", string())],
    ));
    let mut password_hash = Column::new("password_hash", ColumnType::String);
    password_hash.nullable = false;
    let mut email_column = Column::new("email", ColumnType::String);
    email_column.primary_key = true;
    p.services[0]
        .tables
        .push(Table::new("accounts", vec![email_column, password_hash]));
    let mut session = Cookie::new("session", path("session_token"));
    session.max_age_secs = Some(900);
    // The same access token the body carries, for the browser that would rather not keep it in a script.
    let mut access = Cookie::new("access_token", path("access_token"));
    access.max_age_secs = Some(300);
    p.services[0].routes.push(Route {
        name: "login".into(),
        method: HttpMethod::Post,
        path: "/login".into(),
        request: "LoginBody".into(),
        response: "LoginAck".into(),
        status: 200,
        steps: vec![
            // The attempts are counted for all the replicas: a guesser must not get five tries per instance.
            Step::RateLimit {
                id: "g1".into(),
                key: path("input.email"),
                max: 5,
                window_secs: 900,
                shared: true,
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
            // A short access token, verifiable by anyone who has the published public key.
            Step::SignJwt {
                id: "g10".into(),
                key_env: "JWT_SIGNING_KEY".into(),
                issuer: "orders".into(),
                ttl_secs: 300,
                claims: vec![
                    set("sub", path("input.email")),
                    set("roles", lit(serde_json::json!(["member"]))),
                    set("tid", lit("acme")),
                ],
                bind: "access_token".into(),
            },
            Step::Respond {
                id: "g9".into(),
                fields: vec![
                    set("email", path("input.email")),
                    set("access_token", path("access_token")),
                ],
                status: None,
                cookies: vec![session, access],
                headers: vec![],
            },
        ],
    });
}

/// Adds to the first service what an access token needs besides the login that issues it: the route that checks
/// one (`GET /whoami` with `Authorization: Bearer`, `GET /session` with the `access_token` cookie, the claims read
/// as the schema `AccessClaims`) and the public route that publishes the key (`GET /.well-known/jwks.json`), which
/// the services that verify on their own fetch.
pub fn add_jwt(p: &mut Project) {
    let string = || FieldType::String;
    let mut roles = field("roles", string());
    roles.repeated = true;
    // `tid` is the tenant of the account: empty when the token has none, which a flow that uses it must refuse.
    p.messages.push(api(
        "AccessClaims",
        vec![
            field("sub", string()),
            roles.clone(),
            field("tid", string()),
        ],
    ));
    // The token comes from the `Authorization` header, or from the session cookie `login` sets.
    let from = |source: RequestSource| {
        let mut token = field("token", string());
        token.lenient = true; // the flow answers 401 itself
        token.source = Some(source);
        token
    };
    p.messages.push(api(
        "WhoamiRequest",
        vec![from(RequestSource::Header {
            name: "Authorization".into(),
        })],
    ));
    p.messages.push(api(
        "SessionRequest",
        vec![from(RequestSource::Cookie {
            name: "access_token".into(),
        })],
    ));
    p.messages
        .push(api("WhoamiView", vec![field("sub", string()), roles]));
    // The shape of a JWKS (RFC 7517), which the `jwks` step fills.
    p.messages.push(api(
        "Jwk",
        vec![
            field("kty", string()),
            field("crv", string()),
            field("alg", string()),
            field("kid", string()),
            field("x", string()),
        ],
    ));
    let mut keys = msg_field("keys", "Jwk");
    keys.repeated = true;
    p.messages.push(api("Jwks", vec![keys]));
    p.messages.push(api("JwksRequest", vec![]));
    p.services[0].routes.push(Route {
        name: "whoami".into(),
        method: HttpMethod::Get,
        path: "/whoami".into(),
        request: "WhoamiRequest".into(),
        response: "WhoamiView".into(),
        status: 200,
        steps: vec![
            // The same 401 whatever is wrong with the token.
            Step::VerifyJwt {
                id: "k1".into(),
                token: path("input.token"),
                bearer: true,
                source: JwksSource::Keys {
                    key_envs: vec!["JWT_SIGNING_KEY".into()],
                },
                issuer: "orders".into(),
                schema: "AccessClaims".into(),
                bind: "claims".into(),
            },
            Step::Respond {
                id: "k2".into(),
                fields: vec![
                    set("sub", path("claims.sub")),
                    set("roles", path("claims.roles")),
                ],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    });
    p.services[0].routes.push(Route {
        name: "jwks".into(),
        method: HttpMethod::Get,
        path: "/.well-known/jwks.json".into(),
        request: "JwksRequest".into(),
        response: "Jwks".into(),
        status: 200,
        steps: vec![
            Step::Jwks {
                id: "k3".into(),
                key_envs: vec!["JWT_SIGNING_KEY".into()],
                bind: "published".into(),
            },
            Step::Respond {
                id: "k4".into(),
                fields: vec![set("keys", path("published.keys"))],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    });
    // The same check, for a browser that keeps the token in a cookie.
    p.services[0].routes.push(Route {
        name: "session".into(),
        method: HttpMethod::Get,
        path: "/session".into(),
        request: "SessionRequest".into(),
        response: "WhoamiView".into(),
        status: 200,
        steps: vec![
            Step::VerifyJwt {
                id: "k5".into(),
                token: path("input.token"),
                bearer: false,
                source: JwksSource::Keys {
                    key_envs: vec!["JWT_SIGNING_KEY".into()],
                },
                issuer: "orders".into(),
                schema: "AccessClaims".into(),
                bind: "claims".into(),
            },
            Step::Respond {
                id: "k6".into(),
                fields: vec![
                    set("sub", path("claims.sub")),
                    set("roles", path("claims.roles")),
                ],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    });
}

/// Adds to the first service a second factor: enrolling an authenticator app (the sealed secret is stored, the `otpauth://`
/// URI and the recovery codes are shown once), checking a code (after a rate limit, the same `401` whatever is wrong), and
/// spending a recovery code when the phone is lost.
pub fn add_totp(p: &mut Project) {
    let string = || FieldType::String;
    let mut codes = field("recovery_codes", string());
    codes.repeated = true;
    p.messages.push(api(
        "TotpSetup",
        vec![
            field("sealed", string()),
            field("otpauth_url", string()),
            field("secret", string()),
        ],
    ));
    p.messages
        .push(api("EnrolBody", vec![field("account", string())]));
    p.messages.push(api(
        "EnrolView",
        vec![
            field("otpauth_url", string()),
            field("secret", string()),
            codes,
        ],
    ));
    p.messages.push(api(
        "CodeBody",
        vec![field("account", string()), field("code", string())],
    ));
    p.messages
        .push(api("CodeAck", vec![field("ok", FieldType::Bool)]));

    let mut account = Column::new("account", ColumnType::String);
    account.primary_key = true;
    p.services[0].tables.push(Table::new(
        "two_factor",
        vec![account, Column::new("secret_sealed", ColumnType::String)],
    ));
    let sql = |id: &str, op: SqlOp, columns: &[&str], values: Vec<Assignment>, missing| Step::Sql {
        id: id.into(),
        op,
        table: "two_factor".into(),
        bind: (op == SqlOp::Select).then(|| "factor".to_string()),
        columns: columns.iter().map(|c| c.to_string()).collect(),
        many: false,
        into: None,
        conditions: if op == SqlOp::Select {
            vec![SqlCondition {
                column: "account".into(),
                op: CmpOp::Eq,
                value: path("input.account"),
                test: Default::default(),
            }]
        } else {
            vec![]
        },
        values,
        order_by: None,
        descending: false,
        limit: None,
        missing,
        joins: vec![],
        then_by: vec![],
        limit_from: None,
        offset: None,
        cursor: vec![],
        total_bind: None,
        more_bind: None,
        last_bind: None,
    };
    let ack = |id: &str| Step::Respond {
        id: id.into(),
        fields: vec![set("ok", lit(true))],
        status: None,
        cookies: vec![],
        headers: vec![],
    };
    let route = |name: &str, path: &str, request: &str, response: &str, steps| Route {
        name: name.into(),
        method: HttpMethod::Post,
        path: path.into(),
        request: request.into(),
        response: response.into(),
        status: 200,
        steps,
    };
    p.services[0].routes.push(route(
        "enrol_totp",
        "/2fa/enrol",
        "EnrolBody",
        "EnrolView",
        vec![
            Step::TotpGenerate {
                id: "f1".into(),
                key_env: "TOTP_ENCRYPTION_KEY".into(),
                issuer: "Orders".into(),
                account: path("input.account"),
                bind: "setup".into(),
            },
            // Only the sealed secret is kept: the URI and the base32 text are shown once.
            sql(
                "f2",
                SqlOp::Insert,
                &[],
                vec![
                    set("account", path("input.account")),
                    set("secret_sealed", path("setup.sealed")),
                ],
                None,
            ),
            Step::RecoveryCodes {
                id: "f3".into(),
                account: path("input.account"),
                count: 8,
                bind: "codes".into(),
            },
            Step::Respond {
                id: "f4".into(),
                fields: vec![
                    set("otpauth_url", path("setup.otpauth_url")),
                    set("secret", path("setup.secret")),
                    set("recovery_codes", path("codes")),
                ],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    ));
    let refused = || {
        Some(SqlMissing {
            status: 401,
            message: "invalid code".into(),
        })
    };
    p.services[0].routes.push(route(
        "verify_totp",
        "/2fa/verify",
        "CodeBody",
        "CodeAck",
        vec![
            // Five tries by account every 15 minutes, before the code is looked at.
            Step::RateLimit {
                id: "f5".into(),
                key: path("input.account"),
                max: 5,
                window_secs: 900,
                shared: false,
            },
            // An account that never enrolled gets the answer of a wrong code.
            sql("f6", SqlOp::Select, &["secret_sealed"], vec![], refused()),
            Step::TotpVerify {
                id: "f7".into(),
                key_env: "TOTP_ENCRYPTION_KEY".into(),
                sealed: path("factor.secret_sealed"),
                code: path("input.code"),
                account: path("input.account"),
            },
            ack("f8"),
        ],
    ));
    p.services[0].routes.push(route(
        "recover_totp",
        "/2fa/recover",
        "CodeBody",
        "CodeAck",
        vec![
            Step::RateLimit {
                id: "f9".into(),
                key: path("input.account"),
                max: 5,
                window_secs: 900,
                shared: false,
            },
            Step::RecoveryCodeUse {
                id: "f10".into(),
                account: path("input.account"),
                code: path("input.code"),
            },
            ack("f11"),
        ],
    ));
}

/// Adds to the first service the four requests of passkeys: registering a device (the account is the one of the verified
/// access token, read from the `Authorization` header) and logging in with it, which gives the access token. It needs
/// [`add_jwt`], whose `AccessClaims` and signing key it uses.
pub fn add_passkeys(p: &mut Project) {
    let string = || FieldType::String;
    let bearer = || {
        let mut token = field("token", string());
        token.lenient = true; // the flow answers 401 itself
        token.source = Some(RequestSource::Header {
            name: "Authorization".into(),
        });
        token
    };
    p.messages.push(api(
        "PasskeyChallenge",
        vec![field("state_id", string()), field("options", string())],
    ));
    p.messages
        .push(api("PasskeyEnrol", vec![bearer(), field("name", string())]));
    p.messages.push(api(
        "PasskeyEnrolAnswer",
        vec![
            bearer(),
            field("state_id", string()),
            field("credential", string()),
        ],
    ));
    p.messages
        .push(api("PasskeyAck", vec![field("ok", FieldType::Bool)]));
    p.messages
        .push(api("PasskeyLoginBody", vec![field("account", string())]));
    // A login that names nobody has no account to count the attempts under: the caller is counted instead.
    let mut client = field("client", string());
    client.lenient = true;
    client.source = Some(RequestSource::Header {
        name: "X-Forwarded-For".into(),
    });
    p.messages.push(api("PasskeyDiscoverBody", vec![client]));
    p.messages.push(api(
        "PasskeyLoginAnswer",
        vec![field("state_id", string()), field("credential", string())],
    ));
    p.messages
        .push(api("PasskeySession", vec![field("access_token", string())]));

    let who = |id: &str| Step::VerifyJwt {
        id: id.into(),
        token: path("input.token"),
        bearer: true,
        source: JwksSource::Keys {
            key_envs: vec!["JWT_SIGNING_KEY".into()],
        },
        issuer: "orders".into(),
        schema: "AccessClaims".into(),
        bind: "claims".into(),
    };
    let route = |name: &str, route: &str, request: &str, response: &str, steps| Route {
        name: name.into(),
        method: HttpMethod::Post,
        path: route.into(),
        request: request.into(),
        response: response.into(),
        status: 200,
        steps,
    };
    let challenge_back = |id: &str| Step::Respond {
        id: id.into(),
        fields: vec![
            set("state_id", path("challenge.state_id")),
            set("options", path("challenge.options")),
        ],
        status: None,
        cookies: vec![],
        headers: vec![],
    };
    let routes = &mut p.services[0].routes;
    routes.push(route(
        "passkey_register_start",
        "/passkeys/register/start",
        "PasskeyEnrol",
        "PasskeyChallenge",
        vec![
            who("p1"),
            Step::PasskeyRegisterStart {
                id: "p2".into(),
                account: path("claims.sub"),
                name: path("input.name"),
                app_url: None,
                bind: "challenge".into(),
            },
            challenge_back("p3"),
        ],
    ));
    routes.push(route(
        "passkey_register_finish",
        "/passkeys/register/finish",
        "PasskeyEnrolAnswer",
        "PasskeyAck",
        vec![
            who("p4"),
            Step::PasskeyRegisterFinish {
                id: "p5".into(),
                state_id: path("input.state_id"),
                credential: path("input.credential"),
                account: path("claims.sub"),
                app_url: None,
            },
            Step::Respond {
                id: "p6".into(),
                fields: vec![set("ok", lit(true))],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    ));
    routes.push(route(
        "passkey_login_start",
        "/passkeys/login/start",
        "PasskeyLoginBody",
        "PasskeyChallenge",
        vec![
            // An account without a passkey is refused: that tells it has none, so the attempts are counted.
            Step::RateLimit {
                id: "p7".into(),
                key: path("input.account"),
                max: 10,
                window_secs: 900,
                shared: false,
            },
            Step::PasskeyLoginStart {
                id: "p8".into(),
                account: Some(path("input.account")),
                app_url: None,
                bind: "challenge".into(),
            },
            challenge_back("p9"),
        ],
    ));
    routes.push(route(
        "passkey_login_finish",
        "/passkeys/login/finish",
        "PasskeyLoginAnswer",
        "PasskeySession",
        vec![
            Step::PasskeyLoginFinish {
                id: "p10".into(),
                state_id: path("input.state_id"),
                credential: path("input.credential"),
                app_url: None,
                bind: "account".into(),
            },
            Step::SignJwt {
                id: "p11".into(),
                key_env: "JWT_SIGNING_KEY".into(),
                issuer: "orders".into(),
                ttl_secs: 300,
                claims: vec![
                    set("sub", path("account")),
                    set("roles", lit(serde_json::json!(["member"]))),
                ],
                bind: "access_token".into(),
            },
            Step::Respond {
                id: "p12".into(),
                fields: vec![set("access_token", path("access_token"))],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    ));
    // The discoverable login: no account in the request. Its second request is `passkey_login_finish`.
    routes.push(route(
        "passkey_discover_start",
        "/passkeys/discover/start",
        "PasskeyDiscoverBody",
        "PasskeyChallenge",
        vec![
            Step::RateLimit {
                id: "p13".into(),
                key: path("input.client"),
                max: 30,
                window_secs: 900,
                shared: false,
            },
            Step::PasskeyLoginStart {
                id: "p14".into(),
                account: None,
                app_url: None,
                bind: "challenge".into(),
            },
            challenge_back("p15"),
        ],
    ));
}

/// Adds to the first service the profile picture of an account, in a private bucket: sent as base64 in a JSON request,
/// recognised by its first bytes (PNG or JPEG, 2 MiB at most, never SVG), stored under the id of the verified account,
/// read back and deleted. It needs [`add_jwt`], whose `AccessClaims` and signing key it uses.
pub fn add_avatars(p: &mut Project) {
    let string = || FieldType::String;
    let bearer = || {
        let mut token = field("token", string());
        token.lenient = true; // the flow answers 401 itself
        token.source = Some(RequestSource::Header {
            name: "Authorization".into(),
        });
        token
    };
    let mut image = field("image", string());
    image.optional = true;
    p.messages.push(api(
        "AvatarUpload",
        vec![bearer(), field("image", string())],
    ));
    p.messages.push(api("AvatarRequest", vec![bearer()]));
    p.messages.push(api("AvatarView", vec![image]));
    p.messages
        .push(api("AvatarAck", vec![field("content_type", string())]));
    p.messages
        .push(api("AvatarDone", vec![field("ok", FieldType::Bool)]));
    // The direct way: the service only signs, the client talks to the store (or its CDN).
    p.messages.push(api(
        "AvatarSign",
        vec![
            bearer(),
            field("content_type", string()),
            field("size", FieldType::Int64),
        ],
    ));
    p.messages
        .push(api("AvatarUrl", vec![field("url", string())]));

    let who = |id: &str| Step::VerifyJwt {
        id: id.into(),
        token: path("input.token"),
        bearer: true,
        source: JwksSource::Keys {
            key_envs: vec!["JWT_SIGNING_KEY".into()],
        },
        issuer: "orders".into(),
        schema: "AccessClaims".into(),
        bind: "claims".into(),
    };
    let route = |name: &str, method, request: &str, response: &str, steps| Route {
        name: name.into(),
        method,
        path: "/avatar".into(),
        request: request.into(),
        response: response.into(),
        status: 200,
        steps,
    };
    let routes = &mut p.services[0].routes;
    routes.push(route(
        "put_avatar",
        HttpMethod::Put,
        "AvatarUpload",
        "AvatarAck",
        vec![
            who("v1"),
            // The type comes from the bytes, never from what the client says.
            Step::ImageSniff {
                id: "v2".into(),
                input: path("input.image"),
                allow: vec![ImageKind::Png, ImageKind::Jpeg],
                max_bytes: 2 * 1024 * 1024,
                bind: "content_type".into(),
            },
            Step::ObjectPut {
                id: "v3".into(),
                bucket_env: "AVATAR_BUCKET".into(),
                key: path("claims.sub"),
                body: path("input.image"),
                content_type: path("content_type"),
            },
            Step::Respond {
                id: "v4".into(),
                fields: vec![set("content_type", path("content_type"))],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    ));
    routes.push(route(
        "get_avatar",
        HttpMethod::Get,
        "AvatarRequest",
        "AvatarView",
        vec![
            who("v5"),
            Step::ObjectGet {
                id: "v6".into(),
                bucket_env: "AVATAR_BUCKET".into(),
                key: path("claims.sub"),
                bind: "avatar".into(),
            },
            Step::Respond {
                id: "v7".into(),
                fields: vec![set("image", path("avatar"))],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    ));
    routes.push(route(
        "delete_avatar",
        HttpMethod::Delete,
        "AvatarRequest",
        "AvatarDone",
        vec![
            who("v8"),
            Step::ObjectDelete {
                id: "v9".into(),
                bucket_env: "AVATAR_BUCKET".into(),
                key: path("claims.sub"),
            },
            Step::Respond {
                id: "v10".into(),
                fields: vec![set("ok", lit(true))],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    ));
    // Direct to the store: a signed URL to upload the avatar, and one to read it, so that no byte crosses the service.
    let mut upload = route(
        "sign_avatar_upload",
        HttpMethod::Post,
        "AvatarSign",
        "AvatarUrl",
        vec![
            who("v11"),
            Step::ObjectPresignPut {
                id: "v12".into(),
                bucket_env: "AVATAR_BUCKET".into(),
                key: path("claims.sub"),
                content_type: path("input.content_type"),
                size: path("input.size"),
                max_bytes: 2 * 1024 * 1024,
                expires_secs: 300,
                bind: "url".into(),
            },
            Step::Respond {
                id: "v13".into(),
                fields: vec![set("url", path("url"))],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    );
    upload.path = "/avatar/upload-url".into();
    routes.push(upload);
    let mut download = route(
        "sign_avatar_download",
        HttpMethod::Get,
        "AvatarRequest",
        "AvatarUrl",
        vec![
            who("v14"),
            Step::ObjectPresignGet {
                id: "v15".into(),
                bucket_env: "AVATAR_BUCKET".into(),
                key: path("claims.sub"),
                expires_secs: 300,
                bind: "url".into(),
            },
            Step::Respond {
                id: "v16".into(),
                fields: vec![set("url", path("url"))],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    );
    download.path = "/avatar/url".into();
    routes.push(download);
}

/// API keys for a client that is a machine. A company, signed in with a token (the `tid` claim), issues keys scoped to what a
/// robot may do and revokes its own; the robot calls a route with `X-API-Key` and is answered for the company the key belongs to.
/// Needs [`add_jwt`], whose `AccessClaims` and signing key it uses.
pub fn add_api_keys(p: &mut Project) {
    let string = || FieldType::String;
    let header = |name: &str, header: &str| {
        let mut f = field(name, string());
        f.lenient = true; // the flow answers 401 itself
        f.source = Some(RequestSource::Header {
            name: header.into(),
        });
        f
    };
    p.messages.push(api(
        "ApiKeyIssue",
        vec![header("token", "Authorization"), field("name", string())],
    ));
    p.messages.push(api(
        "ApiKeyIssued",
        vec![field("key", string()), field("key_id", string())],
    ));
    p.messages.push(api(
        "ApiKeyRevocation",
        vec![header("token", "Authorization"), field("key_id", string())],
    ));
    p.messages.push(api(
        "ApiKeyRevoked",
        vec![field("revoked", FieldType::Bool)],
    ));
    p.messages
        .push(api("MachineCall", vec![header("api_key", "X-API-Key")]));
    p.messages
        .push(api("MachineAnswer", vec![field("owner", string())]));
    let verify_jwt = |id: &str| Step::VerifyJwt {
        id: id.into(),
        token: path("input.token"),
        bearer: true,
        source: JwksSource::Keys {
            key_envs: vec!["JWT_SIGNING_KEY".into()],
        },
        issuer: "orders".into(),
        schema: "AccessClaims".into(),
        bind: "claims".into(),
    };
    let routes = &mut p.services[0].routes;
    routes.push(Route {
        name: "issue_api_key".into(),
        method: HttpMethod::Post,
        path: "/tenant/api-keys".into(),
        request: "ApiKeyIssue".into(),
        response: "ApiKeyIssued".into(),
        status: 201,
        steps: vec![
            verify_jwt("k1"),
            // The key is in the answer and nowhere else: only its hash is kept.
            Step::ApiKeyCreate {
                id: "k2".into(),
                owner: path("claims.tid"),
                name: path("input.name"),
                scopes: vec!["orders:read".into()],
                expires_in_secs: None,
                bind: "api_key".into(),
                key_id_bind: Some("api_key_id".into()),
            },
            Step::Respond {
                id: "k3".into(),
                fields: vec![
                    set("key", path("api_key")),
                    set("key_id", path("api_key_id")),
                ],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    });
    routes.push(Route {
        name: "revoke_api_key".into(),
        method: HttpMethod::Delete,
        path: "/tenant/api-keys/{key_id}".into(),
        request: "ApiKeyRevocation".into(),
        response: "ApiKeyRevoked".into(),
        status: 200,
        steps: vec![
            verify_jwt("k4"),
            // A company revokes its own keys and no one else's.
            Step::ApiKeyRevoke {
                id: "k5".into(),
                key_id: path("input.key_id"),
                owner: Some(path("claims.tid")),
            },
            Step::Respond {
                id: "k6".into(),
                fields: vec![set("revoked", lit(true))],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    });
    routes.push(Route {
        name: "machine_orders".into(),
        method: HttpMethod::Get,
        path: "/machine/orders".into(),
        request: "MachineCall".into(),
        response: "MachineAnswer".into(),
        status: 200,
        steps: vec![
            Step::ApiKeyVerify {
                id: "k7".into(),
                key: path("input.api_key"),
                scope: Some("orders:read".into()),
                bind: "owner".into(),
                key_id_bind: None,
            },
            Step::Respond {
                id: "k8".into(),
                fields: vec![set("owner", path("owner"))],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    });
}

/// A route that tells the calendar of a visitor from the time zone in its path: the day it is for them, when that day began (not a
/// multiple of 86 400 seconds: a day is 23 or 25 hours when the clocks change), the same time next week and next month on their
/// clock.
// (The variable is `instant`, not `now`: in the compact syntax of the editor `now` is the clock of the database.)
pub fn add_calendar(p: &mut Project) {
    p.messages.push(api(
        "CalendarRequest",
        vec![field("zone", FieldType::String)],
    ));
    p.messages.push(api(
        "CalendarAnswer",
        vec![
            field("today", FieldType::String),
            field("day_start", FieldType::Int64),
            field("next_week", FieldType::Int64),
            field("next_month", FieldType::Int64),
            field("paris_now", FieldType::String),
        ],
    ));
    let zone = || path("input.zone");
    p.services[0].routes.push(Route {
        name: "calendar".into(),
        method: HttpMethod::Get,
        path: "/calendar/{zone}".into(),
        request: "CalendarRequest".into(),
        response: "CalendarAnswer".into(),
        status: 200,
        steps: vec![
            Step::Let {
                id: "t1".into(),
                name: "instant".into(),
                value_type: FieldType::Int64,
                optional: false,
                repeated: false,
                value: call(Func::NowSeconds, vec![]),
            },
            Step::Respond {
                id: "t2".into(),
                fields: vec![
                    set(
                        "today",
                        call(Func::LocalDate, vec![path("instant"), zone()]),
                    ),
                    set(
                        "day_start",
                        call(Func::StartOfDay, vec![path("instant"), zone()]),
                    ),
                    set(
                        "next_week",
                        call(Func::AddDays, vec![path("instant"), lit(7), zone()]),
                    ),
                    set(
                        "next_month",
                        call(Func::AddMonths, vec![path("instant"), lit(1), zone()]),
                    ),
                    // A zone written in the flow is checked when the project is: a typo would be a 400 at every call.
                    set(
                        "paris_now",
                        call(Func::LocalTime, vec![path("instant"), lit("Europe/Paris")]),
                    ),
                ],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    });
}

/// An invoice route: the number is taken **last**, right before it is written with, from a sequence of the company and the year
/// (`2026`), so the numbers of a year follow one another without a hole even when a run fails; the reference a person reads out
/// is the short text of the number, and the identifier is a UUID that sorts by date. Needs [`add_jwt`], whose `AccessClaims` and
/// signing key it uses.
pub fn add_invoices(p: &mut Project) {
    let mut token = field("token", FieldType::String);
    token.lenient = true; // the flow answers 401 itself
    token.source = Some(RequestSource::Header {
        name: "Authorization".into(),
    });
    p.messages.push(api("InvoiceRequest", vec![token]));
    p.messages.push(api(
        "InvoiceIssued",
        vec![
            field("number", FieldType::Int64),
            field("reference", FieldType::String),
            field("invoice_id", FieldType::Uuid),
        ],
    ));
    p.services[0].routes.push(Route {
        name: "issue_invoice".into(),
        method: HttpMethod::Post,
        path: "/tenant/invoices".into(),
        request: "InvoiceRequest".into(),
        response: "InvoiceIssued".into(),
        status: 201,
        steps: vec![
            Step::VerifyJwt {
                id: "v1".into(),
                token: path("input.token"),
                bearer: true,
                source: JwksSource::Keys {
                    key_envs: vec!["JWT_SIGNING_KEY".into()],
                },
                issuer: "orders".into(),
                schema: "AccessClaims".into(),
                bind: "claims".into(),
            },
            Step::Let {
                id: "v2".into(),
                name: "invoice_id".into(),
                value_type: FieldType::Uuid,
                optional: false,
                repeated: false,
                value: call(Func::NewUuidV7, vec![]),
            },
            // Last before the write: the runs of this company and year wait for one another from here.
            Step::NextNumber {
                id: "v3".into(),
                sequence: "invoices".into(),
                scope: Some(template("{claims.tid}-2026")),
                start: DEFAULT_SEQUENCE_START,
                bind: "number".into(),
            },
            Step::Let {
                id: "v4".into(),
                name: "reference".into(),
                value_type: FieldType::String,
                optional: false,
                repeated: false,
                value: call(Func::ShortId, vec![path("number")]),
            },
            Step::Respond {
                id: "v5".into(),
                fields: vec![
                    set("number", path("number")),
                    set("reference", path("reference")),
                    set("invoice_id", path("invoice_id")),
                ],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    });
}

/// A metered route: a company gets a thousand calls a month, whatever the number of replicas or of simultaneous requests. The
/// check and the use are in the same run, so two calls of the company cannot both take the last one, and what is left after the
/// call is answered. Needs [`add_jwt`], whose `AccessClaims` and signing key it uses.
pub fn add_usage(p: &mut Project) {
    let mut token = field("token", FieldType::String);
    token.lenient = true; // the flow answers 401 itself
    token.source = Some(RequestSource::Header {
        name: "Authorization".into(),
    });
    p.messages.push(api("UsageRequest", vec![token]));
    p.messages.push(api(
        "UsageView",
        vec![field("calls_left", FieldType::Int64)],
    ));
    p.services[0].routes.push(Route {
        name: "usage_report".into(),
        method: HttpMethod::Get,
        path: "/tenant/usage".into(),
        request: "UsageRequest".into(),
        response: "UsageView".into(),
        status: 200,
        steps: vec![
            Step::VerifyJwt {
                id: "u1".into(),
                token: path("input.token"),
                bearer: true,
                source: JwksSource::Keys {
                    key_envs: vec!["JWT_SIGNING_KEY".into()],
                },
                issuer: "orders".into(),
                schema: "AccessClaims".into(),
                bind: "claims".into(),
            },
            Step::CheckQuota {
                id: "u2".into(),
                key: "api_calls".into(),
                tenant: path("claims.tid"),
                limit: lit(1000),
                period: UsagePeriod::Month,
                wanted: None,
                bind: Some("calls_left".into()),
            },
            Step::Meter {
                id: "u3".into(),
                key: "api_calls".into(),
                tenant: path("claims.tid"),
                amount: lit(1),
                idempotency_key: None,
            },
            Step::Respond {
                id: "u4".into(),
                fields: vec![set("calls_left", path("calls_left"))],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    });
}

/// Adds to the first service notes that belong to a tenant and are never really deleted: the table is filtered by its
/// tenant column and keeps its deleted rows (`softDelete`), so the steps below name neither `tenant_id` nor `deleted_at`.
/// The tenant is the `tid` claim of the verified token, bound to the variable `tenant_id` that the table leans on. It needs
/// [`add_jwt`], whose `AccessClaims` and signing key it uses.
pub fn add_notes(p: &mut Project) {
    let string = || FieldType::String;
    let bearer = || {
        let mut token = field("token", string());
        token.lenient = true; // the flow answers 401 itself
        token.source = Some(RequestSource::Header {
            name: "Authorization".into(),
        });
        token
    };
    p.messages
        .push(api("NoteCreate", vec![bearer(), field("body", string())]));
    p.messages.push(api(
        "NoteRequest",
        vec![bearer(), field("note_id", string())],
    ));
    p.messages.push(api(
        "NoteView",
        vec![field("note_id", string()), field("body", string())],
    ));
    p.messages
        .push(api("NoteDone", vec![field("ok", FieldType::Bool)]));

    let mut note_id = Column::new("note_id", ColumnType::String);
    note_id.primary_key = true;
    let mut deleted_at = Column::new(DELETED_AT, ColumnType::Timestamp);
    deleted_at.nullable = true;
    let mut notes = Table::new(
        "notes",
        vec![
            Column::new("tenant_id", ColumnType::String),
            note_id,
            Column::new("body", ColumnType::String),
            deleted_at,
        ],
    );
    notes.tenant_column = Some("tenant_id".into());
    notes.soft_delete = true;
    p.services[0].tables.push(notes);

    // A token without a tenant would write rows with none: refuse it, then bind the variable the table leans on.
    let tenant = |id: &str, branch: &str, fail: &str| {
        vec![
            Step::VerifyJwt {
                id: id.into(),
                token: path("input.token"),
                bearer: true,
                source: JwksSource::Keys {
                    key_envs: vec!["JWT_SIGNING_KEY".into()],
                },
                issuer: "orders".into(),
                schema: "AccessClaims".into(),
                bind: "claims".into(),
            },
            Step::Branch {
                id: branch.into(),
                condition: Condition::Compare {
                    left: path("claims.tid"),
                    op: CmpOp::Eq,
                    right: lit(""),
                },
                then: vec![Step::Fail {
                    id: fail.into(),
                    status: 403,
                    message: "this account has no tenant".into(),
                    retry_after: None,
                }],
                otherwise: vec![],
            },
            Step::Let {
                id: format!("{id}t"),
                name: "tenant_id".into(),
                value_type: FieldType::String,
                optional: false,
                repeated: false,
                value: path("claims.tid"),
            },
        ]
    };
    let sql = |id: &str,
               op: SqlOp,
               columns: &[&str],
               conditions: Vec<SqlCondition>,
               values: Vec<Assignment>,
               bind: Option<&str>| Step::Sql {
        id: id.into(),
        op,
        table: "notes".into(),
        bind: bind.map(String::from),
        columns: columns.iter().map(|c| c.to_string()).collect(),
        many: false,
        into: None,
        conditions,
        values,
        order_by: None,
        descending: false,
        limit: None,
        missing: bind.map(|_| SqlMissing {
            status: 404,
            message: "no such note".into(),
        }),
        joins: vec![],
        then_by: vec![],
        limit_from: None,
        offset: None,
        cursor: vec![],
        total_bind: None,
        more_bind: None,
        last_bind: None,
    };
    let by_id = || {
        vec![SqlCondition {
            column: "note_id".into(),
            op: CmpOp::Eq,
            value: path("input.note_id"),
            test: Default::default(),
        }]
    };
    let route = |name: &str, method, route: &str, request: &str, response: &str, steps| Route {
        name: name.into(),
        method,
        path: route.into(),
        request: request.into(),
        response: response.into(),
        status: 200,
        steps,
    };
    let routes = &mut p.services[0].routes;

    let mut steps = tenant("n1", "n2", "n3");
    steps.extend([
        Step::Let {
            id: "n4".into(),
            name: "note_id".into(),
            value_type: FieldType::String,
            optional: false,
            repeated: false,
            value: new_uuid(),
        },
        // The body is rich text: what is stored (and answered) has lost its scripts and its event attributes.
        sql(
            "n5",
            SqlOp::Insert,
            &[],
            vec![],
            vec![
                set("note_id", path("note_id")),
                set("body", call(Func::SanitizeHtml, vec![path("input.body")])),
            ],
            None,
        ),
        Step::Respond {
            id: "n6".into(),
            fields: vec![
                set("note_id", path("note_id")),
                set("body", call(Func::SanitizeHtml, vec![path("input.body")])),
            ],
            status: None,
            cookies: vec![],
            headers: vec![],
        },
    ]);
    routes.push(route(
        "create_note",
        HttpMethod::Post,
        "/tenant/notes",
        "NoteCreate",
        "NoteView",
        steps,
    ));

    let mut steps = tenant("n7", "n8", "n9");
    steps.extend([
        sql(
            "n10",
            SqlOp::Select,
            &["note_id", "body"],
            by_id(),
            vec![],
            Some("note"),
        ),
        Step::Respond {
            id: "n11".into(),
            fields: vec![
                set("note_id", path("note.note_id")),
                set("body", path("note.body")),
            ],
            status: None,
            cookies: vec![],
            headers: vec![],
        },
    ]);
    routes.push(route(
        "get_note",
        HttpMethod::Get,
        "/tenant/notes/{note_id}",
        "NoteRequest",
        "NoteView",
        steps,
    ));

    let mut steps = tenant("n12", "n13", "n14");
    steps.extend([
        // A delete only marks the row; `RETURNING` says there was one.
        sql(
            "n15",
            SqlOp::Delete,
            &["note_id"],
            by_id(),
            vec![],
            Some("gone"),
        ),
        Step::Respond {
            id: "n16".into(),
            fields: vec![set("ok", lit(true))],
            status: None,
            cookies: vec![],
            headers: vec![],
        },
    ]);
    routes.push(route(
        "delete_note",
        HttpMethod::Delete,
        "/tenant/notes/{note_id}",
        "NoteRequest",
        "NoteDone",
        steps,
    ));
}

/// Adds to the first service a page of posts with their author: two tables of a tenant (soft delete too), a left join along
/// the foreign key, a text search, a sort on two columns, and a page cut by a keyset cursor taken from the request, with the
/// total and whether a page follows. The tenant and `deleted_at` rules of both tables are applied by the generator. It needs
/// [`add_jwt`], whose `AccessClaims` and signing key it uses.
pub fn add_sql_reads(p: &mut Project) {
    let string = || FieldType::String;
    let bearer = || {
        let mut token = field("token", string());
        token.lenient = true; // the flow answers 401 itself
        token.source = Some(RequestSource::Header {
            name: "Authorization".into(),
        });
        token
    };
    let optional = |name: &str, ty: FieldType| {
        let mut f = field(name, ty);
        f.optional = true;
        f
    };
    let mut repeated_items = msg_field("items", "PostItem");
    repeated_items.repeated = true;
    p.messages.push(api(
        "PostsQuery",
        vec![
            bearer(),
            // Absent, the search is empty and matches every title.
            {
                let mut search = field("search", string());
                search.lenient = true;
                search
            },
            optional("page_size", FieldType::Int32),
            optional("after_title", string()),
            optional("after_id", string()),
        ],
    ));
    p.messages.push(api(
        "PostItem",
        vec![
            field("post_id", string()),
            field("title", string()),
            optional("authors_name", string()),
        ],
    ));
    p.messages.push(api(
        "PostsPage",
        vec![
            repeated_items,
            field("total", FieldType::Int64),
            field("more", FieldType::Bool),
            optional("next_title", string()),
            optional("next_id", string()),
        ],
    ));

    let tenant_table = |name: &str, columns: Vec<Column>| {
        let mut t = Table::new(name, columns);
        t.tenant_column = Some("tenant_id".into());
        t.soft_delete = true;
        t
    };
    let deleted_at = || {
        let mut c = Column::new(DELETED_AT, ColumnType::Timestamp);
        c.nullable = true;
        c
    };
    let mut author_id = Column::new("author_id", ColumnType::String);
    author_id.primary_key = true;
    let authors = tenant_table(
        "authors",
        vec![
            Column::new("tenant_id", ColumnType::String),
            author_id,
            Column::new("name", ColumnType::String),
            deleted_at(),
        ],
    );
    let mut post_id = Column::new("post_id", ColumnType::String);
    post_id.primary_key = true;
    let mut author = Column::new("author_id", ColumnType::String);
    author.nullable = true;
    let mut posts = tenant_table(
        "posts",
        vec![
            Column::new("tenant_id", ColumnType::String),
            post_id,
            Column::new("title", ColumnType::String),
            author,
            deleted_at(),
        ],
    );
    posts.foreign_keys.push(ForeignKey {
        name: "fk_posts_author".into(),
        columns: vec!["author_id".into()],
        ref_table: "authors".into(),
        ref_columns: vec!["author_id".into()],
        on_delete: ReferentialAction::NoAction,
        on_update: ReferentialAction::NoAction,
    });
    p.services[0].tables.push(authors);
    p.services[0].tables.push(posts);

    let steps = vec![
        Step::VerifyJwt {
            id: "q1".into(),
            token: path("input.token"),
            bearer: true,
            source: JwksSource::Keys {
                key_envs: vec!["JWT_SIGNING_KEY".into()],
            },
            issuer: "orders".into(),
            schema: "AccessClaims".into(),
            bind: "claims".into(),
        },
        Step::Branch {
            id: "q2".into(),
            condition: Condition::Compare {
                left: path("claims.tid"),
                op: CmpOp::Eq,
                right: lit(""),
            },
            then: vec![Step::Fail {
                id: "q3".into(),
                status: 403,
                message: "this account has no tenant".into(),
                retry_after: None,
            }],
            otherwise: vec![],
        },
        Step::Let {
            id: "q4".into(),
            name: "tenant_id".into(),
            value_type: FieldType::String,
            optional: false,
            repeated: false,
            value: path("claims.tid"),
        },
        Step::Sql {
            id: "q5".into(),
            op: SqlOp::Select,
            table: "posts".into(),
            bind: Some("posts".into()),
            columns: vec!["post_id".into(), "title".into(), "authors.name".into()],
            many: true,
            into: Some("PostItem".into()),
            conditions: vec![SqlCondition {
                column: "title".into(),
                op: CmpOp::Eq,
                value: call(Func::Lower, vec![path("input.search")]),
                test: SqlTest::Contains,
            }],
            values: vec![],
            order_by: Some("title".into()),
            descending: false,
            limit: Some(50),
            missing: None,
            joins: vec![SqlJoin {
                kind: SqlJoinKind::Left,
                table: "authors".into(),
                alias: None,
                on: vec![SqlJoinOn {
                    column: "author_id".into(),
                    ref_column: "author_id".into(),
                }],
            }],
            then_by: vec![SqlSort {
                column: "post_id".into(),
                descending: false,
            }],
            // An absent page size is 20: an operation stands where a number is expected.
            limit_from: Some(call(Func::Default, vec![path("input.page_size"), lit(20)])),
            offset: None,
            cursor: vec![path("input.after_title"), path("input.after_id")],
            total_bind: Some("total".into()),
            more_bind: Some("more".into()),
            last_bind: Some("last".into()),
        },
        Step::Respond {
            id: "q6".into(),
            fields: vec![
                set("items", path("posts")),
                set("total", path("total")),
                set("more", path("more")),
                set("next_title", path("last.title")),
                set("next_id", path("last.post_id")),
            ],
            status: None,
            cookies: vec![],
            headers: vec![],
        },
    ];
    p.services[0].routes.push(Route {
        name: "list_posts".into(),
        method: HttpMethod::Get,
        path: "/tenant/posts".into(),
        request: "PostsQuery".into(),
        response: "PostsPage".into(),
        status: 200,
        steps,
    });
}

/// Adds a mail service, `Mailer`, and what feeds it: `signup` (see [`add_signup`]) now publishes `accounts.registered`, and
/// `Mailer` mails each of those events with a welcome. The service is a ready-made one (`kind: mta`): its consumer has a
/// mail rule instead of steps, and the mail is a template of the service. It needs [`add_signup`].
pub fn add_mailer(p: &mut Project) {
    let string = || FieldType::String;
    p.messages.push(event(
        "AccountRegistered",
        vec![
            field("user_id", string()),
            field("email", string()),
            field("name", string()),
        ],
    ));
    p.topics.push(topic(
        "accounts.registered",
        "AccountRegistered",
        "user_id",
        "Orders",
        460.0,
        460.0,
    ));
    let signup = p.services[0]
        .routes
        .iter_mut()
        .find(|r| r.name == "signup")
        .expect("add_signup first");
    // Before the answer: the event is queued with the transaction, and goes out once the account is.
    signup.steps.insert(
        1,
        Step::Let {
            id: "m1".into(),
            name: "user_id".into(),
            value_type: FieldType::String,
            optional: false,
            repeated: false,
            value: new_uuid(),
        },
    );
    signup.steps.insert(
        2,
        Step::Publish {
            id: "m2".into(),
            topic: "accounts.registered".into(),
            payload: vec![
                set("user_id", path("user_id")),
                set("email", path("input.email")),
                set("name", path("input.company.name")),
            ],
        },
    );
    p.services.push(Service {
        name: "Mailer".into(),
        port: 50054,
        rpcs: vec![],
        http_port: Some(8084),
        routes: vec![],
        tables: vec![],
        dropped_tables: vec![],
        consumers: vec![Consumer {
            name: "welcome".into(),
            topic: "accounts.registered".into(),
            group_id: None,
            idempotent_on: None,
            mail: Some(MailRule {
                recipient: "email".into(),
                optional_recipient: false,
                dedup_field: "user_id".into(),
                dedup_fallback: DedupFallback::Recipient,
                sender_company_via: Some("name".into()),
                reply_to: None,
            }),
            steps: vec![],
        }],
        jobs: vec![],
        position: Position { x: 920.0, y: 460.0 },
        kind: ServiceKind::Mta,
        templates: vec![MailTemplate {
            event_type: "accounts.registered".into(),
            subject: "Welcome, {{ name }}".into(),
            html: "<p>Hello {{ name }},</p>\n<p>Your account <b>{{ email }}</b> is ready.</p>\n"
                .into(),
            text: "Hello {{ name }},\n\nYour account {{ email }} is ready.\n".into(),
            design: None,
        }],
        realtime: None,
        challenge: None,
        compression: false,
    });
}

/// Makes the welcome mail of `Mailer` a composed one: a heading, a paragraph, a button, a divider, two columns and a footer,
/// with variables of the event in the texts. The hand-written HTML and text it replaces are gone from the project (they
/// are derived). It needs [`add_mailer`].
pub fn add_mail_design(p: &mut Project) {
    use crate::maildesign::{MailBlock, MailColumn, MailDesign, MailSettings};
    let mailer = p
        .services
        .iter_mut()
        .find(|s| s.name == "Mailer")
        .expect("add_mailer first");
    let template = &mut mailer.templates[0];
    template.html.clear();
    template.text.clear();
    template.design = Some(MailDesign {
        settings: MailSettings {
            preheader: "Your account {{ email }} is ready".into(),
            footer: "You receive this mail because {{ email }} signed up.".into(),
            ..Default::default()
        },
        blocks: vec![
            MailBlock::heading("Welcome, {{ name }}"),
            MailBlock::paragraph("Your account {{ email }} is ready."),
            MailBlock::button("Open my account", "https://example.com/login"),
            MailBlock::divider(),
            MailBlock::Columns {
                columns: vec![
                    MailColumn {
                        blocks: vec![MailBlock::paragraph("Questions? Reply to this mail.")],
                    },
                    MailColumn {
                        blocks: vec![MailBlock::image(
                            "https://example.com/logo.png",
                            "Company logo",
                        )],
                    },
                ],
            },
        ],
    });
}

/// Adds a ready-made real-time service (`kind: realtime`): the browsers open `GET /v1/events` and the API's events,
/// published on a channel of DragonflyDB, are pushed to the ones they are for.
pub fn add_realtime(p: &mut Project) {
    p.services.push(Service {
        name: "Realtime".into(),
        port: 50055,
        rpcs: vec![],
        http_port: Some(8085),
        routes: vec![],
        tables: vec![],
        dropped_tables: vec![],
        consumers: vec![],
        jobs: vec![],
        position: Position { x: 920.0, y: 640.0 },
        kind: ServiceKind::Realtime,
        templates: vec![],
        realtime: Some(RealtimeConfig::default()),
        challenge: None,
        compression: false,
    });
}

/// Adds a real-time service and makes `Orders.CreateOrder` push `orders.created` to the browsers that may view orders: the
/// event is sent through the channel of that service once the order is committed.
pub fn add_live_events(p: &mut Project) {
    if !p.services.iter().any(|s| s.kind == ServiceKind::Realtime) {
        add_realtime(p);
    }
    let orders = p
        .services
        .iter_mut()
        .find(|s| s.name == "Orders")
        .expect("the Orders service");
    let create = orders
        .rpcs
        .iter_mut()
        .find(|r| r.name == "CreateOrder")
        .expect("the CreateOrder RPC");
    let at = create
        .steps
        .iter()
        .position(|s| matches!(s, Step::Respond { .. }))
        .unwrap_or(create.steps.len());
    create.steps.insert(
        at,
        Step::PublishLive {
            id: "o3b".into(),
            service: "Realtime".into(),
            event: "orders.created".into(),
            account_id: None,
            tenant_id: None,
            permission: Some("orders.view".into()),
            account_ids: None,
            origin: None,
            data: vec![set("id", path("order_id"))],
        },
    );
}

/// Adds to the first service the logic of a flow without a line of code: a route that takes a list of order lines,
/// checks each one with conditions (`or`, `empty`, comparisons), stores it in a loop (`forEach`: one insert per
/// element, all in the transaction of the request, so that one refused line undoes the others), building the text and
/// the default of a column with operations (`lower`, `trim`, `default`, a template); and a second route that loops over
/// the rows of a `many` query. The answers count with `listLength` and `concat`/`toText`.
pub fn add_basket_lines(p: &mut Project) {
    let string = || FieldType::String;
    let mut sku = field("sku", string());
    sku.optional = false;
    let mut note = field("note", string());
    note.optional = true;
    let mut lines = msg_field("lines", "OrderLine");
    lines.repeated = true;
    // Absent, the list is empty and the flow refuses it with its own message.
    lines.lenient = true;
    let mut coupon = field("coupon", string());
    coupon.optional = true;
    p.messages.push(api(
        "OrderLine",
        vec![sku, field("qty", FieldType::Int32), note],
    ));
    p.messages.push(api(
        "AddLines",
        vec![field("order_id", string()), lines, coupon],
    ));
    p.messages
        .push(api("OrderRef", vec![field("order_id", string())]));
    p.messages.push(api(
        "AddedLines",
        vec![field("count", FieldType::Int64), field("summary", string())],
    ));

    let mut lnote = Column::new("note", ColumnType::String);
    lnote.nullable = true;
    let mut line_id = Column::new("line_id", ColumnType::String);
    line_id.primary_key = true;
    p.services[0].tables.push(Table::new(
        "basket_lines",
        vec![
            line_id,
            Column::new("order_id", ColumnType::String),
            Column::new("sku", ColumnType::String),
            Column::new("qty", ColumnType::Int32),
            Column::new("label", ColumnType::String),
            lnote,
        ],
    ));

    let invalid_line = Condition::Or {
        conditions: vec![
            Condition::Compare {
                left: path("line.qty"),
                op: CmpOp::Le,
                right: lit(0),
            },
            Condition::Compare {
                left: path("line.qty"),
                op: CmpOp::Gt,
                right: lit(1000),
            },
            Condition::Empty {
                value: call(Func::Trim, vec![path("line.sku")]),
            },
        ],
    };
    let steps = vec![
        Step::Branch {
            id: "l1".into(),
            condition: Condition::Empty {
                value: path("input.lines"),
            },
            then: vec![Step::Fail {
                id: "l2".into(),
                status: 400,
                message: "an order has at least one line".into(),
                retry_after: None,
            }],
            otherwise: vec![],
        },
        // A coupon is optional: absent, a fixed word stands for it.
        Step::Let {
            id: "l3".into(),
            name: "coupon".into(),
            value_type: string(),
            optional: false,
            repeated: false,
            value: call(
                Func::Upper,
                vec![call(Func::Default, vec![path("input.coupon"), lit("none")])],
            ),
        },
        Step::ForEach {
            id: "l4".into(),
            item: "line".into(),
            list: path("input.lines"),
            steps: vec![
                Step::Branch {
                    id: "l5".into(),
                    condition: invalid_line,
                    then: vec![Step::Fail {
                        id: "l6".into(),
                        status: 400,
                        message: "a line needs a sku and a quantity from 1 to 1000".into(),
                        retry_after: None,
                    }],
                    otherwise: vec![],
                },
                Step::Sql {
                    id: "l7".into(),
                    op: SqlOp::Insert,
                    table: "basket_lines".into(),
                    bind: None,
                    columns: vec![],
                    many: false,
                    into: None,
                    conditions: vec![],
                    values: vec![
                        set("line_id", new_uuid()),
                        set("order_id", path("input.order_id")),
                        set(
                            "sku",
                            call(Func::Lower, vec![call(Func::Trim, vec![path("line.sku")])]),
                        ),
                        set("qty", path("line.qty")),
                        set("label", template("{input.order_id}/{line.sku} ({coupon})")),
                        set(
                            "note",
                            call(Func::Default, vec![path("line.note"), lit("-")]),
                        ),
                    ],
                    order_by: None,
                    descending: false,
                    limit: None,
                    missing: None,
                    joins: vec![],
                    then_by: vec![],
                    limit_from: None,
                    offset: None,
                    cursor: vec![],
                    total_bind: None,
                    more_bind: None,
                    last_bind: None,
                },
            ],
        },
        Step::Respond {
            id: "l8".into(),
            fields: vec![
                set("count", call(Func::ListLength, vec![path("input.lines")])),
                set(
                    "summary",
                    call(
                        Func::Concat,
                        vec![
                            lit("added "),
                            call(
                                Func::ToText,
                                vec![call(Func::ListLength, vec![path("input.lines")])],
                            ),
                            lit(" lines"),
                        ],
                    ),
                ),
            ],
            status: None,
            cookies: vec![],
            headers: vec![],
        },
    ];
    p.services[0].routes.push(Route {
        name: "add_lines".into(),
        method: HttpMethod::Post,
        path: "/orders/{order_id}/lines".into(),
        request: "AddLines".into(),
        response: "AddedLines".into(),
        status: 200,
        steps,
    });

    let steps = vec![
        Step::Sql {
            id: "l9".into(),
            op: SqlOp::Select,
            table: "basket_lines".into(),
            bind: Some("found".into()),
            columns: vec!["sku".into(), "qty".into(), "note".into()],
            many: true,
            into: Some("OrderLine".into()),
            conditions: vec![SqlCondition {
                column: "order_id".into(),
                op: CmpOp::Eq,
                value: path("input.order_id"),
                test: SqlTest::Compare,
            }],
            values: vec![],
            order_by: Some("sku".into()),
            descending: false,
            limit: None,
            missing: None,
            joins: vec![],
            then_by: vec![],
            limit_from: None,
            offset: None,
            cursor: vec![],
            total_bind: None,
            more_bind: None,
            last_bind: None,
        },
        Step::ForEach {
            id: "l10".into(),
            item: "row".into(),
            list: path("found"),
            steps: vec![Step::Branch {
                id: "l11".into(),
                condition: Condition::Present {
                    value: path("row.note"),
                },
                then: vec![Step::Log {
                    id: "l12".into(),
                    level: LogLevel::Info,
                    message: "line {row.sku} x {row.qty} has a note".into(),
                }],
                otherwise: vec![Step::Log {
                    id: "l13".into(),
                    level: LogLevel::Info,
                    message: "line {row.sku} x {row.qty}".into(),
                }],
            }],
        },
        Step::Respond {
            id: "l14".into(),
            fields: vec![
                set("count", call(Func::ListLength, vec![path("found")])),
                set("summary", template("lines of {input.order_id}")),
            ],
            status: None,
            cookies: vec![],
            headers: vec![],
        },
    ];
    p.services[0].routes.push(Route {
        name: "get_lines".into(),
        method: HttpMethod::Get,
        path: "/orders/{order_id}/lines".into(),
        request: "OrderRef".into(),
        response: "AddedLines".into(),
        status: 200,
        steps,
    });
}

/// Adds to the first service a receipt route that uses what the HTTP API carries beyond text and numbers: a
/// `timestamp` (RFC 3339 in JSON) and `bytes` (base64), a field that may be left out (`lenient`: it takes its default
/// instead of a 400), and free response headers whose values come from the flow.
pub fn add_receipts(p: &mut Project) {
    // The services also answer `GET /openapi.json` with the document of their routes.
    p.openapi.serve = true;
    let mut note = field("note", FieldType::String);
    note.lenient = true;
    note.rules = vec![Rule::Length { min: 0, max: 200 }];
    p.messages.push(api(
        "ReceiptBody",
        vec![
            field("order_id", FieldType::String),
            field("issued_at", FieldType::Timestamp),
            field("stamp", FieldType::Bytes),
            note,
        ],
    ));
    p.messages.push(api(
        "ReceiptView",
        vec![
            field("order_id", FieldType::String),
            field("issued_at", FieldType::Timestamp),
            field("stamp", FieldType::Bytes),
        ],
    ));
    p.services[0].routes.push(Route {
        name: "put_receipt".into(),
        method: HttpMethod::Put,
        path: "/orders/{order_id}/receipt".into(),
        request: "ReceiptBody".into(),
        response: "ReceiptView".into(),
        status: 200,
        steps: vec![Step::Respond {
            id: "rc1".into(),
            fields: vec![
                set("order_id", path("input.order_id")),
                set("issued_at", path("input.issued_at")),
                set("stamp", path("input.stamp")),
            ],
            status: None,
            cookies: vec![],
            // The browser must not keep a receipt, and the client learns where it lives.
            headers: vec![
                ResponseHeader {
                    name: "Cache-Control".into(),
                    value: lit("no-store"),
                },
                ResponseHeader {
                    name: "X-Order-Id".into(),
                    value: path("input.order_id"),
                },
                // A text with a value in it: the new expressions go where a response header takes a value.
                ResponseHeader {
                    name: "X-Receipt-Ref".into(),
                    value: template("receipt-{input.order_id}"),
                },
            ],
        }],
    });
}

/// Adds to the first service two calls to external APIs, with no code: Stripe "create a PaymentIntent" (a form body,
/// the secret key as a bearer token, the order as the idempotency key so that a replay charges once) and Twilio "send an
/// SMS" (basic authentication with the account SID and the auth token, a form body, the status read to decide).
/// `STRIPE_API_URL` is `https://api.stripe.com`; `TWILIO_API_URL` carries the account in its path
/// (`https://api.twilio.com/2010-04-01/Accounts/AC…`).
pub fn add_http_calls(p: &mut Project) {
    let string = || FieldType::String;
    p.messages.push(api(
        "PaymentIntentRequest",
        vec![
            field("order_id", string()),
            field("amount", FieldType::Int64),
            field("currency", string()),
        ],
    ));
    p.messages.push(api(
        "StripePaymentIntent",
        vec![
            field("id", string()),
            field("status", string()),
            field("client_secret", string()),
        ],
    ));
    p.messages.push(api(
        "PaymentIntentView",
        vec![
            field("payment_id", string()),
            field("client_secret", string()),
        ],
    ));
    p.messages.push(api(
        "SmsRequest",
        vec![field("to", string()), field("text", string())],
    ));
    p.messages.push(api(
        "TwilioMessage",
        vec![field("sid", string()), field("status", string())],
    ));
    p.messages.push(api("SmsAck", vec![field("sid", string())]));
    let routes = &mut p.services[0].routes;
    routes.push(Route {
        name: "create_payment_intent".into(),
        method: HttpMethod::Post,
        path: "/payments/intents".into(),
        request: "PaymentIntentRequest".into(),
        response: "PaymentIntentView".into(),
        status: 201,
        steps: vec![
            Step::HttpCall {
                id: "i1".into(),
                method: HttpMethod::Post,
                base_url_env: "STRIPE_API_URL".into(),
                path: "/v1/payment_intents".into(),
                query: vec![],
                headers: vec![],
                auth: Some(HttpAuthKind::Bearer),
                auth_env: Some("STRIPE_SECRET_KEY".into()),
                auth_password_env: None,
                auth_header: None,
                body: vec![
                    set("amount", path("input.amount")),
                    set("currency", path("input.currency")),
                    set("metadata[order_id]", path("input.order_id")),
                ],
                body_format: HttpBodyFormat::Form,
                timeout_secs: 10,
                retries: 2,
                idempotency_key: Some(path("input.order_id")),
                response: Some("StripePaymentIntent".into()),
                bind: Some("intent".into()),
                status_bind: None,
                on_error: HttpOnError::Fail,
                error_status: 502,
                error_message: "the payment provider did not accept the payment".into(),
                // Five failures in thirty seconds cut the calls for thirty seconds; twenty payments at a time at most.
                circuit: Some(CircuitConfig::default()),
                max_concurrent: Some(20),
            },
            Step::Respond {
                id: "i2".into(),
                fields: vec![
                    set("payment_id", path("intent.id")),
                    set("client_secret", path("intent.client_secret")),
                ],
                status: None,
                cookies: vec![],
                headers: vec![],
            },
        ],
    });
    routes.push(Route {
        name: "send_sms".into(),
        method: HttpMethod::Post,
        path: "/sms".into(),
        request: "SmsRequest".into(),
        response: "SmsAck".into(),
        status: 201,
        steps: vec![
            Step::HttpCall {
                id: "s1".into(),
                method: HttpMethod::Post,
                base_url_env: "TWILIO_API_URL".into(),
                path: "/Messages.json".into(),
                query: vec![],
                headers: vec![],
                auth: Some(HttpAuthKind::Basic),
                auth_env: Some("TWILIO_ACCOUNT_SID".into()),
                auth_password_env: Some("TWILIO_AUTH_TOKEN".into()),
                auth_header: None,
                body: vec![
                    set("To", path("input.to")),
                    set("From", lit("+15005550006")),
                    set("Body", path("input.text")),
                ],
                body_format: HttpBodyFormat::Form,
                timeout_secs: 10,
                // A message sent twice is two messages: no idempotency key here, so no retry.
                retries: 0,
                idempotency_key: None,
                response: Some("TwilioMessage".into()),
                bind: Some("message".into()),
                status_bind: Some("sms_status".into()),
                on_error: HttpOnError::Continue,
                error_status: 502,
                error_message: "the SMS provider did not answer correctly".into(),
                // An open circuit is a call nobody answered (status 0): the branch below answers a 502 at once.
                circuit: Some(CircuitConfig {
                    failures: 3,
                    window_secs: 60,
                    open_secs: 20,
                }),
                max_concurrent: None,
            },
            Step::Branch {
                id: "s2".into(),
                condition: Condition::Compare {
                    left: path("sms_status"),
                    op: CmpOp::Eq,
                    right: lit(201),
                },
                then: vec![Step::Respond {
                    id: "s3".into(),
                    fields: vec![set("sid", path("message.sid"))],
                    status: None,
                    cookies: vec![],
                    headers: vec![],
                }],
                otherwise: vec![Step::Fail {
                    id: "s4".into(),
                    status: 502,
                    message: "the SMS provider refused the message".into(),
                    retry_after: None,
                }],
            },
        ],
    });
}
