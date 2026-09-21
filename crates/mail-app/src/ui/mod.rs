//! The Dioxus shell.
//!
//! Thin on purpose: every decision lives in [`crate::view`], which is tested without a window.
//! What is here is layout, event wiring, and the one thing a UI can get dangerously wrong —
//! rendering a stranger's HTML.

use crate::view::{
    Listing, Reading, Shell, Shortcut, Stamp, SyncState, badge_filter, hover_actions,
    nothing_to_show, op_for, synced,
};
use chrono::Local;
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// How many rows the list pane asks for at a time.
const PAGE: u32 = 100;

mod composer;
mod style;

use composer::Composer;
use style::STYLE;

/// Keep the app's root focused, so the keyboard has somewhere to land.
///
/// A keydown targets the focused element and bubbles *up*. `body` is that element until
/// something focusable is clicked, and `body` is the root div's parent — so a Rust `onkeydown`
/// on the div is never reached. `tabindex` alone does not fix it; `autofocus` does not either,
/// being a form-control attribute WebKit ignores on a div; and focusing it from Rust needs an
/// async task, which is the one thing that does not work here — a future spawned from a
/// component body is never polled, so neither `spawn` nor `document::eval` nor `use_future` ever
/// runs. Injected into the page head instead, where it needs nothing from Dioxus at all.
const KEEP_FOCUS: &str = r#"<script>
document.addEventListener("DOMContentLoaded", () => {
  const hold = () => {
    const app = document.querySelector(".app");
    if (app) { app.focus(); return true; }
    return false;
  };
  if (!hold()) { const t = setInterval(() => { if (hold()) clearInterval(t); }, 50); }
  // A click on anything that cannot take focus hands it back to `body`, which would silently
  // turn the keyboard off until the next click on a button.
  document.addEventListener("focusin", (event) => {
    if (event.target === document.body) { hold(); }
  });
});
</script>"#;

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
                .with_menu(None)
                .with_custom_head(KEEP_FOCUS.to_owned()),
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
    // Resolved once. The sidebar's places are fixed after construction, so what each badge
    // counts never changes — only the answer does.
    let badge_filters: Vec<Option<Filter>> = use_hook(|| {
        crate::view::default_places()
            .iter()
            .map(|place| badge_filter(&place.source))
            .collect()
    });

    // Depends on `revision` and nothing else. It used to read `shell`, which subscribes a memo
    // to *every* change of it — so each keystroke in the search box re-ran one indexed count
    // per place. Six queries per character, about half a frame on a ten-thousand-message
    // mailbox, to recompute numbers that could not have moved.
    let badges = use_memo(move || {
        let _ = revision();
        let store = consume_context::<Arc<SqliteStore>>();
        let now = chrono::Utc::now();
        badge_filters
            .iter()
            .map(|filter| match store.count(filter.as_ref()?, now) {
                Ok(0) | Err(_) => None,
                Ok(n) => Some(n),
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

    // Why the pane is empty, when it is. Counted rather than assumed: the shell cannot add an
    // account, so the first run needs to name the command that can.
    let nothing = use_memo(move || {
        let _ = revision();
        let store = consume_context::<Arc<SqliteStore>>();
        nothing_to_show(accounts(&store).len(), &shell.read().search)
    });

    // The keyboard.
    //
    // A keydown targets the focused element and bubbles *up*, `body` is that element until
    // something focusable is clicked, and `body` is the app div's parent — so a handler on the
    // div is simply never reached. `tabindex` alone does not fix it and `autofocus` does not
    // either: that attribute is for form controls and WebKit ignores it on a div. Both compiled,
    // rendered, and did nothing, which nothing in this repository could have told me.
    //
    // So the div is focused from JavaScript, and refocused whenever focus falls back to `body` —
    // which is what happens after a click on anything that is not itself focusable. Written as a
    // fire-and-forget script rather than a task with a channel because a future spawned from a
    // component body is never polled here: `use_hook`'s closure runs, `spawn` inside it does not,
    // and `use_future` does not run at all. Spawning works from an event handler, which is where
    // the Sync button does it.
    // Whether a text box has focus. A letter is a shortcut while reading and a letter while
    // writing, and the client that confuses the two archives a conversation because someone
    // typed "e" into a reply. The composer counts wholesale: its fields are many and focus can
    // sit between them.
    let mut in_a_field = use_signal(|| false);

    let on_key = move |event: Event<KeyboardData>| {
        // `Key`'s Display is the DOM key name — "e", "ArrowDown", "Escape" — which is the
        // vocabulary `view::shortcut` is written against.
        let typing = in_a_field() || shell.read().composing.is_some();
        let Some(action) = crate::view::shortcut(&event.key().to_string(), typing) else {
            return;
        };
        let store = consume_context::<Arc<SqliteStore>>();
        let open = shell.read().open;
        match action {
            Shortcut::Next | Shortcut::Previous => {
                let ids: Vec<ThreadId> = threads().iter().map(|t| t.id).collect();
                if let Some(id) = crate::view::step(open, &ids, action == Shortcut::Next) {
                    shell.write().open(id);
                }
            }
            Shortcut::Back => {
                if shell.read().composing.is_some() {
                    // Saving first, exactly as the Close button does. A second way to close that
                    // silently dropped the text would be worse than no keyboard.
                    let current = shell.read().composing.clone();
                    if composer::persist(&store, current.as_ref()).is_ok() {
                        shell.write().close_composer();
                        revision += 1;
                    }
                } else {
                    shell.write().open = None;
                }
            }
            Shortcut::Reply | Shortcut::ReplyAll => {
                let scope = if action == Shortcut::Reply {
                    ReplyScope::Sender
                } else {
                    ReplyScope::All
                };
                if let Some(id) = open
                    && let Ok(draft) = start_reply(&store, id, scope)
                {
                    shell.write().compose(&draft);
                    revision += 1;
                }
            }
            _ => {
                // Resolved against the open thread's own summary, so the keyboard reaches
                // exactly what that row's buttons offer and nothing else.
                let Some(id) = open else { return };
                let Some(summary) = threads().iter().find(|t| t.id == id).cloned() else {
                    return;
                };
                if let Some(kind) = crate::view::op_for_shortcut(action, &summary)
                    && apply_op(&store, id, kind)
                {
                    revision += 1;
                }
            }
        }
    };

    rsx! {
        style { {STYLE} }
        div { class: "app",
            tabindex: "0",
            onkeydown: on_key,
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
                    onfocusin: move |_| in_a_field.set(true),
                    onfocusout: move |_| in_a_field.set(false),
                    value: "{shell.read().search}",
                    oninput: move |e| {
                        shell.write().search = e.value();
                        pages.set(1);
                    },
                }
                if threads().is_empty() && drafts().is_empty() {
                    p { class: "empty", "{nothing().message()}" }
                    if let Some(command) = nothing().command() {
                        pre { class: "command", "{command}" }
                    }
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
                        let when = crate::view::listed(draft.updated, chrono::Utc::now(), &Local);
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
                        let when = crate::view::listed(summary.last_date, chrono::Utc::now(), &Local);
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

    // Only when there is something to load. The offer used to sit above every conversation in
    // the mailbox, plain-text ones included, which is how a security control turns into
    // furniture nobody reads.
    let anything_blocked = messages.iter().any(|(_, reading)| {
        matches!(
            reading,
            Reading::Html {
                blocked_remote: true,
                ..
            }
        )
    });

    rsx! {
        h1 { "{loaded.summary.subject}" }
        if anything_blocked && !shell.read().show_remote_images {
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
                    Reading::Html { html, .. } => rsx! {
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
    crate::view::stamp(message.date, &Local, Stamp::Full)
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

    /// A mailbox shaped like a real one, for looking at the layout rather than exercising it.
    ///
    /// The fixture above holds one message from "Ada" with the subject "hi", which is a size
    /// nothing can be wrong at. Real mail has long subjects, long display names, CJK — this user
    /// is in Taiwan and half their mail is Chinese — and enough rows to fill the pane.
    fn realistic() -> (Arc<SqliteStore>, tempfile::TempDir) {
        let (store, dir) = seeded();
        let rows: [(&str, &str, &str, bool); 6] = [
            (
                "國立臺灣大學計算機及資訊網路中心",
                "ccnoreply@ntu.edu.tw",
                "【重要】臺大計中信箱系統維護通知：本週六 02:00 至 06:00 暫停服務",
                false,
            ),
            (
                "GitHub",
                "notifications@github.test",
                "[rust-lang/rust] Re: Tracking issue for `let`-chains stabilisation (#53667)",
                false,
            ),
            (
                "Dr. Wolfgang Amadeus Pemberton-Featherstonehaugh",
                "w.pemberton@example.test",
                "Re: Re: Re: Fwd: supervision meeting — moved to Thursday",
                true,
            ),
            ("Mum", "mum@example.test", "dinner?", false),
            (
                "Stripe",
                "receipts@stripe.test",
                "Your receipt from Anthropic, PBC #2847-1932",
                true,
            ),
            (
                "arXiv cs.PL",
                "no-reply@arxiv.test",
                "New submissions in cs.PL: 14 papers",
                true,
            ),
        ];
        for (n, (name, email, subject, read)) in rows.iter().enumerate() {
            // Real bytes for the first one, so the reader has an HTML part to sanitize and
            // draw rather than falling back to text for every message in the fixture.
            let bytes = if n == 1 {
                format!(
                    "From: {name} <{email}>\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\n\
                     Content-Type: text/html; charset=utf-8\r\n\r\n\
                     <h2>Tracking issue for <code>let</code>-chains</h2>\
                     <p>There are <b>3</b> new comments on this issue.</p>\
                     <blockquote>Stabilisation report is up; please review.</blockquote>\
                     <p><a href=\"https://example.test/issues/53667\">View it on GitHub</a></p>\r\n"
                )
                .into_bytes()
            } else if n == 4 {
                // A receipt with a tracking pixel, which is what a receipt actually is. This is
                // the one message in the fixture that has something to load.
                format!(
                    "From: {name} <{email}>\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\n\
                     Content-Type: text/html; charset=utf-8\r\n\r\n\
                     <p>Thanks for your payment.</p>\
                     <img src=\"https://track.stripe.test/open/2847-1932.gif\" width=\"1\" \
                     height=\"1\">\r\n"
                )
                .into_bytes()
            } else {
                subject.as_bytes().to_vec()
            };
            let raw = store.blobs().put(&store.connection(), &bytes).unwrap();
            let message = Message {
                id: MessageId::generate(),
                thread: ThreadId::generate(),
                account: ACCOUNT,
                key: MessageKey::Rfc(format!("real{n}@example.test")),
                // Spread over days, so the date column has more than one shape in it.
                date: chrono::Utc::now() - chrono::TimeDelta::try_hours(n as i64 * 19).unwrap(),
                from: Address {
                    name: Some((*name).to_owned()),
                    email: (*email).to_owned(),
                },
                reply_to: vec![],
                to: vec![],
                cc: vec![],
                bcc: vec![],
                subject: (*subject).to_owned(),
                in_reply_to: None,
                references: vec![],
                rfc_message_id: Some(format!("real{n}@example.test")),
                read: if *read {
                    ReadState::Read
                } else {
                    ReadState::Unread
                },
                star: Star::Unstarred,
                mailbox: MailboxRole::Inbox,
                labels: vec![],
                body: Body::Present {
                    text: Some((*subject).to_owned()),
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
                                uidl: format!("real{n}"),
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
        }
        (store, dir)
    }

    /// The shell's markup, rendered from the real components against a real database.
    ///
    /// `rebuild_in_place` proved the components *run*; it never looked at what they produced.
    /// This does, which is the difference between "no panic" and "there is a list on the page".
    fn markup(store: Arc<SqliteStore>) -> String {
        let mut dom = VirtualDom::new(App).with_root_context(store);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
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

    /// Wrap rendered markup in a self-contained page and write it to `target/`.
    ///
    /// Two files: the page as a light-mode desktop draws it, and the same page with
    /// `color-scheme: dark` forced on the root. The stylesheet declares `color-scheme: light
    /// dark` and leans on the `Canvas`/`CanvasText` system colours, so the dark rendering is not
    /// a second stylesheet to keep in step — it is the same one, resolved the other way, which
    /// is exactly the thing that is easy to write and never look at.
    fn dump(name: &str, body: &str) {
        let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        for (suffix, root) in [("", ""), ("-dark", " style=\"color-scheme: dark\"")] {
            let page = format!(
                "<!doctype html>\n<html lang=\"en\"{root}><head><meta charset=\"utf-8\">\n\
                 <title>mailo</title>\n<style>{STYLE}</style>\n</head>\n\
                 <body>{body}</body></html>\n"
            );
            let out = target.join(format!("{name}{suffix}.html"));
            std::fs::write(&out, page).unwrap();
            println!("wrote {}", out.display());
        }
    }

    /// Write the shell to `target/shell.html`, stylesheet and all, so it can be looked at.
    ///
    /// `#[ignore]`d because it is a tool, not an assertion. It exists because the shell's layout
    /// had never been seen: the desktop window is a WebView surface owned by the compositor, and
    /// under rootless XWayland an X11 grab of it returns `BadMatch`, so there was no way from
    /// here to a picture of it. This produces the same markup and the same stylesheet as a
    /// static page, which any browser will render and screenshot headlessly:
    ///
    /// ```text
    /// cargo test -p mail-app --bins -- --ignored render_the_shell_to_a_file
    /// google-chrome --headless --screenshot=shell.png --window-size=1200,800 target/shell.html
    /// ```
    ///
    /// It is the markup and the CSS, not the running application: nothing here clicks, and a
    /// WebView is not a browser. It is still the difference between looking and guessing.
    /// A database with nothing in it: no account, no mail, no drafts.
    ///
    /// The first thing anyone sees, and the one state the fixtures never covered because every
    /// one of them seeds an account before rendering.
    fn empty() -> (Arc<SqliteStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
        (store, dir)
    }

    #[tokio::test]
    #[ignore = "writes target/first-run.html for a human or a headless browser to look at"]
    async fn render_the_first_run_to_a_file() {
        let (store, _dir) = empty();
        dump("first-run", &markup(store));
    }

    #[tokio::test]
    #[ignore = "writes target/shell.html for a human or a headless browser to look at"]
    async fn render_the_shell_to_a_file() {
        let (store, _dir) = seeded();
        dump("shell", &markup(store));
    }

    /// Renders the reader pane on the first thread in the store.
    ///
    /// `App` owns its own `Shell` signal, so nothing outside it can open a conversation; the
    /// reader is reached by rendering it directly, which is also the only way to look at the
    /// half of the shell an empty selection never shows.
    #[component]
    fn ReaderHarness(thread: ThreadId) -> Element {
        let shell = use_signal(Shell::default);
        rsx! {
            div { class: "app",
                div { class: "places" }
                div { class: "list" }
                div { class: "reader", Reader { thread, shell } }
            }
        }
    }

    /// The thread whose subject contains `needle`.
    fn thread_like(store: &SqliteStore, needle: &str) -> ThreadId {
        store
            .threads(
                &Query {
                    filter: Filter::All,
                    sort: Sort {
                        property: Property::Date,
                        dir: SortDir::Desc,
                    },
                    page: PageReq {
                        after: None,
                        limit: 50,
                    },
                },
                chrono::Utc::now(),
            )
            .unwrap()
            .items
            .into_iter()
            .find(|t| t.subject.contains(needle))
            .unwrap_or_else(|| panic!("no thread matching {needle:?} in the fixture"))
            .id
    }

    /// The reader pane's markup for one thread.
    fn reader_markup(store: Arc<SqliteStore>, thread: ThreadId) -> String {
        let mut dom = VirtualDom::new_with_props(ReaderHarness, ReaderHarnessProps { thread })
            .with_root_context(store);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[tokio::test]
    async fn the_reader_does_not_offer_to_load_images_a_message_does_not_have() {
        // The offer sat above every conversation in the mailbox — plain text included — because
        // nothing asked whether anything had been blocked. A control that is always on screen is
        // furniture, and this one asks the user to make network requests on a sender's behalf.
        let (store, _dir) = realistic();
        let thread = thread_like(&store, "rust-lang");
        let markup = reader_markup(store, thread);

        assert!(
            markup.contains("Tracking issue"),
            "the body is there: {markup}"
        );
        assert!(
            !markup.contains("Load remote images"),
            "offered to load images for a message that has none:\n{markup}"
        );
    }

    #[tokio::test]
    async fn the_reader_offers_to_load_images_when_it_blocked_some() {
        // And the other direction, so the gate is not simply "never".
        let (store, _dir) = realistic();
        let thread = thread_like(&store, "receipt");
        let markup = reader_markup(store, thread);

        assert!(
            markup.contains("Load remote images"),
            "a tracking pixel was blocked and nothing said so:\n{markup}"
        );
        assert!(
            !markup.contains("track.stripe.test"),
            "the blocked URL reached the document:\n{markup}"
        );
    }

    #[tokio::test]
    async fn the_first_run_names_the_command_that_gets_you_out_of_it() {
        // A database with no account looked exactly like an empty mailbox: six folders, a Sync
        // button and "Nothing here." The shell cannot add an account, so that was the end of the
        // road rather than a state with a way out.
        let (store, _dir) = empty();
        let markup = markup(store);

        assert!(markup.contains("No account yet"), "{markup}");
        // The angle brackets come back escaped, which is the renderer doing its job; asserting
        // on one spelling of the escape would be asserting on dioxus rather than on the shell.
        assert!(
            markup.contains("mailo account add"),
            "the command is not on the page:\n{markup}"
        );
        assert!(
            markup.contains("class=\"command\""),
            "it is not set apart from the prose, so it reads as italic advice:\n{markup}"
        );
        assert!(
            !markup.contains("Nothing here"),
            "it still says the thing that told a new user nothing:\n{markup}"
        );
    }

    #[tokio::test]
    async fn a_configured_account_with_an_empty_folder_is_not_told_to_add_an_account() {
        // The other direction. `seeded()` has one account and mail in the inbox, so nothing on
        // the page should be setup advice.
        let (store, _dir) = seeded();
        let markup = markup(store);
        assert!(!markup.contains("No account yet"), "{markup}");
    }

    #[tokio::test]
    async fn the_root_can_hold_focus_so_the_keyboard_has_somewhere_to_land() {
        // A keydown targets the focused element and bubbles up, so a handler on an element that
        // can never hold focus is never called. This asserts the one half of that which markup
        // can carry; the other half is `KEEP_FOCUS`, injected into the page head.
        let (store, _dir) = seeded();
        let markup = markup(store);
        assert!(
            markup.contains(r#"tabindex="0""#),
            "the app root is not focusable:\n{markup}"
        );
    }

    #[tokio::test]
    async fn the_focus_script_targets_the_element_that_carries_the_handler() {
        // Two halves of one mechanism in two files: the script focuses `.app`, and `.app` is the
        // class on the div the key handler is attached to. If either is renamed without the
        // other the keyboard stops working silently.
        assert!(KEEP_FOCUS.contains(".app"), "{KEEP_FOCUS}");
        let (store, _dir) = seeded();
        assert!(markup(store).contains(r#"class="app""#));
    }

    #[tokio::test]
    async fn the_shell_draws_the_mail_it_holds() {
        // `rebuild_in_place` only proved the components run. This is the first assertion in the
        // project about what they actually put on the page.
        let (store, _dir) = realistic();
        let markup = markup(store);

        for expected in ["Inbox", "Drafts", "Search all mail", "GitHub", "dinner?"] {
            assert!(markup.contains(expected), "no {expected:?} in:\n{markup}");
        }
        // Long subjects are ellipsised by CSS, not truncated in the markup — the full text has
        // to be there for the browser to do it and for a wider window to show more.
        assert!(
            markup.contains("stabilisation (#53667)"),
            "the subject was cut short before it reached the page:\n{markup}"
        );
    }

    #[tokio::test]
    #[ignore = "writes target/reader.html for a human or a headless browser to look at"]
    async fn render_the_reader_to_a_file() {
        let (store, _dir) = realistic();
        let thread = thread_like(&store, "rust-lang");
        dump("reader", &reader_markup(store, thread));
    }

    /// The same, with a mailbox shaped like a real one. See [`render_the_shell_to_a_file`].
    #[tokio::test]
    #[ignore = "writes target/shell-real.html for a human or a headless browser to look at"]
    async fn render_the_shell_with_real_mail() {
        let (store, _dir) = realistic();
        dump("shell-real", &markup(store));
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

    #[tokio::test]
    #[ignore = "writes target/composer.html for a human or a headless browser to look at"]
    async fn render_the_composer_to_a_file() {
        // The composer is the other pane nothing has ever looked at: it only exists while a
        // draft is open, which the shell's own signal decides, so it is reached through the
        // harness that already exists for the hook-order test.
        let (mut dom, _toggle, _dir) = harness(true);
        dom.rebuild_in_place();
        let body = format!(
            "<div class=\"app\"><div class=\"places\"></div><div class=\"list\"></div>\
             <div class=\"reader\">{}</div></div>",
            dioxus_ssr::render(&dom)
        );
        dump("composer", &body);
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

#[cfg(test)]
mod reactivity_tests {
    //! What a keystroke costs.
    //!
    //! A memo that reads a signal is subscribed to *every* change of it, so reading `shell` to
    //! get something that never changes — the sidebar's places — makes an unrelated write
    //! recompute it. The list must re-query when the search box changes; the badges must not.

    use super::*;
    use dioxus_core::{NoOpMutations, VirtualDom};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// How many times the badge memo has run.
    static BADGE_RUNS: AtomicUsize = AtomicUsize::new(0);
    /// The shell signal, published so the test can write it the way a keystroke does.
    static TYPE_NOW: AtomicUsize = AtomicUsize::new(0);

    /// A stand-in for `App`'s badge memo: depends on `revision`, not on `shell`.
    #[component]
    fn Badges() -> Element {
        let mut shell = use_signal(Shell::default);
        let revision = use_signal(|| 0u64);

        let filters: Vec<Option<Filter>> = use_hook(|| {
            crate::view::default_places()
                .iter()
                .map(|place| badge_filter(&place.source))
                .collect()
        });
        let badges = use_memo(move || {
            let _ = revision();
            BADGE_RUNS.fetch_add(1, Ordering::SeqCst);
            filters.len()
        });

        // A write to `shell`, driven from the test: this is the keystroke. Done in an effect
        // rather than during render, because writing a signal while rendering is not what a
        // key press does and not what is being measured.
        use_effect(move || {
            if TYPE_NOW.swap(0, Ordering::SeqCst) > 0 {
                shell.write().search.push('x');
            }
        });

        let typed = shell.read().search.len();
        rsx! { div { "{badges()} {typed}" } }
    }

    #[tokio::test]
    async fn typing_in_the_search_box_does_not_recount_every_badge() {
        BADGE_RUNS.store(0, Ordering::SeqCst);
        TYPE_NOW.store(0, Ordering::SeqCst);
        let mut dom = VirtualDom::new(Badges);
        dom.rebuild_in_place();
        let after_first = BADGE_RUNS.load(Ordering::SeqCst);

        for _ in 0..5 {
            // A keystroke: write `shell`, then let the dom settle.
            TYPE_NOW.store(1, Ordering::SeqCst);
            dom.mark_dirty(dioxus_core::ScopeId::APP);
            dom.render_immediate(&mut NoOpMutations);
            dom.render_immediate(&mut NoOpMutations);
        }

        let runs = BADGE_RUNS.load(Ordering::SeqCst);
        assert_eq!(
            runs,
            after_first,
            "the badge memo re-ran {} times for keystrokes that changed no count; each run is \
             one indexed query per place",
            runs - after_first
        );
    }
}
