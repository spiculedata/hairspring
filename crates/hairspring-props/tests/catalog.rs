//! The run catalog: how ids get indexes, and what a server accepts when it rebuilds one from updates.
use hairspring_props::*;

fn row(id: &str, kind: AssertKind, decided: bool) -> CatalogRow {
    CatalogRow {
        id: id.to_string(),
        kind,
        msg: "m".to_string(),
        loc: None,
        decided,
    }
}

fn update(idx: u32, r: CatalogRow) -> CatalogUpdate {
    CatalogUpdate { idx, row: r }
}

#[test]
fn updates_extend_the_catalog_one_index_at_a_time() {
    let c = RunCatalog::new()
        .apply(&update(0, row("a.b", AssertKind::Always, true)), 10)
        .unwrap()
        .apply(&update(1, row("a.c", AssertKind::Sometimes, true)), 10)
        .unwrap();
    assert_eq!(c.len(), 2);
    assert_eq!(c.get(1).unwrap().id, "a.c");
    assert_eq!(c.index_of("a.b"), Some(0));
}

#[test]
fn a_gap_in_the_indexes_is_refused() {
    let r = RunCatalog::new().apply(&update(1, row("a.b", AssertKind::Always, true)), 10);
    assert!(r.is_err());
}

#[test]
fn a_duplicate_id_is_refused() {
    let c = RunCatalog::new()
        .apply(&update(0, row("a.b", AssertKind::Always, true)), 10)
        .unwrap();
    assert!(c
        .apply(&update(1, row("a.b", AssertKind::Always, true)), 10)
        .is_err());
}

#[test]
fn the_cap_is_enforced() {
    let c = RunCatalog::new()
        .apply(&update(0, row("a.b", AssertKind::Always, true)), 1)
        .unwrap();
    assert!(c
        .apply(&update(1, row("a.c", AssertKind::Always, true)), 1)
        .is_err());
}

#[test]
fn only_an_undecided_row_can_be_decided_and_only_once() {
    let c = RunCatalog::new()
        .apply(&update(0, row("a.b", AssertKind::Always, false)), 10)
        .unwrap();
    let c = c
        .apply(&update(0, row("a.b", AssertKind::Sometimes, true)), 10)
        .unwrap();
    assert_eq!(c.get(0).unwrap().kind, AssertKind::Sometimes);
    assert!(c
        .apply(&update(0, row("a.b", AssertKind::Always, true)), 10)
        .is_err());
}

#[test]
fn a_decision_cannot_rename_the_id_or_stay_undecided() {
    let c = RunCatalog::new()
        .apply(&update(0, row("a.b", AssertKind::Always, false)), 10)
        .unwrap();
    assert!(c
        .clone()
        .apply(&update(0, row("a.other", AssertKind::Always, true)), 10)
        .is_err());
    assert!(c
        .apply(&update(0, row("a.b", AssertKind::Always, false)), 10)
        .is_err());
}

#[test]
fn hostile_rows_are_refused() {
    let bad_ids = [
        "".to_string(),
        "has space".to_string(),
        "x".repeat(129),
        "bidi\u{202e}".to_string(),
    ];
    for id in bad_ids {
        assert!(
            RunCatalog::new()
                .apply(&update(0, row(&id, AssertKind::Always, true)), 10)
                .is_err(),
            "{id:?}"
        );
    }
    let mut r = row("a.b", AssertKind::Always, true);
    r.msg = "x\u{202e}y".to_string();
    assert!(RunCatalog::new().apply(&update(0, r), 10).is_err());
    let mut r = row("a.b", AssertKind::Always, true);
    r.loc = Some(Loc {
        file: "f\n".into(),
        line: 1,
        func: None,
    });
    assert!(RunCatalog::new().apply(&update(0, r), 10).is_err());
}

#[test]
fn a_catalog_loaded_from_rows_must_have_unique_ids() {
    let rows = vec![
        row("a.b", AssertKind::Always, true),
        row("a.b", AssertKind::Sometimes, true),
    ];
    assert!(RunCatalog::from_rows(rows).is_err());
}
