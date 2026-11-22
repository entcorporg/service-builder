use crate::{GeneratedFile, generate};
use sb_model::{Column, ColumnType, ForeignKey, ReferentialAction, Table, sample};

fn project() -> sb_model::Project {
    let mut p = sample::orders();
    let key = |name: &str, ty| {
        let mut c = Column::new(name, ty);
        c.primary_key = true;
        c
    };
    let mut articles = Table::new(
        "articles",
        vec![
            key("id", ColumnType::Uuid),
            Column::new("nom", ColumnType::String),
            Column::new("tenant_id", ColumnType::Uuid),
        ],
    );
    articles.tenant_column = Some("tenant_id".into());
    let mut tags = Table::new(
        "etiquettes",
        vec![
            key("id", ColumnType::Uuid),
            Column::new("article_id", ColumnType::Uuid),
            Column::new("nom", ColumnType::String),
        ],
    );
    tags.foreign_keys.push(ForeignKey {
        name: "etiquettes_article_fk".into(),
        columns: vec!["article_id".into()],
        ref_table: "articles".into(),
        ref_columns: vec!["id".into()],
        on_delete: ReferentialAction::Cascade,
        on_update: ReferentialAction::NoAction,
    });
    p.services[0].tables.extend([articles, tags]);
    p
}

fn sdl_file(files: &[GeneratedFile]) -> Option<&str> {
    files
        .iter()
        .find(|f| f.path == "contracts/graphql/orders.graphql")
        .map(|f| f.content.as_str())
}

#[test]
fn a_service_that_asks_for_it_gets_its_schema_in_the_contracts() {
    let mut p = project();
    p.services[0].graphql = true;
    let files = generate(&p).unwrap();
    let sdl = sdl_file(&files).expect("sdl written");
    assert!(sdl.contains("type Articles {"), "{sdl}");
    assert!(sdl.contains("  nom: String!"), "{sdl}");
    // The tenant is not a field, and the relations go both ways.
    assert!(!sdl.contains("tenantId"), "{sdl}");
    assert!(sdl.contains("  article: Articles!"), "{sdl}");
    assert!(
        sdl.contains("  etiquettes(first: Int, after: String): EtiquettesConnection!"),
        "{sdl}"
    );
    assert!(sdl.contains("  articles(id: ID!): Articles"), "{sdl}");
    assert!(
        sdl.contains("  articlesConnection(first: Int, after: String): ArticlesConnection!"),
        "{sdl}"
    );
    // Deterministic.
    assert_eq!(sdl_file(&generate(&p).unwrap()), Some(sdl));
}

#[test]
fn a_project_that_does_not_ask_writes_nothing() {
    let files = generate(&project()).unwrap();
    assert!(!files.iter().any(|f| f.path.contains("graphql")));
}

fn served() -> sb_model::Project {
    let mut p = sample::showcase();
    let extra = project();
    let n = extra.services[0].tables.len();
    p.services[0]
        .tables
        .extend(extra.services[0].tables[n - 2..].iter().cloned());
    p.services[0].graphql = true;
    p
}

fn find<'a>(files: &'a [GeneratedFile], path: &str) -> Option<&'a str> {
    files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.content.as_str())
}

#[test]
fn the_endpoint_is_served_with_the_token_check_of_the_service() {
    let files = generate(&served()).unwrap();
    let code = find(&files, "services/orders/src/transport/graphql.rs").expect("endpoint");
    assert!(code.contains("\"access_token\""), "{code}");
    assert!(code.contains("NON_AUTHENTIFIE"), "{code}");
    assert!(
        code.contains("eda_runtime::jwt::verify(eda_runtime::jwt::Source::Keys(&["),
        "{code}"
    );
    assert!(code.contains("tenant: Some(\"tenant_id\")"), "{code}");
    assert!(code.contains("Child { field: \"etiquettes\""), "{code}");
    assert!(find(&files, "runtime/src/graphql.rs").is_some());
    let http = find(&files, "services/orders/src/transport/http.rs").unwrap();
    assert!(http.contains(".merge(super::graphql::router())"));
    assert!(
        find(&files, "services/orders/Cargo.toml")
            .unwrap()
            .contains("\"graphql\"")
    );
    assert!(
        find(&files, "Cargo.toml")
            .unwrap()
            .contains("async-graphql")
    );
    assert!(
        find(&files, "runtime/Cargo.toml")
            .unwrap()
            .contains("graphql = [\"dep:async-graphql\"]")
    );
    // Deterministic.
    let again = generate(&served()).unwrap();
    assert_eq!(
        find(&again, "services/orders/src/transport/graphql.rs"),
        Some(code)
    );
}

#[test]
fn without_graphql_nothing_of_the_endpoint_is_written() {
    let files = generate(&sample::showcase()).unwrap();
    assert!(!files.iter().any(|f| f.path.ends_with("graphql.rs")));
    assert!(!files.iter().any(|f| f.content.contains("async-graphql")));
    assert!(!files.iter().any(|f| f.content.contains("super::graphql")));
}

fn source(name: &str, path: &str, fields: &[(&str, ColumnType)]) -> sb_model::GraphqlSource {
    sb_model::GraphqlSource {
        name: name.into(),
        base_url_env: "STOCK_URL".into(),
        path: path.into(),
        fields: fields
            .iter()
            .map(|(n, ty)| sb_model::GraphqlSourceField {
                name: (*n).into(),
                ty: *ty,
                nullable: false,
            })
            .collect(),
        key: Some("id".into()),
        relations: vec![],
    }
}

