//! Saved views survive a round trip, in both [`Store`] implementations, and agree.
//!
//! Asked of both and compared, for the reason `drafts.rs` gives: the parity proptest only sees
//! what a filter can ask, and a view is not something a filter can ask about.
//!
//! A view's row is its serde form whole (migration 0024), so the row is pinned by a frozen
//! fixture here beside the domain's own `view.json`: `tests/fixtures/view_row.json` is what
//! this build writes into `views.view`, and a later build must still read it.

use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store, StoreError};

struct Both {
    sqlite: SqliteStore,
    memory: MemoryStore,
    _dir: tempfile::TempDir,
}

fn both() -> Both {
    let dir = tempfile::tempdir().unwrap();
    Both {
        sqlite: SqliteStore::in_memory(dir.path()).unwrap(),
        memory: MemoryStore::new(),
        _dir: dir,
    }
}

const FIXTURE_ID: ViewId = ViewId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1"));
const LABEL: LabelId = LabelId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d1"));

/// A view with every field away from its simplest value, so the round trip proves each.
fn view(name: &str) -> View {
    View {
        id: ViewId::generate(),
        name: name.to_owned(),
        kind: ViewKind::Query,
        filter: Filter::And(vec![
            Filter::Read(ReadState::Unread),
            Filter::From(TextMatch::Contains("billing".to_owned())),
            Filter::HasLabel(LABEL),
        ]),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Asc,
        },
        group_by: Some(GroupKey::Label(LABEL)),
        threading: Threading::Threaded,
        shown: vec![Property::From, Property::Subject],
        hover: vec![OpKind::Archive, OpKind::Star, OpKind::Snooze],
    }
}

fn put(b: &Both, view: &View) {
    b.sqlite.put_view(view).unwrap();
    b.memory.put_view(view).unwrap();
}

fn names(store: &dyn Store) -> Vec<String> {
    store
        .views()
        .unwrap()
        .into_iter()
        .map(|view| view.name)
        .collect()
}

#[test]
fn a_kept_view_reads_back_whole_from_both_stores() {
    let b = both();
    let kept = view("Bills");
    put(&b, &kept);
    assert_eq!(b.sqlite.views().unwrap(), vec![kept.clone()]);
    assert_eq!(b.memory.views().unwrap(), vec![kept]);
}

#[test]
fn views_list_in_the_order_they_were_first_kept_in_both_stores() {
    let b = both();
    // Not alphabetical, so an ORDER BY name would show.
    for name in ["Receipts", "Alerts", "Newsletters"] {
        put(&b, &view(name));
    }
    let want = ["Receipts", "Alerts", "Newsletters"];
    assert_eq!(names(&b.sqlite), want);
    assert_eq!(names(&b.memory), want);
}

#[test]
fn keeping_a_view_again_changes_it_in_place_in_both_stores() {
    let b = both();
    let first = view("Receipts");
    put(&b, &first);
    put(&b, &view("Alerts"));
    let mut edited = first.clone();
    edited.name = "Paid".to_owned();
    edited.group_by = Some(GroupKey::Read);
    edited.hover = vec![OpKind::Trash];
    put(&b, &edited);
    for store in [&b.sqlite as &dyn Store, &b.memory] {
        let views = store.views().unwrap();
        assert_eq!(views.len(), 2, "an edit is not a second view");
        assert_eq!(views[0], edited, "an edited view keeps its place");
        assert_eq!(views[1].name, "Alerts");
    }
}

#[test]
fn a_deleted_view_is_gone_and_a_second_delete_says_so_in_both_stores() {
    let b = both();
    let gone = view("Receipts");
    put(&b, &gone);
    put(&b, &view("Alerts"));
    for store in [&b.sqlite as &dyn Store, &b.memory] {
        store.delete_view(gone.id).unwrap();
        assert_eq!(names(store), ["Alerts"]);
        match store.delete_view(gone.id) {
            Err(StoreError::NoView(id)) => assert_eq!(id, gone.id),
            other => panic!("a second delete should be NoView, got {other:?}"),
        }
    }
}

#[test]
fn a_view_kept_after_a_delete_still_goes_last_in_both_stores() {
    let b = both();
    let first = view("Receipts");
    put(&b, &first);
    put(&b, &view("Alerts"));
    b.sqlite.delete_view(first.id).unwrap();
    b.memory.delete_view(first.id).unwrap();
    put(&b, &view("Newsletters"));
    put(&b, &view("Bills"));
    let want = ["Alerts", "Newsletters", "Bills"];
    assert_eq!(names(&b.sqlite), want);
    assert_eq!(names(&b.memory), want);
}

/// The view `tests/fixtures/view_row.json` was written from.
fn fixture_view() -> View {
    View {
        id: FIXTURE_ID,
        name: "Unread from billing".to_owned(),
        kind: ViewKind::Query,
        filter: Filter::And(vec![
            Filter::Read(ReadState::Unread),
            Filter::From(TextMatch::Contains("billing".to_owned())),
        ]),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        group_by: Some(GroupKey::Star),
        threading: Threading::Threaded,
        shown: Vec::new(),
        hover: vec![OpKind::Archive, OpKind::MarkRead, OpKind::Star],
    }
}

fn fixture() -> &'static str {
    include_str!("fixtures/view_row.json").trim_end()
}

#[test]
fn a_view_is_written_as_its_frozen_row() {
    // What this build writes. If this fails, the row's schema changed: that is a migration and
    // a new fixture beside this one, never an edit to this one.
    let b = both();
    b.sqlite.put_view(&fixture_view()).unwrap();
    let stored: String = b
        .sqlite
        .connection()
        .query_row("SELECT view FROM views", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored, fixture());
}

#[test]
fn the_frozen_rows_still_read_back() {
    // This package's row, and the domain's own frozen `View`, each put in a row as an earlier
    // build would have left it.
    let domain = include_str!("../../mail-domain/tests/fixtures/view.json");
    let b = both();
    for (position, (id, text)) in [
        (FIXTURE_ID.to_string(), fixture()),
        ("0b0b0b0b-0b0b-0b0b-0b0b-0b0b0b0b0b0b".to_owned(), domain),
    ]
    .into_iter()
    .enumerate()
    {
        b.sqlite
            .connection()
            .execute(
                "INSERT INTO views (id, view, position) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, text, position as i64],
            )
            .unwrap();
    }
    let views = b.sqlite.views().expect("a frozen row must still decode");
    assert_eq!(views[0], fixture_view());
    assert_eq!(views[1].name, "Unread receipts");
    assert_eq!(
        views[1].group_by,
        Some(GroupKey::Property(Property::From)),
        "the domain fixture's grouping"
    );
}
