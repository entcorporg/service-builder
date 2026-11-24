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
                values: Default::default(),
            })
            .collect(),
        key: Some("id".into()),
        relations: vec![],
        computed: vec![],
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
        fixed_params: [("archive".to_string(), "tous".to_string())].into(),
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
    assert!(code.contains("Gateway::from_env(SOURCES, ROOTS)"), "{code}");
    assert!(code.contains("base_url_env: \"STOCK_URL\""), "{code}");
    assert!(
        code.contains(
            "Relation { field: \"mouvements\", target: \"Mouvement\", via: \"id\", target_field: \"article_id\", many: true, batch_param: None, fixed_params: &[(\"archive\", \"tous\")] }"
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

#[test]
fn the_endpoint_path_and_the_roles_of_a_table_are_in_the_generated_code() {
    let mut p = served();
    p.services[0].graphql_path = Some("/api/graph".into());
    let n = p.services[0].tables.len();
    p.services[0].tables[n - 2].graphql_roles = vec!["admin".into(), "support".into()];
    let files = generate(&p).unwrap();
    let code = find(&files, "services/orders/src/transport/graphql.rs").expect("endpoint");
    assert!(
        code.contains(".route(\"/api/graph\", post(handle))"),
        "{code}"
    );
    assert!(!code.contains(".route(\"/graphql\""), "{code}");
    assert!(code.contains("roles: &[\"admin\", \"support\"],"), "{code}");
    assert!(code.contains("roles: &[],"), "{code}");
    assert!(
        code.contains("claims") && code.contains("\"roles\""),
        "{code}"
    );
    assert!(code.contains("tenant, roles,"), "{code}");
}

#[test]
fn the_roles_of_a_mutation_are_in_the_generated_code_and_checked() {
    let mut p = served();
    let route = p.services[0]
        .routes
        .iter()
        .find(|r| {
            r.method != sb_model::HttpMethod::Get && r.body_format != sb_model::BodyFormat::Csv
        })
        .expect("a writing route")
        .name
        .clone();
    p.services[0]
        .graphql_mutation_roles
        .insert(route, vec!["admin".into()]);
    let files = generate(&p).unwrap();
    let code = find(&files, "services/orders/src/transport/graphql.rs").expect("endpoint");
    assert!(code.contains("roles: &[\"admin\"] },"), "{code}");
    assert!(
        sb_model::validate(&p)
            .iter()
            .all(|d| d.code != "E_GRAPHQL_ROLE")
    );
    // A route that does not exist is refused.
    p.services[0]
        .graphql_mutation_roles
        .insert("nothing".into(), vec![]);
    let codes: Vec<String> = sb_model::validate(&p).into_iter().map(|d| d.code).collect();
    assert!(codes.iter().any(|c| c == "E_GRAPHQL_ROLE"), "{codes:?}");
}

#[test]
fn the_graphql_path_and_roles_are_checked() {
    let mut p = served();
    p.services[0].graphql_path = Some("graph".into());
    p.services[0].tables[0].graphql_roles = vec!["".into()];
    let codes: Vec<String> = sb_model::validate(&p).into_iter().map(|d| d.code).collect();
    assert!(codes.iter().any(|c| c == "E_GRAPHQL_PATH"), "{codes:?}");
    assert!(codes.iter().any(|c| c == "E_GRAPHQL_ROLE"), "{codes:?}");
}

#[test]
fn the_tenant_claim_can_be_named() {
    let files = generate(&served()).unwrap();
    let code = find(&files, "services/orders/src/transport/graphql.rs").expect("endpoint");
    assert!(code.contains("[\"tenantId\", \"tenant_id\"]"), "{code}");
    let mut p = served();
    p.services[0].graphql_tenant_claim = Some("tid".into());
    let files = generate(&p).unwrap();
    let code = find(&files, "services/orders/src/transport/graphql.rs").expect("endpoint");
    assert!(code.contains("let tenant = [\"tid\"]"), "{code}");
    assert!(!code.contains("\"tenantId\""), "{code}");
}

/// The gateway of a dashboard: a root with arguments and a guard, blocks, and computed fields.
fn dashboard() -> sb_model::Project {
    use sb_model::{
        GraphqlArg, GraphqlBlock, GraphqlComputed, GraphqlComputedKind, GraphqlGuard, GraphqlRoot,
    };
    let mut p = gateway();
    let svc = &mut p.services[0];
    svc.graphql_sources[1]
        .fields
        .push(sb_model::GraphqlSourceField {
            name: "prix".into(),
            ty: ColumnType::Int32,
            nullable: false,
            values: Default::default(),
        });
    svc.graphql_sources[1].computed = vec![
        GraphqlComputed {
            name: "valeur".into(),
            kind: GraphqlComputedKind::Product {
                fields: vec!["quantite".into(), "prix".into()],
            },
        },
        GraphqlComputed {
            name: "taux".into(),
            kind: GraphqlComputedKind::Ratio {
                numerator: "quantite".into(),
                denominator: "prix".into(),
            },
        },
    ];
    svc.graphql_sources[0].computed = vec![GraphqlComputed {
        name: "valeurStock".into(),
        kind: GraphqlComputedKind::Sum {
            relation: "mouvements".into(),
            field: "quantite".into(),
            times: Some("prix".into()),
        },
    }];
    svc.graphql_roots = vec![GraphqlRoot {
        name: "tableauDeBord".into(),
        args: vec![
            GraphqlArg {
                name: "entrepotId".into(),
                ty: ColumnType::Uuid,
                default: None,
            },
            GraphqlArg {
                name: "limit".into(),
                ty: ColumnType::Int32,
                default: Some("6".into()),
            },
        ],
        guard: Some(GraphqlGuard {
            source: "Article".into(),
            path: "/entrepots/{entrepotId}".into(),
        }),
        blocks: vec![
            GraphqlBlock {
                field: "articles".into(),
                source: "Article".into(),
                many: true,
                params: [("limit".to_string(), "limit".to_string())].into(),
                fixed_params: [("archive".to_string(), "tous".to_string())].into(),
                path: None,
            },
            GraphqlBlock {
                field: "premierMouvement".into(),
                source: "Mouvement".into(),
                many: false,
                params: [("entrepot_id".to_string(), "entrepotId".to_string())].into(),
                fixed_params: Default::default(),
                path: None,
            },
        ],
        sums: vec![],
    }];
    p
}

#[test]
fn a_gateway_declares_roots_with_arguments_blocks_a_guard_and_computed_fields() {
    let p = dashboard();
    let errors: Vec<_> = sb_model::validate(&p)
        .into_iter()
        .filter(|d| d.severity == sb_model::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let files = generate(&p).unwrap();
    let sdl = sdl_file(&files).expect("sdl written");
    assert!(sdl.contains("type TableauDeBord {"), "{sdl}");
    assert!(sdl.contains("  articles: [Article!]"), "{sdl}");
    assert!(sdl.contains("  premierMouvement: Mouvement"), "{sdl}");
    assert!(
        sdl.contains("  tableauDeBord(entrepotId: ID!, limit: Int = 6): TableauDeBord!"),
        "{sdl}"
    );
    assert!(sdl.contains("  valeurStock: Long"), "{sdl}");
    assert!(sdl.contains("  taux: Float"), "{sdl}");
    let code = find(&files, "services/orders/src/transport/graphql.rs").expect("endpoint");
    for wanted in [
        "Arg { name: \"limit\", scalar: \"Int\", default: Some(\"6\") }",
        "guard: Some(Guard { source: \"Article\", path: \"/entrepots/{entrepotId}\" })",
        "Block { field: \"articles\", source: \"Article\", many: true, params: &[(\"limit\", \"limit\")], fixed_params: &[(\"archive\", \"tous\")], path: None }",
        "static FORWARD: &[&str] = &[];",
        "Kind::Sum { relation: \"mouvements\", field: \"quantite\", times: Some(\"prix\") }",
        "Kind::Ratio(\"quantite\", \"prix\")",
        "fixed_params: &[(\"archive\", \"tous\")] }",
    ] {
        assert!(code.contains(wanted), "{wanted} in {code}");
    }
    // Deterministic.
    let again = generate(&dashboard()).unwrap();
    assert_eq!(
        find(&again, "services/orders/src/transport/graphql.rs"),
        Some(code)
    );
}

#[test]
fn a_gateway_forwards_the_headers_it_is_told_to_and_reads_block_paths() {
    let mut p = dashboard();
    p.services[0].graphql_forward_headers = vec!["X-Frontend-ID".into()];
    p.services[0].graphql_roots[0].blocks[1].path = Some("/entrepots/{entrepotId}/capacite".into());
    let files = generate(&p).unwrap();
    let code = find(&files, "services/orders/src/transport/graphql.rs").expect("endpoint");
    assert!(
        code.contains("static FORWARD: &[&str] = &[\"x-frontend-id\"];"),
        "{code}"
    );
    assert!(
        code.contains("path: Some(\"/entrepots/{entrepotId}/capacite\")"),
        "{code}"
    );
    assert!(
        !sb_model::validate(&p)
            .iter()
            .any(|d| d.code.starts_with("E_GRAPHQL")),
        "valid"
    );
    p.services[0].graphql_forward_headers = vec!["Cookie".into()];
    p.services[0].graphql_roots[0].blocks[1].path = Some("/entrepots/{inconnu}/capacite".into());
    let codes: Vec<String> = sb_model::validate(&p).into_iter().map(|d| d.code).collect();
    for wanted in ["E_GRAPHQL_FORWARD_HEADER", "E_GRAPHQL_ROOT_BLOCK"] {
        assert!(codes.iter().any(|c| c == wanted), "{wanted} in {codes:?}");
    }
}

#[test]
fn the_roots_and_computed_fields_of_a_gateway_are_checked() {
    let mut p = dashboard();
    let svc = &mut p.services[0];
    svc.graphql_roots[0].guard.as_mut().unwrap().path = "/entrepots/{inconnu}".into();
    svc.graphql_roots[0].blocks[0].source = "Nothing".into();
    svc.graphql_roots[0].blocks[1]
        .params
        .insert("x".into(), "nope".into());
    svc.graphql_sources[1].computed[0].name = "quantite".into();
    svc.graphql_sources[0].computed[0] = sb_model::GraphqlComputed {
        name: "somme".into(),
        kind: sb_model::GraphqlComputedKind::Sum {
            relation: "absente".into(),
            field: "quantite".into(),
            times: None,
        },
    };
    let codes: Vec<String> = sb_model::validate(&p).into_iter().map(|d| d.code).collect();
    for wanted in [
        "E_GRAPHQL_ROOT_GUARD",
        "E_GRAPHQL_ROOT_BLOCK",
        "E_GRAPHQL_COMPUTED",
    ] {
        assert!(codes.iter().any(|c| c == wanted), "{wanted} in {codes:?}");
    }
}

#[test]
fn a_root_total_sums_a_many_block_and_a_double_operand_is_accepted() {
    let mut p = dashboard();
    let svc = &mut p.services[0];
    svc.graphql_sources[1]
        .fields
        .push(sb_model::GraphqlSourceField {
            name: "cout_unitaire".into(),
            ty: ColumnType::Double,
            nullable: false,
            values: Default::default(),
        });
    svc.graphql_roots[0].blocks.push(sb_model::GraphqlBlock {
        field: "mouvements".into(),
        source: "Mouvement".into(),
        many: true,
        params: Default::default(),
        fixed_params: Default::default(),
        path: None,
    });
    svc.graphql_roots[0].sums = vec![sb_model::GraphqlRootSum {
        name: "valeurTotale".into(),
        block: "mouvements".into(),
        field: "quantite".into(),
        times: Some("cout_unitaire".into()),
    }];
    let errors: Vec<_> = sb_model::validate(&p)
        .into_iter()
        .filter(|d| d.severity == sb_model::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let files = generate(&p).unwrap();
    let sdl = sdl_file(&files).expect("sdl written");
    assert!(sdl.contains("  valeurTotale: Long"), "{sdl}");
    let code = find(&files, "services/orders/src/transport/graphql.rs").expect("endpoint");
    assert!(
        code.contains("RootSum { field: \"valeurTotale\", block: \"mouvements\", value: \"quantite\", times: Some(\"cout_unitaire\") }"),
        "{code}"
    );
    // A total over a block that is not a list, or over a non-numeric field, is refused.
    let mut bad = p.clone();
    bad.services[0].graphql_roots[0].sums[0].block = "premierMouvement".into();
    bad.services[0].graphql_roots[0].sums[0].field = "id".into();
    let codes: Vec<String> = sb_model::validate(&bad)
        .into_iter()
        .map(|d| d.code)
        .collect();
    assert!(codes.iter().any(|c| c == "E_GRAPHQL_ROOT_SUM"), "{codes:?}");
}

#[test]
fn a_field_maps_its_values_to_an_enumeration_and_a_date_gives_the_days_left() {
    use sb_model::{GraphqlComputed, GraphqlComputedKind};
    let mut p = dashboard();
    let article = &mut p.services[0].graphql_sources[0];
    article.fields.push(sb_model::GraphqlSourceField {
        name: "categorie".into(),
        ty: ColumnType::String,
        nullable: true,
        values: [("sortie", "SORTIE"), ("entree", "ENTREE")]
            .into_iter()
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
            .collect(),
    });
    article.fields.push(sb_model::GraphqlSourceField {
        name: "dlc".into(),
        ty: ColumnType::String,
        nullable: true,
        values: Default::default(),
    });
    article.computed.push(GraphqlComputed {
        name: "joursRestants".into(),
        kind: GraphqlComputedKind::DaysUntil {
            field: "dlc".into(),
        },
    });
    let errors: Vec<_> = sb_model::validate(&p)
        .into_iter()
        .filter(|d| d.severity == sb_model::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let files = generate(&p).unwrap();
    let sdl = sdl_file(&files).expect("sdl written");
    assert!(
        sdl.contains("enum ArticleCategorie {\n  ENTREE\n  SORTIE\n}"),
        "{sdl}"
    );
    assert!(sdl.contains("  categorie: ArticleCategorie\n"), "{sdl}");
    assert!(sdl.contains("  joursRestants: Long\n"), "{sdl}");
    let code = find(&files, "services/orders/src/transport/graphql.rs").expect("endpoint");
    for wanted in [
        "scalar: \"ArticleCategorie\", nullable: true, values: &[(\"entree\", \"ENTREE\"), (\"sortie\", \"SORTIE\")] }",
        "Computed { field: \"joursRestants\", kind: Kind::DaysUntil(\"dlc\") }",
    ] {
        assert!(code.contains(wanted), "{wanted} in {code}");
    }
    let again = generate(&p).unwrap();
    assert_eq!(files, again);
    // Refused: a bad enumeration name, a date field that does not exist.
    let mut bad = p.clone();
    bad.services[0].graphql_sources[0].fields[1]
        .values
        .insert("x".into(), "minuscule".into());
    bad.services[0].graphql_sources[0].computed[0].kind = GraphqlComputedKind::DaysUntil {
        field: "absent".into(),
    };
    let codes: Vec<String> = sb_model::validate(&bad)
        .into_iter()
        .map(|d| d.code)
        .collect();
    for wanted in ["E_GRAPHQL_SOURCE_VALUES", "E_GRAPHQL_COMPUTED"] {
        assert!(codes.iter().any(|c| c == wanted), "{wanted} in {codes:?}");
    }
}
