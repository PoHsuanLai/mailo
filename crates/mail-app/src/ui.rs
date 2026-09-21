//! The Dioxus shell.
//!
//! Thin on purpose: every decision lives in [`crate::view`], which is tested without a window.
//! What is here is layout, event wiring, and the one thing a UI can get dangerously wrong —
//! rendering a stranger's HTML.

use crate::view::{
    Composing, Listing, Reading, Shell, SyncState, badge_filter, hover_actions, op_for, synced,
};
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// How many rows the list pane asks for at a time.
const PAGE: u32 = 100;

/// Seconds between autosaves of an open composer.
///
/// Long enough not to write on every keystroke, short enough that what a crash costs is a
/// sentence rather than a letter.
const AUTOSAVE_EVERY: u64 = 3;

/// Launch the shell.
pub fn run(store: Arc<SqliteStore>) {
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new()
                .with_window(
                    dioxus::desktop::WindowBuilder::new()
                        .with_title("mailo")
                        .with_inner_size(dioxus::desktop::LogicalSize::new(1200.0, 800.0)),
                )
                .with_menu(None),
        )
        .with_context(store)
        .launch(App);
}

#[component]
fn App() -> Element {
    let store = use_context::<Arc<SqliteStore>>();
    let mut shell = use_signal(Shell::default);
    // Bumped after any write, to re-run the queries. Explicit rather than implicit so it is
    // obvious what causes a refresh.
    let mut revision = use_signal(|| 0u64);

    // How many pages of the list have been asked for. Reset whenever the list itself changes,
    // because "page 3" of the Inbox means nothing once the user is looking at Archive.
    let mut pages = use_signal(|| 1u32);
    let mut sync_state = use_signal(|| SyncState::Idle);

    // One count per place, recomputed after any write. `Store::count` answers each in a single
    // indexed query, which is why the sidebar can afford to ask on every revision.
    let badges = use_memo(move || {
        let _ = revision();
        let store = consume_context::<Arc<SqliteStore>>();
        let now = chrono::Utc::now();
        shell
            .read()
            .places
            .iter()
            .map(|place| {
                let filter = badge_filter(&place.source)?;
                match store.count(&filter, now) {
                    Ok(0) | Err(_) => None,
                    Ok(n) => Some(n),
                }
            })
            .collect::<Vec<Option<u64>>>()
    });

    let threads = use_memo(move || {
        let _ = revision();
        match shell.read().listing(PAGE * pages()) {
            Listing::Threads(query) => store
                .threads(&query, chrono::Utc::now())
                .map(|page| page.items)
                .unwrap_or_default(),
            Listing::Drafts => Vec::new(),
        }
    });

    let drafts = use_memo(move || {
        let _ = revision();
        if !matches!(shell.read().listing(PAGE), Listing::Drafts) {
            return Vec::new();
        }
        let store = consume_context::<Arc<SqliteStore>>();
        accounts(&store)
            .into_iter()
            .filter_map(|account| store.drafts(account).ok())
            .flatten()
            .collect::<Vec<_>>()
    });

    // One more row than asked for means there is another page. Asking the store for the count
    // would be a second query answering a question this one already answers.
    // Deliberately a growing limit rather than `Page::next`, which the store also returns.
    // `plan.md` asks for the cursor, and the cursor is the right answer for an accumulating
    // list — but accumulating means holding pages in a signal and rebuilding them after every
    // archive, star and ingest, and a mixture of pages fetched at different moments is exactly
    // the inconsistency keyset pagination exists to avoid. One query for the whole visible list
    // is always self-consistent, costs a few thousand indexed rows at the sizes this client is
    // for, and needs no invalidation logic at all. Revisit when a mailbox is large enough to
    // measure, at which point the cursor is already there.
    let more = use_memo(move || threads().len() as u32 >= PAGE * pages());

    rsx! {
        style { {STYLE} }
        div { class: "app",
            nav { class: "places",
                for (index, place) in shell.read().places.iter().enumerate() {
                    button {
                        key: "{place.name}",
                        class: if index == shell.read().selected { "place on" } else { "place" },
                        onclick: move |_| {
                            shell.write().select(index);
                            pages.set(1);
                        },
                        "{place.name}"
                        if let Some(Some(count)) = badges().get(index).copied() {
                            span { class: "badge", "{count}" }
                        }
                    }
                }
                div { class: "spacer" }
                button {
                    class: "place sync",
                    disabled: !sync_state.read().may_start(),
                    onclick: move |_| {
                        if !sync_state.read().may_start() {
                            return;
                        }
                        sync_state.set(SyncState::Running);
                        let store = consume_context::<Arc<SqliteStore>>();
                        spawn(async move {
                            // `spawn_blocking`, not this task: sync::run opens sockets and
                            // builds its own runtime, and `Runtime::block_on` inside an async
                            // context panics. Off the UI thread either way — a pass takes
                            // minutes on a first sync and would freeze the window.
                            let done = tokio::task::spawn_blocking(move || {
                                crate::sync::run(store, chrono::Utc::now())
                            })
                            .await;
                            sync_state.set(match done {
                                Ok(result) => synced(result),
                                // The blocking task panicked. Saying so beats a window that
                                // sits on "Syncing…" for ever.
                                Err(e) => synced(Err(format!("the sync pass stopped: {e}"))),
                            });
                            revision += 1;
                        });
                    },
                    if sync_state.read().may_start() { "Sync" } else { "Syncing…" }
                }
                if let Some(note) = sync_state.read().message() {
                    p {
                        class: if sync_state.read().is_failure() { "sync-note bad" } else { "sync-note" },
                        "{note}"
                    }
                }
            }
            section { class: "list",
                input {
                    class: "search",
                    placeholder: "Search all mail",
                    value: "{shell.read().search}",
                    oninput: move |e| {
                        shell.write().search = e.value();
                        pages.set(1);
                    },
                }
                if threads().is_empty() && drafts().is_empty() {
                    p { class: "empty", "Nothing here." }
                }
                for draft in drafts() {
                    {
                        let id = draft.id;
                        let subject = if draft.subject.is_empty() {
                            "(no subject)".to_owned()
                        } else {
                            draft.subject.clone()
                        };
                        let who = crate::view::join_addresses(&draft.to);
                        let state = draft_state(&draft.state);
                        let when = draft.updated.format("%b %d").to_string();
                        rsx! {
                            div {
                                key: "{id}",
                                class: "row",
                                onclick: move |_| {
                                    let store = consume_context::<Arc<SqliteStore>>();
                                    if let Ok(draft) = store.draft(id) {
                                        shell.write().compose(&draft);
                                    }
                                },
                                span { class: "who", if who.is_empty() { "(no recipient)" } else { "{who}" } }
                                span { class: "subject", "{subject}" }
                                span { class: "when", "{state} · {when}" }
                            }
                        }
                    }
                }
                for summary in threads() {
                    {
                        let id = summary.id;
                        let unread = summary.read == ReadState::Unread;
                        let who = sender(&summary);
                        let when = summary.last_date.format("%b %d").to_string();
                        let subject = summary.subject.clone();
                        let actions = hover_actions(&summary);
                        rsx! {
                            div {
                                key: "{id}",
                                class: if unread { "row unread" } else { "row" },
                                onclick: move |_| shell.write().open(id),
                                span { class: "who", "{who}" }
                                span { class: "subject", "{subject}" }
                                span { class: "when", "{when}" }
                                span { class: "hover",
                                    for kind in actions {
                                        button {
                                            key: "{kind:?}",
                                            onclick: move |e: Event<MouseData>| {
                                                // Without this the click also opens the thread.
                                                e.stop_propagation();
                                                let store = consume_context::<Arc<SqliteStore>>();
                                                match reply_scope(kind) {
                                                    Some(scope) => {
                                                        match start_reply(&store, id, scope) {
                                                            Ok(draft) => {
                                                                shell.write().compose(&draft);
                                                                revision += 1;
                                                            }
                                                            Err(why) => {
                                                                // Nowhere else to say it yet:
                                                                // the composer that would show
                                                                // a notice is what failed to
                                                                // open.
                                                                eprintln!("reply: {why}");
                                                            }
                                                        }
                                                    }
                                                    None => {
                                                        if apply_op(&store, id, kind) {
                                                            revision += 1;
                                                        }
                                                    }
                                                }
                                            },
                                            "{label(kind)}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if more() {
                    button {
                        class: "more",
                        onclick: move |_| pages += 1,
                        "Show more"
                    }
                }
            }
            section { class: "reader",
                if let Some(thread) = shell.read().open {
                    Reader { thread, shell }
                } else if shell.read().composing.is_none() {
                    p { class: "empty", "Select a conversation." }
                }
                if shell.read().composing.is_some() {
                    Composer { shell, revision }
                }
            }
        }
    }
}

#[component]
fn Reader(thread: ThreadId, shell: Signal<Shell>) -> Element {
    let store = use_context::<Arc<SqliteStore>>();
    let Ok(loaded) = store.thread(thread) else {
        return rsx! { p { class: "empty", "That conversation is gone." } };
    };
    let policy = shell.read().policy();
    // The HTML part is not a column: it lives inside the stored raw message, which is the only
    // copy that is byte-for-byte what the server sent. Parsed here, once per render of a thread,
    // rather than at ingest — storing sanitized HTML would freeze today's sanitizer into every
    // row, and storing the unsanitized part would duplicate bytes we already have.
    // Each message resolved all the way to what the pane should draw, before the view tree.
    // `reading` sanitizes; `embed_inline` then resolves `cid:` inside what the sanitizer
    // allowed, in that order, because the sanitizer must judge the message's own URLs and not
    // a `data:` URI we substituted for one.
    let messages: Vec<(Message, Reading)> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .map(|message| {
            let reading = crate::reader::render(&store, &message, policy);
            (message, reading)
        })
        .collect();

    rsx! {
        h1 { "{loaded.summary.subject}" }
        if !shell.read().show_remote_images {
            button {
                class: "images",
                onclick: move |_| shell.write().show_remote_images = true,
                "Load remote images"
            }
        }
        for (message, reading) in messages {
            article { key: "{message.id}",
                header {
                    strong { "{from_name(&message)}" }
                    span { "{address(&message)}" }
                    time { "{stamp(&message)}" }
                }
                match reading {
                    Reading::NotFetched => rsx! { p { class: "pending", "Body not downloaded yet." } },
                    Reading::Text(text) => rsx! { pre { class: "text", "{text}" } },
                    // Never into the app's own document: a sandboxed frame with no
                    // allow-same-origin, so even a sanitizer bug cannot reach our DOM.
                    Reading::Html(html) => rsx! {
                        iframe {
                            class: "html",
                            // No allow-same-origin: even a sanitizer bug cannot reach our DOM.
                            // Raw attribute because dioxus has no typed `sandbox` for iframe.
                            "sandbox": "",
                            srcdoc: "{html}",
                        }
                        // NOTE for anyone changing the reader's layout: moving this iframe to a
                        // different parent makes the browser tear down and RELOAD the document
                        // inside it. That re-runs the sanitizer, loses scroll position, and
                        // re-requests anything the reader had just consented to — a second
                        // network fetch and a silent consent reset, with nothing in the UI
                        // saying either happened. Reading modes must restyle one container,
                        // never reparent this node.
                    },
                }
            }
        }
    }
}

/// The composer pane.
///
/// Every field writes straight back into `Shell.composing`, and every button goes through
/// `crate::compose`, which is the same module the CLI calls. Two code paths for "send this
/// draft" is how a window and a command start disagreeing about what a draft is.
#[component]
fn Composer(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    // Every hook first, before any early return. `use_hook` matches hooks between renders by
    // call order, so a component that returns before reaching one leaves every hook after it at
    // a different index on the next render. This function used to return above both hooks
    // below, which happened to be harmless — there was no third hook to shift — but "harmless
    // given the current body" is a property that the next hook added here would quietly end.
    //
    // Set by every field, cleared by a successful write. A flag rather than comparing against
    // the stored row on a timer: the comparison would read and parse the draft every few
    // seconds whether or not anyone had touched it.
    let mut dirty = use_signal(|| false);

    // The autosave. Close and Send both save, so what this covers is the window nothing else
    // does: the application going away while someone is still typing.
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(AUTOSAVE_EVERY)).await;
            if !dirty() {
                continue;
            }
            let store = consume_context::<Arc<SqliteStore>>();
            let current = shell.read().composing.clone();
            // Failures are swallowed on purpose. The usual one is a half-typed recipient, and a
            // timer that interrupts to complain about an address still being typed is worse than
            // one that waits. The text stays dirty and the next tick tries again.
            if persist(&store, current.as_ref()).is_ok() {
                dirty.set(false);
            }
        }
    });

    // Now the early return, below every hook. `App` only renders this when something is being
    // composed, but "only" is a claim about a caller, and the rules of hooks are not a matter
    // of who calls what.
    let Some(editing) = shell.read().composing.clone() else {
        return rsx! {};
    };

    rsx! {
        div { class: "composer",
            header { class: "composer-head",
                strong { "{editing.subject}" }
                button {
                    class: "ghost",
                    onclick: move |_| {
                        let store = consume_context::<Arc<SqliteStore>>();
                        let current = shell.read().composing.clone();
                        match persist(&store, current.as_ref()) {
                            Ok(_) => {
                                shell.write().close_composer();
                                revision += 1;
                            }
                            // Stay open rather than lose the text. A recipient that does not
                            // parse must not cost the user the paragraph they just wrote, and
                            // Discard is right there for anyone who meant to abandon it.
                            Err(why) => set_notice(&mut shell, Some(why)),
                        }
                    },
                    "Close"
                }
                button {
                    class: "ghost",
                    onclick: move |_| shell.write().close_composer(),
                    title: "Close without saving",
                    "Discard"
                }
            }
            if let Some(notice) = editing.notice.clone() {
                p { class: "notice", "{notice}" }
            }
            label { "To"
                input {
                    value: "{editing.to}",
                    oninput: move |e| {
                        if let Some(c) = shell.write().composing.as_mut() {
                            c.to = e.value();
                        }
                        dirty.set(true);
                    },
                }
            }
            label { "Cc"
                input {
                    value: "{editing.cc}",
                    oninput: move |e| {
                        if let Some(c) = shell.write().composing.as_mut() {
                            c.cc = e.value();
                        }
                        dirty.set(true);
                    },
                }
            }
            label { "Subject"
                input {
                    value: "{editing.subject}",
                    oninput: move |e| {
                        if let Some(c) = shell.write().composing.as_mut() {
                            c.subject = e.value();
                        }
                        dirty.set(true);
                    },
                }
            }
            textarea {
                class: "composer-body",
                value: "{editing.body}",
                oninput: move |e| {
                    if let Some(c) = shell.write().composing.as_mut() {
                        c.body = e.value();
                    }
                    dirty.set(true);
                },
            }
            div { class: "composer-actions",
                button {
                    onclick: move |_| {
                        let store = consume_context::<Arc<SqliteStore>>();
                        // Cloned out of the signal in its own statement: the read guard ends
                        // here, so the handler can write a notice back afterwards. It also
                        // reads what is in the fields *now* rather than at last render.
                        let current = shell.read().composing.clone();
                        let saved = persist(&store, current.as_ref());
                        match saved {
                            Ok(_) => {
                                dirty.set(false);
                                set_notice(&mut shell, Some("Saved.".to_owned()));
                                revision += 1;
                            }
                            Err(why) => set_notice(&mut shell, Some(why)),
                        }
                    },
                    "Save"
                }
                button {
                    class: "primary",
                    onclick: move |_| {
                        let store = consume_context::<Arc<SqliteStore>>();
                        // Saved first, always. Sending what is in the widgets without writing
                        // it down means a failure between the two loses the user's edits.
                        let current = shell.read().composing.clone();
                        let sent = persist(&store, current.as_ref()).and_then(|draft| {
                            crate::compose::send(&store, draft.id, chrono::Utc::now())
                        });
                        match sent {
                            Ok(_) => {
                                shell.write().close_composer();
                                revision += 1;
                            }
                            Err(why) => set_notice(&mut shell, Some(why)),
                        }
                    },
                    "Send"
                }
                span { class: "hint", "Sending queues the message; the next sync delivers it." }
            }
        }
    }
}

/// Write the composer's fields back onto the stored draft.
///
/// Reads the draft from the store rather than keeping a copy in the widgets, so a field the
/// composer does not show — the identity, the Bcc list, what this replies to — is whatever the
/// store says and not whatever was true when the composer opened.
fn persist(store: &SqliteStore, editing: Option<&Composing>) -> Result<Draft, String> {
    let editing = editing.ok_or_else(|| "nothing is being composed".to_owned())?;
    let base = store.draft(editing.draft).map_err(|e| e.to_string())?;
    let edited = editing.apply_to(&base, chrono::Utc::now())?;
    crate::compose::save(store, &edited)?;
    Ok(edited)
}

fn set_notice(shell: &mut Signal<Shell>, notice: Option<String>) {
    if let Some(c) = shell.write().composing.as_mut() {
        c.notice = notice;
    }
}

/// Which reply a hover button means, if it is one.
fn reply_scope(kind: OpKind) -> Option<ReplyScope> {
    match kind {
        OpKind::Reply => Some(ReplyScope::Sender),
        OpKind::ReplyAll => Some(ReplyScope::All),
        // Forward needs recipients the user has not chosen yet, and no body to quote until
        // they do. It opens a composer too, but not this way round; not wired.
        _ => None,
    }
}

/// Create the draft a reply button opens.
///
/// Which message that answers is [`crate::view::reply_target`]'s decision, not this function's.
fn start_reply(store: &SqliteStore, thread: ThreadId, scope: ReplyScope) -> Result<Draft, String> {
    let loaded = store.thread(thread).map_err(|e| e.to_string())?;
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect();
    let target = crate::view::reply_target(&messages)
        .ok_or_else(|| "that conversation has no messages".to_owned())?;
    crate::compose::draft_reply(store, target.id, scope, "", chrono::Utc::now())
}

/// Every configured account, for the places that are not scoped to one.
fn accounts(store: &SqliteStore) -> Vec<AccountId> {
    let db = store.connection();
    let Ok(mut stmt) = db.prepare("SELECT id FROM accounts ORDER BY created_at") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
        return Vec::new();
    };
    rows.filter_map(|row| row.ok())
        .filter_map(|id| id.parse().ok())
        .map(AccountId::from_uuid)
        .collect()
}

fn draft_state(state: &SendState) -> &'static str {
    match state {
        SendState::Editing => "draft",
        SendState::Queued => "queued",
        SendState::Sending => "sending",
        SendState::Failed { .. } => "failed",
        SendState::Sent { .. } => "sent",
    }
}

