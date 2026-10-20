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

/// One look's worth of time, and whatever it woke.
async fn step(dom: &mut VirtualDom) {
    tokio::time::sleep(LOOK).await;
    let _ = tokio::time::timeout(Duration::from_millis(5), dom.wait_for_work()).await;
    dom.render_immediate(&mut dioxus_core::NoOpMutations);
}

async fn settle(dom: &mut VirtualDom) {
    for _ in 0..20 {
        step(dom).await;
    }
}

/// An account row written through the window's own store.
fn insert_here(store: &SqliteStore, name: &str) {
    mail_store::testing::seed_account(store, mail_domain::id::new_account_id(), name);
}

/// An account row written by its own connection, which is what another process's write is to
/// the window's.
fn insert_elsewhere(path: &Path, name: &str) {
    mail_store::testing::seed_account_from_another_connection(path, name);
}

/// A window on a store of its own in a scratch directory, with the revision at rest.
async fn window(path: &Path, dir: &Path) -> (Arc<SqliteStore>, VirtualDom) {
    let store = Arc::new(SqliteStore::open(path, dir).unwrap());
    dispatching();
    let mut dom = VirtualDom::new(Looking).with_root_context(store.clone());
    dom.rebuild_in_place();
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 0, "it moved on its own");
    (store, dom)
}

/// Heard, not looked for: in a test the window looks only every [`STEADY`], a minute, so a move
/// within a few looks is the system saying the file was written to.
#[tokio::test]
async fn what_another_connection_stores_moves_the_revision_and_what_the_window_stores_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let (store, mut dom) = window(&path, dir.path()).await;

    insert_here(&store, "the-window");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 0, "the window's own write moved it");

    insert_elsewhere(&path, "the-watch");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 1, "a write by another process went unheard");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 1, "and is counted once");
}

#[tokio::test]
async fn a_write_heard_with_nothing_committed_moves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let (_store, mut dom) = window(&path, dir.path()).await;

    // The system says the database was written to, and nobody committed: the data version, not
    // the word, decides.
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::SystemTime::now())
        .unwrap();
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 0, "nothing was committed, and it moved");
}

#[tokio::test]
async fn heard_while_the_writer_is_busy_it_looks_again_soon_and_not_at_the_next_steady() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let (store, mut dom) = window(&path, dir.path()).await;

    // The writer held by another thread, as an ingest holds it, while another process commits.
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (taken, took) = std::sync::mpsc::channel::<()>();
    let writer = {
        let store = store.clone();
        std::thread::spawn(move || {
            let _writer = mail_store::testing::hold_writer(&store);
            taken.send(()).unwrap();
            let _ = held.recv();
        })
    };
    took.recv().unwrap();
    insert_elsewhere(&path, "the-watch");
    settle(&mut dom).await;
    assert_eq!(
        revision(&dom),
        0,
        "it read the data version past a held writer"
    );

    // Let go: the look it owes comes within a few looks, far inside a `STEADY`.
    release.send(()).unwrap();
    writer.join().unwrap();
    settle(&mut dom).await;
    assert_eq!(
        revision(&dom),
        1,
        "what it heard waited for the next STEADY"
    );
}

#[tokio::test]
async fn a_store_that_cannot_be_watched_is_looked_at_instead() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        watch(&dir.path().join("gone").join("mail.db")).is_none(),
        "a directory that is not there was watched"
    );
    let mut written = None;
    assert!(
        !next(&mut written, LOOK).await,
        "with nothing watched, a look is all there is"
    );
    // And a watch that is there hears a write to the `-wal` as one to the database.
    let path = dir.path().join("mail.db");
    let mut written = watch(&path);
    assert!(
        written.is_some(),
        "a scratch directory could not be watched"
    );
    std::fs::write(dir.path().join("mail.db-wal"), b"x").unwrap();
    assert!(
        next(&mut written, Duration::from_secs(5)).await,
        "a write to the -wal went unheard"
    );
}
