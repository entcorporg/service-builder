//! The GraphQL endpoint of a gateway: a service with no database that serves the REST APIs of other services as one graph.
//!
//! Every type is a [`Source`]: `GET <path>` lists its rows, `GET <path>/<key>` reads one. A relation is a field that reads
//! another source, and the lookups it makes for every row of a response go through a per-request loader (a dataloader): they
//! are deduplicated, grouped by relation, and sent as one call per relation (`batch_param`) or, without it, one call per
//! distinct value, all at once. The cookie `access_token` of the caller goes along, so the service behind applies its own
//! checks and its own tenant. Errors carry a machine code in `errors[].extensions.code`; depth, complexity and introspection
//! are bounded like for the tables (`GRAPHQL_MAX_DEPTH`, `GRAPHQL_MAX_COMPLEXITY`, `GRAPHQL_INTROSPECTION`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_graphql::dynamic::{Field, FieldFuture, FieldValue, InputValue, Object, ResolverContext, Scalar, Schema, TypeRef};
use async_graphql::{Error, ErrorExtensions, Request};
use serde_json::{Map, Value as Json};
use tokio::sync::watch;

use crate::graphql::{add_codes, object, parent_json, prop, type_ref};
use crate::http_client::{Auth, BodyFormat, Call, Fields};

/// A property of a row that is a field.
pub struct Prop {
    /// The JSON property of the answer.
    pub name: &'static str,
    /// The GraphQL field.
    pub field: &'static str,
    pub scalar: &'static str,
    pub nullable: bool,
}

/// A field that reads another source.
pub struct Relation {
    pub field: &'static str,
    pub target: &'static str,
    pub via: &'static str,
    pub target_field: &'static str,
    pub many: bool,
    pub batch_param: Option<&'static str>,
}

pub struct Source {
    pub name: &'static str,
    pub base_url_env: &'static str,
    pub path: &'static str,
    /// The name of the query: `<query>(<key>: ID!)` and `<query>List(first: Int)`.
    pub query: &'static str,
    pub props: &'static [Prop],
    pub key: Option<Prop>,
    pub relations: &'static [Relation],
}

const MAX_LIST: i64 = 100;

