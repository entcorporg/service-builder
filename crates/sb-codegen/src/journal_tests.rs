//! Tests of the statements generated from the journal of schema changes.

use sb_model::{Column, ColumnType, Project, sample};

use crate::generate;

fn schema(p: &Project) -> String {
    generate(p)
        .unwrap()
        .into_iter()
        .find(|f| f.path == "services/orders/src/app/schema.rs")
        .expect("schema.rs")
        .content
}

fn at(s: &str, needle: &str) -> usize {
    s.find(needle)
        .unwrap_or_else(|| panic!("missing {needle}"))
}

#[test]
fn without_a_journal_the_schema_is_the_additive_one() {
    let s = schema(&sample::orders());
    for word in ["RENAME", "DROP COLUMN", "DROP TABLE", "DROP DEFAULT", "NOT NULL;"] {
        assert!(!s.contains(word), "{word}");
    }
}

#[test]
fn the_journal_is_replayed_in_a_safe_order() {
    let s = schema(&sample::showcase());
    let rename = at(&s, "RENAME COLUMN");
    let create = at(&s, "CREATE TABLE IF NOT EXISTS");
    let retype = at(&s, "TYPE BIGINT USING");
    let drop_col = at(&s, "DROP COLUMN IF EXISTS");
    let drop_table = at(&s, "DROP TABLE IF EXISTS");
    let constraint = s.find("ADD CONSTRAINT").unwrap_or(usize::MAX);
    assert!(rename < create, "renames come before creations");
    assert!(create < retype && retype < drop_col && drop_col < drop_table);
    assert!(drop_table < constraint, "drops come before the constraints");
    assert!(s.contains("RENAME COLUMN \\\"total\\\" TO \\\"amount\\\""));
    assert!(s.contains("\\\"legacy_flag\\\" CASCADE"));
    assert!(s.contains("DROP TABLE IF EXISTS \\\"order_drafts\\\" CASCADE"));
}

#[test]
fn a_rename_chain_replays_every_step_oldest_first() {
    let mut p = sample::orders();
    p.services[0].tables[0].columns[1].renamed_from = vec!["a".into(), "b".into()];
    p.services[0].tables[0].renamed_from = vec!["old_orders".into()];
    let s = schema(&p);
    let ab = at(&s, "RENAME COLUMN \\\"a\\\" TO \\\"b\\\"");
    let bc = at(&s, "RENAME COLUMN \\\"b\\\" TO \\\"amount\\\"");
    assert!(ab < bc);
    assert!(
        at(&s, "RENAME TO \\\"orders\\\"") < ab,
        "tables are renamed before their columns"
    );
}

#[test]
fn nullability_changes_fill_nulls_with_the_default_first() {
    let mut p = sample::orders();
    let amount = &mut p.services[0].tables[0].columns[1];
    amount.default = Some("0".into());
    amount.nullable_from = Some(true);
    let s = schema(&p);
    assert!(at(&s, "SET \\\"amount\\\" = 0 WHERE") < at(&s, "SET NOT NULL"));
    let mut p = sample::orders();
    let mut memo = Column::new("memo", ColumnType::String);
    memo.nullable = true;
    memo.nullable_from = Some(false);
    p.services[0].tables[0].columns.push(memo);
    assert!(schema(&p).contains("DROP NOT NULL"));
}

#[test]
fn an_unsound_conversion_is_refused_before_generating() {
    let mut p = sample::orders();
    p.services[0].tables[0].columns[1].retyped_from = Some(ColumnType::Uuid);
    assert!(generate(&p).is_err());
}
