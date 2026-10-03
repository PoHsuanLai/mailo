use super::*;
use crate::ui::fixtures::dispatching;
use dioxus::dioxus_core::VirtualDom;

/// Just the revision the window would keep, moved by [`use_external_changes`].
#[component]
fn Looking() -> Element {
    let revision = use_signal(|| 0u64);
    use_context_provider(|| revision);
    use_external_changes(revision, consume_context::<Arc<SqliteStore>>());
    rsx! {}
}

fn revision(dom: &VirtualDom) -> u64 {
    dom.in_scope(ScopeId::APP, || *consume_context::<Signal<u64>>().peek())
}

async fn settle(dom: &mut VirtualDom) {
    for _ in 0..10 {
        tokio::time::sleep(LOOK * 2).await;
        let _ = tokio::time::timeout(Duration::from_millis(5), dom.wait_for_work()).await;
        dom.render_immediate(&mut dioxus_core::NoOpMutations);
    }
}

#[tokio::test]
async fn what_another_connection_stores_moves_the_revision_and_what_the_window_stores_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let store = Arc::new(SqliteStore::open(&path, dir.path()).unwrap());
    dispatching();
    let mut dom = VirtualDom::new(Looking).with_root_context(store.clone());
    dom.rebuild_in_place();
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 0, "it moved on its own");

    let insert = |connection: &rusqlite::Connection, id: &str| {
        connection
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, ?1, '{}', datetime('now'))",
                [id],
            )
            .unwrap();
    };
    insert(&store.connection(), "the-window");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 0, "the window's own write moved it");

    insert(&rusqlite::Connection::open(&path).unwrap(), "the-watch");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 1, "a write by another process went unseen");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 1, "and is seen once");
}