type Failure = (&'static str, String);
type Rows = Arc<Result<Vec<Json>, Failure>>;

fn refuse(code: &'static str, message: &str) -> Error {
    Error::new(message).extend_with(move |_, e| e.set("code", code))
}

fn text_of(v: &Json) -> Option<String> {
    match v {
        Json::Null => None,
        Json::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn source<'a>(sources: &'a [Source], name: &str) -> Option<&'a Source> {
    sources.iter().find(|s| s.name == name)
}

/// What the caller said, shared by the resolvers of one request.
struct Context {
    sources: &'static [Source],
    cookie: Option<String>,
    loader: Mutex<Loader>,
}

#[derive(Default)]
struct Loader {
    slots: HashMap<(usize, usize, String), watch::Sender<Option<Rows>>>,
    pending: HashMap<(usize, usize), Vec<String>>,
}

fn upstream(status: u16) -> Failure {
    match status {
        401 | 403 => ("ACCES_REFUSE", "the service behind refused the request".into()),
        0 | 429 | 500..=599 => ("SERVICE_INDISPONIBLE", "the service behind is unavailable".into()),
        _ => ("ERREUR_INTERNE", "the service behind answered something unexpected".into()),
    }
}

fn into_error((code, message): Failure) -> Error {
    refuse(code, &message)
}

fn rows_of(answer: Json) -> Vec<Json> {
    match answer {
        Json::Array(rows) => rows,
        Json::Object(mut o) => match o.remove("items") {
            Some(Json::Array(rows)) => rows,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

async fn get(target: &Source, cookie: Option<&str>, path: &str, query: Fields) -> Result<Option<Json>, Failure> {
    let mut headers = Fields::default();
    if let Some(token) = cookie {
        headers.set("Cookie", format!("access_token={token}"));
    }
    let call = Call {
        id: "graphql",
        method: "GET",
        base_url_env: target.base_url_env,
        path,
        query,
        headers,
        auth: Auth::None,
        body: Fields::default(),
        format: BodyFormat::Json,
        timeout_secs: 10,
        retries: 1,
        idempotency_key: None,
        resilience: Default::default(),
    };
    let answer = crate::http_client::send(call).await.map_err(|e| {
        tracing::error!(error = %e, "graphql gateway call failed");
        ("ERREUR_INTERNE", "internal error".to_string())
    })?;
    if answer.status == 404 {
        return Ok(None);
    }
    if !answer.ok() {
        return Err(upstream(answer.status));
    }
    answer
        .parse::<Json>()
        .map(Some)
        .map_err(|_| ("SERVICE_INDISPONIBLE", "the service behind sent an unreadable answer".to_string()))
}

async fn rows_for(target: &Source, cookie: Option<&str>, query: Fields) -> Result<Vec<Json>, Failure> {
    get(target, cookie, target.path, query).await.map(|a| rows_of(a.unwrap_or(Json::Null)))
}

impl Context {
    /// The rows of relation `rel` of source `src` for `value`: this call is grouped with those made at the same time.
    async fn load(&self, src: usize, rel: usize, value: String) -> Result<Vec<Json>, Error> {
        let slot = (src, rel, value.clone());
        let mut rx = {
            let mut l = self.loader.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(tx) = l.slots.get(&slot) {
                tx.subscribe()
            } else {
                let (tx, rx) = watch::channel(None);
                l.slots.insert(slot, tx);
                l.pending.entry((src, rel)).or_default().push(value);
                rx
            }
        };
        // The siblings of this row register their value meanwhile; whoever comes back first sends them all.
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        let batch = self.loader.lock().unwrap_or_else(|e| e.into_inner()).pending.remove(&(src, rel)).unwrap_or_default();
        if !batch.is_empty() {
            let mut results = self.fetch(src, rel, &batch).await;
            let l = self.loader.lock().unwrap_or_else(|e| e.into_inner());
            for v in batch {
                let rows = results.remove(&v).unwrap_or_else(|| Arc::new(Ok(Vec::new())));
                if let Some(tx) = l.slots.get(&(src, rel, v)) {
                    tx.send_replace(Some(rows));
                }
            }
        }
        let got = rx
            .wait_for(Option::is_some)
            .await
            .map_err(|_| refuse("ERREUR_INTERNE", "internal error"))?
            .clone()
            .ok_or_else(|| refuse("ERREUR_INTERNE", "internal error"))?;
        match &*got {
            Ok(rows) => Ok(rows.clone()),
            Err(e) => Err(into_error(e.clone())),
        }
    }

    async fn fetch(&self, src: usize, rel: usize, values: &[String]) -> HashMap<String, Rows> {
        let relation = &self.sources[src].relations[rel];
        let Some(target) = source(self.sources, relation.target) else {
            return HashMap::new();
        };
        let cookie = self.cookie.as_deref();
        // The service behind may ignore the filter: only the rows that match are kept.
        let matching = |rows: Vec<Json>, wanted: &str| -> Vec<Json> {
            rows.into_iter().filter(|r| r.get(relation.target_field).and_then(text_of).as_deref() == Some(wanted)).collect()
        };
        let mut out = HashMap::new();
        if let Some(param) = relation.batch_param {
            let mut query = Fields::default();
            query.set(param, values.join(","));
            let result = rows_for(target, cookie, query).await;
            for v in values {
                let rows = result.clone().map(|rows| matching(rows, v));
                out.insert(v.clone(), Arc::new(rows));
            }
        } else {
            let calls = values.iter().map(|v| async move {
                let mut query = Fields::default();
                query.set(relation.target_field, v.as_str());
                (v.clone(), rows_for(target, cookie, query).await)
            });
            for (v, result) in join_all(calls).await {
                let rows = result.map(|rows| matching(rows, &v));
                out.insert(v, Arc::new(rows));
            }
        }
        out
    }
}

/// Runs the futures together and gathers their results, in order.
async fn join_all<F: std::future::Future>(calls: impl IntoIterator<Item = F>) -> Vec<F::Output> {
    let mut pending: Vec<std::pin::Pin<Box<F>>> = calls.into_iter().map(Box::pin).collect();
    let mut done: Vec<Option<F::Output>> = pending.iter().map(|_| None).collect();
    std::future::poll_fn(move |cx| {
        let mut all = true;
        for (f, d) in pending.iter_mut().zip(done.iter_mut()) {
            if d.is_none() {
                match f.as_mut().poll(cx) {
                    std::task::Poll::Ready(v) => *d = Some(v),
                    std::task::Poll::Pending => all = false,
                }
            }
        }
        if all {
            std::task::Poll::Ready(done.iter_mut().filter_map(Option::take).collect())
        } else {
            std::task::Poll::Pending
        }
    })
    .await
}

fn context<'a>(ctx: &'a ResolverContext<'_>) -> Result<&'a Context, Error> {
    ctx.data::<Context>()
}

/// The schema of the sources. `max_depth` and `max_complexity` bound a query; `introspection` says whether the schema can be
/// asked for.
pub fn schema(
    sources: &'static [Source],
    max_depth: usize,
    max_complexity: usize,
    introspection: bool,
) -> Result<Schema, async_graphql::dynamic::SchemaError> {
    let mut query = Object::new("Query");
    let mut builder = Schema::build("Query", None, None);
    for s in ["DateTime", "JSON", "Long"] {
        builder = builder.register(Scalar::new(s));
    }
    for (si, s) in sources.iter().enumerate() {
        let mut ty = Object::new(s.name);
        for p in s.props {
            ty = ty.field(Field::new(p.field, type_ref(p.scalar, p.nullable), move |ctx| {
                FieldFuture::new(async move { Ok(prop(parent_json(&ctx)?, p.name)) })
            }));
        }
        for (ri, r) in s.relations.iter().enumerate() {
            let Some(target) = source(sources, r.target) else { continue };
            let reference = if r.many { TypeRef::named_nn_list_nn(target.name) } else { TypeRef::named(target.name) };
            ty = ty.field(Field::new(r.field, reference, move |ctx| {
                FieldFuture::new(async move {
                    let Some(value) = parent_json(&ctx)?.get(r.via).and_then(text_of) else {
                        return Ok(if r.many { Some(FieldValue::list(Vec::<FieldValue>::new())) } else { None });
                    };
                    let rows = context(&ctx)?.load(si, ri, value).await?;
                    Ok(if r.many {
                        Some(FieldValue::list(rows.into_iter().map(object)))
                    } else {
                        rows.into_iter().next().map(object)
                    })
                })
            }));
        }
        builder = builder.register(ty);

        if let Some(key) = s.key.as_ref() {
            let by_key = Field::new(s.query, TypeRef::named(s.name), move |ctx| {
                FieldFuture::new(async move {
                    let req = context(&ctx)?;
                    let target = &req.sources[si];
                    let value = ctx.args.try_get(key.field)?;
                    let value = value.string().map(str::to_owned).or_else(|_| value.i64().map(|n| n.to_string()))?;
                    let segment = crate::http_client::segment(&value)
                        .map_err(|_| refuse("PARAMETRE_INVALIDE", "the key is not valid"))?;
                    let path = format!("{}/{segment}", target.path);
                    let found = get(target, req.cookie.as_deref(), &path, Fields::default()).await.map_err(into_error)?;
                    Ok(found.filter(Json::is_object).map(object))
                })
            })
            .argument(InputValue::new(key.field, TypeRef::named_nn(key.scalar)));
            query = query.field(by_key);
        }
        let list = Field::new(format!("{}List", s.query), TypeRef::named_nn_list_nn(s.name), move |ctx| {
            FieldFuture::new(async move {
                let req = context(&ctx)?;
                let target = &req.sources[si];
                let mut q = Fields::default();
                if let Some(first) = ctx.args.get("first") {
                    let first = first.i64()?;
                    if first < 0 {
                        return Err(refuse("PARAMETRE_INVALIDE", "`first` cannot be negative"));
                    }
                    q.set("limit", first.min(MAX_LIST));
                }
                let mut rows = rows_for(target, req.cookie.as_deref(), q).await.map_err(into_error)?;
                rows.truncate(MAX_LIST as usize);
                Ok(Some(FieldValue::list(rows.into_iter().map(object))))
            })
        })
        .argument(InputValue::new("first", TypeRef::named(TypeRef::INT)));
        query = query.field(list);
    }
    builder = builder.register(query).limit_depth(max_depth).limit_complexity(max_complexity);
    if !introspection {
        builder = builder.disable_introspection();
    }
    builder.finish()
}

/// The endpoint of a gateway: its schema, built once.
pub struct Gateway {
    schema: Schema,
    sources: &'static [Source],
}

impl Gateway {
    /// Limits and introspection come from the environment, like for the tables.
    pub fn from_env(sources: &'static [Source]) -> anyhow::Result<Self> {
        let number = |name: &str, default: usize| std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default);
        let introspection = std::env::var("GRAPHQL_INTROSPECTION")
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "false" | "0" | "no"))
            .unwrap_or(true);
        let schema = schema(sources, number("GRAPHQL_MAX_DEPTH", 8), number("GRAPHQL_MAX_COMPLEXITY", 200), introspection)
            .map_err(|e| anyhow::anyhow!("cannot build the GraphQL schema: {e}"))?;
        Ok(Self { schema, sources })
    }

    /// Runs the request of a client; `token` is the `access_token` the services behind receive.
    pub async fn execute(&self, token: Option<String>, body: Json) -> Json {
        let request: Request = match serde_json::from_value(body) {
            Ok(r) => r,
            Err(_) => return crate::graphql::error_body("REQUETE_INVALIDE", "the body is not a GraphQL request"),
        };
        let data = Context { sources: self.sources, cookie: token, loader: Mutex::new(Loader::default()) };
        let response = self.schema.execute(request.data(data)).await;
        let mut out = serde_json::to_value(&response).unwrap_or_else(|_| Json::Object(Map::new()));
        add_codes(&mut out);
        out
    }
}

#[cfg(all(test, feature = "http-client-tests"))]
mod tests {
    use super::*;
    use axum::extract::{Query, State};
    use axum::http::HeaderMap;
    use axum::{Json as AxJson, Router, routing::get};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, Default)]
    struct Fake {
        article_calls: Arc<AtomicUsize>,
        movement_calls: Arc<AtomicUsize>,
        cookies: Arc<Mutex<Vec<String>>>,
    }

    fn note(fake: &Fake, headers: &HeaderMap) {
        let cookie = headers.get("cookie").and_then(|v| v.to_str().ok()).unwrap_or_default();
        fake.cookies.lock().unwrap().push(cookie.to_string());
    }

    async fn articles(State(fake): State<Fake>, headers: HeaderMap) -> AxJson<Json> {
        fake.article_calls.fetch_add(1, Ordering::SeqCst);
        note(&fake, &headers);
        AxJson(serde_json::json!([
            { "id": "a1", "nom": "Vis" },
            { "id": "a2", "nom": "Ecrou" },
            { "id": "a3", "nom": "Rondelle" }
        ]))
    }

    async fn article(axum::extract::Path(id): axum::extract::Path<String>) -> axum::response::Response {
        use axum::response::IntoResponse;
        if id == "a1" {
            AxJson(serde_json::json!({ "id": "a1", "nom": "Vis" })).into_response()
        } else {
            axum::http::StatusCode::NOT_FOUND.into_response()
        }
    }

    async fn movements(
        State(fake): State<Fake>,
        headers: HeaderMap,
        Query(q): Query<HashMap<String, String>>,
    ) -> AxJson<Json> {
        fake.movement_calls.fetch_add(1, Ordering::SeqCst);
        note(&fake, &headers);
        let wanted: Vec<&str> = q
            .get("article_id")
            .or_else(|| q.get("article_ids"))
            .map(|v| v.split(',').collect())
            .unwrap_or_default();
        let all = [("m1", "a1", 5), ("m2", "a1", -2), ("m3", "a2", 7)];
        let rows: Vec<Json> = all
            .iter()
            .filter(|(_, a, _)| wanted.contains(a))
            .map(|(id, a, n)| serde_json::json!({ "id": id, "article_id": a, "quantite": n }))
            .collect();
        AxJson(Json::Array(rows))
    }

    async fn upstream_api() -> (Fake, String) {
        let fake = Fake::default();
        let app = Router::new()
            .route("/articles", get(articles))
            .route("/articles/{id}", get(article))
            .route("/mouvements", get(movements))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        (fake, url)
    }

    fn sources(env: &'static str, batch_param: Option<&'static str>) -> &'static [Source] {
        let relations: &'static [Relation] = Box::leak(Box::new([Relation {
            field: "mouvements",
            target: "Mouvement",
            via: "id",
            target_field: "article_id",
            many: true,
            batch_param,
        }]));
        Box::leak(Box::new([
            Source {
                name: "Article",
                base_url_env: env,
                path: "/articles",
                query: "article",
                props: &[
                    Prop { name: "id", field: "id", scalar: "ID", nullable: false },
                    Prop { name: "nom", field: "nom", scalar: "String", nullable: false },
                ],
                key: Some(Prop { name: "id", field: "id", scalar: "ID", nullable: false }),
                relations,
            },
            Source {
                name: "Mouvement",
                base_url_env: env,
                path: "/mouvements",
                query: "mouvement",
                props: &[
                    Prop { name: "id", field: "id", scalar: "ID", nullable: false },
                    Prop { name: "article_id", field: "articleId", scalar: "ID", nullable: false },
                    Prop { name: "quantite", field: "quantite", scalar: "Int", nullable: false },
                ],
                key: None,
                relations: &[],
            },
        ]))
    }

    const NESTED: &str = "{ articleList { nom mouvements { quantite } } }";

    async fn run(env: &'static str, batch: Option<&'static str>, query: &str) -> (Fake, Json) {
        let (fake, url) = upstream_api().await;
        unsafe { std::env::set_var(env, url) };
        let gateway = Gateway::from_env(sources(env, batch)).unwrap();
        let out = gateway.execute(Some("tok".into()), serde_json::json!({ "query": query })).await;
        (fake, out)
    }

    #[tokio::test]
    async fn a_nested_query_makes_one_call_per_distinct_row_and_forwards_the_token() {
        let (fake, out) = run("EDA_TEST_GW_PLAIN", None, NESTED).await;
        assert!(out.get("errors").is_none(), "{out}");
        let list = out["data"]["articleList"].as_array().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0]["mouvements"].as_array().unwrap().len(), 2);
        assert_eq!(list[1]["mouvements"][0]["quantite"], 7);
        assert_eq!(list[2]["mouvements"].as_array().unwrap().len(), 0);
        assert_eq!(fake.article_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fake.movement_calls.load(Ordering::SeqCst), 3);
        assert!(fake.cookies.lock().unwrap().iter().all(|c| c == "access_token=tok"));
    }

    #[tokio::test]
    async fn a_batch_parameter_makes_one_call_for_all_the_rows() {
        let (fake, out) = run("EDA_TEST_GW_BATCH", Some("article_ids"), NESTED).await;
        assert!(out.get("errors").is_none(), "{out}");
        let list = out["data"]["articleList"].as_array().unwrap();
        assert_eq!(list[0]["mouvements"].as_array().unwrap().len(), 2);
        assert_eq!(list[1]["mouvements"].as_array().unwrap().len(), 1);
        assert_eq!(list[2]["mouvements"].as_array().unwrap().len(), 0);
        assert_eq!(fake.movement_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_row_is_read_by_key_and_a_missing_one_is_null() {
        let (_, out) = run("EDA_TEST_GW_KEY", None, "{ a: article(id: \"a1\") { nom } b: article(id: \"zz\") { nom } }").await;
        assert!(out.get("errors").is_none(), "{out}");
        assert_eq!(out["data"]["a"]["nom"], "Vis");
        assert!(out["data"]["b"].is_null());
    }

    #[tokio::test]
    async fn a_service_that_is_down_is_a_coded_error() {
        unsafe { std::env::set_var("EDA_TEST_GW_DOWN", "http://127.0.0.1:1") };
        let gateway = Gateway::from_env(sources("EDA_TEST_GW_DOWN", None)).unwrap();
        let out = gateway.execute(None, serde_json::json!({ "query": "{ articleList { nom } }" })).await;
        assert_eq!(out["errors"][0]["extensions"]["code"], "SERVICE_INDISPONIBLE");
    }
}
