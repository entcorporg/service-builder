//! Tests of the journal of schema changes: old names, dropped names, type and nullability changes.

use crate::project::*;
use crate::sample;
use crate::validate::validate;

fn diagnostics(p: &Project) -> Vec<(String, String)> {
    validate(p).into_iter().map(|d| (d.code, d.path)).collect()
}

fn has(p: &Project, code: &str, path: &str) -> bool {
    diagnostics(p).contains(&(code.to_string(), path.to_string()))
}

fn codes(p: &Project) -> Vec<String> {
    diagnostics(p).into_iter().map(|d| d.0).collect()
}

const COL1: &str = "/services/0/tables/0/columns/1";

#[test]
fn a_project_without_a_journal_serializes_without_it() {
    let p = sample::orders();
    let json = serde_json::to_string(&p).unwrap();
    for key in [
        "renamedFrom",
        "retypedFrom",
        "nullableFrom",
        "droppedColumns",
        "droppedTables",
    ] {
        assert!(!json.contains(key), "{key}");
    }
    let back: Project = serde_json::from_str(&json).unwrap();
    assert_eq!(back, p);
}

#[test]
fn the_showcase_journal_is_valid() {
    let p = sample::showcase();
    assert_eq!(p.services[0].tables[0].columns[1].renamed_from, ["total"]);
    assert!(validate(&p).is_empty(), "{:?}", validate(&p));
}

#[test]
fn old_names_and_dropped_names_must_be_unambiguous() {
    let mut p = sample::orders();
    p.services[0].tables[0].columns[1].renamed_from = vec!["status".into()];
    assert!(has(&p, "E_RENAMED_FROM", &format!("{COL1}/renamedFrom/0")));
    p.services[0].tables[0].columns[1].renamed_from = vec!["Bad Name".into()];
    assert!(has(&p, "E_RENAMED_FROM", &format!("{COL1}/renamedFrom/0")));
    p.services[0].tables[0].columns[1].renamed_from = vec!["old".into()];
    p.services[0].tables[0].columns[2].renamed_from = vec!["old".into()];
    assert!(has(&p, "E_RENAMED_FROM", &format!("{COL1}/renamedFrom/0")));
    p.services[0].tables[0].columns[2].renamed_from = vec![];
    p.services[0].tables[0].dropped_columns = vec!["old".into(), "amount".into()];
    assert!(has(&p, "E_DROPPED_NAME", "/services/0/tables/0/droppedColumns/0"));
    assert!(has(&p, "E_DROPPED_NAME", "/services/0/tables/0/droppedColumns/1"));
    p.services[0].tables[0].dropped_columns.clear();
    p.services[0].tables[0].renamed_from = vec!["orders".into()];
    assert!(has(&p, "E_RENAMED_FROM", "/services/0/tables/0/renamedFrom/0"));
    p.services[0].tables[0].renamed_from = vec!["old_orders".into()];
    p.services[0].dropped_tables = vec!["old_orders".into(), "orders".into()];
    assert!(has(&p, "E_RENAMED_FROM", "/services/0/tables/0/renamedFrom/0"));
    assert!(has(&p, "E_DROPPED_NAME", "/services/0/droppedTables/1"));
}

#[test]
fn a_type_change_needs_a_sound_cast() {
    let mut p = sample::orders();
    let at = format!("{COL1}/retypedFrom");
    p.services[0].tables[0].columns[1].retyped_from = Some(ColumnType::Uuid);
    assert!(has(&p, "E_RETYPE_UNSUPPORTED", &at));
    p.services[0].tables[0].columns[1].retyped_from = Some(ColumnType::Int64);
    assert!(has(&p, "E_RETYPE_UNSUPPORTED", &at));
    p.services[0].tables[0].columns[1].retyped_from = Some(ColumnType::Double);
    assert!(has(&p, "W_RETYPE_LOSSY", &at));
    p.services[0].tables[0].columns[1].retyped_from = Some(ColumnType::Int32);
    assert!(
        !codes(&p)
            .iter()
            .any(|c| c.starts_with("E_RETYPE") || c == "W_RETYPE_LOSSY")
    );
}

#[test]
fn a_type_change_is_refused_in_a_foreign_key() {
    let mut p = sample::showcase();
    let svc = p
        .services
        .iter_mut()
        .find(|s| s.tables.iter().any(|t| !t.foreign_keys.is_empty()))
        .unwrap();
    let fk = svc
        .tables
        .iter()
        .find_map(|t| t.foreign_keys.first().cloned())
        .unwrap();
    let target = svc.tables.iter_mut().find(|t| t.name == fk.ref_table).unwrap();
    let col = target
        .columns
        .iter_mut()
        .find(|c| c.name == fk.ref_columns[0])
        .unwrap();
    col.retyped_from = Some(ColumnType::String);
    assert!(codes(&p).contains(&"E_RETYPE_FK".to_string()));
}

#[test]
fn becoming_required_without_a_default_warns() {
    let mut p = sample::orders();
    p.services[0].tables[0].columns[1].nullable_from = Some(true);
    assert!(has(&p, "W_REQUIRED_NO_DEFAULT", &format!("{COL1}/nullableFrom")));
    p.services[0].tables[0].columns[1].default = Some("0".into());
    assert!(!codes(&p).contains(&"W_REQUIRED_NO_DEFAULT".to_string()));
}

#[test]
fn the_conversions_are_the_sound_ones() {
    use ColumnType::*;
    assert!(Int32.converts_to(Int64) && Int64.converts_to(Int32));
    assert!(Bool.converts_to(Int32) && !Bool.converts_to(Int64));
    assert!(Uuid.converts_to(String) && String.converts_to(Uuid));
    assert!(!Uuid.converts_to(Int32) && !Timestamp.converts_to(Bool) && !Jsonb.converts_to(Int64));
    assert!(!String.converts_to(String));
}
