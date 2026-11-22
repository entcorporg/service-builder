//! The read side of a service's GraphQL endpoint: a schema built once from the description of its tables (the same one the
//! `.graphql` contract is written from), and run over the service's database.
//!
//! Every query is filtered by the tenant of the token when the table has a tenant column, and a column of the tenant is never a
//! field. Errors carry a machine code in `errors[].extensions.code`. The depth and the complexity of a query are limited, and
//! introspection can be turned off (`GRAPHQL_INTROSPECTION=off`).

use async_graphql::dynamic::{
    Field, FieldFuture, FieldValue, InputValue, Object, ResolverContext, Scalar, Schema, TypeRef,
};
use async_graphql::{Error, ErrorExtensions, Request, Value};
use serde_json::{Map, Value as Json};

/// A column that is a field.
pub struct Column {
    pub name: &'static str,
    pub field: &'static str,
    /// `String`, `Boolean`, `Int`, `Float`, `ID`, `Long`, `DateTime` or `JSON`.
    pub scalar: &'static str,
    pub nullable: bool,
}

/// A foreign key seen from the child: the field that points to the parent row.
pub struct Parent {
    pub field: &'static str,
    pub column: &'static str,
    pub table: &'static str,
    pub ref_column: &'static str,
    pub nullable: bool,
}

/// A foreign key seen from the parent: the connection of the rows that point at it.
pub struct Child {
    pub field: &'static str,
    pub table: &'static str,
    pub column: &'static str,
    pub ref_column: &'static str,
}

pub struct Table {
    pub name: &'static str,
    pub ty: &'static str,
    pub query: &'static str,
    pub columns: &'static [Column],
    pub keys: &'static [Column],
    pub tenant: Option<&'static str>,
    /// The roles allowed to read the table; empty: any signed-in caller.
    pub roles: &'static [&'static str],
    pub parents: &'static [Parent],
    pub children: &'static [Child],
}

const DEFAULT_PAGE: usize = 20;
const MAX_PAGE: usize = 100;

#[derive(Clone)]
struct Access {
    db: sqlx::PgPool,
    tenant: Option<String>,
    roles: Vec<String>,
    tables: &'static [Table],
}

fn refuse(code: &'static str, message: &str) -> Error {
    Error::new(message).extend_with(move |_, e| e.set("code", code))
}

fn internal(error: impl std::fmt::Display) -> Error {
    tracing::error!(%error, "graphql query failed");
    refuse("ERREUR_INTERNE", "internal error")
}

/// A mutation: a route of the service (`POST`, `PUT`, `PATCH` or `DELETE`) played over the loopback. `params` are the `{params}`
/// of `path`, read from the `input` of the mutation; the rest of `input` is the JSON body when the method has one.
pub struct Mutation {
    pub field: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub params: &'static [&'static str],
    pub body: bool,
    /// The roles allowed to call it; empty: any signed-in caller.
    pub roles: &'static [&'static str],
}

/// Where the routes of the service listen and who is asking: the mutation sends the caller's own `access_token`, so the route
/// checks the token, the tenant and the input as it does for any client.
#[derive(Clone)]
pub struct Loopback {
    pub base: String,
    pub token: String,
}

/// The `{code, message}` of a route's refusal: the code the flow named, or the one of its status.
fn route_error(status: u16, body: &[u8]) -> Error {
    let v: Json = serde_json::from_slice(body).unwrap_or(Json::Null);
    let text = |k: &str| v.get(k).and_then(Json::as_str).map(str::to_owned);
    let code = text("code").unwrap_or_else(|| {
        match status {
            400 | 422 => "REQUETE_INVALIDE",
            401 => "NON_AUTHENTIFIE",
            403 => "ACCES_REFUSE",
            404 => "INTROUVABLE",
            409 => "CONFLIT",
            429 => "TROP_DE_REQUETES",
            _ => "ERREUR_INTERNE",
        }
        .to_owned()
    });
    let message = text("message").or_else(|| text("error")).unwrap_or_else(|| "the call was refused".to_owned());
    Error::new(message).extend_with(move |_, e| {
        e.set("code", code.clone());
        e.set("status", status);
    })
}

