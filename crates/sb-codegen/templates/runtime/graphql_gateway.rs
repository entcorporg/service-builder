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

use async_graphql::dynamic::{Enum, EnumItem, Field, FieldFuture, FieldValue, InputValue, Object, ResolverContext, Scalar, Schema, TypeRef};
use async_graphql::{Error, ErrorExtensions, Request, Value};
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
    /// The scalar, or the name of the enumeration when `values` is not empty.
    pub scalar: &'static str,
    pub nullable: bool,
    /// `(upstream value, enumeration value)`: the field is an enumeration.
    pub values: &'static [(&'static str, &'static str)],
}

/// A field that reads another source.
pub struct Relation {
    pub field: &'static str,
    pub target: &'static str,
    pub via: &'static str,
    pub target_field: &'static str,
    pub many: bool,
    pub batch_param: Option<&'static str>,
    /// `(query parameter, value)`, constants added to every call; the filter on the relation wins on the same name.
    pub fixed_params: &'static [(&'static str, &'static str)],
}

/// What a computed field is made of (the names are the JSON properties of the rows).
pub enum Kind {
    Product(&'static [&'static str]),
    Ratio(&'static str, &'static str),
    /// The sum over the rows of the relation `relation` of `field`, or of `field * times`.
    Sum { relation: &'static str, field: &'static str, times: Option<&'static str> },
    /// The whole days from today (UTC) to the date of `field`.
    DaysUntil(&'static str),
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The day number of a text starting with `YYYY-MM-DD` (a date, or a timestamp read in its own day).
fn day_of(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' || (b.len() > 10 && b[10].is_ascii_digit()) {
        return None;
    }
    let n = |r: std::ops::Range<usize>| text.get(r).filter(|t| t.bytes().all(|c| c.is_ascii_digit())).and_then(|t| t.parse::<i64>().ok());
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let last = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 28 + i64::from(leap),
        _ => return None,
    };
    (1..=last).contains(&d).then(|| days_from_civil(y, m, d))
}

/// The whole days from `today` (a day number) to the date in `row[field]`; `None` when it is absent or not a date.
fn days_until(row: &Json, field: &str, today: i64) -> Option<i64> {
    day_of(row.get(field)?.as_str()?).map(|d| d - today)
}

/// Today (UTC) as a day number: the clock is read here, at run time, never in the generated code.
fn today() -> i64 {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    (secs / 86_400) as i64
}

pub struct Computed {
    pub field: &'static str,
    pub kind: Kind,
}

/// A number of a row: an integer stays exact, a `double` is rounded to the nearest integer when the result is a `Long`.
#[derive(Clone, Copy)]
enum Num {
    I(i64),
    F(f64),
}

impl Num {
    fn mul(self, other: Num) -> Option<Num> {
        match (self, other) {
            (Num::I(a), Num::I(b)) => a.checked_mul(b).map(Num::I),
            (a, b) => Some(Num::F(a.float() * b.float())),
        }
    }

    fn float(self) -> f64 {
        match self {
            Num::I(n) => n as f64,
            Num::F(f) => f,
        }
    }

    /// Rounded to the nearest integer (halves away from zero); `None` when it does not fit a `Long`.
    fn to_long(self) -> Option<i64> {
        match self {
            Num::I(n) => Some(n),
            Num::F(f) => {
                let r = f.round();
                (r.is_finite() && r >= -9.2e18 && r <= 9.2e18).then_some(r as i64)
            }
        }
    }
}

fn num(row: &Json, k: &str) -> Option<Num> {
    let v = row.get(k)?;
    v.as_i64().map(Num::I).or_else(|| v.as_f64().map(Num::F))
}

/// `sum(field [* times])` over `rows`, each term rounded to a `Long`; `None` when a row lacks an operand or it overflows.
fn sum_rows(rows: &[Json], field: &str, times: Option<&str>) -> Option<i64> {
    // No field: the total counts the rows.
    if field.is_empty() {
        return i64::try_from(rows.len()).ok();
    }
    rows.iter().try_fold(0i64, |acc, row| {
        let n = num(row, field)?;
        let n = match times {
            Some(t) => n.mul(num(row, t)?)?,
            None => n,
        };
        acc.checked_add(n.to_long()?)
    })
}

/// An argument of a root; `default` is the text of its default value.
pub struct Arg {
    pub name: &'static str,
    pub scalar: &'static str,
    pub default: Option<&'static str>,
    /// Without a default: nullable, and left out of the query parameters when the caller omits it.
    pub optional: bool,
}

/// A field of a root: the first row (or the rows) of `source`, asked with query parameters taken from the arguments.
pub struct Block {
    pub field: &'static str,
    pub source: &'static str,
    pub many: bool,
    /// `(query parameter, argument)`.
    pub params: &'static [(&'static str, &'static str)],
    /// `(query parameter, value)`, constants; an argument of `params` wins on the same name.
    pub fixed_params: &'static [(&'static str, &'static str)],
    /// Path of the call instead of the source's, `{arg}` standing for an argument; the answer is then one object.
    pub path: Option<&'static str>,
}

/// `GET <path>` on `source`, read before the blocks: a refusal stops the request. `{arg}` stands for an argument.
pub struct Guard {
    pub source: &'static str,
    pub path: &'static str,
}

/// A query field with arguments that answers blocks, each of which may fail on its own.
pub struct Root {
    pub field: &'static str,
    pub ty: &'static str,
    pub args: &'static [Arg],
    pub guard: Option<Guard>,
    pub blocks: &'static [Block],
    pub sums: &'static [RootSum],
}

/// `field` of the root: the sum of `value [* times]` over the rows of its block `block` (`Long`, see [`sum_rows`]).
pub struct RootSum {
    pub field: &'static str,
    pub block: &'static str,
    pub value: &'static str,
    pub times: Option<&'static str>,
}