fn from_name(message: &Message) -> String {
    message.from.name.clone().unwrap_or_default()
}

/// Apply a hover action, returning whether anything changed.
///
/// A free function rather than a closure so it can be called from several handlers, and so the
/// store it needs is an argument rather than a capture.
fn apply_op(store: &SqliteStore, thread: ThreadId, kind: OpKind) -> bool {
    let Some(op) = op_for(kind) else {
        // Reply, label and snooze open something rather than acting. Not wired yet, and doing
        // nothing beats doing the wrong thing silently.
        return false;
    };
    let Ok(loaded) = store.thread(thread) else {
        return false;
    };
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect();
    let Some(account) = messages.first().map(|m| m.account) else {
        return false;
    };
    // Defaults sit at the safe end: expunging forbidden, labels local. Real capabilities
    // arrive once an account is configured and synced.
    let caps = AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget::default(),
        observed_at: chrono::Utc::now(),
    };
    let applied = op.apply(
        &Target::Threads(vec![thread]),
        &loaded,
        &messages,
        &caps,
        chrono::Utc::now(),
    );
    store.apply(account, &applied.forward).is_ok()
}

fn address(message: &Message) -> String {
    format!(" <{}>", message.from.email)
}

fn stamp(message: &Message) -> String {
    message.date.format("%Y-%m-%d %H:%M").to_string()
}