/// The path of the route with its `{params}` filled from `input`, which are then taken out of it. A value is a text made of
/// letters, digits and `-_.~`: nothing that could leave the path.
fn fill_path(m: &Mutation, input: &mut Map<String, Json>) -> Result<String, Error> {
    let mut path = m.path.to_owned();
    for p in m.params {
        let value = input.remove(*p).as_ref().and_then(text_of).ok_or_else(|| refuse("PARAMETRE_INVALIDE", "a path parameter is missing"))?;
        if value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || "-_.~".contains(c)) {
            return Err(refuse("PARAMETRE_INVALIDE", "a path parameter is not valid"));
        }
        path = path.replace(&format!("{{{p}}}"), &value);
    }
    Ok(path)
}

#[cfg(feature = "graphql-mutations")]
async fn call_route(lb: &Loopback, m: &Mutation, input: Map<String, Json>, path: String) -> Result<Json, Error> {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = CLIENT.get_or_init(reqwest::Client::new);
    let method = reqwest::Method::from_bytes(m.method.as_bytes()).map_err(internal)?;
    let mut request = client
        .request(method, format!("{}{}", lb.base.trim_end_matches('/'), path))
        .header("Cookie", format!("access_token={}", lb.token))
        .timeout(std::time::Duration::from_secs(30));
    if m.body {
        request = request.json(&Json::Object(input));
    }
    let response = request.send().await.map_err(internal)?;
    let status = response.status().as_u16();
    let bytes = response.bytes().await.map_err(internal)?;
    if !(200..300).contains(&status) {
        return Err(route_error(status, &bytes));
    }
    if bytes.is_empty() { Ok(Json::Null) } else { serde_json::from_slice(&bytes).map_err(internal) }
}

#[cfg(not(feature = "graphql-mutations"))]
async fn call_route(_: &Loopback, _: &Mutation, _: Map<String, Json>, _: String) -> Result<Json, Error> {
    Err(internal("mutations are not built into this service"))
}

fn mutation_field(m: &'static Mutation) -> Field {
    Field::new(m.field, TypeRef::named("JSON"), move |ctx| {
        FieldFuture::new(async move {
            let lb = ctx.data::<Loopback>().map_err(|_| refuse("NON_AUTHENTIFIE", "not signed in"))?;
            if !m.roles.is_empty() {
                let access = ctx.data::<Access>().map_err(internal)?;
                if !m.roles.iter().any(|r| access.roles.iter().any(|h| h == r)) {
                    return Err(refuse("ACCES_REFUSE", "your role cannot call this mutation"));
                }
            }
            let mut input = match ctx.args.get("input") {
                None => Map::new(),
                Some(v) => match v.as_value().clone().into_json() {
                    Ok(Json::Object(o)) => o,
                    Ok(Json::Null) => Map::new(),
                    _ => return Err(refuse("PARAMETRE_INVALIDE", "input must be an object")),
                },
            };
            let path = fill_path(m, &mut input)?;
            let out = call_route(lb, m, input, path).await?;
            Ok(Some(FieldValue::value(Value::from_json(out).map_err(internal)?)))
        })
    })
    .argument(InputValue::new("input", TypeRef::named("JSON")))
}

fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn text_of(v: &Json) -> Option<String> {
    match v {
        Json::Null => None,
        Json::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn table<'a>(tables: &'a [Table], name: &str) -> Option<&'a Table> {
    tables.iter().find(|t| t.name == name)
}

/// The rows of `table` that match `filters` (column, text value), a page at a time.
async fn rows(
    access: &Access,
    table: &Table,
    filters: &[(&str, String)],
    limit: usize,
    offset: usize,
) -> Result<Vec<Json>, Error> {
    if !table.roles.is_empty() && !table.roles.iter().any(|r| access.roles.iter().any(|h| h == r)) {
        return Err(refuse("ACCES_REFUSE", "your role cannot read this data"));
    }
    let mut conditions = Vec::new();
    let mut binds: Vec<&str> = Vec::new();
    for (column, value) in filters {
        binds.push(value);
        conditions.push(format!("{}::text = ${}", quote(column), binds.len()));
    }
    if let Some(column) = table.tenant {
        // A table of a tenant is never read without one.
        let tenant = access
            .tenant
            .as_deref()
            .ok_or_else(|| refuse("ACCES_REFUSE", "the token names no tenant"))?;
        binds.push(tenant);
        conditions.push(format!("{}::text = ${}", quote(column), binds.len()));
    }
    let filter = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };
    let order = if table.keys.is_empty() {
        "1".to_string()
    } else {
        table.keys.iter().map(|k| quote(k.name)).collect::<Vec<_>>().join(", ")
    };
    let sql = format!(
        "SELECT to_jsonb(t)::text FROM {} t{filter} ORDER BY {order} LIMIT {limit} OFFSET {offset}",
        quote(table.name)
    );
    let mut query = sqlx::query_scalar::<_, String>(&sql);
    for b in binds {
        query = query.bind(b);
    }
    let found = query.fetch_all(&access.db).await.map_err(internal)?;
    found
        .iter()
        .map(|r| serde_json::from_str(r).map_err(internal))
        .collect()
}

