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
    pub parents: &'static [Parent],
    pub children: &'static [Child],
}

const DEFAULT_PAGE: usize = 20;
const MAX_PAGE: usize = 100;

#[derive(Clone)]
struct Access {
    db: sqlx::PgPool,
    tenant: Option<String>,
    tables: &'static [Table],
}

fn refuse(code: &'static str, message: &str) -> Error {
    Error::new(message).extend_with(move |_, e| e.set("code", code))
}

fn internal(error: impl std::fmt::Display) -> Error {
    tracing::error!(%error, "graphql query failed");
    refuse("ERREUR_INTERNE", "internal error")
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
    max_depth: usize,
    max_complexity: usize,
    introspection: bool,
) -> Result<Schema, async_graphql::dynamic::SchemaError> {
    let mut query = Object::new("Query");
    let mut builder = Schema::build("Query", None, None);
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
    pub fn from_env(tables: &'static [Table]) -> anyhow::Result<Self> {
        let number = |name: &str, default: usize| {
            std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
        };
        let introspection = std::env::var("GRAPHQL_INTROSPECTION")
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "false" | "0" | "no"))
            .unwrap_or(true);
        let schema = schema(tables, number("GRAPHQL_MAX_DEPTH", 8), number("GRAPHQL_MAX_COMPLEXITY", 200), introspection)
            .map_err(|e| anyhow::anyhow!("cannot build the GraphQL schema: {e}"))?;
        Ok(Self { schema, tables })
    }

    /// Runs the request of a client for `tenant`; the answer is the JSON body of the response, every error with a `code`.
    pub async fn execute(&self, db: sqlx::PgPool, tenant: Option<String>, body: Json) -> Json {
        let request: Request = match serde_json::from_value(body) {
            Ok(r) => r,
            Err(_) => return error_body("REQUETE_INVALIDE", "the body is not a GraphQL request"),
        };
        let response = self.schema.execute(request.data(Access { db, tenant, tables: self.tables })).await;
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
        parents: &[],
        children: &[Child { field: "again", table: "articles", column: "id", ref_column: "id" }],
    }];

    fn endpoint(introspection: bool, depth: usize, complexity: usize) -> Endpoint {
        Endpoint { schema: schema(TABLES, depth, complexity, introspection).unwrap(), tables: TABLES }
    }

    fn pool() -> sqlx::PgPool {
        sqlx::postgres::PgPoolOptions::new().connect_lazy("postgres://nobody@127.0.0.1:1/none").unwrap()
    }

    async fn code_of(e: &Endpoint, tenant: Option<&str>, query: &str) -> String {
        let out = e.execute(pool(), tenant.map(str::to_owned), serde_json::json!({ "query": query })).await;
        out.pointer("/errors/0/extensions/code").and_then(Json::as_str).unwrap_or("none").to_owned()
    }

    #[tokio::test]
    async fn a_table_of_a_tenant_is_not_read_without_one() {
        let e = endpoint(true, 8, 200);
        assert_eq!(code_of(&e, None, "{ articlesConnection { edges { node { nom } } } }").await, "ACCES_REFUSE");
        assert_eq!(code_of(&e, None, "{ articles(id: \"1\") { nom } }").await, "ACCES_REFUSE");
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
        let bad = endpoint(true, 8, 200).execute(pool(), None, serde_json::json!(7)).await;
        assert_eq!(bad["errors"][0]["extensions"]["code"], "REQUETE_INVALIDE");
    }
}