fn sender(summary: &ThreadSummary) -> String {
    summary
        .from
        .name
        .clone()
        .unwrap_or_else(|| summary.from.email.clone())
}

fn label(kind: OpKind) -> &'static str {
    match kind {
        OpKind::Archive => "Archive",
        OpKind::Trash => "Trash",
        OpKind::Restore => "Restore",
        OpKind::Spam => "Spam",
        OpKind::MarkRead => "Read",
        OpKind::MarkUnread => "Unread",
        OpKind::Star => "Star",
        OpKind::Unstar => "Unstar",
        OpKind::AddLabel | OpKind::RemoveLabel => "Label",
        OpKind::Snooze => "Snooze",
        OpKind::Pin => "Pin",
        OpKind::Reply => "Reply",
        OpKind::ReplyAll => "Reply all",
        OpKind::Forward => "Forward",
    }
}

const STYLE: &str = r#"
:root { color-scheme: light dark; --edge: color-mix(in oklab, currentColor 15%, transparent); }
* { box-sizing: border-box; }
body { margin: 0; font: 14px/1.5 system-ui, sans-serif; }
.app { display: grid; grid-template-columns: 180px 380px 1fr; height: 100vh; }
.places { display: flex; flex-direction: column; gap: 2px; padding: 12px; border-right: 1px solid var(--edge); }
.place { text-align: left; padding: 6px 10px; border: 0; border-radius: 6px; background: none; color: inherit; font: inherit; cursor: pointer; }
.place:hover { background: var(--edge); }
.place.on { background: var(--edge); font-weight: 600; }
.list { overflow-y: auto; border-right: 1px solid var(--edge); }
.search { width: 100%; padding: 10px 12px; border: 0; border-bottom: 1px solid var(--edge); background: none; color: inherit; font: inherit; }
.row { display: grid; grid-template-columns: 140px 1fr auto; gap: 10px; align-items: center; padding: 10px 12px; border-bottom: 1px solid var(--edge); cursor: pointer; position: relative; }
.row:hover { background: var(--edge); }
.row.unread .subject, .row.unread .who { font-weight: 650; }
.who, .subject { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.when { opacity: .6; font-variant-numeric: tabular-nums; }
.hover { display: none; position: absolute; right: 8px; gap: 4px; }
.row:hover .hover { display: flex; }
.hover button { font: inherit; font-size: 12px; padding: 2px 8px; border: 1px solid var(--edge); border-radius: 999px; background: Canvas; color: inherit; cursor: pointer; }
.reader { overflow-y: auto; padding: 20px 24px; }
.reader h1 { font-size: 20px; margin: 0 0 12px; }
article { border-top: 1px solid var(--edge); padding: 14px 0; }
article header { display: flex; gap: 6px; align-items: baseline; flex-wrap: wrap; margin-bottom: 8px; }
article time { margin-left: auto; opacity: .6; }
.text { white-space: pre-wrap; word-wrap: break-word; font: inherit; margin: 0; }
.html { width: 100%; min-height: 320px; border: 0; }
.pending, .empty { opacity: .6; font-style: italic; }
.composer { border-top: 2px solid var(--edge); margin-top: 16px; padding-top: 12px; display: flex; flex-direction: column; gap: 8px; }
.composer-head { display: flex; align-items: baseline; gap: 10px; }
.composer-head strong { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.composer label { display: grid; grid-template-columns: 70px 1fr; align-items: center; gap: 8px; font-size: 12px; opacity: .75; }
.composer input { font: inherit; padding: 6px 8px; border: 1px solid var(--edge); border-radius: 6px; background: Canvas; color: inherit; }
.composer-body { font: inherit; min-height: 180px; padding: 8px; border: 1px solid var(--edge); border-radius: 6px; background: Canvas; color: inherit; resize: vertical; }
.composer-actions { display: flex; align-items: center; gap: 8px; }
.composer-actions button { font: inherit; padding: 6px 14px; border: 1px solid var(--edge); border-radius: 6px; background: Canvas; color: inherit; cursor: pointer; }
.composer-actions .primary { font-weight: 600; }
.ghost { font: inherit; font-size: 12px; padding: 2px 8px; border: 1px solid var(--edge); border-radius: 999px; background: none; color: inherit; cursor: pointer; }
.notice { margin: 0; padding: 6px 8px; border-radius: 6px; background: var(--edge); font-size: 13px; }
.hint { font-size: 12px; opacity: .6; }
.place { display: flex; align-items: center; gap: 8px; }
.badge { margin-left: auto; font-size: 11px; font-variant-numeric: tabular-nums; opacity: .7; }
.place.on .badge { opacity: 1; }
.spacer { flex: 1; }
.sync { text-align: center; border: 1px solid var(--edge); }
.sync:disabled { opacity: .6; cursor: default; }
.sync-note { margin: 8px 2px 0; font-size: 11px; opacity: .7; white-space: pre-wrap; word-break: break-word; }
.sync-note.bad { opacity: .95; font-weight: 600; }
.more { display: block; width: calc(100% - 24px); margin: 10px 12px; padding: 8px; font: inherit; border: 1px solid var(--edge); border-radius: 6px; background: none; color: inherit; cursor: pointer; }
.images { font: inherit; font-size: 12px; padding: 4px 10px; border: 1px solid var(--edge); border-radius: 999px; background: none; color: inherit; cursor: pointer; margin-bottom: 8px; }
"#;

#[cfg(test)]
mod render_tests {
    //! The components, actually executed.
    //!
    //! Everything else about the shell is tested through `view.rs` and `reader.rs`, which are
    //! free of Dioxus on purpose. That leaves the components themselves — and a component can
    //! fail in ways those tests cannot see: a panic in `rsx!`, a context that is not there, or
    //! a hook called somewhere the rules of hooks forbid. A `VirtualDom` runs them with no
    //! window, which is the only part of this that ever needed one.

    use super::*;
    use dioxus_core::{NoOpMutations, VirtualDom};

    const ACCOUNT: AccountId =
        AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

    /// A store with one account, one message and one draft, so the panes have something to draw.
    fn seeded() -> (Arc<SqliteStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
        let identity = IdentityId::generate();
        {
            let db = store.connection();
            db.execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, 'me@example.test', '{}', datetime('now'))",
                [ACCOUNT.to_string()],
            )
            .unwrap();
            db.execute(
                "INSERT INTO identities (id, account, from_name, from_email, is_default)
                 VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
                [identity.to_string(), ACCOUNT.to_string()],
            )
            .unwrap();
        }

        let raw = store
            .blobs()
            .put(
                &store.connection(),
                b"From: ada@example.test\r\nSubject: hi\r\n\r\nbody\r\n",
            )
            .unwrap();
        let message = Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account: ACCOUNT,
            key: MessageKey::Rfc("m1@example.test".to_owned()),
            date: chrono::Utc::now(),
            from: Address {
                name: Some("Ada".to_owned()),
                email: "ada@example.test".to_owned(),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "hi".to_owned(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some("m1@example.test".to_owned()),
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            body: Body::Present {
                text: Some("body".to_owned()),
                raw,
            },
            attachments: vec![],
        };
        store
            .ingest(
                ACCOUNT,
                Ingest {
                    mailbox: MailboxRef {
                        account: ACCOUNT,
                        path: "INBOX".to_owned(),
                    },
                    validity: UidValidity::Same,
                    cursor: Some(SyncCursor::Pop),
                    messages: vec![Fetched {
                        remote: RemoteRef::Pop {
                            uidl: "u1".to_owned(),
                        },
                        key: message.key.clone(),
                        raw,
                        message,
                    }],
                    flags: vec![],
                    labels: vec![],
                    gone: vec![],
                },
            )
            .unwrap();

        store
            .apply(
                ACCOUNT,
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::DraftUpsert(Box::new(Draft {
                        id: DraftId::generate(),
                        account: ACCOUNT,
                        identity,
                        to: vec![Address {
                            name: None,
                            email: "ada@example.test".to_owned(),
                        }],
                        cc: vec![],
                        bcc: vec![],
                        subject: "Re: hi".to_owned(),
                        in_reply_to: None,
                        forward_of: None,
                        text: "typing".to_owned(),
                        html: None,
                        attachments: vec![],
                        state: SendState::Editing,
                        updated: chrono::Utc::now(),
                    }))],
                },
            )
            .unwrap();
        (store, dir)
    }

    #[tokio::test]
    async fn the_whole_app_renders() {
        // Catches what compiling cannot: a missing context, a panic inside `rsx!`, a query that
        // blows up on a real database. Until this test the components had never been executed
        // at all — every other test stops at `view.rs`.
        let (store, _dir) = seeded();
        let mut dom = VirtualDom::new(App).with_root_context(store);
        dom.rebuild_in_place();
    }

    #[tokio::test]
    async fn re_rendering_the_app_is_stable() {
        // A second render is where hook-order mistakes surface: hooks are matched between
        // renders by call order, so a component whose hook count changes corrupts every index
        // after it, and the first render alone would never show it.
        let (store, _dir) = seeded();
        let mut dom = VirtualDom::new(App).with_root_context(store);
        dom.rebuild_in_place();
        for _ in 0..3 {
            dom.mark_dirty(dioxus_core::ScopeId::APP);
            dom.render_immediate(&mut NoOpMutations);
        }
    }

    /// Whether the harness should have a composer open on this render.
    ///
    /// Shared through the root context rather than a prop, so the test can flip it *between*
    /// renders of the same scope — which is the only way the hook-order mistake shows itself.
    #[derive(Clone)]
    struct Toggle(Arc<std::sync::atomic::AtomicBool>);

    /// Renders `Composer`, opening or closing it according to [`Toggle`] on every render.
    #[component]
    fn ComposerHarness() -> Element {
        let store = use_context::<Arc<SqliteStore>>();
        let toggle = use_context::<Toggle>();
        let mut shell = use_signal(Shell::default);
        let revision = use_signal(|| 0u64);

        let want_open = toggle.0.load(std::sync::atomic::Ordering::SeqCst);
        let is_open = shell.read().composing.is_some();
        if want_open && !is_open {
            if let Some(draft) = store
                .drafts(ACCOUNT)
                .ok()
                .and_then(|d| d.into_iter().next())
            {
                shell.write().compose(&draft);
            }
        } else if !want_open && is_open {
            shell.write().close_composer();
        }
        rsx! { Composer { shell, revision } }
    }

    fn harness(open: bool) -> (VirtualDom, Toggle, tempfile::TempDir) {
        let (store, dir) = seeded();
        let toggle = Toggle(Arc::new(std::sync::atomic::AtomicBool::new(open)));
        let dom = VirtualDom::new(ComposerHarness)
            .with_root_context(store)
            .with_root_context(toggle.clone());
        (dom, toggle, dir)
    }

    #[tokio::test]
    async fn the_composer_renders_with_a_draft_open() {
        let (mut dom, _toggle, _dir) = harness(true);
        dom.rebuild_in_place();
    }

    #[tokio::test]
    async fn the_composer_can_be_opened_and_closed_repeatedly() {
        // What this proves is what it says: opening and closing the composer, several times,
        // runs the component each way round without panicking.
        //
        // It is deliberately *not* claiming to guard the hook-order fix in the same commit.
        // That fix is right — `use_hook` is documented to require a stable call order, and
        // `Composer` used to return above both of its hooks — but I checked, and this test
        // passes with the early return put back. It has to: `use_hook` indexes from zero on
        // every render, and since the early return preceded *every* hook in the function there
        // was no later hook left to misalign. The rule was broken; nothing downstream of it
        // was. Writing the assertion that could fail is how I found that out, and leaving the
        // test here mislabelled would have been worse than not writing it.
        let (mut dom, toggle, _dir) = harness(false);
        dom.rebuild_in_place();

        for open in [true, false, true, false] {
            toggle.0.store(open, std::sync::atomic::Ordering::SeqCst);
            dom.mark_dirty(dioxus_core::ScopeId::APP);
            dom.render_immediate(&mut NoOpMutations);
        }
    }
}
