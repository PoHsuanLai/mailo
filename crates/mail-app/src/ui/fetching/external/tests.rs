use super::*;
use crate::ui::fixtures::dispatching;
use dioxus::dioxus_core::VirtualDom;
use mail_core::ipc::wire::{self, Response};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// Just the revision the window would keep, moved by [`use_external_changes`].
#[component]
fn Looking() -> Element {
    let revision = use_signal(|| 0u64);
    use_context_provider(|| revision);
    use_external_changes(
        revision,
        consume_context::<Arc<SqliteStore>>(),
        consume_context::<Doors>(),
    );
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

/// A door in `dir`, never the person's, as this platform makes one: a socket in `dir`, or on
/// Windows a named pipe with its lock in `dir`.
fn scratch(dir: &Path, name: &str) -> latchkey::Agent {
    // The test directory's own name keeps two runs' pipes apart: a pipe's namespace is the
    // machine's, not the directory's.
    let unique = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("test");
    let environment = latchkey::Environment {
        runtime_dir: Some(dir.as_os_str()),
        tmpdir: Some(dir.as_os_str()),
        home: None,
        local_app_data: Some(dir.as_os_str()),
        user: Some(unique),
    };
    latchkey::Agent::in_environment(name, latchkey::here(), &environment).unwrap()
}

/// Knock on `agent`, as [`Doors::server`] knocks on the watch's.
fn knocking(agent: latchkey::Agent) -> Doors {
    Doors(Arc::new(move || {
        // As the watch's door is: told of each pass, so looking slows down.
        let changes = mail_core::ipc::client::subscribe(&agent).ok().flatten()?;
        Some((changes, TOLD))
    }))
}

/// A daemon that answers every knock with `answer` and keeps the connection, until `hang_up`
/// is sent to, when it closes the connection and its door.
fn door(agent: &latchkey::Agent, answer: String) -> std::sync::mpsc::Sender<()> {
    let listening = agent.listen().unwrap();
    let (hang_up, told) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut stream = listening.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&mut stream).read_line(&mut line).unwrap();
        stream.write_all(answer.as_bytes()).unwrap();
        stream.flush().unwrap();
        let _ = told.recv();
        drop(stream);
        drop(listening);
    });
    hang_up
}

#[tokio::test]
async fn what_another_connection_stores_moves_the_revision_and_what_the_window_stores_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let store = Arc::new(SqliteStore::open(&path, dir.path()).unwrap());
    dispatching();
    let mut dom = VirtualDom::new(Looking)
        .with_root_context(store.clone())
        .with_root_context(Doors::server());
    dom.rebuild_in_place();
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 0, "it moved on its own");

    insert_here(&store, "the-window");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 0, "the window's own write moved it");

    insert_elsewhere(&path, "the-watch");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 1, "a write by another process went unseen");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 1, "and is seen once");
}

#[tokio::test]
async fn told_while_the_writer_is_busy_it_looks_again_soon_and_not_at_the_next_told() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let store = Arc::new(SqliteStore::open(&path, dir.path()).unwrap());
    let agent = scratch(dir.path(), "mailo-watch-busy");
    let watching = mail_core::ipc::watching::claim_at(&agent).unwrap();
    dispatching();
    let mut dom = VirtualDom::new(Looking)
        .with_root_context(store.clone())
        .with_root_context(knocking(agent));
    dom.rebuild_in_place();
    for _ in 0..100 {
        if watching.listeners() == 1 {
            break;
        }
        step(&mut dom).await;
    }
    assert_eq!(watching.listeners(), 1, "the window never subscribed");
    settle(&mut dom).await;

    // The writer held by another thread, as an ingest holds it, while the watch says it stored.
    insert_elsewhere(&path, "the-watch");
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
    watching.changed(mail_domain::id::new_account_id());
    step(&mut dom).await;
    assert_eq!(
        revision(&dom),
        0,
        "it read the data version past a held writer"
    );

    // Let go: the look it owes comes within a few looks, far inside a `TOLD`.
    release.send(()).unwrap();
    writer.join().unwrap();
    settle(&mut dom).await;
    assert_eq!(
        revision(&dom),
        1,
        "what it was told about waited for the next TOLD"
    );
}

#[tokio::test]
async fn a_watch_that_says_it_stored_something_moves_the_revision_within_one_look() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let store = Arc::new(SqliteStore::open(&path, dir.path()).unwrap());
    let agent = scratch(dir.path(), "mailo-watch-told");
    let watching = mail_core::ipc::watching::claim_at(&agent).unwrap();
    dispatching();
    let mut dom = VirtualDom::new(Looking)
        .with_root_context(store.clone())
        .with_root_context(knocking(agent));
    dom.rebuild_in_place();
    for _ in 0..100 {
        if watching.listeners() == 1 {
            break;
        }
        step(&mut dom).await;
    }
    assert_eq!(watching.listeners(), 1, "the window never subscribed");
    settle(&mut dom).await;

    // Stored, and not said: while subscribed the window looks only every `TOLD`, far longer
    // than this, so a move now would be the looking and not the telling.
    insert_elsewhere(&path, "the-watch");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 0, "subscribed, it still looked every LOOK");

    watching.changed(mail_domain::id::new_account_id());
    step(&mut dom).await;
    assert_eq!(revision(&dom), 1, "told, it did not look");
    // Told of a pass that stored nothing new: the data version decides, so nothing moves.
    watching.changed(mail_domain::id::new_account_id());
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 1, "one commit counted twice");
}

#[tokio::test]
async fn a_subscription_that_ends_leaves_the_window_looking_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let store = Arc::new(SqliteStore::open(&path, dir.path()).unwrap());
    let agent = scratch(dir.path(), "mailo-watch-gone");
    let hang_up = door(&agent, wire::line(Response::Subscribed).unwrap());
    dispatching();
    let mut dom = VirtualDom::new(Looking)
        .with_root_context(store.clone())
        .with_root_context(knocking(agent));
    dom.rebuild_in_place();
    settle(&mut dom).await;

    insert_elsewhere(&path, "while-subscribed");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 0, "subscribed, it still looked every LOOK");

    // The watch stops: its connection closes and its door goes with it.
    hang_up.send(()).unwrap();
    settle(&mut dom).await;
    assert_eq!(
        revision(&dom),
        1,
        "the end of the subscription was not a reason to look"
    );

    insert_elsewhere(&path, "after");
    settle(&mut dom).await;
    assert_eq!(
        revision(&dom),
        2,
        "with no door it did not go back to looking"
    );
}

#[tokio::test]
async fn a_daemon_from_another_build_leaves_the_window_looking() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let store = Arc::new(SqliteStore::open(&path, dir.path()).unwrap());
    let agent = scratch(dir.path(), "mailo-watch-old");
    // What a daemon from before subscriptions says: its own version, and a request it cannot
    // read.
    let old = format!(
        r#"{{"version":{},"body":{{"refused":"unreadable message: unknown variant `subscribe`"}}}}"#,
        mail_core::ipc::wire::VERSION
    );
    let _hang_up = door(&agent, format!("{old}\n"));
    dispatching();
    let mut dom = VirtualDom::new(Looking)
        .with_root_context(store.clone())
        .with_root_context(knocking(agent));
    dom.rebuild_in_place();
    settle(&mut dom).await;

    insert_elsewhere(&path, "the-old-watch");
    settle(&mut dom).await;
    assert_eq!(
        revision(&dom),
        1,
        "a refused subscription stopped the looking"
    );
}
