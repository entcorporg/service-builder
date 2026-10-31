//! Turns a validated [`sb_model::Project`] into a Cargo workspace of Rust services:
//! protobuf contracts, one tonic gRPC server + rdkafka consumers per service, and a
//! docker-compose for Kafka.

mod expr;
mod proto;
pub mod rawcheck;
mod service;
mod workspace;
mod writer;

use std::fs;
use std::io;
use std::path::Path;

use sb_model::{Diagnostic, Project, has_errors};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GeneratedFile {
    /// Path relative to the workspace root, always `/`-separated.
    pub path: String,
    pub content: String,
}

impl GeneratedFile {
    fn new(path: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CodegenError {
    #[error("the project has {} error(s)", .0.iter().filter(|d| d.severity == sb_model::Severity::Error).count())]
    Invalid(Vec<Diagnostic>),
    #[error("internal generator error: {0}")]
    Internal(String),
}

/// Full diagnostics of a project: model validation plus Rust syntax of raw blocks.
pub fn diagnose(project: &Project) -> Vec<Diagnostic> {
    let mut d = sb_model::validate(project);
    d.extend(rawcheck::check(project));
    d
}

/// Generates the whole workspace. Fails with the diagnostics if the project has errors.
pub fn generate(project: &Project) -> Result<Vec<GeneratedFile>, CodegenError> {
    let diagnostics = diagnose(project);
    if has_errors(&diagnostics) {
        return Err(CodegenError::Invalid(diagnostics));
    }
    // The rows read by SQL steps are typed like messages (never written to the .proto).
    let typed = project.with_sql_rows();
    let project = &typed;
    let mut files = workspace::files(project);
    for svc in &project.services {
        files.extend(service::generate(project, svc)?);
    }
    // Safety net: a generator bug must surface here, not as a cryptic compiler error later.
    for f in files.iter().filter(|f| f.path.ends_with(".rs")) {
        syn::parse_file(&f.content)
            .map_err(|e| CodegenError::Internal(format!("{} is not valid Rust: {e}", f.path)))?;
    }
    Ok(files)
}

/// Directory of a service inside the generated workspace (`services/orders`).
pub fn service_dir(service: &sb_model::Service) -> String {
    service::dir(service)
}

/// Formats the Rust files with `rustfmt` when it is installed (best effort: files are left
/// untouched otherwise). Kept out of [`generate`] so generation stays pure and deterministic.
/// Files are formatted in parallel since this runs after every edit in the editor.
pub fn rustfmt(files: &mut [GeneratedFile]) {
    std::thread::scope(|scope| {
        for f in files.iter_mut().filter(|f| f.path.ends_with(".rs")) {
            scope.spawn(move || {
                if let Some(formatted) = rustfmt_one(&f.content) {
                    f.content = formatted;
                }
            });
        }
    });
}

fn rustfmt_one(source: &str) -> Option<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Feed stdin from another thread so a large file cannot deadlock against the stdout pipe.
    let mut stdin = child.stdin.take()?;
    let input = source.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child.wait_with_output().ok()?;
    let _ = writer.join();
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Writes the files under `dir`, creating directories as needed. Existing files with the
/// same path are overwritten; other files are left alone.
pub fn write_to_dir(files: &[GeneratedFile], dir: &Path) -> io::Result<()> {
    for f in files {
        let target = dir.join(&f.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, &f.content)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_model::sample;

    fn file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
        &files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("missing {path}"))
            .content
    }

    #[test]
    fn generates_expected_layout() {
        let files = generate(&sample::orders()).unwrap();
        let mut paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                ".gitignore",
                "Cargo.toml",
                "README.md",
                "contracts/Cargo.toml",
                "contracts/build.rs",
                "contracts/proto/orders_demo.proto",
                "contracts/src/lib.rs",
                "docker-compose.yml",
                "runtime/Cargo.toml",
                "runtime/src/crypto.rs",
                "runtime/src/db.rs",
                "runtime/src/dedup.rs",
                "runtime/src/jobs.rs",
                "runtime/src/jwt.rs",
                "runtime/src/lib.rs",
                "runtime/src/outbox.rs",
                "runtime/src/rate_limit.rs",
                "runtime/src/totp.rs",
                "runtime/src/validation.rs",
                "runtime/src/webauthn.rs",
                "runtime/src/webhook.rs",
                "services/notifications/Cargo.toml",
                "services/notifications/src/app/bootstrap.rs",
                "services/notifications/src/app/config.rs",
                "services/notifications/src/app/ctx.rs",
                "services/notifications/src/app/mod.rs",
                "services/notifications/src/flows/consumers.rs",
                "services/notifications/src/flows/mod.rs",
                "services/notifications/src/main.rs",
                "services/notifications/src/transport/kafka.rs",
                "services/notifications/src/transport/mod.rs",
                "services/orders/Cargo.toml",
                "services/orders/src/app/bootstrap.rs",
                "services/orders/src/app/config.rs",
                "services/orders/src/app/ctx.rs",
                "services/orders/src/app/mod.rs",
                "services/orders/src/app/schema.rs",
                "services/orders/src/flows/api.rs",
                "services/orders/src/flows/jobs.rs",
                "services/orders/src/flows/mod.rs",
                "services/orders/src/flows/rpc.rs",
                "services/orders/src/main.rs",
                "services/orders/src/transport/grpc.rs",
                "services/orders/src/transport/http.rs",
                "services/orders/src/transport/mod.rs",
                "services/orders/src/transport/outbox.rs",
                "services/orders/src/transport/scheduler.rs",
                "services/payments/Cargo.toml",
                "services/payments/src/app/bootstrap.rs",
                "services/payments/src/app/config.rs",
                "services/payments/src/app/ctx.rs",
                "services/payments/src/app/mod.rs",
                "services/payments/src/app/schema.rs",
                "services/payments/src/flows/consumers.rs",
                "services/payments/src/flows/mod.rs",
                "services/payments/src/main.rs",
                "services/payments/src/transport/kafka.rs",
                "services/payments/src/transport/mod.rs",
                "services/payments/src/transport/outbox.rs",
            ]
        );
    }

    #[test]
    fn rpc_flow_snapshot() {
        let files = generate(&sample::orders()).unwrap();
        let flows = file(&files, "services/orders/src/flows/rpc.rs");
        assert!(
            flows.contains("pub async fn create_order(ctx: &Ctx, input: pb::CreateOrderRequest)")
        );
        assert!(flows.contains("let order_id: String = eda_runtime::new_id();"));
        assert!(flows.contains(
            "eda_runtime::outbox::enqueue(&mut tx, topics::ORDERS_CREATED, Some(&key), &event)"
        ));
        assert!(
            !flows.contains("ctx.producer"),
            "flows never talk to Kafka directly"
        );
        assert!(flows.contains("topics::ORDERS_CREATED"));
        assert!(flows.contains("let mut tx = ctx.db.begin().await?;"));
        // The transaction (state + queued events) is committed before the RPC answers.
        let commit = flows.find("tx.commit().await?;").unwrap();
        assert!(commit < flows.find("return Ok(reply);").unwrap());
        assert!(flows.contains("// publish to orders.created"));
    }

    #[test]
    fn consumer_flow_uses_branch_and_rpc_client() {
        let files = generate(&sample::orders()).unwrap();
        let payments = file(&files, "services/payments/src/flows/consumers.rs");
        assert!(payments.contains("if (approved.clone()) == (true) {"));
        assert!(payments.contains("} else {"));
        let notifications = file(&files, "services/notifications/src/flows/consumers.rs");
        assert!(notifications.contains("ctx.orders.clone().cancel_order("));
        let ctx = file(&files, "services/notifications/src/app/ctx.rs");
        assert!(
            ctx.contains("pub orders: pb::orders_client::OrdersClient<tonic::transport::Channel>")
        );
        assert!(ctx.contains("ORDERS_URL"));
    }

    #[test]
    fn main_only_sequences_the_start_up() {
        let files = generate(&sample::orders()).unwrap();
        let orders = file(&files, "services/orders/src/main.rs");
        // Orders has both an RPC and HTTP routes: the two servers run side by side.
        assert!(orders.contains("transport::grpc::serve(&config, ctx.clone())"));
        assert!(orders.contains("transport::http::serve(&config, ctx)"));
        assert!(!orders.contains("spawn_consumers"));
        let payments = file(&files, "services/payments/src/main.rs");
        assert!(payments.contains("transport::kafka::spawn_consumers(&config, &ctx)"));
        assert!(payments.contains("app::bootstrap::wait_for_shutdown()"));
        // No topic, group or message type in `main`: that is the transport's business.
        for needle in [
            "eda_runtime::consume",
            "topics::",
            "pb::",
            "tonic",
            "ctrl_c",
        ] {
            assert!(!payments.contains(needle), "main.rs mentions {needle}");
            assert!(!orders.contains(needle), "main.rs mentions {needle}");
        }
    }

    #[test]
    fn transports_hold_the_wiring() {
        let files = generate(&sample::orders()).unwrap();
        let grpc = file(&files, "services/orders/src/transport/grpc.rs");
        assert!(grpc.contains("OrdersServer::new"));
        assert!(grpc.contains("flows::rpc::create_order"));
        let kafka = file(&files, "services/payments/src/transport/kafka.rs");
        assert!(kafka.contains("eda_runtime::consume::<pb::OrderCreated"));
        assert!(kafka.contains("\"payments-on_order_created\""));
        assert!(kafka.contains("flows::consumers::on_order_created"));
        assert!(file(&files, "services/orders/src/app/config.rs").contains("ORDERS_PORT"));
        assert!(file(&files, "services/payments/src/app/bootstrap.rs").contains("ctrl_c"));
        // Flows know neither Kafka nor tonic wiring.
        let flows = file(&files, "services/payments/src/flows/consumers.rs");
        assert!(!flows.contains("eda_runtime::consume"));
    }

    #[test]
    fn routes_become_axum_handlers_that_delegate_to_flows() {
        let files = generate(&sample::orders()).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(http.contains(".route(\"/orders\", post(handle_place_order))"));
        assert!(http.contains(".route(\"/orders/{order_id}\", get(handle_get_order))"));
        // A body for POST (+ the status of the route), a query string for GET, path parameters read by name.
        assert!(http.contains("Json(input): Json<pb::PlaceOrderBody>"));
        // `place_order` sets a status and a cookie in its Respond: it answers with a `Reply`; `get_order` does not.
        assert!(http.contains(") -> Result<Response, ApiError>"));
        assert!(http.contains(".unwrap_or(StatusCode::CREATED)"));
        assert!(
            http.contains("response.headers_mut().append(axum::http::header::SET_COOKIE, value);")
        );
        assert!(http.contains("(StatusCode::OK, Json(body))"));
        assert!(http.contains("Query(mut input): Query<pb::GetOrderRequest>"));
        assert!(http.contains("input.order_id = param(&path, \"order_id\")?;"));
        assert!(http.contains("flows::api::get_order(&ctx, input)"));
        // Errors: the flow's `HttpError` picks the status, anything else stays out of the answer.
        assert!(http.contains("eda_runtime::http::HttpError"));
        assert!(http.contains("\"internal error\""));

        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains("pub async fn place_order(ctx: &Ctx, input: pb::PlaceOrderBody)"));
        assert!(api.contains("let mut tx = ctx.db.begin().await?;"));
        assert!(api.contains("eda_runtime::outbox::enqueue(&mut tx, topics::ORDERS_CREATED"));
        assert!(api.contains("return Ok(reply);"));
        assert!(api.contains("-> anyhow::Result<eda_runtime::http::Reply<pb::OrderView>>"));
        assert!(api.contains("pub async fn get_order(ctx: &Ctx, input: pb::GetOrderRequest) -> anyhow::Result<pb::OrderView>"));
        assert!(api.contains(
            "return Ok(eda_runtime::http::Reply::new(reply).status(201).cookie(eda_runtime::http::Cookie::new(\"last_order\", order_id.clone()).max_age(3600)));"
        ));
        assert!(api.contains(
            "eda_runtime::rate_limit::check(\"h8\", &(\"place_order\".to_string()), 100, 60)?;"
        ));

        let config = file(&files, "services/orders/src/app/config.rs");
        assert!(config.contains("ORDERS_HTTP_PORT"));
        assert!(config.contains("\"8081\""));
        assert!(config.contains("CORS_ORIGINS"));
        assert!(file(&files, "services/orders/Cargo.toml").contains("axum.workspace = true"));
    }

    #[test]
    fn a_response_can_be_built_by_the_logic_and_read_field_by_field() {
        use sb_model::{Assignment, Expr, FieldType, Step};
        let mut p = sample::orders();
        let path = |p: &str| Expr::Path { path: p.into() };
        p.services[0].routes[1].steps = vec![
            Step::Let {
                id: "v".into(),
                name: "view".into(),
                value_type: FieldType::Message {
                    name: "OrderView".into(),
                },
                optional: false,
                repeated: false,
                value: Expr::Raw {
                    code: "{ pb::OrderView { order_id: input.order_id.clone(), ..Default::default() } }"
                        .into(),
                },
            },
            Step::Respond {
                id: "r".into(),
                fields: vec![
                    Assignment {
                        field: "order_id".into(),
                        value: path("view.order_id"),
                    },
                    Assignment {
                        field: "status".into(),
                        value: path("view.status"),
                    },
                ], status: None, cookies: vec![]},
        ];
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains("let view: pb::OrderView = { pb::OrderView {"));
        assert!(api.contains("order_id: view.order_id.clone(),"));
    }

    #[test]
    fn sql_steps_become_typed_queries_in_the_transaction() {
        let files = generate(&sample::orders()).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        // Insert: the placeholders follow the order of the values.
        assert!(api.contains(
            "INSERT INTO \\\"orders\\\" (\\\"order_id\\\", \\\"amount\\\") VALUES ($1, $2)"
        ));
        assert!(api.contains(".execute(&mut *tx)"));
        // Select one: a row struct of the ticked columns, 404 when nothing matches.
        assert!(api.contains("struct OrdersRowH6 {"));
        assert!(api.contains("WHERE \\\"order_id\\\" = $1 LIMIT 1"));
        assert!(api.contains("fetch_optional(&mut *tx)"));
        assert!(api.contains("HttpError::not_found"));
        assert!(api.contains("order_id: order.order_id.clone(),"));
        // The tables are created at start-up, additively.
        let schema = file(&files, "services/orders/src/app/schema.rs");
        assert!(schema.contains("CREATE TABLE IF NOT EXISTS \\\"orders\\\""));
        assert!(schema.contains("PRIMARY KEY (\\\"order_id\\\")"));
        assert!(
            schema.contains(
                "ADD COLUMN IF NOT EXISTS \\\"status\\\" TEXT NOT NULL DEFAULT 'PENDING'"
            )
        );
        assert!(
            file(&files, "services/orders/src/app/bootstrap.rs")
                .contains("crate::app::schema::apply(&db)")
        );
        // Services without a table, and without an idempotent consumer, get nothing.
        assert!(
            !files
                .iter()
                .any(|f| f.path == "services/notifications/src/app/schema.rs")
        );
    }

    #[test]
    fn timestamps_and_uuids_are_read_as_text_and_written_through_a_cast() {
        use sb_model::{Column, ColumnType, SqlOp, Step};
        let mut p = sample::orders();
        let table = &mut p.services[0].tables[0];
        table.columns.push(Column::new("ext_id", ColumnType::Uuid));
        table.columns[3].nullable = false;
        if let Step::Sql { columns, .. } = &mut p.services[0].routes[1].steps[0] {
            columns.push("created_at".into());
            columns.push("ext_id".into());
        }
        // The new column has to be written by the insert.
        if let Step::Sql {
            op: SqlOp::Insert,
            values,
            ..
        } = &mut p.services[0].routes[0].steps[3]
        {
            values.push(sb_model::Assignment {
                field: "ext_id".into(),
                value: sb_model::Expr::Path {
                    path: "order_id".into(),
                },
            });
            values.push(sb_model::Assignment {
                field: "created_at".into(),
                value: sb_model::Expr::Raw {
                    code: "\"2026-01-01T00:00:00Z\".to_string()".into(),
                },
            });
        }
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains("to_char(\\\"created_at\\\" AT TIME ZONE 'UTC'"));
        assert!(api.contains("\\\"ext_id\\\"::text AS \\\"ext_id\\\""));
        assert!(api.contains("$3::uuid"));
        assert!(api.contains("$4::timestamptz"));
    }

    #[test]
    fn jsonb_columns_are_read_as_text_and_written_through_a_cast() {
        use sb_model::{Column, ColumnType, SqlOp, Step};
        let mut p = sample::orders();
        p.services[0].tables[0]
            .columns
            .push(Column::new("meta", ColumnType::Jsonb));
        if let Step::Sql { columns, .. } = &mut p.services[0].routes[1].steps[0] {
            columns.push("meta".into());
        }
        if let Step::Sql {
            op: SqlOp::Insert,
            values,
            ..
        } = &mut p.services[0].routes[0].steps[3]
        {
            values.push(sb_model::Assignment {
                field: "meta".into(),
                value: sb_model::Expr::Literal {
                    value: r#"{"vip":true}"#.into(),
                },
            });
        }
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains("\\\"meta\\\"::text AS \\\"meta\\\""));
        assert!(api.contains("$3::jsonb"));
        let schema = file(&files, "services/orders/src/app/schema.rs");
        assert!(schema.contains("ADD COLUMN IF NOT EXISTS \\\"meta\\\" JSONB"));
    }

    #[test]
    fn composite_keys_uniques_and_indexes_are_applied_after_the_tables() {
        use sb_model::{
            Column, ColumnType, ForeignKey, Index, ReferentialAction, Table, UniqueConstraint,
        };
        let mut p = sample::orders();
        let key = |name: &str, ty| {
            let mut c = Column::new(name, ty);
            c.primary_key = true;
            c
        };
        // The referencing table comes first: its foreign key must wait for the other table.
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
            on_update: ReferentialAction::Restrict,
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
        shipments.indexes.push(Index {
            name: "shipments_id_order_idx".into(),
            columns: vec!["id".into(), "order_id".into()],
            unique: true,
        });
        let lines = Table::new(
            "order_lines",
            vec![
                key("order_id", ColumnType::String),
                key("line_no", ColumnType::Int32),
            ],
        );
        p.services[0].tables.extend([shipments, lines]);
        assert!(!sb_model::has_errors(&sb_model::validate(&p)));
        let files = generate(&p).unwrap();
        let schema = file(&files, "services/orders/src/app/schema.rs");
        let at = |needle: &str| {
            schema
                .find(needle)
                .unwrap_or_else(|| panic!("`{needle}` is missing from\n{schema}"))
        };
        assert!(schema.contains("PRIMARY KEY (\\\"order_id\\\", \\\"line_no\\\")"));
        assert!(schema.contains(
            "ADD CONSTRAINT \\\"shipments_line_uq\\\" UNIQUE (\\\"order_id\\\", \\\"line_no\\\")"
        ));
        assert!(schema.contains(
            "CREATE INDEX IF NOT EXISTS \\\"shipments_order_idx\\\" ON \\\"shipments\\\" (\\\"order_id\\\")"
        ));
        assert!(schema.contains(
            "CREATE UNIQUE INDEX IF NOT EXISTS \\\"shipments_id_order_idx\\\" ON \\\"shipments\\\" (\\\"id\\\", \\\"order_id\\\")"
        ));
        assert!(schema.contains(
            "FOREIGN KEY (\\\"order_id\\\", \\\"line_no\\\") REFERENCES \\\"order_lines\\\" (\\\"order_id\\\", \\\"line_no\\\") ON DELETE CASCADE ON UPDATE RESTRICT"
        ));
        // Guarded, so that a restart does not fail on a constraint that exists.
        assert!(schema.contains("conname = 'shipments_line_fk'"));
        // Both tables exist before the foreign key that links them.
        assert!(at("CREATE TABLE IF NOT EXISTS \\\"order_lines\\\"") < at("shipments_line_fk"));
        assert!(at("CREATE INDEX IF NOT EXISTS") < at("FOREIGN KEY"));
    }

    #[test]
    fn fail_returns_an_http_error_and_5xx_hide_their_message() {
        let files = generate(&sample::orders()).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        // The sample's `place_order` refuses a non-positive amount; the transaction is dropped by the return.
        assert!(api.contains(
            "let error = eda_runtime::http::HttpError::new(400, \"the amount must be positive\");"
        ));
        assert!(api.contains("return Err(error.into());"));
        // Retry-After and the generic 5xx text live in the answer of the HTTP module.
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(http.contains("retry_after: e.retry_after"));
        assert!(http.contains("status.is_server_error()"));
        assert!(http.contains("axum::http::header::RETRY_AFTER"));
    }

    #[test]
    fn fail_can_wait_before_a_retry() {
        use sb_model::{Expr, Step};
        let mut p = sample::orders();
        p.services[0].routes[1].steps.insert(
            0,
            Step::Fail {
                id: "f9".into(),
                status: 429,
                message: "slow down".into(),
                retry_after: Some(Expr::Literal { value: 30.into() }),
            },
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains("HttpError::new(429, \"slow down\")"));
        assert!(api.contains("error.with_retry_after(u64::try_from(30).unwrap_or(0))"));
    }

    #[test]
    fn variables_can_be_uuids_optional_or_lists() {
        use sb_model::{Expr, FieldType, Step};
        let mut p = sample::orders();
        let var = |name: &str,
                   ty: FieldType,
                   optional: bool,
                   repeated: bool,
                   value: serde_json::Value| Step::Let {
            id: format!("v_{name}"),
            name: name.into(),
            value_type: ty,
            optional,
            repeated,
            value: Expr::Literal { value },
        };
        p.services[0].routes[1].steps.splice(
            0..0,
            [
                var(
                    "trace",
                    FieldType::Uuid,
                    false,
                    false,
                    "123e4567-e89b-12d3-a456-426614174000".into(),
                ),
                var(
                    "limit",
                    FieldType::Int64,
                    true,
                    false,
                    serde_json::Value::Null,
                ),
                var(
                    "tags",
                    FieldType::String,
                    false,
                    true,
                    serde_json::json!(["a", "b"]),
                ),
            ],
        );
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(
            api.contains(
                "let trace: String = \"123e4567-e89b-12d3-a456-426614174000\".to_string();"
            )
        );
        assert!(api.contains("let limit: Option<i64> = None;"));
        assert!(
            api.contains("let tags: Vec<String> = vec![\"a\".to_string(), \"b\".to_string()];")
        );
    }

    #[test]
    fn password_and_token_steps_call_the_runtime() {
        use sb_model::{Expr, Step};
        let path = |p: &str| Expr::Path { path: p.into() };
        let mut p = sample::orders();
        p.services[0].routes[0].steps.splice(
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
            ],
        );
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains("let token: String = eda_runtime::crypto::random_token();"));
        assert!(
            api.contains("let digest: String = eda_runtime::crypto::sha256_hex(&token.clone());")
        );
        assert!(api.contains(
            "let hash: String = eda_runtime::crypto::hash_password(digest.clone()).await?;"
        ));
        // A plain string fills the optional `stored`; `null` is an unknown account.
        assert!(api.contains(
            "eda_runtime::crypto::verify_password(token.clone(), Some(hash.clone())).await;"
        ));
        assert!(api.contains("eda_runtime::crypto::verify_password(token.clone(), None).await;"));
        // The runtime ships the module and what it needs.
        assert!(file(&files, "runtime/src/crypto.rs").contains("spawn_blocking"));
        assert!(file(&files, "runtime/Cargo.toml").contains("argon2.workspace = true"));
        assert!(file(&files, "Cargo.toml").contains("argon2 = \"0.6\""));
    }

    #[test]
    fn cookies_keep_the_safe_defaults_unless_relaxed() {
        use sb_model::{Cookie, Expr, SameSite, Step};
        let mut p = sample::orders();
        let value = || Expr::Path {
            path: "order_id".into(),
        };
        let mut theme = Cookie::new(
            "theme",
            Expr::Literal {
                value: "dark".into(),
            },
        );
        theme.http_only = false;
        theme.secure = false;
        theme.same_site = SameSite::Strict;
        theme.path = Some("/app".into());
        let mut cross = Cookie::new("cross", value());
        cross.same_site = SameSite::None;
        let mut gone = Cookie::new("old", Expr::Literal { value: "".into() });
        gone.max_age_secs = Some(0);
        if let Some(Step::Respond { cookies, .. }) = p.services[0].routes[0].steps.last_mut() {
            cookies.extend([theme, cross, gone]);
        }
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains(".cookie(eda_runtime::http::Cookie::new(\"theme\", \"dark\".to_string()).path(\"/app\").http_only(false).secure(false).same_site(eda_runtime::http::SameSite::Strict))"));
        assert!(api.contains(".cookie(eda_runtime::http::Cookie::new(\"cross\", order_id.clone()).same_site(eda_runtime::http::SameSite::None))"));
        // `Max-Age=0` is how a cookie is deleted (logout).
        assert!(api.contains("Cookie::new(\"old\", \"\".to_string()).max_age(0)"));
    }

    #[test]
    fn a_single_use_token_is_consumed_by_one_statement() {
        let mut p = sample::orders();
        sample::add_single_use_token(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        // One UPDATE ... RETURNING: the state, the expiry (database clock, no parameter) and the marking
        // happen together, so two requests with the same token cannot both get a row.
        assert!(api.contains(
            "UPDATE \\\"auth_tokens\\\" SET \\\"used\\\" = $1 WHERE \\\"token_hash\\\" = $2 AND \\\"kind\\\" = $3 AND \\\"used\\\" = $4 AND \\\"expires_at\\\" > now() RETURNING \\\"account_id\\\", \\\"new_email\\\""
        ), "{api}");
        assert!(api.contains(".fetch_optional(&mut *tx)"));
        // No row: the answer the author chose, not the default 404.
        assert!(api.contains("HttpError::new(400, \"invalid or expired token\")"));
        assert!(!api.contains("auth_tokens: no row matches"));
        assert!(api.contains("struct AuthTokensRowT2 {"));
        // The table is created, with the timestamp column the clock is compared with.
        let schema = file(&files, "services/orders/src/app/schema.rs");
        assert!(schema.contains("\\\"expires_at\\\" TIMESTAMPTZ NOT NULL"));
    }

    #[test]
    fn jobs_are_scheduled_and_stopped_with_the_service() {
        let mut p = sample::orders();
        sample::add_single_use_token(&mut p);
        sample::add_cleaner_job(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        // The flow has no input and runs in its own transaction.
        let jobs = file(&files, "services/orders/src/flows/jobs.rs");
        assert!(jobs.contains("pub async fn cleaner(ctx: &Ctx) -> anyhow::Result<()>"));
        assert!(jobs.contains("let mut tx = ctx.db.begin().await?;"));
        assert!(jobs.contains("DELETE FROM \\\"auth_tokens\\\" WHERE \\\"expires_at\\\" <= now()"));
        assert!(jobs.contains("tx.commit().await?;"));
        assert!(!jobs.contains("input"), "{jobs}");
        let scheduler = file(&files, "services/orders/src/transport/scheduler.rs");
        assert!(scheduler.contains("Duration::from_secs(420)"));
        assert!(scheduler.contains("flows::jobs::cleaner(&ctx).await"));
        // main starts them, and stops them whatever ended the service.
        let main = file(&files, "services/orders/src/main.rs");
        assert!(main.contains("let scheduler = transport::scheduler::Scheduler::spawn(&ctx);"));
        assert!(main.contains("scheduler.stop().await;"));
        assert!(main.find("scheduler.stop()").unwrap() < main.rfind("result").unwrap());
        // A service with no job gets none of it.
        let mut without = sample::orders();
        without.services[0].jobs.clear();
        let plain = generate(&without).unwrap();
        assert!(
            !plain
                .iter()
                .any(|f| f.path.ends_with("scheduler.rs") || f.path.ends_with("flows/jobs.rs"))
        );
        assert!(!file(&plain, "services/orders/src/main.rs").contains("scheduler"));
    }

    #[test]
    fn an_idempotent_consumer_records_its_events_in_its_transaction() {
        let files = generate(&sample::orders()).unwrap();
        let flow = file(&files, "services/payments/src/flows/consumers.rs");
        // The record is written before anything else, in the transaction the handler commits at the end.
        let begin = flow.find("let mut tx = ctx.db.begin().await?;").unwrap();
        let check = flow
            .find("eda_runtime::dedup::first_time(&mut tx, \"on_order_created\", &input.order_id.to_string())")
            .unwrap();
        assert!(begin < check, "{flow}");
        assert!(flow.contains("duplicate event skipped"));
        assert!(
            check < flow.find("// let approved").unwrap(),
            "before the steps"
        );
        // The table is created with the schema, and old rows are purged.
        let schema = file(&files, "services/payments/src/app/schema.rs");
        assert!(schema.contains("eda_runtime::dedup::CREATE_TABLE,"));
        assert!(
            file(&files, "services/payments/src/app/bootstrap.rs")
                .contains("crate::app::schema::apply(&db)")
        );
        assert!(
            file(&files, "services/payments/src/transport/kafka.rs")
                .contains("eda_runtime::dedup::spawn_purge(ctx.db.clone());")
        );
        // A consumer that does not ask for it keeps its flow as it was.
        let other = file(&files, "services/notifications/src/flows/consumers.rs");
        assert!(!other.contains("dedup"));
        assert!(!file(&files, "services/notifications/src/transport/kafka.rs").contains("dedup"));
    }

    #[test]
    fn a_signed_webhook_reads_the_raw_body_and_verifies_it_before_parsing() {
        let mut p = sample::orders();
        sample::add_stripe_webhook(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        let handler = &http[http.find("async fn handle_stripe_webhook(").unwrap()..];
        // The extractors give the bytes, not a parsed body...
        assert!(handler.contains("headers: HeaderMap,"));
        assert!(handler.contains("body: Bytes,"));
        assert!(
            !handler[..handler.find("flows::api::stripe_webhook").unwrap()].contains("Json(input)")
        );
        // ...checked against the header, the secret of the environment and the tolerance, and only then read.
        let check = handler.find("eda_runtime::webhook::check(\"STRIPE_WEBHOOK_SECRET\", headers.get(\"Stripe-Signature\")").unwrap();
        let parse = handler
            .find("Json::<pb::StripeEvent>::from_bytes(&body)")
            .unwrap();
        assert!(
            check < parse,
            "the signature is verified before the body is read"
        );
        assert!(handler.contains("&body, 300)?;"));
        // The other routes keep their extractor.
        assert!(http.contains("Json(input): Json<pb::PlaceOrderBody>"));
        // A service that cannot verify its webhooks does not start.
        assert!(
            file(&files, "services/orders/src/app/config.rs")
                .contains("eda_runtime::webhook::secret(\"STRIPE_WEBHOOK_SECRET\")?;")
        );
        assert!(
            file(&files, "services/orders/src/flows/api.rs")
                .contains("signature in `Stripe-Signature` already verified")
        );
        assert!(file(&files, "runtime/Cargo.toml").contains("hmac.workspace = true"));
        assert!(file(&files, "Cargo.toml").contains("hmac = \"0.13\""));
        // Nothing of it in a project that has no signed webhook.
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/src/app/config.rs").contains("webhook"));
        assert!(!file(&plain, "services/orders/src/transport/http.rs").contains("HeaderMap"));
    }

    #[test]
    fn encryption_steps_read_their_key_from_the_environment_and_fail_fast() {
        let mut p = sample::orders();
        sample::add_sealed_text(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains("let sealed: String = eda_runtime::crypto::encrypt(\"NOTES_KEY\", &input.text.clone())?;"));
        assert!(api.contains(
            "let opened: String = eda_runtime::crypto::decrypt(\"NOTES_KEY\", &sealed.clone())?;"
        ));
        // Missing or malformed, the key stops the service at start-up, once however many steps use it.
        let config = file(&files, "services/orders/src/app/config.rs");
        assert_eq!(
            config
                .matches("eda_runtime::crypto::key(\"NOTES_KEY\")?;")
                .count(),
            1,
            "{config}"
        );
        assert!(file(&files, "runtime/Cargo.toml").contains("aes-gcm.workspace = true"));
        assert!(file(&files, "Cargo.toml").contains("aes-gcm = \"0.10\""));
        assert!(
            !file(
                &generate(&sample::orders()).unwrap(),
                "services/orders/src/app/config.rs"
            )
            .contains("crypto::key")
        );
    }

    #[test]
    fn the_rules_of_a_request_are_checked_before_any_step() {
        let mut p = sample::orders();
        sample::add_signup(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        // One function per schema that has rules, the nested one included.
        let body = &http[http.find("fn check_signup_body(").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("eda_runtime::validation::email(\"email\", v)?;"));
        assert!(body.contains("eda_runtime::validation::length(\"password\", v, 10, 128)?;"));
        // An optional field is checked when present, a list element by element, a nested schema by its own function.
        assert!(body.contains("if let Some(v) = &m.siren {"));
        assert!(body.contains("for v in &m.contacts {"));
        // The path of the field travels with the message: `company.name`.
        assert!(
            body.contains("check_signup_company(item).map_err(|e| format!(\"company.{e}\"))?;")
        );
        assert!(http.contains("fn check_signup_company(m: &pb::SignupCompany)"));
        // The handler runs them after the request is read and before the flow.
        let handler = &http[http.find("async fn handle_signup(").unwrap()..];
        assert!(
            handler
                .find("check_signup_body(&input).map_err(invalid)?;")
                .unwrap()
                < handler.find("flows::api::signup").unwrap()
        );
        // A schema without rules costs nothing, and the other routes are left alone.
        assert!(!http.contains("check_place_order_body"));
        assert!(
            !file(
                &generate(&sample::orders()).unwrap(),
                "services/orders/src/transport/http.rs"
            )
            .contains("fn invalid(")
        );
    }

    #[test]
    fn headers_and_cookies_fill_the_fields_they_are_declared_for() {
        let files = generate(&sample::showcase()).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        let compact = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let whoami = &http[http.find("async fn handle_whoami(").unwrap()..];
        let whoami = compact(&whoami[..whoami.find("\n}\n").unwrap()]);
        // The header is read whatever its case, and always overwrites what the query string said.
        assert!(whoami.contains("headers: HeaderMap,"), "{whoami}");
        assert!(
            whoami.contains("input.token = headers.get(\"authorization\")"),
            "{whoami}"
        );
        let session = &http[http.find("async fn handle_session(").unwrap()..];
        let session = compact(&session[..session.find("\n}\n").unwrap()]);
        assert!(session.contains("eda_runtime::http::cookie("), "{session}");
        assert!(session.contains("\"access_token\""), "{session}");
        // The fields are filled before the flow runs.
        assert!(
            session.find("input.token =").unwrap() < session.find("flows::api::session").unwrap()
        );
        // A route that reads nothing from the request does not take the headers.
        let jwks = &http[http.find("async fn handle_jwks(").unwrap()..];
        assert!(!jwks[..jwks.find("\n}\n").unwrap()].contains("HeaderMap"));
    }

    #[test]
    fn a_second_factor_checks_codes_in_the_transaction_and_keeps_its_own_tables() {
        let mut p = sample::orders();
        sample::add_totp(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        let flat = api.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("let setup: pb::TotpSetup = { let setup = eda_runtime::totp::generate(\"TOTP_ENCRYPTION_KEY\", \"Orders\", &input.account.clone())?;"), "{flat}");
        // The check runs on the transaction of the handler: it rolls back with the rest.
        assert!(
            flat.contains("eda_runtime::totp::verify( &mut tx, \"TOTP_ENCRYPTION_KEY\", &factor.secret_sealed.clone(), &input.code.clone(), &input.account.clone(), ) .await?;")
                || flat.contains("eda_runtime::totp::verify(&mut tx, \"TOTP_ENCRYPTION_KEY\""),
            "{flat}"
        );
        assert!(flat.contains("let codes: Vec<String> = eda_runtime::totp::issue_recovery_codes(&mut tx, &input.account.clone(), 8).await?;"), "{flat}");
        assert!(
            flat.contains("eda_runtime::totp::use_recovery_code(&mut tx"),
            "{flat}"
        );
        // The tables the steps keep their state in come with the schema, and the key is checked at start-up.
        let schema = file(&files, "services/orders/src/app/schema.rs");
        assert!(schema.contains("eda_runtime::totp::CREATE_STEPS_TABLE,"));
        assert!(schema.contains("eda_runtime::totp::CREATE_RECOVERY_TABLE,"));
        assert!(
            file(&files, "services/orders/src/app/config.rs")
                .contains("eda_runtime::crypto::key(\"TOTP_ENCRYPTION_KEY\")?;")
        );
        assert!(file(&files, "Cargo.toml").contains("sha1 = \"0.11\""));
        // A service that has no second factor carries none of it.
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/src/app/schema.rs").contains("totp::"));
        // Only enrolling and verifying need the table of steps; recovery codes have their own.
        let mut only_recovery = p.clone();
        only_recovery.services[0]
            .routes
            .retain(|r| r.name != "verify_totp");
        let schema = file(
            &generate(&only_recovery).unwrap(),
            "services/orders/src/app/schema.rs",
        )
        .to_string();
        assert!(!schema.contains("CREATE_STEPS_TABLE") && schema.contains("CREATE_RECOVERY_TABLE"));
    }

    #[test]
    fn passkeys_need_the_site_address_the_feature_and_their_tables() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_passkeys(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        let flat = api.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("let challenge: pb::PasskeyChallenge = { let challenge = eda_runtime::webauthn::register_start(&mut tx, &claims.sub.clone(), &input.name.clone()).await?;"), "{flat}");
        assert!(flat.contains("eda_runtime::webauthn::register_finish(&mut tx, &input.state_id.clone(), &input.credential.clone(), &claims.sub.clone()).await?;"), "{flat}");
        assert!(
            flat.contains("eda_runtime::webauthn::login_start(&mut tx, &input.account.clone())"),
            "{flat}"
        );
        assert!(flat.contains("let account: String = eda_runtime::webauthn::login_finish(&mut tx, &input.state_id.clone(), &input.credential.clone()).await?;"), "{flat}");
        // The runtime module is built with its feature, which only this service turns on.
        assert!(
            file(&files, "services/orders/Cargo.toml")
                .contains("eda-runtime = { path = \"../../runtime\", features = [\"webauthn\"] }")
        );
        assert!(
            file(&files, "runtime/src/lib.rs")
                .contains("#[cfg(feature = \"webauthn\")]\npub mod webauthn;")
        );
        // Its tables come with the schema, and the site address is checked at start-up.
        let schema = file(&files, "services/orders/src/app/schema.rs");
        for table in [
            "CREATE_STATES_TABLE",
            "CREATE_CREDENTIALS_TABLE",
            "CREATE_CREDENTIALS_INDEX",
        ] {
            assert!(
                schema.contains(&format!("eda_runtime::webauthn::{table},")),
                "{table}"
            );
        }
        assert!(
            file(&files, "services/orders/src/app/config.rs")
                .contains("eda_runtime::webauthn::check_config()?;")
        );
        // Nothing of it in a service without passkeys.
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/Cargo.toml").contains("webauthn"));
        assert!(!file(&plain, "services/orders/src/app/schema.rs").contains("webauthn"));
        assert!(!file(&plain, "services/orders/src/app/config.rs").contains("webauthn"));
    }

    #[test]
    fn the_showcase_generates_a_workspace_of_every_feature() {
        let files = generate(&sample::showcase()).unwrap();
        for path in [
            "services/orders/src/flows/jobs.rs",
            "services/orders/src/transport/scheduler.rs",
            "services/orders/src/app/schema.rs",
            "runtime/src/webhook.rs",
            "runtime/src/crypto.rs",
        ] {
            assert!(files.iter().any(|f| f.path == path), "{path}");
        }
        let schema = file(&files, "services/orders/src/app/schema.rs");
        assert!(
            schema.contains("FOREIGN KEY")
                && schema.contains("JSONB")
                && schema.contains("CREATE INDEX")
        );
    }

    #[test]
    fn tokens_are_signed_checked_and_their_keys_published() {
        let mut p = sample::orders();
        sample::add_login(&mut p);
        sample::add_jwt(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        // Signed: the author's claims, each as the JSON of its own type; the standard ones are the step's.
        assert!(
            api.contains(
                "claims.insert(\"sub\".to_string(), eda_runtime::jwt::json!(input.email.clone()));"
            ),
            "{api}"
        );
        assert!(api.contains(
            "claims.insert(\"roles\".to_string(), eda_runtime::jwt::json!([\"member\"]));"
        ));
        assert!(
            api.contains("eda_runtime::jwt::sign(\"JWT_SIGNING_KEY\", \"orders\", 300, claims)?")
        );
        // Checked: read as the schema of the claims, with the key of the environment.
        assert!(api.contains("let claims: pb::AccessClaims ="));
        assert!(api.contains("eda_runtime::jwt::verify(eda_runtime::jwt::Source::Keys(&[\"JWT_SIGNING_KEY\"]), \"orders\", &token).await?"));
        // Published: the public key, in the shape of a JWKS.
        assert!(api.contains("eda_runtime::jwt::public_keys(&[\"JWT_SIGNING_KEY\"])?"));
        assert!(api.contains("pb::Jwk { kty: k.kty, crv: k.crv, alg: k.alg, kid: k.kid, x: k.x, ..Default::default() }"));
        // A service that cannot sign or verify does not start.
        let config = file(&files, "services/orders/src/app/config.rs");
        assert_eq!(
            config
                .matches("eda_runtime::jwt::JwtKeys::from_env(\"JWT_SIGNING_KEY\")?;")
                .count(),
            1,
            "{config}"
        );
        assert!(file(&files, "runtime/Cargo.toml").contains("jsonwebtoken.workspace = true"));
        assert!(file(&files, "Cargo.toml").contains("jsonwebtoken = { version = \"11\""));
    }

    #[test]
    fn a_remote_jwks_and_a_bearer_header_are_read_from_the_environment() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        if let sb_model::Step::VerifyJwt { source, bearer, .. } =
            &mut p.services[0].routes[2].steps[0]
        {
            *source = sb_model::JwksSource::Url {
                url_env: "AUTH_JWKS_URL".into(),
            };
            *bearer = true;
        }
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains("let token = eda_runtime::jwt::bearer(&token);"));
        assert!(api.contains("eda_runtime::jwt::Source::Url(\"AUTH_JWKS_URL\")"));
        assert!(
            file(&files, "services/orders/src/app/config.rs")
                .contains("eda_runtime::jwt::jwks_url(\"AUTH_JWKS_URL\")?;")
        );
    }

    #[test]
    fn routes_sharing_a_path_are_served_by_one_method_router() {
        let mut p = sample::orders();
        let mut delete = p.services[0].routes[1].clone();
        delete.name = "delete_order".into();
        delete.method = sb_model::HttpMethod::Delete;
        p.services[0].routes.push(delete);
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(http.contains(
            ".route(\"/orders/{order_id}\", get(handle_get_order).delete(handle_delete_order))"
        ));
        // Only the free functions that are used are imported.
        assert!(http.contains("routing::{get, post}"));
    }

    #[test]
    fn services_without_routes_do_not_pull_the_http_stack() {
        let files = generate(&sample::orders()).unwrap();
        for svc in ["payments", "notifications"] {
            assert!(!file(&files, &format!("services/{svc}/Cargo.toml")).contains("axum"));
            assert!(
                !file(&files, &format!("services/{svc}/src/app/config.rs")).contains("http_port")
            );
        }
    }

    #[test]
    fn api_schemas_are_json_in_the_contracts() {
        let files = generate(&sample::orders()).unwrap();
        let build = file(&files, "contracts/build.rs");
        assert!(
            build.contains(
                "for message in [\"PlaceOrderBody\", \"GetOrderRequest\", \"OrderView\"]"
            )
        );
        assert!(build.contains("serde::Serialize, serde::Deserialize"));
        assert!(file(&files, "contracts/Cargo.toml").contains("serde.workspace = true"));
        // A project without API schemas keeps the plain build script.
        let mut p = sample::orders();
        p.services[0].routes.clear();
        p.messages.retain(|m| m.kind != sb_model::MessageKind::Api);
        let plain = generate(&p).unwrap();
        assert!(!file(&plain, "contracts/build.rs").contains("serde"));
    }

    #[test]
    fn publishing_goes_through_the_outbox_and_its_relay() {
        let files = generate(&sample::orders()).unwrap();
        // Services that publish start the relay; the one that only consumes and calls does not.
        for svc in ["orders", "payments"] {
            let main = file(&files, &format!("services/{svc}/src/main.rs"));
            assert!(main.contains("transport::outbox::spawn_relay(&config, &ctx)?;"));
            let boot = file(&files, &format!("services/{svc}/src/app/bootstrap.rs"));
            assert!(boot.contains("eda_runtime::outbox::migrate(&db)"));
            let config = file(&files, &format!("services/{svc}/src/app/config.rs"));
            assert!(config.contains("OUTBOX_MAX_ATTEMPTS"), "{svc}");
            assert!(
                config.contains("\"5\""),
                "default attempts come from the project"
            );
        }
        let notifications = file(&files, "services/notifications/src/main.rs");
        assert!(!notifications.contains("outbox"));
        assert!(!file(&files, "services/notifications/src/app/bootstrap.rs").contains("outbox"));
        // Every service owns a database, configurable per service.
        let config = file(&files, "services/orders/src/app/config.rs");
        assert!(config.contains("ORDERS_DATABASE_URL"));
        assert!(config.contains("localhost:5432/orders"));
        assert!(file(&files, "services/orders/src/app/ctx.rs").contains("pub db: sqlx::PgPool"));
    }

    #[test]
    fn dead_letter_topics_are_provisioned_and_named() {
        let files = generate(&sample::orders()).unwrap();
        let boot = file(&files, "services/orders/src/app/bootstrap.rs");
        assert!(boot.contains("topics::ORDERS_CREATED_DLQ"));
        let contracts = file(&files, "contracts/src/lib.rs");
        assert!(contracts.contains("pub const ORDERS_CREATED_DLQ: &str = \"orders.created.dlq\";"));
    }

    #[test]
    fn declared_producers_are_documented_and_provision_their_topic() {
        let mut p = sample::orders();
        // Notifications owns a new topic but has no step publishing there yet.
        p.topics[0].producers.push("Notifications".into());
        let files = generate(&p).unwrap();
        assert!(
            file(&files, "contracts/src/lib.rs").contains("Produced by Orders, Notifications.")
        );
        assert!(file(&files, "README.md").contains("- produces `orders.created`"));
        let boot = file(&files, "services/notifications/src/app/bootstrap.rs");
        assert!(boot.contains("topics::ORDERS_CREATED_DLQ"));
        // No step publishes, so no relay and no outbox table for it.
        assert!(!file(&files, "services/notifications/src/main.rs").contains("outbox"));
    }

    #[test]
    fn the_attempts_setting_reaches_the_services() {
        let mut p = sample::orders();
        p.outbox.max_attempts = 9;
        let files = generate(&p).unwrap();
        assert!(file(&files, "services/orders/src/app/config.rs").contains("\"9\""));
    }

    #[test]
    fn local_infrastructure_has_a_database() {
        let files = generate(&sample::orders()).unwrap();
        let compose = file(&files, "docker-compose.yml");
        assert!(compose.contains("postgres:16"));
        assert!(compose.contains("postgres-data"));
        assert!(file(&files, "Cargo.toml").contains("sqlx"));
    }

    #[test]
    fn refuses_invalid_projects() {
        let mut p = sample::orders();
        p.topics[0].message = "Nope".into();
        match generate(&p) {
            Err(CodegenError::Invalid(d)) => assert!(has_errors(&d)),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn refuses_bad_rust_blocks() {
        let mut p = sample::orders();
        p.services[1].consumers[0].steps.push(sb_model::Step::Code {
            id: "x".into(),
            code: "let = ;".into(),
        });
        assert!(matches!(generate(&p), Err(CodegenError::Invalid(_))));
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(
            generate(&sample::orders()).unwrap(),
            generate(&sample::orders()).unwrap()
        );
    }

    #[test]
    fn rustfmt_keeps_code_valid() {
        let mut files = generate(&sample::orders()).unwrap();
        rustfmt(&mut files);
        for f in files.iter().filter(|f| f.path.ends_with(".rs")) {
            syn::parse_file(&f.content).unwrap();
        }
        // Whether or not rustfmt is installed, comments survive.
        assert!(
            file(&files, "services/orders/src/flows/rpc.rs")
                .contains("// publish to orders.created")
        );
    }

    #[test]
    fn writes_files_to_disk() {
        let dir = std::env::temp_dir().join(format!("sb-codegen-test-{}", std::process::id()));
        let files = generate(&sample::orders()).unwrap();
        write_to_dir(&files, &dir).unwrap();
        assert!(dir.join("services/orders/src/main.rs").exists());
        assert!(dir.join("contracts/proto/orders_demo.proto").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