async fn connection(
    access: &Access,
    table: &Table,
    filters: &[(&str, String)],
    first: Option<i64>,
    after: Option<String>,
) -> Result<Json, Error> {
    let size = usize::try_from(first.unwrap_or(DEFAULT_PAGE as i64))
        .map_err(|_| refuse("PARAMETRE_INVALIDE", "`first` cannot be negative"))?
        .min(MAX_PAGE);
    let offset: usize = match after {
        Some(cursor) => cursor
            .parse()
            .map_err(|_| refuse("PARAMETRE_INVALIDE", "`after` is not a cursor of this endpoint"))?,
        None => 0,
    };
    let mut found = rows(access, table, filters, size + 1, offset).await?;
    let has_next = found.len() > size;
    found.truncate(size);
    let end = (!found.is_empty()).then(|| (offset + found.len()).to_string());
    let edges: Vec<Json> = found
        .into_iter()
        .enumerate()
        .map(|(i, node)| serde_json::json!({ "node": node, "cursor": (offset + i + 1).to_string() }))
        .collect();
    Ok(serde_json::json!({ "edges": edges, "pageInfo": { "hasNextPage": has_next, "endCursor": end } }))
}

pub(crate) fn type_ref(scalar: &str, nullable: bool) -> TypeRef {
    if nullable { TypeRef::named(scalar) } else { TypeRef::named_nn(scalar) }
}

pub(crate) fn parent_json<'a>(ctx: &'a ResolverContext<'_>) -> Result<&'a Json, Error> {
    ctx.parent_value.try_downcast_ref::<Json>()
}

fn access<'a>(ctx: &'a ResolverContext<'_>) -> Result<&'a Access, Error> {
    ctx.data::<Access>()
}

fn arg_text(ctx: &ResolverContext<'_>, name: &str) -> Result<String, Error> {
    match ctx.args.try_get(name)?.as_value() {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Boolean(b) => Ok(b.to_string()),
        _ => Err(refuse("PARAMETRE_INVALIDE", "unexpected argument")),
    }
}

fn page_args(ctx: &ResolverContext<'_>) -> Result<(Option<i64>, Option<String>), Error> {
    let first = match ctx.args.get("first") {
        Some(v) => Some(v.i64()?),
        None => None,
    };
    let after = match ctx.args.get("after") {
        Some(v) => Some(v.string()?.to_string()),
        None => None,
    };
    Ok((first, after))
}

fn page_field_args(field: Field) -> Field {
    field
        .argument(InputValue::new("first", TypeRef::named(TypeRef::INT)))
        .argument(InputValue::new("after", TypeRef::named(TypeRef::STRING)))
}

/// The object a JSON value stands for.
pub(crate) fn object(v: Json) -> FieldValue<'static> {
    FieldValue::owned_any(v)
}

pub(crate) fn prop(parent: &Json, key: &str) -> Option<FieldValue<'static>> {
    match parent.get(key) {
        None | Some(Json::Null) => None,
        Some(v) => Value::from_json(v.clone()).ok().map(FieldValue::value),
    }
}