pub struct Source {
    pub name: &'static str,
    pub base_url_env: &'static str,
    pub path: &'static str,
    /// The name of the query: `<query>(<key>: ID!)` and `<query>List(first: Int)`.
    pub query: &'static str,
    /// The roles allowed to read the type; empty: any signed-in caller.
    pub roles: &'static [&'static str],
    pub props: &'static [Prop],
    pub key: Option<Prop>,
    pub relations: &'static [Relation],
    pub computed: &'static [Computed],
}

const MAX_LIST: i64 = 100;

/// Why a call to a service behind failed. A root or a block says what the service said (`error`, `code`, `message`).
#[derive(Clone)]
struct Failure {
    code: String,
    message: String,
    error: Option<String>,
}

impl Failure {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.to_owned(), message: message.into(), error: None }
    }
}

type Rows = Arc<Result<Vec<Json>, Failure>>;

fn refuse(code: &str, message: &str) -> Error {
    let code = code.to_owned();
    Error::new(message).extend_with(move |_, e| e.set("code", code))
}

/// Whether the caller holds one of the roles of the type (no role asked: anyone).
fn allowed(req: &Context, src: &Source) -> Result<(), Failure> {
    if src.roles.is_empty() || src.roles.iter().any(|r| req.roles.iter().any(|h| h == r)) {
        Ok(())
    } else {
        Err(Failure::new("ACCES_REFUSE", "your role does not give access to this data"))
    }
}

