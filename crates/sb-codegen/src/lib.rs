//! Turns a validated [`sb_model::Project`] into a Cargo workspace of Rust services:
//! protobuf contracts, one tonic gRPC server + rdkafka consumers per service, and a
//! docker-compose for Kafka.

pub mod asyncapi;
mod expr;
mod graphql;
#[cfg(test)]
mod graphql_tests;
mod http_call;
#[cfg(test)]
mod journal_tests;
mod mta;
pub mod openapi;
pub mod pods;
mod proto;
pub mod rawcheck;
mod realtime;
/// The CSV module of the generated runtime, compiled here too so that its own tests run with the generator's.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../templates/runtime/csv.rs"]
mod runtime_csv;
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
        files.extend(match svc.kind {
            sb_model::ServiceKind::Standard => service::generate(project, svc)?,
            sb_model::ServiceKind::Mta => mta::generate(project, svc)?,
            sb_model::ServiceKind::Realtime => realtime::generate(project, svc)?,
        });
    }
    // The GraphQL schemas of the services that ask for one, next to the .proto: they are contracts.
    for svc in &project.services {
        if let Some(sdl) = graphql::sdl(svc) {
            files.push(GeneratedFile::new(
                format!(
                    "contracts/graphql/{}.graphql",
                    sb_model::names::snake(&svc.name)
                ),
                sdl,
            ));
        }
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
                "asyncapi.yaml",
                "contracts/Cargo.toml",
                "contracts/build.rs",
                "contracts/proto/orders_demo.proto",
                "contracts/src/lib.rs",
                "docker-compose.yml",
                "runtime/Cargo.toml",
                "runtime/src/api_key.rs",
                "runtime/src/audit.rs",
                "runtime/src/challenge.rs",
                "runtime/src/crypto.rs",
                "runtime/src/csv.rs",
                "runtime/src/db.rs",
                "runtime/src/dedup.rs",
                "runtime/src/flags.rs",
                "runtime/src/html.rs",
                "runtime/src/http_client.rs",
                "runtime/src/jobs.rs",
                "runtime/src/jwt.rs",
                "runtime/src/lib.rs",
                "runtime/src/live.rs",
                "runtime/src/logic.rs",
                "runtime/src/outbox.rs",
                "runtime/src/password.rs",
                "runtime/src/rate_limit.rs",
                "runtime/src/resilience.rs",
                "runtime/src/s3.rs",
                "runtime/src/sequence.rs",
                "runtime/src/short_id.rs",
                "runtime/src/time_zone.rs",
                "runtime/src/totp.rs",
                "runtime/src/usage.rs",
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
                "services/orders/openapi.json",
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
    fn an_include_is_generated_as_the_steps_of_its_fragment() {
        let files = generate(&sb_model::sample::showcase()).unwrap();
        let all: String = files.iter().map(|f| f.content.as_str()).collect();
        assert_eq!(all.matches("a task of the service starts").count(), 1);
        assert!(
            all.contains("// include start_note") || all.contains("a task of the service starts")
        );
    }

    #[test]
    fn a_loop_runs_its_steps_in_the_transaction_of_the_handler() {
        let files = generate(&sb_model::sample::showcase()).unwrap();
        let api = files
            .iter()
            .find(|f| f.path == "services/orders/src/flows/api.rs")
            .unwrap()
            .content
            .clone();
        let start = api
            .find("for line in input.lines.clone() {")
            .expect("a loop");
        let body = &api[start..];
        let end = body.find("\n    }\n").unwrap();
        let body = &body[..end];
        // The insert of each element uses the same `tx` as the rest of the flow, and a refusal returns from the handler.
        assert!(body.contains(".execute(&mut *tx).await?;"), "{body}");
        assert!(body.contains("return Err(error.into());"), "{body}");
        assert!(!body.contains("tx.commit()"), "{body}");
        // The rows of a query are a list like any other.
        assert!(api.contains("for row in found.clone() {"));
    }

    #[test]
    fn rpc_flow_snapshot() {
        let files = generate(&sample::orders()).unwrap();
        let flows = file(&files, "services/orders/src/flows/rpc.rs");
        assert!(
            flows.contains("pub async fn create_order(ctx: &Ctx, input: pb::CreateOrderRequest)")
        );
        assert!(flows.contains("let order_id: String = eda_runtime::logic::new_uuid();"));
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
    fn a_204_route_answers_without_a_body() {
        let mut p = sample::orders();
        let name = {
            let r = p.services[0]
                .routes
                .iter_mut()
                .find(|r| r.name == "get_order")
                .unwrap();
            r.status = 204;
            r.name.clone()
        };
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(http.contains(".map(|_| StatusCode::NO_CONTENT)"), "{name}");
    }

    #[test]
    fn routes_become_axum_handlers_that_delegate_to_flows() {
        let files = generate(&sample::orders()).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(http.contains(".route(\"/orders\", post(handle_place_order))"));
        assert!(http.contains(".route(\"/orders/{order_id}\", get(handle_get_order))"));
        // A body for POST (+ the status of the route), a query string for GET, path parameters read by name.
        // (its fields are required: the body is looked at as JSON first, then read as the schema)
        assert!(http.contains("Json(body): Json<serde_json::Value>"));
        assert!(http.contains("require_place_order(&body).map_err(invalid)?;"));
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
                mutable: false,
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
                ], status: None, cookies: vec![], headers: vec![]},
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
    fn an_upsert_conflicts_on_its_key_and_updates_the_other_columns() {
        use sb_model::{SqlOp, Step};
        let mut p = sample::orders();
        if let Step::Sql { op, conflict, .. } = &mut p.services[0].routes[0].steps[3] {
            *op = SqlOp::Upsert;
            *conflict = vec!["order_id".into()];
        }
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(
            api.contains("ON CONFLICT (\\\"order_id\\\") DO UPDATE SET \\\"amount\\\" = EXCLUDED.\\\"amount\\\""),
            "{api}"
        );
    }

    #[test]
    fn a_read_model_is_fed_by_an_idempotent_consumer_that_upserts_and_deletes() {
        use sb_model::{CmpOp, Condition, Consumer, Expr, SqlCondition, SqlOp, Step, validate};
        let mut p = sample::orders();
        let mut upsert = p.services[0].routes[0].steps[3].clone();
        if let Step::Sql {
            id,
            op,
            conflict,
            values,
            ..
        } = &mut upsert
        {
            *id = "rm2".into();
            *op = SqlOp::Upsert;
            *conflict = vec!["order_id".into()];
            // The route read its own variables; an event is read through `input`.
            for a in values.iter_mut() {
                if let Expr::Path { path } = &mut a.value {
                    if !path.starts_with("input.") {
                        *path = format!("input.{path}");
                    }
                }
            }
        } else {
            panic!("sample changed");
        }
        let mut delete = upsert.clone();
        if let Step::Sql {
            id,
            op,
            values,
            conflict,
            conditions,
            ..
        } = &mut delete
        {
            *id = "rm1".into();
            *op = SqlOp::Delete;
            values.clear();
            conflict.clear();
            *conditions = vec![SqlCondition {
                column: "order_id".into(),
                op: CmpOp::Eq,
                value: Expr::Path {
                    path: "input.order_id".into(),
                },
                test: Default::default(),
                or: vec![],
            }];
        }
        p.services[0].consumers.push(Consumer {
            name: "project_orders".into(),
            topic: "orders.created".into(),
            group_id: None,
            idempotent_on: Some("order_id".into()),
            mail: None,
            steps: vec![Step::Branch {
                id: "rm0".into(),
                condition: Condition::Compare {
                    left: Expr::Path {
                        path: "input.amount".into(),
                    },
                    op: CmpOp::Eq,
                    right: Expr::Literal { value: 0.into() },
                },
                then: vec![delete],
                otherwise: vec![upsert],
            }],
        });
        let d = validate(&p);
        assert!(d.iter().all(|d| !d.code.starts_with("E_")), "{d:?}");
        let files = generate(&p).unwrap();
        let all: String = files.iter().map(|f| f.content.clone()).collect();
        assert!(all.contains("ON CONFLICT"));
        assert!(all.contains("DELETE FROM"));
        assert!(all.contains("processed_events"));
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
    fn require_permission_refuses_a_caller_without_the_permission() {
        use sb_model::{Expr, Step};
        let mut p = sample::orders();
        let steps = &mut p.services[0].routes[1].steps;
        steps.insert(
            0,
            Step::RequirePermission {
                id: "rp1".into(),
                claims: Expr::Path {
                    path: "granted".into(),
                },
                permission: "crm.manage".into(),
                status: 403,
                message: "Forbidden".into(),
            },
        );
        steps.insert(
            0,
            Step::Let {
                id: "rp0".into(),
                name: "granted".into(),
                value_type: sb_model::FieldType::String,
                optional: false,
                repeated: true,
                mutable: false,
                value: Expr::Literal {
                    value: serde_json::json!(["crm.view"]),
                },
            },
        );
        assert_eq!(
            sb_model::validate(&p),
            vec![],
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains(".iter().any(|p| p.as_str() == \"crm.manage\")"));
        assert!(api.contains("HttpError::new(403, \"Forbidden\")"));
        let mut q = sample::orders();
        q.services[0].routes[1].steps.insert(
            0,
            Step::RequirePermission {
                id: "rp1".into(),
                claims: Expr::Path {
                    path: "input.nothing".into(),
                },
                permission: "".into(),
                status: 200,
                message: "".into(),
            },
        );
        let codes: Vec<String> = sb_model::validate(&q).into_iter().map(|d| d.code).collect();
        for code in [
            "E_FAIL_STATUS",
            "E_FAIL_MESSAGE",
            "E_REQUIRE_PERMISSION",
            "E_UNKNOWN_PATH",
        ] {
            assert!(codes.contains(&code.to_string()), "{code}: {codes:?}");
        }
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
                code: None,
                retry_after: Some(Expr::Literal { value: 30.into() }),
            },
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains("HttpError::new(429, \"slow down\")"));
        assert!(api.contains("error.with_retry_after(u64::try_from(30).unwrap_or(0))"));
    }

    #[test]
    fn fail_and_missing_can_carry_a_machine_code() {
        use sb_model::{SqlMissing, Step};
        let mut p = sample::orders();
        sample::add_single_use_token(&mut p);
        p.services[0].routes[1].steps.insert(
            0,
            Step::Fail {
                id: "f9".into(),
                status: 409,
                message: "stock would go negative".into(),
                code: Some("STOCK_INSUFFISANT".into()),
                retry_after: None,
            },
        );
        for route in &mut p.services[0].routes {
            for step in &mut route.steps {
                if let Step::Sql {
                    missing: Some(missing),
                    ..
                } = step
                {
                    *missing = SqlMissing {
                        status: 404,
                        message: "no such row".into(),
                        code: Some("ROW_ABSENTE".into()),
                    };
                }
            }
        }
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains("let error = error.with_code(\"STOCK_INSUFFISANT\");"));
        let all: String = files.iter().map(|f| f.content.as_str()).collect();
        assert!(all.contains("HttpError::new(404, \"no such row\").with_code(\"ROW_ABSENTE\")"));
        // The body keeps `error` and adds `code` and `message`.
        assert!(all.contains("body.insert(\"code\", code);"));
        let spec = all
            .lines()
            .find(|l| l.contains("STOCK_INSUFFISANT") && l.contains("code `"));
        assert!(spec.is_some() || all.contains("(code `STOCK_INSUFFISANT`)"));
        // Without a code the output of a project is unchanged.
        let plain = generate(&sample::orders()).unwrap();
        assert!(
            !plain
                .iter()
                .any(|f| f.path.starts_with("services/") && f.content.contains("with_code(\""))
        );
    }

    #[test]
    fn a_mutable_variable_can_be_assigned_inside_a_branch() {
        use sb_model::{Condition, Expr, FieldType, Step};
        let lit = |v: serde_json::Value| Expr::Literal { value: v };
        let let_step = |mutable: bool| Step::Let {
            id: "v_total".into(),
            name: "total".into(),
            value_type: FieldType::Int64,
            optional: false,
            repeated: false,
            mutable,
            value: lit(0.into()),
        };
        let assign = Step::Assign {
            id: "a_total".into(),
            name: "total".into(),
            value: lit(5.into()),
        };
        let branch = Step::Branch {
            id: "b".into(),
            condition: Condition::Test {
                value: lit(true.into()),
            },
            then: vec![assign],
            otherwise: vec![],
        };
        let mut p = sample::orders();
        p.services[0].routes[1]
            .steps
            .splice(0..0, [let_step(true), branch.clone()]);
        let files = generate(&p).expect("a mutable variable is assignable");
        let code: String = files.iter().map(|f| f.content.as_str()).collect();
        assert!(code.contains("let mut total: i64 = 0;"), "{code}");
        assert!(code.contains("total = 5;"));

        let mut p = sample::orders();
        p.services[0].routes[1]
            .steps
            .splice(0..0, [let_step(false), branch]);
        let report = sb_model::validate(&p);
        assert!(
            report.iter().any(|d| d.code == "E_ASSIGN_IMMUTABLE"),
            "{report:?}"
        );
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
            mutable: false,
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
    fn a_csv_route_reads_a_text_body_and_a_csv_route_answers_text_csv() {
        let mut p = sample::orders();
        sample::add_basket_lines(&mut p);
        sample::add_csv_lines(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");

        // Import: the body is text, typed by the columns of the item, every refused line reported, then read as the schema.
        let handler = &http[http.find("async fn handle_import_lines(").unwrap()..];
        let handler = &handler[..handler.find("flows::api::import_lines").unwrap()];
        assert!(handler.contains("body: String,"), "{handler}");
        assert!(!handler.contains("Json("), "{handler}");
        assert!(handler.contains("eda_runtime::csv::items(&body, &[eda_runtime::csv::Column { key: \"sku\", kind: eda_runtime::csv::Kind::Text, required: true }, eda_runtime::csv::Column { key: \"qty\", kind: eda_runtime::csv::Kind::Int32, required: true }, eda_runtime::csv::Column { key: \"note\", kind: eda_runtime::csv::Kind::Text, required: false }])"), "{handler}");
        assert!(handler.contains("StatusCode::BAD_REQUEST"));
        assert!(handler.contains("serde_json::json!({ \"lines\": rows })"));
        // The file is not a JSON string: its limit is its own, and the route has it.
        assert!(http.contains(".layer(DefaultBodyLimit::max(5242880))"));
        assert!(http.contains("extract::{DefaultBodyLimit,"));

        // Export: the list of the answer written as `text/csv` with the columns of the item.
        let handler = &http[http.find("async fn handle_export_lines(").unwrap()..];
        let handler = &handler[..handler.find("\nasync fn ").unwrap_or(handler.len())];
        assert!(
            handler.contains("eda_runtime::csv::write(json.get(\"lines\")"),
            "{handler}"
        );
        assert!(
            handler.contains("&[\"sku\", \"qty\", \"note\"]"),
            "{handler}"
        );
        assert!(handler.contains("text/csv; charset=utf-8"));
        assert!(!handler.contains("Json(reply.body)"));
        // Its flow returns a `Reply`, which carries the list to the handler.
        let flows = file(&files, "services/orders/src/flows/api.rs");
        assert!(flows.contains("pub async fn export_lines(ctx: &Ctx, input: pb::OrderRef) -> anyhow::Result<eda_runtime::http::Reply<pb::LinesFile>>"));
        // The routes that stay JSON are as they were.
        let handler = &http[http.find("async fn handle_get_lines(").unwrap()..];
        assert!(
            handler[..handler.find("\nasync fn ").unwrap_or(handler.len())].contains("Json(body)")
        );

        // The runtime carries the module, and the OpenAPI document says what the routes take and give.
        assert!(file(&files, "runtime/src/lib.rs").contains("pub mod csv;"));
        let doc: serde_json::Value =
            serde_json::from_str(file(&files, "services/orders/openapi.json")).unwrap();
        let import = &doc["paths"]["/orders/{order_id}/lines/import"]["post"];
        assert!(
            import["requestBody"]["content"]["text/csv"]["schema"]["description"]
                .as_str()
                .unwrap()
                .contains("`sku`, `qty`, `note`")
        );
        assert!(import["requestBody"]["content"]["application/json"].is_null());
        let export = &doc["paths"]["/orders/{order_id}/lines/export"]["get"];
        assert_eq!(
            export["responses"]["200"]["content"]["text/csv"]["schema"]["type"],
            "string"
        );
        assert!(export["responses"]["200"]["content"]["application/json"].is_null());
    }

    #[test]
    fn a_partial_csv_import_keeps_the_valid_lines_and_reports_the_refused_ones() {
        let mut p = sample::orders();
        sample::add_basket_lines(&mut p);
        sample::add_csv_lines(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        let handler = &http[http.find("async fn handle_import_lines_partial(").unwrap()..];
        let handler = &handler[..handler.find("\nasync fn ").unwrap_or(handler.len())];
        assert!(
            handler.contains("let (rows, rejected) = eda_runtime::csv::items_partial(&body,"),
            "{handler}"
        );
        assert!(
            handler.contains("\"rejected\".to_string()")
                && handler.contains("eda_runtime::csv::Rejected::json"),
            "{handler}"
        );
        assert!(!handler.contains("Json(reply.body)"), "{handler}");
        // The all-or-nothing import next to it is as it was.
        let strict = &http[http.find("async fn handle_import_lines(").unwrap()..];
        let strict = &strict[..strict.find("\nasync fn ").unwrap_or(strict.len())];
        assert!(
            strict.contains("eda_runtime::csv::items(&body,"),
            "{strict}"
        );
        assert!(!strict.contains("rejected"), "{strict}");
        assert!(
            file(&files, "runtime/src/csv.rs").contains("pub fn items_partial("),
            "the runtime carries the partial read"
        );
        // Its flow returns a `Reply`: the handler completes the answer.
        let flows = file(&files, "services/orders/src/flows/api.rs");
        assert!(flows.contains("pub async fn import_lines_partial("));
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
        let parse = handler.find("serde_json::from_slice(&body)").unwrap();
        assert!(
            check < parse,
            "the signature is verified before the body is read"
        );
        assert!(handler.contains("&body, 300)?;"));
        // The other routes keep their extractor.
        assert!(http.contains("Json(body): Json<serde_json::Value>"));
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
    fn a_field_can_take_the_raw_body_in_base64_before_it_is_parsed() {
        let mut p = sample::orders();
        sample::add_stripe_webhook(&mut p);
        let event = p
            .messages
            .iter_mut()
            .find(|m| m.name == "StripeEvent")
            .unwrap();
        let mut raw = sb_model::Field::new("raw", sb_model::FieldType::String);
        raw.source = Some(sb_model::RequestSource::RawBody);
        event.fields.push(raw);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        let handler = &http[http.find("async fn handle_stripe_webhook(").unwrap()..];
        let check = handler.find("eda_runtime::webhook::check(").unwrap();
        let encode = handler
            .find("let __raw_body = eda_runtime::webhook::encode(&body);")
            .unwrap();
        let parse = handler.find("serde_json::from_slice(&body)").unwrap();
        assert!(
            check < encode && encode < parse,
            "signed, then archived as received, then parsed"
        );
        assert!(handler.contains("input.raw = __raw_body;"));
        // The body is not a header or a parameter of the contract.
        assert!(
            !file(&files, "services/orders/openapi.json")
                .contains("\"in\": \"header\", \"name\": \"raw\"")
        );

        // Without a signature: the route still takes the bytes.
        let mut plain = sample::orders();
        sample::add_stripe_webhook(&mut plain);
        let route = plain.services[0].routes.last_mut().unwrap();
        route.steps.remove(0);
        let event = plain
            .messages
            .iter_mut()
            .find(|m| m.name == "StripeEvent")
            .unwrap();
        let mut raw = sb_model::Field::new("raw", sb_model::FieldType::String);
        raw.source = Some(sb_model::RequestSource::RawBody);
        event.fields.push(raw);
        let files = generate(&plain).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        let handler = &http[http.find("async fn handle_stripe_webhook(").unwrap()..];
        assert!(handler.contains("body: Bytes,"));
        assert!(!handler.contains("headers: HeaderMap,"));
        assert!(!handler.contains("webhook::check"));
        assert!(handler.contains("input.raw = __raw_body;"));
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
    fn a_passkey_step_may_name_the_origin_of_the_request() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_passkeys(&mut p);
        let origin = sb_model::Expr::Literal {
            value: serde_json::json!("https://admin.example.com"),
        };
        for route in &mut p.services[0].routes {
            for step in &mut route.steps {
                match step {
                    sb_model::Step::PasskeyRegisterStart { app_url, .. }
                    | sb_model::Step::PasskeyRegisterFinish { app_url, .. }
                    | sb_model::Step::PasskeyLoginStart { app_url, .. }
                    | sb_model::Step::PasskeyLoginFinish { app_url, .. } => {
                        *app_url = Some(origin.clone())
                    }
                    _ => {}
                }
            }
        }
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        // It goes through the JSON as `appUrl`.
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"appUrl\""), "{json}");
        assert_eq!(serde_json::from_str::<sb_model::Project>(&json).unwrap(), p);
        // And reaches the runtime as an `Option<&str>` in the four steps (five calls: the login starts twice).
        let api = file(&generate(&p).unwrap(), "services/orders/src/flows/api.rs").to_string();
        let flat = api.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.matches("Some((").count() >= 5, "{flat}");
        assert!(!flat.contains(", None).await?"), "{flat}");
        assert!(
            !flat.contains("login_discoverable_start(&mut tx, None)"),
            "{flat}"
        );
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
        assert!(flat.contains("let challenge: pb::PasskeyChallenge = { let challenge = eda_runtime::webauthn::register_start(&mut tx, &claims.sub.clone(), &input.name.clone(), None).await?;"), "{flat}");
        assert!(flat.contains("eda_runtime::webauthn::register_finish(&mut tx, &input.state_id.clone(), &input.credential.clone(), &claims.sub.clone(), None).await?;"), "{flat}");
        assert!(
            flat.contains(
                "eda_runtime::webauthn::login_start(&mut tx, &input.account.clone(), None)"
            ),
            "{flat}"
        );
        // A login that names nobody starts a challenge that names nobody; its second request is the same step.
        assert!(
            flat.contains("eda_runtime::webauthn::login_discoverable_start(&mut tx, None).await?"),
            "{flat}"
        );
        assert!(flat.contains("let account: String = eda_runtime::webauthn::login_finish(&mut tx, &input.state_id.clone(), &input.credential.clone(), None).await?;"), "{flat}");
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
            "ADD_STATES_ORIGIN",
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
    fn a_live_event_is_pushed_after_the_commit_through_the_channel_of_the_realtime_service() {
        let mut p = sample::orders();
        sample::add_live_events(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let rpc = file(&files, "services/orders/src/flows/rpc.rs");
        let flat = rpc.split_whitespace().collect::<Vec<_>>().join(" ");
        // The buffer lives with the transaction, the event is kept in it, and sent right after the commit.
        assert!(
            flat.contains("let mut live = eda_runtime::live::Live::default();"),
            "{flat}"
        );
        assert!(
            flat.contains("live.push(\"live-events\", eda_runtime::live::Event { kind: \"orders.created\", account_id: None, tenant_id: None, permission: Some(\"orders.view\"), account_ids: Vec::new(), origin: None, grants: None, data: vec![(\"id\", (order_id.clone()).to_string())], });"),
            "{flat}"
        );
        let commit = flat.find("tx.commit().await?; live.flush().await;");
        assert!(commit.is_some(), "{flat}");
        assert!(flat.find("live.push(").unwrap() < commit.unwrap());
        // Only this service brings the Redis client in, and it checks where the server is at start-up.
        assert!(file(&files, "services/orders/Cargo.toml").contains("features = [\"live\"]"));
        assert!(
            file(&files, "services/orders/src/app/config.rs")
                .contains("eda_runtime::live::check_config()?;")
        );
        assert!(
            file(&files, "runtime/src/lib.rs")
                .contains("#[cfg(feature = \"live\")]\npub mod live;")
        );
        assert!(file(&files, "docker-compose.yml").contains("dragonfly:"));
        // Nothing of it in a service that pushes nothing.
        for (path, absent) in [
            ("services/payments/Cargo.toml", "live"),
            ("services/payments/src/flows/consumers.rs", "live"),
            ("services/payments/src/app/config.rs", "live::"),
        ] {
            assert!(!file(&files, path).contains(absent), "{path}");
        }
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/Cargo.toml").contains("live"));
        assert!(!file(&plain, "services/orders/src/flows/rpc.rs").contains("live"));
    }

    #[test]
    fn a_shared_rate_limit_counts_in_the_store_and_only_its_service_brings_the_client_in() {
        let mut p = sample::orders();
        let mut found = 0;
        for step in &mut p.services[0].routes[0].steps {
            if let sb_model::Step::RateLimit { shared, .. } = step {
                *shared = true;
                found += 1;
            }
        }
        assert_eq!(found, 1, "the sample has one rate limit on its first route");
        let files = generate(&p).unwrap();
        let flat = file(&files, "services/orders/src/flows/api.rs");
        assert!(
            flat.contains(
                "eda_runtime::rate_limit::check_shared(\"h8\", &(\"place_order\".to_string()), 100, 60).await?;"
            ),
            "{flat}"
        );
        assert!(!flat.contains("rate_limit::check(\"h8\""), "{flat}");
        assert!(
            file(&files, "services/orders/Cargo.toml").contains("features = [\"shared-limit\"]")
        );
        assert!(
            file(&files, "services/orders/src/app/config.rs")
                .contains("eda_runtime::rate_limit::check_config()?;")
        );
        assert!(file(&files, "runtime/Cargo.toml").contains("shared-limit = [\"dep:redis\"]"));
        // The store is in the stack even though the project has no real-time service, and the service waits for it.
        let compose = file(&files, "docker-compose.yml");
        assert!(
            compose.contains("dragonfly:") && compose.contains("6379:6379"),
            "{compose}"
        );
        // Without the option: none of it.
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/Cargo.toml").contains("shared-limit"));
        assert!(
            !file(&plain, "services/orders/src/app/config.rs").contains("rate_limit::check_config")
        );
        assert!(!file(&plain, "docker-compose.yml").contains("dragonfly"));
    }

    #[test]
    fn meters_and_quotas_keep_their_uses_in_the_database_of_the_service() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_usage(&mut p);
        let files = generate(&p).unwrap();
        let flat = file(&files, "services/orders/src/flows/api.rs").replace("\n", " ");
        let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
        // The check holds a lock until the transaction ends, so it is in the run of the use that follows it.
        let check = flat.find(
            "let calls_left: i64 = eda_runtime::usage::check(&mut tx, \"api_calls\", &(claims.tid.clone()), 1000, 1_i64, eda_runtime::usage::Period::Month).await?;",
        );
        let meter = flat.find(
            "eda_runtime::usage::meter(&mut tx, \"api_calls\", &(claims.tid.clone()), 1, None).await?;",
        );
        assert!(check.is_some() && meter.is_some(), "{flat}");
        assert!(check < meter);
        // The table, its indexes and the view are created with the schema, only in the service that meters.
        let schema = file(&files, "services/orders/src/app/schema.rs");
        for statement in [
            "CREATE_EVENTS_TABLE",
            "CREATE_EVENTS_INDEX",
            "CREATE_EVENTS_UNIQUE",
            "CREATE_DAILY_VIEW",
        ] {
            assert!(
                schema.contains(&format!("eda_runtime::usage::{statement},")),
                "{statement}"
            );
        }
        assert!(file(&files, "runtime/src/lib.rs").contains("pub mod usage;"));
        assert!(
            file(&files, "runtime/src/usage.rs")
                .contains("CREATE TABLE IF NOT EXISTS usage_events")
        );
        assert!(!file(&files, "services/payments/src/app/schema.rs").contains("usage"));
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/src/app/schema.rs").contains("usage"));
        // The quota is one of the answers the document of the route lists.
        let openapi = file(&files, "services/orders/openapi.json");
        assert!(
            openapi.contains("The quota is used up for the period"),
            "{openapi}"
        );
    }

    #[test]
    fn a_challenge_is_handed_out_on_its_path_and_checked_first_by_the_route_that_asks_for_it() {
        let mut p = sample::orders();
        sample::add_challenge(&mut p);
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(
            http.contains(".route(\"/challenge\", get(challenge_document))"),
            "{http}"
        );
        assert!(
            http.contains("eda_runtime::challenge::issue(\"CHALLENGE_HMAC_KEY\", 1000000, 300)"),
            "{http}"
        );
        // Never cached: each visitor gets their own.
        assert!(
            http.contains("(axum::http::header::CACHE_CONTROL, \"no-store\")"),
            "{http}"
        );
        // The key is checked when the service starts; the solutions already accepted live in the database of the service.
        assert!(
            file(&files, "services/orders/src/app/config.rs")
                .contains("eda_runtime::challenge::check_config(\"CHALLENGE_HMAC_KEY\")?;")
        );
        assert!(
            file(&files, "services/orders/src/app/schema.rs")
                .contains("eda_runtime::challenge::CREATE_USES_TABLE,")
        );
        // The route checks it before anything else.
        let api = file(&files, "services/orders/src/flows/api.rs");
        let check = api
            .find("eda_runtime::challenge::verify(&mut tx, \"CHALLENGE_HMAC_KEY\", 1000000, &(input.challenge.clone())).await?;")
            .unwrap_or_else(|| panic!("{api}"));
        let fn_start = api.find("pub async fn contact(").unwrap();
        let first_step = api[fn_start..].find("eda_runtime::").unwrap() + fn_start;
        assert!(
            first_step <= check,
            "nothing but the transaction comes before the check"
        );
        // The document of the service tells the front where the challenge is, and the proxy sends the path to the service.
        let openapi = file(&files, "services/orders/openapi.json");
        assert!(
            openapi.contains("\"/challenge\"") && openapi.contains("\"maxnumber\""),
            "{openapi}"
        );
        assert!(
            openapi.contains("The challenge is missing, wrong, expired or already used"),
            "{openapi}"
        );
        assert!(file(&files, "runtime/src/lib.rs").contains("pub mod challenge;"));
        // A service that has none: none of it.
        let plain = generate(&sample::orders()).unwrap();
        let http = file(&plain, "services/orders/src/transport/http.rs");
        assert!(!http.contains("challenge"), "{http}");
        assert!(!file(&plain, "services/orders/src/app/schema.rs").contains("challenge"));
        assert!(!file(&plain, "services/orders/src/app/config.rs").contains("challenge"));
        assert!(!file(&plain, "services/orders/openapi.json").contains("challenge"));
    }

    #[test]
    fn a_password_is_judged_before_it_is_hashed_and_only_its_service_brings_the_estimator_in() {
        let mut p = sample::orders();
        sample::add_signup(&mut p);
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs").replace('\n', " ");
        let api = api.split_whitespace().collect::<Vec<_>>().join(" ");
        let judge = api
            .find("eda_runtime::password::check(&(input.password.clone()), &[(input.email.clone()).as_str()], 3, true).await?;")
            .unwrap_or_else(|| panic!("{api}"));
        let hash = api.find("eda_runtime::crypto::hash_password").unwrap();
        assert!(judge < hash, "the password is judged before it is hashed");
        assert!(file(&files, "services/orders/Cargo.toml").contains("\"password-strength\""));
        assert!(
            file(&files, "runtime/Cargo.toml")
                .contains("password-strength = [\"dep:zxcvbn\", \"dep:reqwest\"]")
        );
        assert!(
            file(&files, "runtime/src/lib.rs")
                .contains("#[cfg(feature = \"password-strength\")]\npub mod password;")
        );
        assert!(file(&files, "Cargo.toml").contains("zxcvbn = \"3\""));
        let openapi = file(&files, "services/orders/openapi.json");
        assert!(
            openapi.contains("The password is too easy to guess"),
            "{openapi}"
        );
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/Cargo.toml").contains("password-strength"));
    }

    #[test]
    fn api_keys_are_issued_checked_and_revoked_in_the_transaction_of_the_handler() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_api_keys(&mut p);
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs").replace('\n', " ");
        let api = api.split_whitespace().collect::<Vec<_>>().join(" ");
        for expected in [
            "let (api_key, api_key_id): (String, String) = { let created = eda_runtime::api_key::create(&mut tx, &(claims.tid.clone()), &(input.name.clone()), &[\"orders:read\"], None).await?; (created.key, created.key_id) };",
            "eda_runtime::api_key::revoke(&mut tx, &(input.key_id.clone()), Some((claims.tid.clone()).as_str())).await?;",
            "let owner: String = eda_runtime::api_key::verify(&mut tx, &(input.api_key.clone()), Some(\"orders:read\")).await?.owner;",
        ] {
            assert!(api.contains(expected), "{expected}\n{api}");
        }
        // The table (only its hash of each secret) is created with the schema, only in the service that has the steps.
        let schema = file(&files, "services/orders/src/app/schema.rs");
        assert!(
            schema.contains("eda_runtime::api_key::CREATE_TABLE,")
                && schema.contains("eda_runtime::api_key::CREATE_OWNER_INDEX,")
        );
        assert!(!file(&files, "services/payments/src/app/schema.rs").contains("api_key"));
        assert!(file(&files, "runtime/src/lib.rs").contains("pub mod api_key;"));
        // The document says what a client may be answered.
        let openapi = file(&files, "services/orders/openapi.json");
        for answer in [
            "The API key is missing or not valid",
            "The API key is not allowed to do this",
            "No such API key",
        ] {
            assert!(openapi.contains(answer), "{answer}");
        }
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/src/app/schema.rs").contains("api_key"));
    }

    #[test]
    fn a_service_that_asks_for_it_compresses_its_answers_and_only_it_builds_the_codecs() {
        let mut p = sample::orders();
        p.services[0].compression = true;
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(
            http.contains("use tower_http::compression::{")
                && http.contains("CompressionLayer,")
                && http.contains("predicate::{DefaultPredicate, Predicate, SizeAbove},"),
            "{http}"
        );
        // After the CORS layer (so that its headers are on the compressed answer), and not for small answers.
        let cors = http.find(".layer(cors(config))").unwrap();
        let compression = http
            .find(".layer(CompressionLayer::new().compress_when(DefaultPredicate::new().and(SizeAbove::new(256))))")
            .unwrap_or_else(|| panic!("{http}"));
        assert!(cors < compression);
        assert!(
            file(&files, "services/orders/Cargo.toml")
                .contains("tower-http = { workspace = true, features = [\"compression-full\"] }")
        );
        // The other services do not pay for the codecs, and a project that asks for nothing writes nothing of it.
        assert!(!file(&files, "services/payments/Cargo.toml").contains("compression"));
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/src/transport/http.rs").contains("ompression"));
        assert!(!file(&plain, "services/orders/Cargo.toml").contains("compression"));
    }

    #[test]
    fn an_html_text_is_cleaned_by_the_runtime_and_only_the_service_that_does_it_builds_the_parser()
    {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_notes(&mut p);
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(
            api.contains("eda_runtime::html::sanitize(&(input.body.clone()))"),
            "{api}"
        );
        assert!(file(&files, "services/orders/Cargo.toml").contains("\"html\""));
        assert!(file(&files, "runtime/Cargo.toml").contains("html = [\"dep:ammonia\"]"));
        assert!(
            file(&files, "runtime/src/lib.rs")
                .contains("#[cfg(feature = \"html\")]\npub mod html;")
        );
        assert!(file(&files, "Cargo.toml").contains("ammonia = \"4\""));
        assert!(!file(&files, "services/payments/Cargo.toml").contains("html"));
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/Cargo.toml").contains("\"html\""));
    }

    #[test]
    fn a_call_through_a_circuit_names_its_policy_and_one_without_is_left_as_it_was() {
        let files = generate(&sample::showcase()).unwrap();
        let flat = |path: &str| {
            file(&files, path)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        // httpCall: the policy is a field of the call, with the dependency the address in the environment names.
        let api = flat("services/orders/src/flows/api.rs");
        assert!(
            api.contains("resilience: eda_runtime::resilience::Policy { circuit: Some(eda_runtime::resilience::Circuit { failures: 5, window_secs: 30, open_secs: 30 }), max_concurrent: Some(20) },"),
            "{api}"
        );
        assert!(
            api.contains("resilience: eda_runtime::resilience::Policy { circuit: Some(eda_runtime::resilience::Circuit { failures: 3, window_secs: 60, open_secs: 20 }), max_concurrent: None },"),
            "{api}"
        );
        // callRpc: through the circuit of the service called, a refusal a 503, and the verdict from what the service answered.
        let consumers = flat("services/notifications/src/flows/consumers.rs");
        let rpc = consumers
            .find("eda_runtime::resilience::enter(eda_runtime::resilience::Policy { circuit: Some(eda_runtime::resilience::Circuit { failures: 5, window_secs: 30, open_secs: 30 }), max_concurrent: None }, \"Orders\", \"n2\")?;")
            .unwrap_or_else(|| panic!("{consumers}"));
        let verdict = consumers
            .find("tonic::Code::Unavailable | tonic::Code::DeadlineExceeded")
            .unwrap();
        assert!(rpc < verdict);
        assert!(
            consumers.contains("__result }?.into_inner()"),
            "{consumers}"
        );
        // Without the options, the call is the plain one: nothing of the policy is written.
        let mut bare = sample::orders();
        for step in bare
            .services
            .iter_mut()
            .flat_map(|s| s.consumers.iter_mut())
            .flat_map(|c| c.steps.iter_mut())
        {
            if let sb_model::Step::CallRpc {
                circuit,
                max_concurrent,
                ..
            } = step
            {
                (*circuit, *max_concurrent) = (None, None);
            }
        }
        let plain = generate(&bare).unwrap();
        let consumers = file(&plain, "services/notifications/src/flows/consumers.rs");
        assert!(
            consumers.contains(".await?.into_inner()") && !consumers.contains("resilience"),
            "{consumers}"
        );
        assert!(!file(&plain, "services/orders/src/flows/api.rs").contains("resilience: "));
    }

    #[test]
    fn numbers_without_a_gap_ids_that_sort_and_short_ids_are_written_where_they_are_asked() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_invoices(&mut p);
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs").replace('\n', " ");
        let api = api.split_whitespace().collect::<Vec<_>>().join(" ");
        for expected in [
            "let invoice_id: String = eda_runtime::logic::new_uuid_v7();",
            "let number: i64 = eda_runtime::sequence::next(&mut tx, \"invoices\", &(format!(\"{}-2026\", claims.tid.clone())), 1_i64).await?;",
            "let reference: String = eda_runtime::short_id::encode(number.clone())?;",
        ] {
            assert!(api.contains(expected), "{expected}\n{api}");
        }
        // The number is taken after the identifier and before the reference that is made of it.
        let at = |needle: &str| api.find(needle).unwrap();
        assert!(
            at("new_uuid_v7") < at("sequence::next")
                && at("sequence::next") < at("short_id::encode")
        );
        // The table is created with the schema, Sqids only built by the service that asks.
        assert!(
            file(&files, "services/orders/src/app/schema.rs")
                .contains("eda_runtime::sequence::CREATE_TABLE,")
        );
        assert!(file(&files, "services/orders/Cargo.toml").contains("\"short-id\""));
        assert!(file(&files, "runtime/Cargo.toml").contains("short-id = [\"dep:sqids\"]"));
        assert!(
            file(&files, "runtime/src/lib.rs")
                .contains("#[cfg(feature = \"short-id\")]\npub mod short_id;")
        );
        assert!(
            file(&files, "Cargo.toml").contains("sqids = \"0.4\"")
                && file(&files, "Cargo.toml").contains("\"v7\"")
        );
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/Cargo.toml").contains("short-id"));
        assert!(!file(&plain, "services/orders/src/app/schema.rs").contains("sequence"));
    }

    #[test]
    fn calendar_arithmetic_goes_through_the_zones_of_the_runtime_and_only_its_service_builds_them()
    {
        let mut p = sample::orders();
        sample::add_calendar(&mut p);
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs").replace('\n', " ");
        let api = api.split_whitespace().collect::<Vec<_>>().join(" ");
        for expected in [
            "let instant: i64 = eda_runtime::time_zone::now_seconds();",
            "today: eda_runtime::time_zone::local_date(instant.clone(), &(input.zone.clone()))?",
            "day_start: eda_runtime::time_zone::start_of_day(instant.clone(), &(input.zone.clone()))?",
            "next_week: eda_runtime::time_zone::add_days(instant.clone(), 7, &(input.zone.clone()))?",
            "next_month: eda_runtime::time_zone::add_months(instant.clone(), 1, &(input.zone.clone()))?",
            "paris_now: eda_runtime::time_zone::local_time(instant.clone(), &(\"Europe/Paris\".to_string()))?",
        ] {
            assert!(api.contains(expected), "{expected}\n{api}");
        }
        assert!(file(&files, "services/orders/Cargo.toml").contains("\"time-zones\""));
        assert!(file(&files, "runtime/Cargo.toml").contains("time-zones = [\"dep:jiff\"]"));
        assert!(
            file(&files, "runtime/src/lib.rs")
                .contains("#[cfg(feature = \"time-zones\")]\npub mod time_zone;")
        );
        // The zones are bundled: the service does not read the tzdata of the machine it runs on.
        assert!(file(&files, "Cargo.toml").contains("tzdb-bundle-always"));
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/Cargo.toml").contains("time-zones"));
    }

    #[test]
    fn a_change_is_traced_in_its_transaction_and_the_journal_is_read_by_its_company() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_audit(&mut p);
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs").replace('\n', " ");
        let api = api.split_whitespace().collect::<Vec<_>>().join(" ");
        for expected in [
            "eda_runtime::audit::record(&mut tx, eda_runtime::audit::Event { action: \"invoice.sent\", target_type: \"invoice\", target: &(input.invoice_id.clone()), actor: &(claims.sub.clone()), tenant: Some(&(claims.tid.clone())), before: Some(eda_runtime::audit::Values::new().set(\"status\", \"draft\".to_string())), after: Some(eda_runtime::audit::Values::new().set(\"status\", \"sent\".to_string())) }).await?;",
            "let __page = eda_runtime::audit::read(&mut tx, eda_runtime::audit::Filter { tenant: &(claims.tid.clone()), target_type: None, target: None, action: None, cursor: Some(",
            "limit: 50 }).await?;",
            "let next_cursor: Option<i64> = __page.next;",
            "let entries: Vec<pb::AuditEntry> = __page.entries.into_iter().map(|e| pb::AuditEntry {",
        ] {
            assert!(api.contains(expected), "{expected}\n{api}");
        }
        // The trace is written in the transaction of the change, before it commits.
        let trace = api.find("eda_runtime::audit::record(").unwrap();
        let commit = api[trace..].find("tx.commit().await?;").unwrap();
        assert!(commit > 0);
        // The journal, its indexes and the trigger that keeps it append-only are created with the schema, only in this service.
        let schema = file(&files, "services/orders/src/app/schema.rs");
        for statement in [
            "CREATE_TABLE",
            "CREATE_TENANT_INDEX",
            "CREATE_TARGET_INDEX",
            "CREATE_GUARD_FUNCTION",
            "CREATE_GUARD_TRIGGER",
        ] {
            assert!(
                schema.contains(&format!("eda_runtime::audit::{statement},")),
                "{statement}"
            );
        }
        assert!(!file(&files, "services/payments/src/app/schema.rs").contains("audit"));
        assert!(file(&files, "runtime/src/lib.rs").contains("pub mod audit;"));
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/src/app/schema.rs").contains("audit"));
    }

    #[test]
    fn a_flag_is_asked_in_the_transaction_with_what_it_starts_from_and_its_table_is_created() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_flags(&mut p);
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs").replace('\n', " ");
        let api = api.split_whitespace().collect::<Vec<_>>().join(" ");
        for expected in [
            "let __decision = eda_runtime::flags::evaluate(&mut tx, \"new_checkout\", &(claims.sub.clone()), &eda_runtime::flags::Properties::new().set(\"tenant\", claims.tid.clone()), eda_runtime::flags::Start { enabled: true, rollout: 25 }).await?;",
            "let new_checkout: bool = __decision.on;",
            "let variant: String = __decision.variant;",
        ] {
            assert!(api.contains(expected), "{expected}\n{api}");
        }
        assert!(
            file(&files, "services/orders/src/app/schema.rs")
                .contains("eda_runtime::flags::CREATE_TABLE,")
        );
        assert!(!file(&files, "services/payments/src/app/schema.rs").contains("flags"));
        assert!(file(&files, "runtime/src/lib.rs").contains("pub mod flags;"));
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/src/app/schema.rs").contains("flags"));
    }

    #[test]
    fn the_live_steps_send_their_control_messages_after_the_commit() {
        let mut p = sample::orders();
        sample::add_live_events(&mut p);
        let rpc = p
            .services
            .iter_mut()
            .find(|s| s.name == "Orders")
            .unwrap()
            .rpcs
            .iter_mut()
            .find(|r| r.name == "CreateOrder")
            .unwrap();
        let at = rpc
            .steps
            .iter()
            .position(|s| matches!(s, sb_model::Step::Respond { .. }))
            .unwrap();
        rpc.steps.insert(
            at,
            serde_json::from_value(serde_json::json!({
                "type": "liveGrants", "id": "g1", "service": "Realtime",
                "accountId": {"kind": "path", "path": "order_id"},
                "tenantId": {"kind": "path", "path": "order_id"},
            }))
            .unwrap(),
        );
        let path = |p: &str| serde_json::json!({"kind": "path", "path": p});
        rpc.steps.insert(
            at,
            serde_json::from_value(serde_json::json!({
                "type": "sessionTrace", "id": "t1", "service": "Realtime",
                "sessionId": path("order_id"), "accountId": path("order_id"),
                "ttlSeconds": {"kind": "literal", "value": 900},
            }))
            .unwrap(),
        );
        rpc.steps.insert(
            at,
            serde_json::from_value(serde_json::json!({
                "type": "sessionRevoke", "id": "t2", "service": "Realtime", "sessionId": path("order_id"),
            }))
            .unwrap(),
        );
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let flat = file(&files, "services/orders/src/flows/rpc.rs")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            flat.contains("kind: \"grants.updated\", account_id: Some((order_id.clone()).to_string()), tenant_id: Some((order_id.clone()).to_string()), grants: None, ..Default::default() });"),
            "{flat}"
        );
        assert!(
            flat.contains("live.trace(\"sess\", (order_id.clone()).to_string(), (order_id.clone()).to_string(), None, 900"),
            "{flat}"
        );
        assert!(
            flat.contains("live.revoke(\"sess\", (order_id.clone()).to_string());"),
            "{flat}"
        );
        let commit = flat
            .find("tx.commit().await?; live.flush().await;")
            .expect("flushed after the commit");
        assert!(flat.find("live.trace(").unwrap() < commit);
    }

    #[test]
    fn objects_are_stored_in_a_bucket_of_the_environment_and_images_get_room_in_the_body() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_avatars(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        let flat = api.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("let content_type: String = eda_runtime::s3::sniff_image(&input.image.clone(), &[\"png\", \"jpeg\"], 2097152)?.to_string();"), "{flat}");
        assert!(flat.contains("eda_runtime::s3::put(\"AVATAR_BUCKET\", &claims.sub.clone(), &input.image.clone(), &content_type.clone()"), "{flat}");
        assert!(flat.contains("let avatar: Option<String> = eda_runtime::s3::get(\"AVATAR_BUCKET\", &claims.sub.clone()).await?;"), "{flat}");
        assert!(
            flat.contains(
                "eda_runtime::s3::delete(\"AVATAR_BUCKET\", &claims.sub.clone()).await?;"
            ),
            "{flat}"
        );
        // The direct way: the service signs, the client talks to the store.
        assert!(flat.contains("let url: String = eda_runtime::s3::presign_put(\"AVATAR_BUCKET\", &claims.sub.clone(), &input.content_type.clone(), input.size.clone(), 2097152, 300).await?;"), "{flat}");
        assert!(flat.contains("let url: String = eda_runtime::s3::presign_get(\"AVATAR_BUCKET\", &claims.sub.clone(), 300).await?;"), "{flat}");
        // The SDK is built only for the service that stores objects, and a missing bucket stops it at start-up.
        assert!(file(&files, "services/orders/Cargo.toml").contains("features = [\"s3\"]"));
        assert!(
            file(&files, "services/orders/src/app/config.rs")
                .contains("eda_runtime::s3::check_config(&[\"AVATAR_BUCKET\"])?;")
        );
        // A 2 MiB image is 2.8 MB of base64 in the JSON: the route takes a bigger body than axum's 2 MB, and only that route.
        let http = file(&files, "services/orders/src/transport/http.rs");
        let limit = (2 * 1024 * 1024usize).div_ceil(3) * 4 + 16 * 1024;
        assert!(
            http.contains(&format!(".layer(DefaultBodyLimit::max({limit}))")),
            "{http}"
        );
        assert_eq!(http.matches("DefaultBodyLimit::max").count(), 1);
        assert!(http.contains("DefaultBodyLimit"));
        // Nothing of it elsewhere.
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/Cargo.toml").contains("s3"));
        assert!(
            !file(&plain, "services/orders/src/transport/http.rs").contains("DefaultBodyLimit")
        );
        assert!(!file(&plain, "services/orders/src/app/config.rs").contains("s3::"));
        assert!(
            file(&files, "runtime/src/lib.rs").contains("#[cfg(feature = \"s3\")]\npub mod s3;")
        );
    }

    #[test]
    fn an_outbound_call_is_built_from_the_flow_and_its_client_is_only_built_where_it_is_used() {
        let mut p = sample::orders();
        sample::add_http_calls(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        let flat = api.split_whitespace().collect::<Vec<_>>().join(" ");
        // Stripe: form body, bearer key, the order as idempotency key, a stop on failure, the answer in a schema.
        for expected in [
            "__path.push_str(\"/v1/payment_intents\");",
            "__body.set(\"metadata[order_id]\", input.order_id.clone());",
            "auth: eda_runtime::http_client::Auth::Bearer(\"STRIPE_SECRET_KEY\"),",
            "format: eda_runtime::http_client::BodyFormat::Form,",
            "retries: 2,",
            "idempotency_key: Some(input.order_id.clone()),",
            "base_url_env: \"STRIPE_API_URL\",",
            "__answer.require_success(502, \"the payment provider did not accept the payment\")?;",
            "let intent: pb::StripePaymentIntent = __answer.parse()?;",
            // Twilio: basic authentication, the flow reads the status.
            "auth: eda_runtime::http_client::Auth::Basic(\"TWILIO_ACCOUNT_SID\", \"TWILIO_AUTH_TOKEN\"),",
            "__body.set(\"From\", \"+15005550006\");",
            "let sms_status: i32 = i32::from(__answer.status);",
            "let message: pb::TwilioMessage = __answer.parse()?;",
        ] {
            assert!(flat.contains(expected), "{expected}\n{flat}");
        }
        // The Twilio call goes on whatever the answer: no early return after it.
        assert!(
            !flat.contains("__answer.require_success(502, \"the SMS"),
            "{flat}"
        );
        // The runtime client and the check of the addresses and secrets at start-up.
        assert!(
            file(&files, "services/orders/Cargo.toml").contains("features = [\"http-client\"]")
        );
        let config = file(&files, "services/orders/src/app/config.rs");
        assert!(
            config.contains("eda_runtime::http_client::check_config(&[(\"STRIPE_API_URL\", &[\"STRIPE_SECRET_KEY\"][..]), (\"TWILIO_API_URL\", &[\"TWILIO_ACCOUNT_SID\", \"TWILIO_AUTH_TOKEN\"][..])])?;")
                || config.split_whitespace().collect::<Vec<_>>().join(" ").contains("http_client::check_config"),
            "{config}"
        );
        assert!(
            file(&files, "runtime/src/lib.rs")
                .contains("#[cfg(feature = \"http-client\")]\npub mod http_client;")
        );
        assert!(file(&files, "runtime/Cargo.toml").contains("http-client = [\"dep:reqwest\"]"));
        // Nothing of it in a service that makes no call.
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "services/orders/Cargo.toml").contains("http-client"));
        assert!(!file(&plain, "services/orders/src/app/config.rs").contains("http_client"));
    }

    #[test]
    fn the_values_of_a_path_go_through_the_segment_encoder_and_optional_values_are_left_out_when_absent()
     {
        let mut p = sample::orders();
        sample::add_http_calls(&mut p);
        let step = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "create_payment_intent")
            .map(|r| &mut r.steps[0])
            .unwrap();
        let sb_model::Step::HttpCall { path, headers, .. } = step else {
            panic!("an httpCall")
        };
        *path = "/v1/customers/{input.order_id}/charges".into();
        headers.push(sb_model::Assignment {
            field: "X-Region".into(),
            value: sb_model::Expr::Literal { value: "eu".into() },
        });
        let files = generate(&p).unwrap();
        let flat = file(&files, "services/orders/src/flows/api.rs")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(flat.contains("__path.push_str(\"/v1/customers/\"); __path.push_str(&eda_runtime::http_client::segment(&input.order_id.clone().to_string())?); __path.push_str(\"/charges\");"), "{flat}");
        assert!(
            flat.contains("__headers.set(\"X-Region\", \"eu\");"),
            "{flat}"
        );
    }

    #[test]
    fn a_tenant_table_is_filtered_and_a_soft_delete_only_marks_the_row() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_notes(&mut p);
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        let flat = api.split_whitespace().collect::<Vec<_>>().join(" ");
        // Insert: the tenant is written with the row, from the variable the handler bound. Nothing in the steps says so.
        assert!(
            flat.contains("INSERT INTO \\\"notes\\\" (\\\"note_id\\\", \\\"body\\\", \\\"tenant_id\\\") VALUES ($1, $2, $3)")
                && flat.contains(".bind(note_id.clone()).bind(eda_runtime::html::sanitize(&(input.body.clone()))).bind(tenant_id.clone())"),
            "{flat}"
        );
        // Select: the tenant, and only the rows that were not deleted.
        assert!(
            flat.contains("FROM \\\"notes\\\" WHERE \\\"note_id\\\" = $1 AND \\\"tenant_id\\\" = $2 AND \\\"deleted_at\\\" IS NULL LIMIT 1"),
            "{flat}"
        );
        // Delete: an update that stamps the row, for this tenant, once.
        assert!(
            flat.contains("UPDATE \\\"notes\\\" SET \\\"deleted_at\\\" = now() WHERE \\\"note_id\\\" = $1 AND \\\"tenant_id\\\" = $2 AND \\\"deleted_at\\\" IS NULL RETURNING \\\"note_id\\\""),
            "{flat}"
        );
        assert!(!flat.contains("DELETE FROM"), "{flat}");
        // A table without these keeps its plain statements.
        let plain = generate(&sample::orders()).unwrap();
        let plain = file(&plain, "services/orders/src/flows/api.rs");
        assert!(!plain.contains("IS NULL") && !plain.contains("tenant"));
    }

    fn with_realtime() -> sb_model::Project {
        let mut p = sample::orders();
        sample::add_realtime(&mut p);
        p
    }

    #[test]
    fn a_realtime_service_is_fixed_code_with_five_settings_in_its_contract() {
        let mut p = with_realtime();
        // Settings that need escaping to be Rust string literals.
        let rt = p
            .services
            .iter_mut()
            .find(|s| s.kind == sb_model::ServiceKind::Realtime)
            .unwrap();
        rt.realtime = Some(sb_model::RealtimeConfig {
            channel: "events:v2".into(),
            path: "/sse/stream".into(),
            access_cookie: "tok".into(),
            issuer: "my \"api\" \\ ✓".into(),
            session_key_prefix: "session".into(),
        });
        let files = generate(&p).unwrap();
        for path in [
            "services/realtime/Cargo.toml",
            "services/realtime/src/main.rs",
            "services/realtime/src/registry.rs",
            "services/realtime/src/http.rs",
            "services/realtime/src/hub.rs",
            "services/realtime/src/sweeper.rs",
            "services/realtime/src/sessions.rs",
            "services/realtime/src/jwt.rs",
            "services/realtime/examples/load.rs",
            "services/realtime/tests/dragonfly.rs",
            "services/realtime/tests/http.rs",
        ] {
            assert!(files.iter().any(|f| f.path == path), "{path}");
        }
        // The project decides five names, and they are Rust literals (the generator checked the file parses).
        let contract = file(&files, "services/realtime/src/contract.rs");
        for line in [
            "pub const CHANNEL: &str = \"events:v2\";",
            "pub const PATH: &str = \"/sse/stream\";",
            "pub const ACCESS_COOKIE: &str = \"tok\";",
            "pub const SESSION_PREFIX: &str = \"session\";",
        ] {
            assert!(contract.contains(line), "{line}\n{contract}");
        }
        assert!(
            contract.contains("pub const ISSUER: &str = \"my \\\"api\\\" \\\\ ✓\";"),
            "{contract}"
        );
        // The port is the one of the service; nothing is left to fill in.
        assert!(
            file(&files, "services/realtime/src/config.rs")
                .contains("number(\"SERVER_PORT\", 8085)")
        );
        for f in files
            .iter()
            .filter(|f| f.path.starts_with("services/realtime/"))
        {
            assert!(
                !f.content.contains("::")
                    || !f.content.contains("::SERVER_PORT::") && !f.content.contains("::CHANNEL::"),
                "{}",
                f.path
            );
        }
        // It is a workspace member with its own dependencies, none of them Kafka, a database or the runtime of the others.
        let cargo = file(&files, "services/realtime/Cargo.toml");
        assert!(
            cargo.contains("name = \"realtime\"") && cargo.contains("name = \"realtime_service\""),
            "{cargo}"
        );
        for dep in ["eda-runtime", "contracts", "sqlx", "rdkafka"] {
            assert!(!cargo.contains(dep), "{dep}");
        }
        // The fixed code does not depend on a general HTTP server: that is where 28 KiB a stream went.
        assert!(
            !cargo.contains("axum") && !cargo.contains("hyper"),
            "{cargo}"
        );
        let workspace = file(&files, "Cargo.toml");
        for dep in ["redis = ", "httparse = ", "rlimit = ", "bytes = "] {
            assert!(workspace.contains(dep), "{dep}");
        }
        assert!(file(&files, "Cargo.toml").contains("\"services/realtime\""));
        // DragonflyDB for development, and the README tells what to set.
        let compose = file(&files, "docker-compose.yml");
        assert!(
            compose.contains("dragonfly:") && compose.contains("6379:6379"),
            "{compose}"
        );
        let readme = file(&files, "README.md");
        assert!(
            readme.contains("### Realtime (real-time service, HTTP port 8085)"),
            "{readme}"
        );
        for word in [
            "GET /sse/stream",
            "events:v2",
            "REALTIME_MAX_CONNECTIONS",
            "DRAGONFLY_URL",
            "--example load",
        ] {
            assert!(readme.contains(word), "{word}");
        }
    }

    #[test]
    fn a_project_without_a_realtime_service_has_no_dragonfly_and_none_of_its_dependencies() {
        let files = generate(&sample::orders()).unwrap();
        assert!(!file(&files, "docker-compose.yml").contains("dragonfly"));
        assert!(
            !files
                .iter()
                .any(|f| f.path.starts_with("services/realtime"))
        );
    }

    fn with_mailer() -> sb_model::Project {
        let mut p = sample::orders();
        sample::add_signup(&mut p);
        sample::add_mailer(&mut p);
        p
    }

    #[test]
    fn a_mail_service_is_fixed_code_with_the_events_and_the_mails_of_the_project() {
        let p = with_mailer();
        let files = generate(&p).unwrap();
        // The fixed service, and the three files that come from the project.
        for path in [
            "services/mailer/Cargo.toml",
            "services/mailer/src/main.rs",
            "services/mailer/src/mta/queue_worker.rs",
            "services/mailer/src/mta/smtp.rs",
            "services/mailer/src/mta/dkim.rs",
            "services/mailer/src/bounce/server.rs",
            "services/mailer/migrations/20260302000001_mail_queue.sql",
            "services/mailer/src/mta_rules.rs",
            "services/mailer/src/core/consumer.rs",
            "services/mailer/src/seed.rs",
        ] {
            assert!(files.iter().any(|f| f.path == path), "{path}");
        }
        // Nothing of an ordinary service in it.
        assert!(
            !files
                .iter()
                .any(|f| f.path.starts_with("services/mailer/src/flows/"))
        );
        let rules = file(&files, "services/mailer/src/mta_rules.rs");
        assert!(
            rules.contains("topic: \"accounts.registered\"")
                && rules.contains("recipient_field: \"email\"")
        );
        assert!(
            rules.contains("dedup_fallback: DedupFallback::Recipient")
                && rules.contains("SenderName::CompanyVia(\"name\")")
        );
        let consumer = file(&files, "services/mailer/src/core/consumer.rs");
        assert!(consumer.contains("pub const TOPICS: &[&str] = &[topics::ACCOUNTS_REGISTERED];"));
        assert!(consumer.contains("eda_runtime::consume::<pb::AccountRegistered, _, _>(&brokers, \"mailer-welcome\", topics::ACCOUNTS_REGISTERED"));
        let seed = file(&files, "services/mailer/src/seed.rs");
        assert!(seed.contains("\"Welcome, {{ name }}\""));
        // Its settings: the database and the port of this service.
        let config = file(&files, "services/mailer/src/core/config.rs");
        assert!(
            config.contains("\"MAILER_DATABASE_URL\"")
                && config.contains("postgres://postgres:postgres@localhost:5432/mailer")
                && config.contains("\"8084\"")
        );
        assert!(
            !config.contains("::SERVER_PORT::") && !config.contains("::DATABASE"),
            "a placeholder was left behind"
        );
        // The events it mails are read as JSON: their messages derive serde in the contracts.
        let build = file(&files, "contracts/build.rs");
        assert!(build.contains("\"AccountRegistered\""), "{build}");
        // The other services are generated as before.
        assert!(
            files
                .iter()
                .any(|f| f.path == "services/orders/src/flows/api.rs")
        );
    }

    #[test]
    fn a_rule_can_send_from_a_declared_sender_with_its_own_dkim_identity() {
        let mut p = with_mailer();
        let mailer = p.services.iter_mut().find(|s| s.name == "Mailer").unwrap();
        mailer.senders = vec![sb_model::MailSender {
            id: "achats".into(),
            email: "achats@mon-entreprise.fr".into(),
            name: Some("Achats".into()),
            dkim_selector: "achats".into(),
            dkim_domain: "mon-entreprise.fr".into(),
        }];
        mailer.consumers[0].mail.as_mut().unwrap().sender = Some("achats".into());
        let files = generate(&p).unwrap();
        let rules = file(&files, "services/mailer/src/mta_rules.rs");
        assert!(rules.contains("sender_id: Some(\"achats\")"), "{rules}");
        assert!(
            rules.contains("email: \"achats@mon-entreprise.fr\"")
                && rules.contains("dkim_selector: \"achats\"")
                && rules.contains("dkim_domain: \"mon-entreprise.fr\""),
            "{rules}"
        );
        assert!(!rules.contains("::SENDERS::"));
        // The key is not in the project: the config reads it from the environment, per sender.
        let config = file(&files, "services/mailer/src/core/config.rs");
        assert!(config.contains("SENDER_{}_DKIM_PRIVATE_KEY"));
        // Same project, same files.
        assert_eq!(files, generate(&p).unwrap());
    }

    #[test]
    fn a_mail_service_without_senders_keeps_the_service_address() {
        let files = generate(&with_mailer()).unwrap();
        let rules = file(&files, "services/mailer/src/mta_rules.rs");
        assert!(rules.contains("sender_id: None"));
        assert!(rules.contains("pub const SENDERS: &[SenderDecl] = &[\n];"));
    }

    #[test]
    fn a_mail_service_comes_with_a_mail_catcher_and_its_instructions() {
        let files = generate(&with_mailer()).unwrap();
        let compose = file(&files, "docker-compose.yml");
        assert!(
            compose.contains("mailpit:")
                && compose.contains("1025:1025")
                && compose.contains("8025:8025")
        );
        serde_yaml_check(compose);
        let readme = file(&files, "README.md");
        assert!(readme.contains("### Mailer (mail service, HTTP port 8084)"));
        assert!(
            readme.contains("DKIM_PRIVATE_KEY_PATH")
                && readme.contains("MAILER_DATABASE_URL")
                && readme.contains("SMTP_HOST")
        );
        assert!(!readme.contains("DB_MAX_CONNECTIONS"));
        // No mail catcher for a project that sends no mail.
        let plain = generate(&sample::orders()).unwrap();
        assert!(!file(&plain, "docker-compose.yml").contains("mailpit"));
    }

    /// The compose file stays three top-level keys with the services indented under `services:`.
    fn serde_yaml_check(compose: &str) {
        let top: Vec<_> = compose
            .lines()
            .filter(|l| !l.starts_with([' ', '#']) && !l.is_empty())
            .collect();
        assert_eq!(top, ["services:", "volumes:"]);
        let services: Vec<_> = compose
            .lines()
            .filter(|l| l.starts_with("  ") && !l.starts_with("   "))
            .map(str::trim)
            .collect();
        assert_eq!(
            services[..4],
            ["kafka:", "postgres:", "mailpit:", "postgres-data:"][..4]
        );
    }

    #[test]
    fn the_fixed_code_of_a_mail_service_carries_the_fixes_and_none_of_its_origin() {
        let files = generate(&with_mailer()).unwrap();
        let mine: Vec<_> = files
            .iter()
            .filter(|f| f.path.starts_with("services/mailer/"))
            .collect();
        // A bare LF never ends a line of the DATA section, and the final dot is not preceded by an empty line.
        let smtp = file(&files, "services/mailer/src/mta/smtp.rs");
        assert!(smtp.contains("split_inclusive") && smtp.contains("body.ends_with(b\"\\r\\n\")"));
        // A PKCS#8 key (what `openssl genrsa` writes) signs as well as a PKCS#1 one.
        assert!(
            file(&files, "services/mailer/src/mta/dkim.rs")
                .contains("PrivateKeyDer::from_pem_slice")
        );
        // The service it was taken from leaves no name behind, in code, comments, migrations or defaults.
        for f in mine {
            let text = f.content.to_lowercase();
            for word in ["clairtyx", "workonva", "svc-mta", "svc_mta", "chantier"] {
                assert!(!text.contains(word), "{} mentions {word}", f.path);
            }
        }
    }

    #[test]
    fn a_mail_that_is_not_a_template_is_refused_before_the_worker_loads_it() {
        let mut p = with_mailer();
        p.services
            .iter_mut()
            .find(|s| s.name == "Mailer")
            .unwrap()
            .templates[0]
            .html = "<p>{{ name </p>".into();
        let Err(CodegenError::Invalid(found)) = generate(&p) else {
            panic!("accepted a broken template")
        };
        let found: Vec<_> = found
            .iter()
            .filter(|d| d.code == "E_MTA_TEMPLATE_SYNTAX")
            .collect();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].path, "/services/3/templates/0/html");
        // A template that uses the built-in filters is fine.
        let mut p = with_mailer();
        p.services
            .iter_mut()
            .find(|s| s.name == "Mailer")
            .unwrap()
            .templates[0]
            .text = "{{ name | upper }} {{ email | urlencode }}".into();
        assert!(generate(&p).is_ok());
    }

    #[test]
    fn a_composed_mail_is_seeded_as_the_html_and_the_text_derived_from_its_blocks() {
        let mut p = with_mailer();
        sample::add_mail_design(&mut p);
        let files = generate(&p).unwrap();
        let mailer = p.services.iter().find(|s| s.name == "Mailer").unwrap();
        let (html, text) = mailer.templates[0].bodies();
        let seed = file(&files, "services/mailer/src/seed.rs");
        // What is seeded is exactly what the model derives, quoted as a Rust string.
        assert!(seed.contains(&format!("{html:?}")));
        assert!(seed.contains(&format!("{text:?}")));
        // The text, asserted whole: it is read by people whose client shows no HTML.
        assert_eq!(
            text,
            "Welcome, {{ name }}\n===================\n\nYour account {{ email }} is ready.\n\nOpen my account: https://example.com/login\n\n----\n\nQuestions? Reply to this mail.\n\nCompany logo\n\n-- \nYou receive this mail because {{ email }} signed up.\n"
        );
        // Rendered the way the service does it: an `.html` template is escaped by Tera, the text is not.
        let mut tera = tera::Tera::default();
        tera.add_raw_templates([("m.html", html.as_str()), ("m.txt", text.as_str())])
            .unwrap();
        let mut ctx = tera::Context::new();
        ctx.insert("name", "<Ann> & co");
        ctx.insert("email", "ann@example.com");
        let html = tera.render("m.html", &ctx).unwrap();
        assert!(
            html.contains(">Welcome, &lt;Ann&gt; &amp; co</h1>"),
            "{html}"
        );
        assert!(html.contains("Your account ann@example.com is ready."));
        assert!(html.contains("<a href=\"https://example.com/login\""));
        assert!(html.contains("<table role=\"presentation\""));
        assert!(!html.contains("{{") && !html.contains("<script"));
        assert!(
            tera.render("m.txt", &ctx)
                .unwrap()
                .starts_with("Welcome, <Ann> & co\n")
        );
    }

    #[test]
    fn a_composed_mail_with_an_unknown_variable_is_refused() {
        let mut p = with_mailer();
        sample::add_mail_design(&mut p);
        p.services
            .iter_mut()
            .find(|s| s.name == "Mailer")
            .unwrap()
            .templates[0]
            .design
            .as_mut()
            .unwrap()
            .blocks[0] = sb_model::MailBlock::heading("Hi {{ nobody }}");
        let Err(CodegenError::Invalid(found)) = generate(&p) else {
            panic!("accepted an unknown variable")
        };
        assert!(found.iter().any(|d| d.code == "E_MAIL_VARIABLE_UNKNOWN"));
    }

    #[test]
    fn a_hand_written_mail_is_seeded_as_it_always_was() {
        let p = with_mailer();
        let files = generate(&p).unwrap();
        let t = &p.services[3].templates[0];
        assert!(t.design.is_none());
        let seed = file(&files, "services/mailer/src/seed.rs");
        assert!(seed.contains(&format!("{:?}", t.html)) && seed.contains(&format!("{:?}", t.text)));
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
                "__claims.insert(\"sub\".to_string(), eda_runtime::jwt::json!(input.email.clone()));"
            ),
            "{api}"
        );
        assert!(api.contains(
            "__claims.insert(\"roles\".to_string(), eda_runtime::jwt::json!([\"member\"]));"
        ));
        assert!(
            api.contains("eda_runtime::jwt::sign(\"JWT_SIGNING_KEY\", \"orders\", 300, __claims)?")
        );
        // The map of the step never carries the name a project may bind (verifyJwt bind = claims).
        assert!(!api.contains("let mut claims"), "{api}");
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
    fn a_token_signed_from_the_claims_of_a_verified_one_reads_the_bound_name() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        // whoami: verifyJwt (bind = claims) then a signJwt whose `sub` reads `claims.sub`.
        let route = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "whoami")
            .unwrap();
        route.steps.insert(
            1,
            sb_model::Step::SignJwt {
                id: "k9".into(),
                key_env: "JWT_SIGNING_KEY".into(),
                issuer: "orders".into(),
                ttl_secs: 300,
                claims: vec![sb_model::Assignment {
                    field: "sub".into(),
                    value: sb_model::Expr::Path {
                        path: "claims.sub".into(),
                    },
                }],
                bind: "renewed".into(),
            },
        );
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(api.contains("let claims: pb::AccessClaims ="), "{api}");
        assert!(
            api.contains(
                "__claims.insert(\"sub\".to_string(), eda_runtime::jwt::json!(claims.sub.clone()));"
            ),
            "{api}"
        );
        assert!(!api.contains("let mut claims"), "{api}");
    }

    #[test]
    fn a_token_of_another_issuer_gets_its_own_status_and_code() {
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        if let sb_model::Step::VerifyJwt { other_issuer, .. } =
            &mut p.services[0].routes[2].steps[0]
        {
            *other_issuer = Some(sb_model::OtherIssuer {
                issuer: "orders-chpwd".into(),
                status: 403,
                message: "password change required".into(),
                code: Some("MDP_CHANGEMENT_REQUIS".into()),
            });
        }
        assert!(
            !sb_model::has_errors(&sb_model::validate(&p)),
            "{:?}",
            sb_model::validate(&p)
        );
        let files = generate(&p).unwrap();
        let api = file(&files, "services/orders/src/flows/api.rs");
        assert!(
            api.contains("Some(eda_runtime::jwt::Other { issuer: \"orders-chpwd\", status: 403, message: \"password change required\", code: Some(\"MDP_CHANGEMENT_REQUIS\") })"),
            "{api}"
        );
        assert!(file(&files, "runtime/src/jwt.rs").contains("pub async fn verify_or"));
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
    fn response_headers_are_set_by_the_reply_and_refused_when_invalid() {
        let files = generate(&sample::showcase()).unwrap();
        let flows = file(&files, "services/orders/src/flows/api.rs");
        let receipt = &flows[flows.find("pub async fn put_receipt(").unwrap()..];
        assert!(receipt.contains(".header(\"Cache-Control\", \"no-store\".to_string())"));
        assert!(receipt.contains(".header(\"X-Order-Id\", input.order_id.clone())"));
        // A header value may be a text with values in it.
        assert!(receipt.contains(".header(\"X-Receipt-Ref\", format!("));
        // The handler checks what the flow computed before it goes on the wire: a bad value is a 500, never a split header.
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(http.contains("eda_runtime::http::header_value_ok(v)"));
        assert!(http.contains("response header refused"));
    }

    #[test]
    fn the_openapi_document_is_written_and_served_only_when_asked() {
        let mut p = sample::showcase();
        p.openapi.serve = false;
        let files = generate(&p).unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(file(&files, "services/orders/openapi.json")).unwrap();
        assert_eq!(doc["openapi"], "3.1.0");
        // A service without routes has none.
        assert!(
            !files
                .iter()
                .any(|f| f.path == "services/payments/openapi.json")
        );
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(!http.contains("openapi"));
        p.openapi.serve = true;
        p.openapi.path = "/docs/api.json".into();
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(http.contains(".route(\"/docs/api.json\", get(openapi_document))"));
        assert!(http.contains("include_str!(\"../../openapi.json\")"));
        let routing = http
            .lines()
            .find(|l| l.contains("routing::{"))
            .expect("the routing import");
        assert!(
            routing
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w == "get"),
            "{routing}"
        );
    }

    #[test]
    fn a_json_name_renames_the_key_of_a_field() {
        let mut p = sample::showcase();
        let f = p
            .messages
            .iter_mut()
            .find(|m| m.name == "ReceiptBody")
            .unwrap()
            .fields
            .iter_mut()
            .find(|f| f.name == "issued_at")
            .unwrap();
        f.json_name = Some("issuedAt".into());
        let files = generate(&p).unwrap();
        let build = file(&files, "contracts/build.rs");
        assert!(build.contains("ReceiptBody.issued_at\", \"#[serde(rename = \\\"issuedAt\\\")]"));
        assert!(!build.contains("rename = \\\"stamp"));
    }

    #[test]
    fn bytes_and_timestamps_travel_as_base64_and_rfc3339() {
        let files = generate(&sample::showcase()).unwrap();
        let build = file(&files, "contracts/build.rs");
        assert!(
            build.contains("ReceiptBody.stamp\", \"#[serde(with = \\\"crate::json::bytes\\\")]")
        );
        assert!(build.contains(
            "ReceiptBody.issued_at\", \"#[serde(with = \\\"crate::json::timestamp\\\")]"
        ));
        assert!(file(&files, "contracts/src/lib.rs").contains("pub mod json;"));
        assert!(file(&files, "contracts/Cargo.toml").contains("base64.workspace = true"));
        assert!(file(&files, "contracts/src/json.rs").contains("pub mod timestamp_list"));
        // Nothing of it for a project that has neither.
        let plain = generate(&sample::orders()).unwrap();
        assert!(!plain.iter().any(|f| f.path == "contracts/src/json.rs"));
        assert!(!file(&plain, "contracts/Cargo.toml").contains("chrono"));
        assert!(!file(&plain, "contracts/build.rs").contains("field_attribute"));
    }

    #[test]
    fn a_field_left_out_is_refused_unless_it_is_lenient() {
        let mut p = sample::orders();
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        // POST: the body is read as JSON first, so that absence can be told from a default.
        assert!(
            http.contains("fn require_place_order(v: &serde_json::Value) -> Result<(), String>")
        );
        assert!(http.contains("if !eda_runtime::http::sent(v, \"amount\") { return Err(\"amount : is required\".into()); }"));
        // The same schema with every field lenient reads the body as before.
        for m in p.messages.iter_mut().filter(|m| m.name == "PlaceOrderBody") {
            for f in &mut m.fields {
                f.lenient = true;
            }
        }
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(!http.contains("require_place_order"));
        assert!(http.contains("Json(input): Json<pb::PlaceOrderBody>"));
    }

    #[test]
    fn required_query_parameters_and_sources_are_checked_by_name() {
        let mut p = sample::orders();
        let get = p
            .messages
            .iter_mut()
            .find(|m| m.name == "GetOrderRequest")
            .unwrap();
        get.fields
            .push(sb_model::Field::new("expand", sb_model::FieldType::String));
        let mut lenient = sb_model::Field::new("lang", sb_model::FieldType::String);
        lenient.lenient = true;
        get.fields.push(lenient);
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(http.contains("RawQuery(raw_query): RawQuery,"));
        assert!(http.contains("eda_runtime::http::query_has(raw_query.as_deref(), \"expand\")"));
        assert!(!http.contains("query_has(raw_query.as_deref(), \"lang\")"));
        // A header field is required too, unless lenient: then it stays empty when absent.
        let get = p
            .messages
            .iter_mut()
            .find(|m| m.name == "GetOrderRequest")
            .unwrap();
        get.fields[1].source = Some(sb_model::RequestSource::Header {
            name: "X-Trace".into(),
        });
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(http.contains("input.expand = headers.get(\"x-trace\").and_then(|v| v.to_str().ok()).map(str::to_string).ok_or_else("));
        let get = p
            .messages
            .iter_mut()
            .find(|m| m.name == "GetOrderRequest")
            .unwrap();
        get.fields[1].lenient = true;
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(http.contains("input.expand = headers.get(\"x-trace\").and_then(|v| v.to_str().ok()).map(str::to_string).unwrap_or_default();"));
    }

    #[test]
    fn a_uuid_path_parameter_is_refused_with_a_400_when_malformed() {
        let mut p = sample::orders();
        p.services[0].routes[1].path = "/orders/{order_id}".into();
        let get = p
            .messages
            .iter_mut()
            .find(|m| m.name == "GetOrderRequest")
            .unwrap();
        assert!(get.fields[0].name == "order_id");
        get.fields[0].ty = sb_model::FieldType::Uuid;
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(http.contains("input.order_id = uuid_param(&path, \"order_id\")?;"));
        assert!(http.contains("fn uuid_param("));
        // A plain string keeps the plain reader and no uuid helper.
        let get = p
            .messages
            .iter_mut()
            .find(|m| m.name == "GetOrderRequest")
            .unwrap();
        get.fields[0].ty = sb_model::FieldType::String;
        let files = generate(&p).unwrap();
        let http = file(&files, "services/orders/src/transport/http.rs");
        assert!(!http.contains("uuid_param"));
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

    #[test]
    fn a_count_step_reads_one_number_with_the_rules_of_its_tables() {
        use sb_model::{SqlOp, Step};
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_sql_reads(&mut p);
        let route = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "list_posts")
            .unwrap();
        let sql_at = route
            .steps
            .iter()
            .position(|s| matches!(s, Step::Sql { .. }))
            .unwrap();
        let mut count = route.steps[sql_at].clone();
        if let Step::Sql {
            id,
            op,
            bind,
            columns,
            many,
            into,
            order_by,
            then_by,
            limit,
            limit_from,
            offset,
            cursor,
            total_bind,
            more_bind,
            last_bind,
            ..
        } = &mut count
        {
            *id = "count_posts".into();
            *op = SqlOp::Count;
            *bind = Some("post_count".into());
            columns.clear();
            *many = false;
            *into = None;
            *order_by = None;
            then_by.clear();
            *limit = None;
            *limit_from = None;
            *offset = None;
            cursor.clear();
            *total_bind = None;
            *more_bind = None;
            *last_bind = None;
        } else {
            panic!("a SQL step was expected");
        }
        route.steps.insert(sql_at + 1, count);
        let diags = sb_model::validate(&p);
        assert!(!sb_model::has_errors(&diags), "{diags:?}");
        let files = generate(&p).unwrap();
        let flat = file(&files, "services/orders/src/flows/api.rs")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let at = flat
            .find("let post_count: i64 = sqlx::query_scalar")
            .expect(&flat);
        let stmt = &flat[at..];
        let stmt = &stmt[..stmt.find(".fetch_one").unwrap()];
        assert!(
            stmt.contains("SELECT count(*) FROM")
                && stmt.contains("deleted_at")
                && !stmt.contains("LIMIT"),
            "{stmt}"
        );
        // A count reads no row: a column or a sort is refused.
        if let Step::Sql { columns, .. } = &mut p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "list_posts")
            .unwrap()
            .steps[sql_at + 1]
        {
            *columns = vec!["post_id".into()];
        }
        let diags = sb_model::validate(&p);
        assert!(
            diags.iter().any(|d| d.code == "E_SQL_COUNT_ROWS"),
            "{diags:?}"
        );
    }

    #[test]
    fn joins_carry_the_rules_of_every_table_and_a_page_is_cut_by_a_cursor() {
        use sb_model::{Expr, SqlCondition, SqlTest, Step};
        let mut p = sample::orders();
        sample::add_jwt(&mut p);
        sample::add_sql_reads(&mut p);
        let flat_of = |p: &sb_model::Project| {
            assert!(
                !sb_model::has_errors(&sb_model::validate(p)),
                "{:?}",
                sb_model::validate(p)
            );
            let files = generate(p).unwrap();
            file(&files, "services/orders/src/flows/api.rs")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        let flat = flat_of(&p);
        // A left join: the tenant and the soft delete of the joined table are in its `ON`, so that a row of another tenant is
        // no match instead of dropping the row; the table of the step has them in its `WHERE`.
        assert!(
            flat.contains("LEFT JOIN \\\"authors\\\" ON \\\"posts\\\".\\\"author_id\\\" = \\\"authors\\\".\\\"author_id\\\" AND \\\"authors\\\".\\\"tenant_id\\\" = $1 AND \\\"authors\\\".\\\"deleted_at\\\" IS NULL WHERE"),
            "{flat}"
        );
        assert!(
            flat.contains("AND \\\"posts\\\".\\\"tenant_id\\\" = $3 AND \\\"posts\\\".\\\"deleted_at\\\" IS NULL AND ($4::text IS NULL OR"),
            "{flat}"
        );
        // Keyset: each key is compared once the keys before it are equal; the sort has both keys; the page is bound, and
        // one more row is read to know whether another page follows.
        assert!(
            flat.contains("(\\\"posts\\\".\\\"title\\\" = $4::text AND \\\"posts\\\".\\\"post_id\\\" > $5::text)")
                && flat.contains("ORDER BY \\\"posts\\\".\\\"title\\\", \\\"posts\\\".\\\"post_id\\\" LIMIT $6")
                && flat.contains(".bind(posts_sql_limit + 1)")
                && flat.contains("clamp(1, 50_i64)"),
            "{flat}"
        );
        // The total repeats the statement without the page: no cursor, no sort, no limit.
        let count = flat
            .split("SELECT count(*)")
            .nth(1)
            .and_then(|c| c.split(".bind(").next())
            .unwrap_or_default();
        assert!(
            count.contains("LEFT JOIN") && !count.contains("::text") && !count.contains("LIMIT"),
            "{count}"
        );
        // The text search escapes the wildcards of the value.
        assert!(flat.contains("replace('%', \"\\\\%\")"), "{flat}");

        // An offset, a list and null tests, without a cursor.
        if let Some(Step::Sql {
            cursor,
            conditions,
            offset,
            ..
        }) = p.services[0]
            .routes
            .iter_mut()
            .find(|r| r.name == "list_posts")
            .map(|r| &mut r.steps[4])
        {
            cursor.clear();
            *offset = Some(Expr::Path {
                path: "input.page_size".into(),
            });
            conditions.push(SqlCondition {
                or: Vec::new(),
                column: "authors.name".into(),
                op: sb_model::CmpOp::Eq,
                value: Expr::Literal {
                    value: serde_json::Value::Null,
                },
                test: SqlTest::IsNotNull,
            });
            conditions.push(SqlCondition {
                or: Vec::new(),
                column: "post_id".into(),
                op: sb_model::CmpOp::Eq,
                value: Expr::Path {
                    path: "input.search".into(),
                },
                test: SqlTest::StartsWith,
            });
            // `(authors.name IS NULL OR post_id ILIKE ..)`: an alternative sits in parentheses inside the AND.
            conditions.push(SqlCondition {
                or: vec![SqlCondition::new(
                    "post_id",
                    sb_model::CmpOp::Eq,
                    Expr::Path {
                        path: "input.search".into(),
                    },
                )],
                column: "authors.name".into(),
                op: sb_model::CmpOp::Eq,
                value: Expr::Literal {
                    value: serde_json::Value::Null,
                },
                test: SqlTest::IsNull,
            });
        }
        let flat = flat_of(&p);
        assert!(
            flat.contains(
                "(\\\"authors\\\".\\\"name\\\" IS NULL OR \\\"posts\\\".\\\"post_id\\\" = $"
            ),
            "{flat}"
        );
        assert!(
            flat.contains("\\\"authors\\\".\\\"name\\\" IS NOT NULL")
                && flat.contains("\\\"posts\\\".\\\"post_id\\\" ILIKE $")
                && flat.contains("LIMIT $6 OFFSET $7")
                && flat.contains("clamp(0, 1_000_000_i64)"),
            "{flat}"
        );
    }
}