/// The schema of the tables. `max_depth` and `max_complexity` bound a query; `introspection` says whether the schema can be
/// asked for.
pub fn schema(
    tables: &'static [Table],
    mutations: &'static [Mutation],
    max_depth: usize,
    max_complexity: usize,
    introspection: bool,
) -> Result<Schema, async_graphql::dynamic::SchemaError> {
    let mut query = Object::new("Query");
    let mut builder = Schema::build("Query", (!mutations.is_empty()).then_some("Mutation"), None);
    if !mutations.is_empty() {
        let mut root = Object::new("Mutation");
        for m in mutations {
            root = root.field(mutation_field(m));
        }
        builder = builder.register(root);
    }
    for s in ["DateTime", "JSON", "Long"] {
        builder = builder.register(Scalar::new(s));
    }

    let mut page = Object::new("PageInfo");
    page = page.field(Field::new("hasNextPage", TypeRef::named_nn(TypeRef::BOOLEAN), |ctx| {
        FieldFuture::new(async move { Ok(prop(parent_json(&ctx)?, "hasNextPage")) })
    }));
    page = page.field(Field::new("endCursor", TypeRef::named(TypeRef::STRING), |ctx| {
        FieldFuture::new(async move { Ok(prop(parent_json(&ctx)?, "endCursor")) })
    }));
    builder = builder.register(page);

    for t in tables {
        let mut ty = Object::new(t.ty);
        for col in t.columns {
            ty = ty.field(Field::new(col.field, type_ref(col.scalar, col.nullable), move |ctx| {
                FieldFuture::new(async move { Ok(prop(parent_json(&ctx)?, col.name)) })
            }));
        }
        for p in t.parents {
            let Some(target) = table(tables, p.table) else { continue };
            ty = ty.field(Field::new(p.field, type_ref(target.ty, p.nullable), move |ctx| {
                FieldFuture::new(async move {
                    let Some(value) = parent_json(&ctx)?.get(p.column).and_then(text_of) else {
                        return Ok(None);
                    };
                    let access = access(&ctx)?;
                    let target = table(access.tables, p.table).ok_or_else(|| internal("unknown table"))?;
                    let found = rows(access, target, &[(p.ref_column, value)], 1, 0).await?;
                    Ok(found.into_iter().next().map(object))
                })
            }));
        }
        for ch in t.children {
            let Some(target) = table(tables, ch.table) else { continue };
            let field = Field::new(
                ch.field,
                TypeRef::named_nn(format!("{}Connection", target.ty)),
                move |ctx| {
                    FieldFuture::new(async move {
                        let Some(value) = parent_json(&ctx)?.get(ch.ref_column).and_then(text_of) else {
                            return Ok(None);
                        };
                        let (first, after) = page_args(&ctx)?;
                        let access = access(&ctx)?;
                        let target = table(access.tables, ch.table).ok_or_else(|| internal("unknown table"))?;
                        let page = connection(access, target, &[(ch.column, value)], first, after).await?;
                        Ok(Some(object(page)))
                    })
                },
            );
            ty = ty.field(page_field_args(field));
        }
        builder = builder.register(ty);

        let mut edge = Object::new(format!("{}Edge", t.ty));
        edge = edge.field(Field::new("node", TypeRef::named_nn(t.ty), |ctx| {
            FieldFuture::new(async move { Ok(parent_json(&ctx)?.get("node").cloned().map(object)) })
        }));
        edge = edge.field(Field::new("cursor", TypeRef::named_nn(TypeRef::STRING), |ctx| {
            FieldFuture::new(async move { Ok(prop(parent_json(&ctx)?, "cursor")) })
        }));
        builder = builder.register(edge);

        let mut conn = Object::new(format!("{}Connection", t.ty));
        conn = conn.field(Field::new(
            "edges",
            TypeRef::named_nn_list_nn(format!("{}Edge", t.ty)),
            |ctx| {
                FieldFuture::new(async move {
                    let edges = parent_json(&ctx)?.get("edges").and_then(Json::as_array).cloned().unwrap_or_default();
                    Ok(Some(FieldValue::list(edges.into_iter().map(object))))
                })
            },
        ));
        conn = conn.field(Field::new("pageInfo", TypeRef::named_nn("PageInfo"), |ctx| {
            FieldFuture::new(async move { Ok(parent_json(&ctx)?.get("pageInfo").cloned().map(object)) })
        }));
        builder = builder.register(conn);

        if !t.keys.is_empty() {
            let mut by_key = Field::new(t.query, TypeRef::named(t.ty), move |ctx| {
                FieldFuture::new(async move {
                    let access = access(&ctx)?;
                    let target = table(access.tables, t.name).ok_or_else(|| internal("unknown table"))?;
                    let mut filters = Vec::new();
                    for k in target.keys {
                        filters.push((k.name, arg_text(&ctx, k.field)?));
                    }
                    let found = rows(access, target, &filters, 1, 0).await?;
                    Ok(found.into_iter().next().map(object))
                })
            });
            for k in t.keys {
                by_key = by_key.argument(InputValue::new(k.field, TypeRef::named_nn(k.scalar)));
            }
            query = query.field(by_key);
        }
        let list = Field::new(
            format!("{}Connection", t.query),
            TypeRef::named_nn(format!("{}Connection", t.ty)),
            move |ctx| {
                FieldFuture::new(async move {
                    let (first, after) = page_args(&ctx)?;
                    let access = access(&ctx)?;
                    let target = table(access.tables, t.name).ok_or_else(|| internal("unknown table"))?;
                    Ok(Some(object(connection(access, target, &[], first, after).await?)))
                })
            },
        );
        query = query.field(page_field_args(list));
    }

    builder = builder.register(query).limit_depth(max_depth).limit_complexity(max_complexity);
    if !introspection {
        builder = builder.disable_introspection();
    }
    builder.finish()
}

