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
                "runtime/src/db.rs",
                "runtime/src/lib.rs",
                "runtime/src/outbox.rs",
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
                "services/orders/src/flows/mod.rs",
                "services/orders/src/flows/rpc.rs",
                "services/orders/src/main.rs",
                "services/orders/src/transport/grpc.rs",
                "services/orders/src/transport/http.rs",
                "services/orders/src/transport/mod.rs",
                "services/orders/src/transport/outbox.rs",
                "services/payments/Cargo.toml",
                "services/payments/src/app/bootstrap.rs",
                "services/payments/src/app/config.rs",
                "services/payments/src/app/ctx.rs",
                "services/payments/src/app/mod.rs",
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
        assert!(http.contains("(StatusCode::CREATED, Json(body))"));
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
                ],
            },
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
        // Services without a table get nothing.
        assert!(
            !files
                .iter()
                .any(|f| f.path == "services/payments/src/app/schema.rs")
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
        } = &mut p.services[0].routes[0].steps[2]
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
        } = &mut p.services[0].routes[0].steps[2]
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