/// A service with no table, routes and a token check, that reads articles and their movements from another service.
fn gateway() -> sb_model::Project {
    let mut p = sample::showcase();
    let svc = &mut p.services[0];
    svc.tables.clear();
    // What reads a table is not part of a service that has none.
    svc.routes
        .retain(|r| !serde_json::to_string(r).unwrap().contains("\"table\""));
    svc.jobs.clear();
    svc.graphql = true;
    let mut article = source(
        "Article",
        "/articles",
        &[("id", ColumnType::Uuid), ("nom", ColumnType::String)],
    );
    article.relations.push(sb_model::GraphqlRelation {
        field: "mouvements".into(),
        target: "Mouvement".into(),
        via: "id".into(),
        target_field: "article_id".into(),
        many: true,
        batch_param: None,
    });
    let movement = source(
        "Mouvement",
        "/mouvements",
        &[
            ("id", ColumnType::Uuid),
            ("article_id", ColumnType::Uuid),
            ("quantite", ColumnType::Int32),
        ],
    );
    svc.graphql_sources = vec![article, movement];
    p
}

#[test]
fn a_gateway_writes_its_schema_and_reads_the_services_behind_it() {
    let files = generate(&gateway()).unwrap();
    let sdl = sdl_file(&files).expect("sdl written");
    assert!(sdl.contains("type Article {"), "{sdl}");
    assert!(sdl.contains("  mouvements: [Mouvement!]!"), "{sdl}");
    assert!(sdl.contains("  article(id: ID!): Article"), "{sdl}");
    assert!(
        sdl.contains("  articleList(first: Int): [Article!]!"),
        "{sdl}"
    );
    assert!(sdl.contains("  articleId: ID!"), "{sdl}");
    let code = find(&files, "services/orders/src/transport/graphql.rs").expect("endpoint");
    assert!(code.contains("Gateway::from_env(SOURCES)"), "{code}");
    assert!(code.contains("base_url_env: \"STOCK_URL\""), "{code}");
    assert!(
        code.contains(
            "Relation { field: \"mouvements\", target: \"Mouvement\", via: \"id\", target_field: \"article_id\", many: true, batch_param: None }"
        ),
        "{code}"
    );
    assert!(code.contains("NON_AUTHENTIFIE"), "{code}");
    assert!(find(&files, "runtime/src/graphql_gateway.rs").is_some());
    let cargo = find(&files, "runtime/Cargo.toml").unwrap();
    assert!(cargo.contains("graphql-gateway = [\"graphql\", \"http-client\"]"));
    assert!(
        find(&files, "services/orders/Cargo.toml")
            .unwrap()
            .contains("\"graphql-gateway\"")
    );
    assert!(
        find(&files, "runtime/src/lib.rs")
            .unwrap()
            .contains("pub mod graphql_gateway;")
    );
    // Deterministic.
    let again = generate(&gateway()).unwrap();
    assert_eq!(
        find(&again, "services/orders/src/transport/graphql.rs"),
        Some(code)
    );
}

#[test]
fn the_sources_of_a_gateway_are_checked() {
    let mut p = gateway();
    p.services[0].graphql_sources[0].relations[0].target = "Nothing".into();
    p.services[0].graphql_sources[1].key = Some("nope".into());
    let codes: Vec<String> = sb_model::validate(&p).into_iter().map(|d| d.code).collect();
    assert!(codes.iter().any(|c| c == "E_GRAPHQL_RELATION"), "{codes:?}");
    assert!(
        codes.iter().any(|c| c == "E_GRAPHQL_SOURCE_KEY"),
        "{codes:?}"
    );
}

#[test]
fn the_routes_that_write_become_mutations_that_play_them() {
    let p = served();
    let writes = p.services[0].routes.iter().any(|r| {
        r.method != sb_model::HttpMethod::Get && r.body_format != sb_model::BodyFormat::Csv
    });
    assert!(writes, "the sample has a writing route");
    let files = generate(&p).unwrap();
    let sdl = sdl_file(&files).expect("sdl");
    assert!(sdl.contains("type Mutation {"), "{sdl}");
    assert!(sdl.contains("(input: JSON): JSON"), "{sdl}");
    let code = find(&files, "services/orders/src/transport/graphql.rs").expect("endpoint");
    assert!(code.contains("static MUTATIONS: &[Mutation]"), "{code}");
    assert!(
        code.contains("Endpoint::from_env(TABLES, MUTATIONS)"),
        "{code}"
    );
    assert!(code.contains("Loopback { base: format!"), "{code}");
    assert!(
        find(&files, "services/orders/Cargo.toml")
            .unwrap()
            .contains("\"graphql-mutations\"")
    );
    assert!(
        find(&files, "runtime/Cargo.toml")
            .unwrap()
            .contains("graphql-mutations = [\"graphql\", \"http-client\"]")
    );
    // Deterministic.
    assert_eq!(
        find(
            &generate(&p).unwrap(),
            "services/orders/src/transport/graphql.rs"
        ),
        Some(code)
    );
}

#[test]
fn a_service_with_no_writing_route_has_no_mutation() {
    let mut p = served();
    p.services[0]
        .routes
        .retain(|r| r.method == sb_model::HttpMethod::Get);
    let files = generate(&p).unwrap();
    assert!(!sdl_file(&files).unwrap_or_default().contains("Mutation"));
    let code = find(&files, "services/orders/src/transport/graphql.rs").unwrap_or_default();
    assert!(!code.contains("MUTATIONS") && !code.contains("Loopback"));
}