/// A refused field alone is `null` (or an empty list), with its error at its path: the rest of the query is answered.
fn deny(ctx: &ResolverContext<'_>, f: Failure) {
    let server = ctx.ctx.set_error_path(into_error(f).into_server_error(ctx.ctx.item.pos));
    ctx.ctx.add_error(server);
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

/// What of the caller goes to the services behind: its token, and the headers the project forwards.
#[derive(Default)]
struct Caller {
    cookie: Option<String>,
    headers: Vec<(String, String)>,
}

/// What the caller said, shared by the resolvers of one request.
struct Context {
    sources: &'static [Source],
    caller: Caller,
    roles: Vec<String>,
    loader: Mutex<Loader>,
    /// The rows of the blocks already asked, by block and arguments.
    blocks: Mutex<HashMap<(usize, String), Arc<tokio::sync::OnceCell<Rows>>>>,
}

#[derive(Default)]
struct Loader {
    slots: HashMap<(usize, usize, String), watch::Sender<Option<Rows>>>,
    pending: HashMap<(usize, usize), Vec<String>>,
}

fn upstream(status: u16) -> Failure {
    match status {
        401 | 403 => Failure::new("ACCES_REFUSE", "the service behind refused the request"),
        0 | 429 | 500..=599 => Failure::new("SERVICE_INDISPONIBLE", "the service behind is unavailable"),
        _ => Failure::new("ERREUR_INTERNE", "the service behind answered something unexpected"),
    }
}

/// The refusal of a 4xx as the service behind wrote it (`{error, code, message}`), for a root or a block: the caller learns
/// the code of the service (`ACCES_REFUSE`, `ENTREPOT_INTROUVABLE`...). A 5xx stays generic.
fn verbatim(status: u16, body: &Json) -> Failure {
    let text = |k: &str| body.get(k).and_then(Json::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    match (400..500).contains(&status).then(|| text("code")).flatten() {
        Some(code) => Failure {
            code,
            message: text("message").unwrap_or_else(|| "the service behind refused the request".into()),
            error: text("error"),
        },
        None if status == 404 => Failure::new("INTROUVABLE", "not found"),
        None => upstream(status),
    }
}

fn into_error(f: Failure) -> Error {
    let Failure { code, message, error } = f;
    let extended = refuse(&code, &message);
    extended.extend_with(move |_, e| {
        e.set("message", message.clone());
        if let Some(error) = error {
            e.set("error", error);
        }
    })
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

/// `verbatim`: a refusal of the service is reported as it wrote it (see [`verbatim`]), and a 404 is a failure, not an absence.
async fn get(
    target: &Source,
    caller: &Caller,
    path: &str,
    query: Fields,
    verbatim_errors: bool,
) -> Result<Option<Json>, Failure> {
    let mut headers = Fields::default();
    for (name, value) in &caller.headers {
        headers.set(name.as_str(), value.as_str());
    }
    if let Some(token) = &caller.cookie {
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
        Failure::new("ERREUR_INTERNE", "internal error")
    })?;
    if answer.status == 404 && !verbatim_errors {
        return Ok(None);
    }
    if !answer.ok() {
        if verbatim_errors {
            return Err(verbatim(answer.status, &answer.parse::<Json>().unwrap_or(Json::Null)));
        }
        return Err(upstream(answer.status));
    }
    answer
        .parse::<Json>()
        .map(Some)
        .map_err(|_| Failure::new("SERVICE_INDISPONIBLE", "the service behind sent an unreadable answer"))
}

async fn rows_for(target: &Source, cookie: &Caller, query: Fields) -> Result<Vec<Json>, Failure> {
    get(target, cookie, target.path, query, false).await.map(|a| rows_of(a.unwrap_or(Json::Null)))
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
        let cookie = &self.caller;
        // The service behind may ignore the filter: only the rows that match are kept.
        let matching = |rows: Vec<Json>, wanted: &str| -> Vec<Json> {
            rows.into_iter().filter(|r| r.get(relation.target_field).and_then(text_of).as_deref() == Some(wanted)).collect()
        };
        let base_query = || {
            let mut query = Fields::default();
            for (param, value) in relation.fixed_params {
                query.set(*param, *value);
            }
            query
        };
        let mut out = HashMap::new();
        if let Some(param) = relation.batch_param {
            let mut query = base_query();
            query.set(param, values.join(","));
            let result = rows_for(target, cookie, query).await;
            for v in values {
                let rows = result.clone().map(|rows| matching(rows, v));
                out.insert(v.clone(), Arc::new(rows));
            }
        } else {
            // The relation points at the key of the target: its route reads one row at `GET <path>/<key>`, no filter needed.
            let by_key = target.key.as_ref().is_some_and(|k| k.name == relation.target_field);
            let calls = values.iter().map(|v| async move {
                let mut query = base_query();
                if by_key {
                    let Ok(segment) = crate::http_client::segment(v) else {
                        return (v.clone(), Ok(Vec::new()));
                    };
                    let path = format!("{}/{segment}", target.path);
                    let found = get(target, cookie, &path, query, false).await;
                    return (v.clone(), found.map(|row| row.filter(Json::is_object).into_iter().collect()));
                }
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

/// The default value of an argument, in the type it is declared with.
fn default_value(scalar: &str, text: &str) -> Value {
    let number = |n: Option<serde_json::Number>| n.map_or_else(|| Value::String(text.to_owned()), Value::Number);
    match scalar {
        "Int" | "Long" => number(text.parse::<i64>().ok().map(Into::into)),
        "Float" => number(text.parse::<f64>().ok().and_then(serde_json::Number::from_f64)),
        "Boolean" => text.parse::<bool>().map_or_else(|_| Value::String(text.to_owned()), Value::Boolean),
        _ => Value::String(text.to_owned()),
    }
}

/// The rows of a block of a root: one call to its source, with the query parameters its arguments give.
async fn read_block(ctx: &ResolverContext<'_>, b: &'static Block) -> Result<Option<FieldValue<'static>>, Error> {
    let rows = block_rows(ctx, b).await;
    let rows = match &*rows {
        Ok(rows) => rows.clone(),
        Err(e) => return Err(into_error(e.clone())),
    };
    Ok(if b.many { Some(FieldValue::list(rows.into_iter().map(object))) } else { rows.into_iter().next().map(object) })
}

/// The rows of a block, read once per request and per arguments: a total over them (`sums`) does not call the service again.
async fn block_rows(ctx: &ResolverContext<'_>, b: &'static Block) -> Rows {
    let (req, args) = match (context(ctx), parent_json(ctx)) {
        (Ok(req), Ok(args)) => (req, args),
        _ => return Arc::new(Err(Failure::new("ERREUR_INTERNE", "internal error"))),
    };
    let key = (b as *const Block as usize, args.to_string());
    let cell = {
        let mut cache = req.blocks.lock().unwrap_or_else(|e| e.into_inner());
        cache.entry(key).or_default().clone()
    };
    cell.get_or_init(|| async { Arc::new(fetch_block(req, args, b).await) }).await.clone()
}

async fn fetch_block(req: &Context, args: &Json, b: &'static Block) -> Result<Vec<Json>, Failure> {
    let target = source(req.sources, b.source).ok_or_else(|| Failure::new("ERREUR_INTERNE", "internal error"))?;
    allowed(req, target)?;
    let mut q = Fields::default();
    for (param, value) in b.fixed_params {
        q.set(*param, *value);
    }
    for (param, arg) in b.params {
        if let Some(v) = args.get(*arg).and_then(text_of) {
            q.set(*param, v);
        }
    }
    let mut path = b.path.unwrap_or(target.path).to_owned();
    if b.path.is_some() {
        if let Some(map) = args.as_object() {
            for (name, value) in map {
                if let Some(v) = text_of(value) {
                    let segment = crate::http_client::segment(&v)
                        .map_err(|_| Failure::new("PARAMETRE_INVALIDE", "the argument is not valid"))?;
                    path = path.replace(&format!("{{{name}}}"), &segment.to_string());
                }
            }
        }
    }
    let answer = get(target, &req.caller, &path, q, true).await?;
    let answer = answer.unwrap_or(Json::Null);
    let mut rows = if b.path.is_some() && answer.is_object() && answer.get("items").is_none() {
        vec![answer]
    } else {
        rows_of(answer)
    };
    rows.truncate(MAX_LIST as usize);
    Ok(rows)
}

fn context<'a>(ctx: &'a ResolverContext<'_>) -> Result<&'a Context, Error> {
    ctx.data::<Context>()
}

/// The schema of the sources. `max_depth` and `max_complexity` bound a query; `introspection` says whether the schema can be
/// asked for.
pub fn schema(
    sources: &'static [Source],
    roots: &'static [Root],
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
            if !p.values.is_empty() {
                builder = builder.register(p.values.iter().fold(Enum::new(p.scalar), |e, (_, name)| e.item(EnumItem::new(*name))));
            }
            ty = ty.field(Field::new(p.field, type_ref(p.scalar, p.nullable), move |ctx| {
                FieldFuture::new(async move {
                    if p.values.is_empty() {
                        return Ok(prop(parent_json(&ctx)?, p.name));
                    }
                    match parent_json(&ctx)?.get(p.name) {
                        None | Some(Json::Null) => Ok(None),
                        Some(v) => match p.values.iter().find(|(up, _)| v.as_str() == Some(*up)) {
                            Some((_, name)) => Ok(Some(FieldValue::value(Value::Enum(async_graphql::Name::new(*name))))),
                            None => Err(refuse("VALEUR_INCONNUE", "the service behind sent a value this field does not map")),
                        },
                    }
                })
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
                    if let Err(f) = allowed(context(&ctx)?, target) {
                        deny(&ctx, f);
                        return Ok(if r.many { Some(FieldValue::list(Vec::<FieldValue>::new())) } else { None });
                    }
                    let rows = context(&ctx)?.load(si, ri, value).await?;
                    Ok(if r.many {
                        Some(FieldValue::list(rows.into_iter().map(object)))
                    } else {
                        rows.into_iter().next().map(object)
                    })
                })
            }));
        }
        for c in s.computed {
            let scalar = if matches!(c.kind, Kind::Ratio(..)) { TypeRef::FLOAT } else { "Long" };
            ty = ty.field(Field::new(c.field, TypeRef::named(scalar), move |ctx| {
                FieldFuture::new(async move {
                    let parent = parent_json(&ctx)?;
                    let value: Option<Json> = match &c.kind {
                        Kind::Product(fields) => fields
                            .iter()
                            .try_fold(Num::I(1), |acc, f| num(parent, f).and_then(|n| acc.mul(n)))
                            .and_then(Num::to_long)
                            .map(Json::from),
                        Kind::Ratio(numerator, denominator) => {
                            let f = |k: &str| parent.get(k).and_then(Json::as_f64);
                            match (f(numerator), f(denominator)) {
                                (Some(n), Some(d)) if d != 0.0 => serde_json::Number::from_f64(n / d).map(Json::Number),
                                _ => None,
                            }
                        }
                        Kind::DaysUntil(field) => days_until(parent, field, today()).map(Json::from),
                        Kind::Sum { relation, field, times } => {
                            let ri = s.relations.iter().position(|r| r.field == *relation);
                            match (ri, ri.and_then(|ri| parent.get(s.relations[ri].via).and_then(text_of).map(|v| (ri, v)))) {
                                (Some(_), Some((ri, via))) => {
                                    let req = context(&ctx)?;
                                    let denied = source(sources, s.relations[ri].target).and_then(|t| allowed(req, t).err());
                                    if let Some(f) = denied {
                                        deny(&ctx, f);
                                        return Ok(None);
                                    }
                                    let rows = context(&ctx)?.load(si, ri, via).await?;
                                    sum_rows(&rows, field, *times).map(Json::from)
                                }
                                (Some(_), None) => Some(Json::from(0)),
                                _ => None,
                            }
                        }
                    };
                    Ok(value.and_then(|v| Value::from_json(v).ok()).map(FieldValue::value))
                })
            }));
        }
        builder = builder.register(ty);

        if let Some(key) = s.key.as_ref() {
            let by_key = Field::new(s.query, TypeRef::named(s.name), move |ctx| {
                FieldFuture::new(async move {
                    let req = context(&ctx)?;
                    let target = &req.sources[si];
                    allowed(req, target).map_err(into_error)?;
                    let value = ctx.args.try_get(key.field)?;
                    let value = value.string().map(str::to_owned).or_else(|_| value.i64().map(|n| n.to_string()))?;
                    let segment = crate::http_client::segment(&value)
                        .map_err(|_| refuse("PARAMETRE_INVALIDE", "the key is not valid"))?;
                    let path = format!("{}/{segment}", target.path);
                    let found = get(target, &req.caller, &path, Fields::default(), false).await.map_err(into_error)?;
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
                allowed(req, target).map_err(into_error)?;
                let mut q = Fields::default();
                if let Some(first) = ctx.args.get("first") {
                    let first = first.i64()?;
                    if first < 0 {
                        return Err(refuse("PARAMETRE_INVALIDE", "`first` cannot be negative"));
                    }
                    q.set("limit", first.min(MAX_LIST));
                }
                let mut rows = rows_for(target, &req.caller, q).await.map_err(into_error)?;
                rows.truncate(MAX_LIST as usize);
                Ok(Some(FieldValue::list(rows.into_iter().map(object))))
            })
        })
        .argument(InputValue::new("first", TypeRef::named(TypeRef::INT)));
        query = query.field(list);
    }
    for root in roots {
        let mut ty = Object::new(root.ty);
        for b in root.blocks {
            let Some(target) = source(sources, b.source) else { continue };
            let reference = if b.many { TypeRef::named_nn_list(target.name) } else { TypeRef::named(target.name) };
            ty = ty.field(Field::new(b.field, reference, move |ctx| {
                FieldFuture::new(async move {
                    match read_block(&ctx, b).await {
                        Ok(value) => Ok(value),
                        // The block alone fails: it is `null`, with its error at its path, and the other blocks are answered.
                        Err(error) => {
                            let server = ctx.ctx.set_error_path(error.into_server_error(ctx.ctx.item.pos));
                            ctx.ctx.add_error(server);
                            Ok(None)
                        }
                    }
                })
            }));
        }
        for s in root.sums {
            let Some(b) = root.blocks.iter().find(|b| b.field == s.block) else { continue };
            ty = ty.field(Field::new(s.field, TypeRef::named("Long"), move |ctx| {
                FieldFuture::new(async move {
                    let rows = block_rows(&ctx, b).await;
                    match &*rows {
                        Ok(rows) => Ok(sum_rows(rows, s.value, s.times).and_then(|n| Value::from_json(Json::from(n)).ok()).map(FieldValue::value)),
                        // The total alone fails: it is `null`, with the error of the block.
                        Err(e) => {
                            let server = ctx.ctx.set_error_path(into_error(e.clone()).into_server_error(ctx.ctx.item.pos));
                            ctx.ctx.add_error(server);
                            Ok(None)
                        }
                    }
                })
            }));
        }
        builder = builder.register(ty);
        let mut field = Field::new(root.field, TypeRef::named_nn(root.ty), move |ctx| {
            FieldFuture::new(async move {
                let req = context(&ctx)?;
                let mut args = Map::new();
                for a in root.args {
                    if let Some(v) = ctx.args.get(a.name) {
                        let v = v.as_value().clone().into_json().map_err(|_| refuse("PARAMETRE_INVALIDE", "unexpected argument"))?;
                        args.insert(a.name.to_owned(), v);
                    }
                }
                if let Some(g) = &root.guard {
                    let target = source(req.sources, g.source).ok_or_else(|| refuse("ERREUR_INTERNE", "internal error"))?;
                    let mut path = g.path.to_owned();
                    for a in root.args {
                        if let Some(v) = args.get(a.name).and_then(text_of) {
                            let segment = crate::http_client::segment(&v)
                                .map_err(|_| refuse("PARAMETRE_INVALIDE", "the argument is not valid"))?;
                            path = path.replace(&format!("{{{}}}", a.name), &segment.to_string());
                        }
                    }
                    get(target, &req.caller, &path, Fields::default(), true).await.map_err(into_error)?;
                }
                Ok(Some(object(Json::Object(args))))
            })
        });
        for a in root.args {
            let input = match a.default {
                Some(text) => InputValue::new(a.name, TypeRef::named(a.scalar)).default_value(default_value(a.scalar, text)),
                None if a.optional => InputValue::new(a.name, TypeRef::named(a.scalar)),
                None => InputValue::new(a.name, TypeRef::named_nn(a.scalar)),
            };
            field = field.argument(input);
        }
        query = query.field(field);
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
    pub fn from_env(sources: &'static [Source], roots: &'static [Root]) -> anyhow::Result<Self> {
        let number = |name: &str, default: usize| std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default);
        let introspection = std::env::var("GRAPHQL_INTROSPECTION")
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "false" | "0" | "no"))
            .unwrap_or(true);
        let schema = schema(sources, roots, number("GRAPHQL_MAX_DEPTH", 8), number("GRAPHQL_MAX_COMPLEXITY", 200), introspection)
            .map_err(|e| anyhow::anyhow!("cannot build the GraphQL schema: {e}"))?;
        Ok(Self { schema, sources })
    }

    /// Runs the request of a client; `token` is the `access_token` the services behind receive, `headers` the ones the project
    /// forwards with it.
    pub async fn execute(&self, token: Option<String>, roles: Vec<String>, headers: Vec<(String, String)>, body: Json) -> Json {
        let request: Request = match serde_json::from_value(body) {
            Ok(r) => r,
            Err(_) => return crate::graphql::error_body("REQUETE_INVALIDE", "the body is not a GraphQL request"),
        };
        let data = Context { sources: self.sources, caller: Caller { cookie: token, headers }, roles, loader: Mutex::new(Loader::default()), blocks: Mutex::default() };
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

    async fn articles(State(fake): State<Fake>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> AxJson<Json> {
        fake.article_calls.fetch_add(1, Ordering::SeqCst);
        note(&fake, &headers);
        let all = [
            ("a1", "Vis", "entree", Some("2000-01-01")),
            ("a2", "Ecrou", "sortie", Some("2999-12-31T10:00:00Z")),
            ("a3", "Rondelle", "autre", None),
        ];
        let limit = q.get("limit").and_then(|l| l.parse().ok()).unwrap_or(all.len());
        AxJson(Json::Array(
            all.iter()
                .take(limit)
                .map(|(id, nom, cat, dlc)| serde_json::json!({ "id": id, "nom": nom, "categorie": cat, "dlc": dlc }))
                .collect(),
        ))
    }

    async fn warehouse(axum::extract::Path(id): axum::extract::Path<String>) -> axum::response::Response {
        use axum::response::IntoResponse;
        match id.as_str() {
            "w1" => AxJson(serde_json::json!({ "id": "w1" })).into_response(),
            "w403" => (
                axum::http::StatusCode::FORBIDDEN,
                AxJson(serde_json::json!({ "error": "forbidden", "code": "ENTREPOT_INTERDIT", "message": "no access" })),
            )
                .into_response(),
            _ => axum::http::StatusCode::NOT_FOUND.into_response(),
        }
    }

    async fn capacity(
        axum::extract::Path(id): axum::extract::Path<String>,
        headers: HeaderMap,
        Query(q): Query<HashMap<String, String>>,
    ) -> AxJson<Json> {
        let front = headers.get("x-frontend-id").and_then(|v| v.to_str().ok()).unwrap_or("none");
        let archive = q.get("archive").map(String::as_str).unwrap_or("none");
        AxJson(serde_json::json!({ "id": format!("{id}|{front}|{archive}") }))
    }

    async fn lots() -> axum::response::Response {
        use axum::response::IntoResponse;
        (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            AxJson(serde_json::json!({ "error": "invalid", "code": "LOT_INVALIDE", "message": "lots are broken" })),
        )
            .into_response()
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
            .filter(|(_, a, _)| wanted.is_empty() || wanted.contains(a))
            .map(|(id, a, n)| serde_json::json!({ "id": id, "article_id": a, "quantite": n, "prix": 10, "cout": 2.5, "note": q.get("archive").cloned().unwrap_or_default() }))
            .collect();
        AxJson(Json::Array(rows))
    }

    async fn upstream_api() -> (Fake, String) {
        let fake = Fake::default();
        let app = Router::new()
            .route("/articles", get(articles))
            .route("/articles/{id}", get(article))
            .route("/mouvements", get(movements))
            .route("/entrepots/{id}", get(warehouse))
            .route("/entrepots/{id}/capacite", get(capacity))
            .route("/lots", get(lots))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        (fake, url)
    }

    fn sources(env: &'static str, batch_param: Option<&'static str>) -> &'static [Source] {
        sources_for(env, batch_param, &[])
    }

    /// The same sources, the movements being reserved to `movement_roles`.
    fn sources_for(env: &'static str, batch_param: Option<&'static str>, movement_roles: &'static [&'static str]) -> &'static [Source] {
        let relations: &'static [Relation] = Box::leak(Box::new([Relation {
            field: "mouvements",
            target: "Mouvement",
            via: "id",
            target_field: "article_id",
            many: true,
            batch_param,
            fixed_params: &[],
        }]));
        Box::leak(Box::new([
            Source {
                name: "Article",
                base_url_env: env,
                path: "/articles",
                query: "article",
                roles: &[],
                props: &[
                    Prop { name: "id", field: "id", scalar: "ID", nullable: false, values: &[] },
                    Prop { name: "nom", field: "nom", scalar: "String", nullable: false, values: &[] },
                    Prop { name: "categorie", field: "categorie", scalar: "ArticleCategorie", nullable: true, values: &[("entree", "ENTREE"), ("sortie", "SORTIE")] },
                    Prop { name: "dlc", field: "dlc", scalar: "String", nullable: true, values: &[] },
                ],
                key: Some(Prop { name: "id", field: "id", scalar: "ID", nullable: false, values: &[] }),
                relations,
                computed: &[Computed { field: "joursRestants", kind: Kind::DaysUntil("dlc") }],
            },
            Source {
                name: "Mouvement",
                base_url_env: env,
                path: "/mouvements",
                query: "mouvement",
                roles: movement_roles,
                props: &[
                    Prop { name: "id", field: "id", scalar: "ID", nullable: false, values: &[] },
                    Prop { name: "article_id", field: "articleId", scalar: "ID", nullable: false, values: &[] },
                    Prop { name: "quantite", field: "quantite", scalar: "Int", nullable: false, values: &[] },
                ],
                key: None,
                // Without `batch_param`, towards the key of Article: one `GET /articles/<id>` per distinct id.
                relations: Box::leak(Box::new([Relation {
                    field: "article",
                    target: "Article",
                    via: "article_id",
                    target_field: "id",
                    many: false,
                    batch_param: None,
                    fixed_params: &[],
                }])),
                computed: &[],
            },
        ]))
    }

    const NESTED: &str = "{ articleList { nom mouvements { quantite } } }";

    async fn run(env: &'static str, batch: Option<&'static str>, query: &str) -> (Fake, Json) {
        let (fake, url) = upstream_api().await;
        unsafe { std::env::set_var(env, url) };
        let gateway = Gateway::from_env(sources(env, batch), &[]).unwrap();
        let out = gateway.execute(Some("tok".into()), Vec::new(), Vec::new(), serde_json::json!({ "query": query })).await;
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

    #[test]
    fn a_date_is_counted_in_days_from_today() {
        let t = day_of("2026-11-24").unwrap();
        assert_eq!(day_of("2026-11-25T08:00:00Z").unwrap() - t, 1);
        assert_eq!(day_of("2024-03-01").unwrap() - day_of("2024-02-28").unwrap(), 2);
        assert_eq!(day_of("1970-01-02"), Some(1));
        for bad in ["", "2026-13-01", "2026-02-30", "2026-1-01", "20261124", "2026-11-245"] {
            assert_eq!(day_of(bad), None, "{bad}");
        }
        let row = serde_json::json!({ "dlc": "2026-11-20", "n": 3 });
        assert_eq!(days_until(&row, "dlc", t), Some(-4));
        assert_eq!(days_until(&row, "absent", t), None);
        assert_eq!(days_until(&row, "n", t), None);
    }

    #[tokio::test]
    async fn an_enumeration_maps_the_upstream_value_and_days_are_counted() {
        let q = "{ articleList { categorie joursRestants } }";
        let (_, out) = run("EDA_TEST_GW_ENUM", None, q).await;
        let list = out["data"]["articleList"].as_array().unwrap();
        assert_eq!(list[0]["categorie"], "ENTREE");
        assert_eq!(list[1]["categorie"], "SORTIE");
        assert!(list[0]["joursRestants"].as_i64().unwrap() < 0, "{out}");
        assert!(list[1]["joursRestants"].as_i64().unwrap() > 100_000, "{out}");
        assert!(list[2]["joursRestants"].is_null());
        // A value the map lacks is the error of that field only (nullable: null).
        assert!(list[2]["categorie"].is_null(), "{out}");
        assert_eq!(out["errors"][0]["extensions"]["code"], "VALEUR_INCONNUE", "{out}");
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
    async fn a_relation_to_a_key_without_batch_parameter_reads_the_row_by_its_path() {
        let q = "{ mouvementList { id article { nom } } }";
        let (fake, out) = run("EDA_TEST_GW_BYKEY", None, q).await;
        let list = out["data"]["mouvementList"].as_array().unwrap();
        assert_eq!(list.len(), 3, "{out}");
        assert_eq!(list[0]["article"]["nom"], "Vis", "{out}");
        assert_eq!(list[1]["article"]["nom"], "Vis", "{out}");
        // a2 is unknown to GET /articles/a2 (404): null, no error.
        assert!(list[2]["article"].is_null(), "{out}");
        assert!(out.get("errors").is_none(), "{out}");
        assert_eq!(fake.article_calls.load(Ordering::SeqCst), 0, "the collection is not read, only the rows by key");
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
        let gateway = Gateway::from_env(sources("EDA_TEST_GW_DOWN", None), &[]).unwrap();
        let out = gateway.execute(None, Vec::new(), Vec::new(), serde_json::json!({ "query": "{ articleList { nom } }" })).await;
        assert_eq!(out["errors"][0]["extensions"]["code"], "SERVICE_INDISPONIBLE");
    }

    #[tokio::test]
    async fn a_type_with_roles_is_null_with_an_error_at_its_path_for_a_caller_without_one() {
        let (_, url) = upstream_api().await;
        unsafe { std::env::set_var("EDA_TEST_GW_ROLES", url) };
        let gateway = Gateway::from_env(sources_for("EDA_TEST_GW_ROLES", None, &["admin"]), &[]).unwrap();
        let run = |roles: Vec<String>, q: &str| {
            let (g, q) = (&gateway, q.to_owned());
            async move { g.execute(Some("tok".into()), roles, Vec::new(), serde_json::json!({ "query": q })).await }
        };
        // The articles are open, the movements nested in them are not: the rows come back, the relation is empty and refused.
        let out = run(vec!["support".into()], NESTED).await;
        assert_eq!(out["data"]["articleList"].as_array().unwrap().len(), 3, "{out}");
        assert!(out["data"]["articleList"][0]["mouvements"].as_array().unwrap().is_empty(), "{out}");
        assert_eq!(out["errors"][0]["extensions"]["code"], "ACCES_REFUSE", "{out}");
        assert_eq!(out["errors"][0]["path"][0], "articleList", "{out}");
        // A listing of the type itself is refused too.
        let out = run(Vec::new(), "{ mouvementList { quantite } }").await;
        assert_eq!(out["errors"][0]["extensions"]["code"], "ACCES_REFUSE", "{out}");
        // The role opens it.
        let out = run(vec!["admin".into()], NESTED).await;
        assert!(out.get("errors").is_none(), "{out}");
        assert_eq!(out["data"]["articleList"][0]["mouvements"].as_array().unwrap().len(), 2, "{out}");
    }

    /// Articles with their stock value, the movements and the broken lots, behind a root with arguments and a guard.
    fn dashboard(env: &'static str) -> (&'static [Source], &'static [Root]) {
        let id = || Prop { name: "id", field: "id", scalar: "ID", nullable: false, values: &[] };
        let relations: &'static [Relation] = Box::leak(Box::new([Relation {
            field: "mouvements",
            target: "Mouvement",
            via: "id",
            target_field: "article_id",
            many: true,
            batch_param: Some("article_ids"),
            fixed_params: &[("archive", "tous")],
        }]));
        let sources: &'static [Source] = Box::leak(Box::new([
            Source {
                name: "Article",
                base_url_env: env,
                path: "/articles",
                query: "article",
                roles: &[],
                props: Box::leak(Box::new([id()])),
                key: None,
                relations,
                computed: Box::leak(Box::new([Computed {
                    field: "valeurStock",
                    kind: Kind::Sum { relation: "mouvements", field: "quantite", times: Some("prix") },
                }])),
            },
            Source {
                name: "Mouvement",
                base_url_env: env,
                path: "/mouvements",
                query: "mouvement",
                roles: &[],
                props: Box::leak(Box::new([
                    id(),
                    Prop { name: "quantite", field: "quantite", scalar: "Int", nullable: false, values: &[] },
                    Prop { name: "note", field: "note", scalar: "String", nullable: false, values: &[] },
                ])),
                key: None,
                relations: &[],
                computed: Box::leak(Box::new([
                    Computed { field: "valeur", kind: Kind::Product(&["quantite", "prix"]) },
                    Computed { field: "taux", kind: Kind::Ratio("quantite", "prix") },
                    Computed { field: "sansDenominateur", kind: Kind::Ratio("quantite", "absent") },
                ])),
            },
            Source { name: "Lot", base_url_env: env, path: "/lots", query: "lot", roles: &[], props: Box::leak(Box::new([id()])), key: None, relations: &[], computed: &[] },
        ]));
        let roots: &'static [Root] = Box::leak(Box::new([Root {
            field: "tableauDeBord",
            ty: "TableauDeBord",
            args: Box::leak(Box::new([
                Arg { name: "entrepotId", scalar: "ID", default: None, optional: false },
                Arg { name: "limit", scalar: "Int", default: Some("2"), optional: false },
                // Left out by the query below: nullable, and the parameter it feeds is not sent.
                Arg { name: "recherche", scalar: "String", default: None, optional: true },
            ])),
            guard: Some(Guard { source: "Article", path: "/entrepots/{entrepotId}" }),
            blocks: Box::leak(Box::new([
                Block { field: "articles", source: "Article", many: true, params: &[("limit", "limit"), ("q", "recherche")], fixed_params: &[], path: None },
                Block { field: "lots", source: "Lot", many: true, params: &[], fixed_params: &[], path: None },
                Block { field: "mouvements", source: "Mouvement", many: true, params: &[], fixed_params: &[], path: None },
            ])),
            sums: &[
                RootSum { field: "valeurTotale", block: "mouvements", value: "quantite", times: Some("cout") },
                RootSum { field: "totalLots", block: "lots", value: "quantite", times: None },            ],
        }]));
        (sources, roots)
    }

    async fn dashboard_run(env: &'static str, query: &str) -> (Fake, Json) {
        let (fake, url) = upstream_api().await;
        unsafe { std::env::set_var(env, url) };
        let (sources, roots) = dashboard(env);
        let gateway = Gateway::from_env(sources, roots).unwrap();
        let out = gateway.execute(Some("tok".into()), Vec::new(), Vec::new(), serde_json::json!({ "query": query })).await;
        (fake, out)
    }

    #[tokio::test]
    async fn a_root_passes_its_arguments_computes_fields_and_keeps_the_blocks_that_work() {
        let q = r#"{ tableauDeBord(entrepotId: "w1") { articles { id valeurStock } lots { id } } }"#;
        let (fake, out) = dashboard_run("EDA_TEST_GW_ROOT", q).await;
        let articles = out["data"]["tableauDeBord"]["articles"].as_array().unwrap();
        assert_eq!(articles.len(), 2, "limit defaults to 2: {out}");
        assert_eq!(articles[0]["valeurStock"], 30, "(5 - 2) * 10: {out}");
        assert_eq!(articles[1]["valeurStock"], 70, "{out}");
        assert!(out["data"]["tableauDeBord"]["lots"].is_null(), "{out}");
        let error = &out["errors"][0];
        assert_eq!(error["path"], serde_json::json!(["tableauDeBord", "lots"]), "{out}");
        assert_eq!(error["extensions"]["code"], "LOT_INVALIDE");
        assert_eq!(error["extensions"]["error"], "invalid");
        assert_eq!(error["extensions"]["message"], "lots are broken");
        assert_eq!(out["errors"].as_array().unwrap().len(), 1, "{out}");
        assert_eq!(fake.article_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fake.movement_calls.load(Ordering::SeqCst), 1, "grouped by the dataloader");
    }

    #[tokio::test]
    async fn a_block_reads_a_path_with_an_argument_fixed_params_and_the_forwarded_headers() {
        let env = "EDA_TEST_GW_PATH";
        let (_fake, url) = upstream_api().await;
        unsafe { std::env::set_var(env, url) };
        let (sources, _) = dashboard(env);
        let roots: &'static [Root] = Box::leak(Box::new([Root {
            field: "capacite",
            ty: "Capacite",
            args: Box::leak(Box::new([Arg { name: "entrepotId", scalar: "ID", default: None, optional: false }])),
            guard: None,
            blocks: Box::leak(Box::new([Block {
                field: "cap",
                source: "Article",
                many: false,
                params: &[],
                fixed_params: &[("archive", "tous")],
                path: Some("/entrepots/{entrepotId}/capacite"),
            }])),
            sums: &[],
        }]));
        let gateway = Gateway::from_env(sources, roots).unwrap();
        let q = r#"{ capacite(entrepotId: "w1") { cap { id } } }"#;
        let headers = vec![("x-frontend-id".to_string(), "front-7".to_string())];
        let out = gateway.execute(Some("tok".into()), Vec::new(), headers, serde_json::json!({ "query": q })).await;
        assert_eq!(out["data"]["capacite"]["cap"]["id"], "w1|front-7|tous", "{out}");
    }

    #[tokio::test]
    async fn the_guard_stops_the_request_with_the_code_of_the_service() {
        let q = r#"{ tableauDeBord(entrepotId: "w403") { articles { id } } }"#;
        let (fake, out) = dashboard_run("EDA_TEST_GW_GUARD", q).await;
        assert!(out["data"].is_null(), "{out}");
        assert_eq!(out["errors"][0]["extensions"]["code"], "ENTREPOT_INTERDIT", "{out}");
        assert_eq!(fake.article_calls.load(Ordering::SeqCst), 0, "no block ran");
        let q = r#"{ tableauDeBord(entrepotId: "nope", limit: 1) { articles { id } } }"#;
        let (_, out) = dashboard_run("EDA_TEST_GW_GUARD2", q).await;
        assert_eq!(out["errors"][0]["extensions"]["code"], "INTROUVABLE", "{out}");
    }

    #[tokio::test]
    async fn a_product_and_a_ratio_are_computed_and_a_missing_operand_is_null() {
        let q = "{ articleList(first: 1) { mouvements { quantite valeur taux sansDenominateur } } }";
        let (_, out) = dashboard_run("EDA_TEST_GW_COMPUTED", q).await;
        let first = &out["data"]["articleList"][0]["mouvements"][0];
        assert_eq!(first["valeur"], 50, "{out}");
        assert_eq!(first["taux"], 0.5, "{out}");
        assert!(first["sansDenominateur"].is_null(), "{out}");
    }

    #[test]
    fn a_double_operand_rounds_the_long_result_to_the_nearest_integer() {
        let rows = [
            serde_json::json!({ "q": 5, "c": 2.5 }),
            serde_json::json!({ "q": -2, "c": 2.5 }),
            serde_json::json!({ "q": 7, "c": 2.5 }),
        ];
        assert_eq!(sum_rows(&rows, "q", Some("c")), Some(13 - 5 + 18));
        assert_eq!(sum_rows(&rows, "q", None), Some(10));
        assert_eq!(sum_rows(&rows, "q", Some("absent")), None, "a missing operand");
        assert_eq!(sum_rows(&[serde_json::json!({ "q": 1e30 })], "q", None), None, "does not fit a Long");
    }

    #[tokio::test]
    async fn a_root_total_sums_a_block_without_calling_the_service_again_and_a_relation_sends_its_fixed_params() {
        let q = r#"{ tableauDeBord(entrepotId: "w1") { valeurTotale mouvements { quantite note } totalLots } }"#;
        let (fake, out) = dashboard_run("EDA_TEST_GW_SUMS", q).await;
        let root = &out["data"]["tableauDeBord"];
        assert_eq!(root["valeurTotale"], 13 - 5 + 18, "{out}");
        assert_eq!(root["mouvements"].as_array().unwrap().len(), 3, "{out}");
        assert!(root["totalLots"].is_null(), "{out}");
        assert_eq!(out["errors"].as_array().unwrap().len(), 1, "the failing block alone: {out}");
        assert_eq!(out["errors"][0]["path"], serde_json::json!(["tableauDeBord", "totalLots"]), "{out}");
        assert_eq!(out["errors"][0]["extensions"]["code"], "LOT_INVALIDE", "{out}");
        assert_eq!(fake.movement_calls.load(Ordering::SeqCst), 1, "the block and its total share one call");
        let q = "{ articleList(first: 1) { mouvements { note } } }";
        let (_, out) = dashboard_run("EDA_TEST_GW_FIXED", q).await;
        assert_eq!(out["data"]["articleList"][0]["mouvements"][0]["note"], "tous", "{out}");
    }
}
