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

/// An account row written by its own connection, which is what another process's write is to
/// the window's.
fn insert(connection: &rusqlite::Connection, id: &str) {
    connection
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, ?1, '{}', datetime('now'))",
            [id],
        )
        .unwrap();
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
        mail_core::ipc::client::subscribe(&agent).ok().flatten()
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

    insert(&store.connection(), "the-window");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 0, "the window's own write moved it");

    insert(&rusqlite::Connection::open(&path).unwrap(), "the-watch");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 1, "a write by another process went unseen");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 1, "and is seen once");
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
    insert(&rusqlite::Connection::open(&path).unwrap(), "the-watch");
    settle(&mut dom).await;
    assert_eq!(revision(&dom), 0, "subscribed, it still looked every LOOK");

    watching.changed(mail_domain::AccountId::generate());
    step(&mut dom).await;
    assert_eq!(revision(&dom), 1, "told, it did not look");
    // Told of a pass that stored nothing new: the data version decides, so nothing moves.
    watching.changed(mail_domain::AccountId::generate());
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

    insert(
        &rusqlite::Connection::open(&path).unwrap(),
        "while-subscribed",
    );
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

    insert(&rusqlite::Connection::open(&path).unwrap(), "after");
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
    // What a version-1 daemon says to a request it cannot read under its own version.
    let old = r#"{"version":1,"body":{"wrong_version":{"daemon":1,"client":2}}}"#;
    let _hang_up = door(&agent, format!("{old}\n"));
    dispatching();
    let mut dom = VirtualDom::new(Looking)
        .with_root_context(store.clone())
        .with_root_context(knocking(agent));
    dom.rebuild_in_place();
    settle(&mut dom).await;

    insert(&rusqlite::Connection::open(&path).unwrap(), "the-old-watch");
    settle(&mut dom).await;
    assert_eq!(
        revision(&dom),
        1,
        "a refused subscription stopped the looking"
    );
}
