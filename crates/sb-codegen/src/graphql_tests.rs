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
