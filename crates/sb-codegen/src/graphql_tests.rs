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