/// The endpoint of a service: its schema, built once.
pub struct Endpoint {
    schema: Schema,
    tables: &'static [Table],
}

impl Endpoint {
    /// Limits and introspection come from the environment: `GRAPHQL_MAX_DEPTH` (8), `GRAPHQL_MAX_COMPLEXITY` (200) and
    /// `GRAPHQL_INTROSPECTION` (`off` turns it off).
    pub fn from_env(tables: &'static [Table], mutations: &'static [Mutation]) -> anyhow::Result<Self> {
        let number = |name: &str, default: usize| {
            std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
        };
        let introspection = std::env::var("GRAPHQL_INTROSPECTION")
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "false" | "0" | "no"))
            .unwrap_or(true);
        let schema = schema(
            tables,
            mutations,
            number("GRAPHQL_MAX_DEPTH", 8),
            number("GRAPHQL_MAX_COMPLEXITY", 200),
            introspection,
        )
        .map_err(|e| anyhow::anyhow!("cannot build the GraphQL schema: {e}"))?;
        Ok(Self { schema, tables })
    }

    /// Runs the request of a client for `tenant`; the answer is the JSON body of the response, every error with a `code`.
    pub async fn execute(&self, db: sqlx::PgPool, tenant: Option<String>, roles: Vec<String>, body: Json) -> Json {
        self.execute_with(db, tenant, roles, None, body).await
    }

    /// Same, for a service that has mutations: they play the routes of the service through `loopback`.
    pub async fn execute_with(
        &self,
        db: sqlx::PgPool,
        tenant: Option<String>,
        roles: Vec<String>,
        loopback: Option<Loopback>,
        body: Json,
    ) -> Json {
        let request: Request = match serde_json::from_value(body) {
            Ok(r) => r,
            Err(_) => return error_body("REQUETE_INVALIDE", "the body is not a GraphQL request"),
        };
        let mut request = request.data(Access { db, tenant, roles, tables: self.tables });
        if let Some(loopback) = loopback {
            request = request.data(loopback);
        }
        let response = self.schema.execute(request).await;
        let mut out = serde_json::to_value(&response).unwrap_or_else(|_| Json::Object(Map::new()));
        add_codes(&mut out);
        out
    }
}

/// The body of an answer made of one error.
pub fn error_body(code: &str, message: &str) -> Json {
    serde_json::json!({ "errors": [{ "message": message, "extensions": { "code": code } }] })
}

