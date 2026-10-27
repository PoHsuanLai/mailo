use super::*;
use crate::ui::fixtures::dispatching;
use dioxus::dioxus_core::VirtualDom;
use mail_core::SqliteStore;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Just the revision the window would keep, moved by [`use_external_changes`].
#[component]
fn Looking() -> Element {
    let revision = use_signal(|| 0u64);
    use_context_provider(|| revision);
    use_external_changes(revision);
    rsx! {}
}

fn revision(dom: &VirtualDom) -> u64 {
    dom.in_scope(ScopeId::APP, || *consume_context::<Signal<u64>>().peek())
}

/// A moment's time, and whatever it woke.
async fn settle(dom: &mut VirtualDom) {
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = tokio::time::timeout(Duration::from_millis(5), dom.wait_for_work()).await;
        dom.render_immediate(&mut dioxus_core::NoOpMutations);
    }
}

/// A window on a store of its own in a scratch directory, with the client the program gives
/// every window, and the revision at rest.
async fn window(path: &Path, dir: &Path) -> (Arc<SqliteStore>, VirtualDom) {
    let store = Arc::new(SqliteStore::open(path, dir).unwrap());
    dispatching();
    let client = mail_client::Client::open(store.clone(), mail_client::ClientConfig::default());
    let mut dom = VirtualDom::new(Looking)
        .with_root_context(store.clone())
        .with_root_context(client);
    dom.rebuild_in_place();
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 0, "it moved on its own");
    (store, dom)
}

/// A commit through the window's own store — what the background watch makes, beside the
/// window — and one by another process both move the revision, each once.
#[tokio::test]
async fn a_commit_here_or_by_another_process_moves_the_revision_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let (store, mut dom) = window(&path, dir.path()).await;

    mail_store::testing::seed_account(&store, mail_domain::id::new_account_id(), "here");
    settle(&mut dom).await;
    assert_eq!(
        revision(&dom),
        1,
        "a commit through the window's store went unheard"
    );

    mail_store::testing::seed_account_from_another_connection(&path, "the-watch");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 2, "a write by another process went unheard");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 2, "and is counted once");
}