/// A refusal of the engine (syntax, depth, complexity...) has no code of ours yet: it gets the closest.
pub(crate) fn add_codes(out: &mut Json) {
    let Some(errors) = out.get_mut("errors").and_then(Json::as_array_mut) else { return };
    for e in errors {
        let message = e.get("message").and_then(Json::as_str).unwrap_or_default().to_lowercase();
        let has_code = e.pointer("/extensions/code").is_some();
        if has_code {
            continue;
        }
        let code = if message.contains("nested too deep") {
            "REQUETE_TROP_PROFONDE"
        } else if message.contains("too complex") {
            "REQUETE_TROP_COMPLEXE"
        } else if message.contains("introspection") {
            "INTROSPECTION_DESACTIVEE"
        } else {
            "REQUETE_INVALIDE"
        };
        if let Some(obj) = e.as_object_mut() {
            let ext = obj.entry("extensions").or_insert_with(|| Json::Object(Map::new()));
            if let Some(ext) = ext.as_object_mut() {
                ext.insert("code".into(), Json::String(code.into()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: Column = Column { name: "id", field: "id", scalar: "ID", nullable: false };
    const NAME: Column = Column { name: "nom", field: "nom", scalar: "String", nullable: false };
    static TABLES: &[Table] = &[Table {
        name: "articles",
        ty: "Articles",
        query: "articles",
        columns: &[ID, NAME],
        keys: &[ID],
        tenant: Some("tenant_id"),
        roles: &["admin"],
        parents: &[],
        children: &[Child { field: "again", table: "articles", column: "id", ref_column: "id" }],
    }];

    static MUTATIONS: &[Mutation] = &[
        Mutation { field: "createArticle", method: "POST", path: "/articles", params: &[], body: true, roles: &[] },
        Mutation { field: "renameArticle", method: "PUT", path: "/articles/{id}", params: &["id"], body: true, roles: &[] },
        Mutation { field: "purgeArticle", method: "DELETE", path: "/articles/{id}", params: &["id"], body: false, roles: &["admin"] },
    ];

    #[tokio::test]
    async fn a_mutation_with_roles_is_refused_before_the_route_is_played() {
        let e = endpoint(true, 8, 200);
        // The loopback points nowhere: a refusal here can only come from the role check.
        let run = |roles: Vec<String>| {
            let lb = Loopback { base: "http://127.0.0.1:1".into(), token: "tok".into() };
            let q = serde_json::json!({ "query": r#"mutation { purgeArticle(input: {id: "7"}) }"# });
            let e = &e;
            async move { e.execute_with(pool(), Some("t".into()), roles, Some(lb), q).await }
        };
        let code = |out: Json| out.pointer("/errors/0/extensions/code").and_then(Json::as_str).unwrap_or("none").to_owned();
        assert_eq!(code(run(vec!["viewer".into()]).await), "ACCES_REFUSE");
        assert_eq!(code(run(vec![]).await), "ACCES_REFUSE");
        assert_ne!(code(run(vec!["admin".into()]).await), "ACCES_REFUSE");
    }

    #[test]
    fn a_path_parameter_is_taken_from_the_input_and_cannot_leave_the_path() {
        let m = &MUTATIONS[1];
        let mut input = serde_json::json!({ "id": "a1-b", "nom": "x" }).as_object().unwrap().clone();
        assert_eq!(fill_path(m, &mut input).ok().as_deref(), Some("/articles/a1-b"));
        assert!(!input.contains_key("id") && input.contains_key("nom"));
        for bad in ["../x", "a/b", "a?b", ""] {
            let mut input = serde_json::json!({ "id": bad }).as_object().unwrap().clone();
            assert!(fill_path(m, &mut input).is_err(), "{bad}");
        }
        assert!(fill_path(m, &mut Map::new()).is_err());
    }

    #[test]
    fn a_refusal_of_a_route_keeps_its_code_and_message() {
        let named = route_error(409, br#"{"error":"stock","code":"STOCK_INSUFFISANT","message":"stock"}"#);
        assert_eq!(named.message, "stock");
        let plain = route_error(404, br#"{"error":"not found"}"#);
        assert_eq!(plain.message, "not found");
        assert_eq!(format!("{:?}", plain.extensions), format!("{:?}", route_error(404, b"").extensions));
    }

    #[tokio::test]
    async fn a_mutation_needs_the_caller_token() {
        let e = endpoint(true, 8, 200);
        let q = r#"mutation { createArticle(input: {nom: "x"}) }"#;
        assert_eq!(code_of(&e, Some("t"), q).await, "NON_AUTHENTIFIE");
    }

    #[cfg(feature = "graphql-mutations")]
    #[tokio::test]
    async fn a_mutation_plays_the_route_with_the_cookie_and_returns_its_error_code() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 4096];
                let n = s.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                assert!(req.contains("access_token=tok"), "{req}");
                let (status, body) = if req.starts_with("PUT /articles/7 ") {
                    ("409 Conflict", r#"{"error":"taken","code":"NOM_PRIS","message":"taken"}"#)
                } else {
                    ("201 Created", r#"{"id":"1"}"#)
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                s.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        let e = endpoint(true, 8, 200);
        let run = |q: &'static str| {
            let lb = Loopback { base: base.clone(), token: "tok".into() };
            let e = &e;
            async move { e.execute_with(pool(), Some("t".into()), vec![], Some(lb), serde_json::json!({ "query": q })).await }
        };
        let ok = run(r#"mutation { createArticle(input: {nom: "x"}) }"#).await;
        assert_eq!(ok["data"]["createArticle"]["id"], "1", "{ok}");
        let refused = run(r#"mutation { renameArticle(input: {id: "7", nom: "x"}) }"#).await;
        assert_eq!(refused["errors"][0]["extensions"]["code"], "NOM_PRIS", "{refused}");
    }

    fn endpoint(introspection: bool, depth: usize, complexity: usize) -> Endpoint {
        Endpoint { schema: schema(TABLES, MUTATIONS, depth, complexity, introspection).unwrap(), tables: TABLES }
    }

    fn pool() -> sqlx::PgPool {
        sqlx::postgres::PgPoolOptions::new().connect_lazy("postgres://nobody@127.0.0.1:1/none").unwrap()
    }

    async fn code_of(e: &Endpoint, tenant: Option<&str>, query: &str) -> String {
        let out = e.execute(pool(), tenant.map(str::to_owned), vec![], serde_json::json!({ "query": query })).await;
        out.pointer("/errors/0/extensions/code").and_then(Json::as_str).unwrap_or("none").to_owned()
    }

    #[tokio::test]
    async fn a_table_of_a_tenant_is_not_read_without_one() {
        let e = endpoint(true, 8, 200);
        assert_eq!(code_of(&e, None, "{ articlesConnection { edges { node { nom } } } }").await, "ACCES_REFUSE");
        assert_eq!(code_of(&e, None, "{ articles(id: \"1\") { nom } }").await, "ACCES_REFUSE");
    }

    #[tokio::test]
    async fn a_table_with_roles_is_refused_to_a_caller_without_one_of_them() {
        let e = endpoint(true, 8, 200);
        let q = serde_json::json!({ "query": "{ articlesConnection { edges { node { nom } } } }" });
        let code = |out: Json| out.pointer("/errors/0/extensions/code").and_then(Json::as_str).unwrap_or("none").to_owned();
        let out = e.execute(pool(), Some("t".into()), vec!["viewer".into()], q.clone()).await;
        assert_eq!(code(out), "ACCES_REFUSE");
        // With the role the refusal is no longer the role's: the (absent) database answers instead.
        let out = e.execute(pool(), Some("t".into()), vec!["admin".into()], q).await;
        assert_ne!(code(out), "ACCES_REFUSE");
    }

    #[tokio::test]
    async fn a_query_too_deep_or_too_complex_is_refused_with_a_code() {
        let deep = "{ articlesConnection { edges { node { again { edges { node { again { edges { node { nom } } } } } } } } } }";
        assert_eq!(code_of(&endpoint(true, 4, 1000), Some("t"), deep).await, "REQUETE_TROP_PROFONDE");
        assert_eq!(code_of(&endpoint(true, 50, 3), Some("t"), deep).await, "REQUETE_TROP_COMPLEXE");
    }

    #[tokio::test]
    async fn introspection_can_be_turned_off_and_a_bad_query_has_a_code() {
        let q = "{ __schema { types { name } } }";
        assert_eq!(code_of(&endpoint(true, 8, 200), Some("t"), q).await, "none");
        assert_ne!(code_of(&endpoint(false, 8, 200), Some("t"), q).await, "none");
        assert_eq!(code_of(&endpoint(true, 8, 200), Some("t"), "{ nothing }").await, "REQUETE_INVALIDE");
        let bad = endpoint(true, 8, 200).execute(pool(), None, vec![], serde_json::json!(7)).await;
        assert_eq!(bad["errors"][0]["extensions"]["code"], "REQUETE_INVALIDE");
    }
}
