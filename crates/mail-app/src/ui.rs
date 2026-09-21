//! The Dioxus shell.
//!
//! Thin on purpose: every decision lives in [`crate::view`], which is tested without a window.
//! What is here is layout, event wiring, and the one thing a UI can get dangerously wrong —
//! rendering a stranger's HTML.

use crate::view::{Composing, Reading, Shell, hover_actions, op_for, reading};
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// How many rows the list pane asks for at a time.
const PAGE: u32 = 100;

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

    let threads = use_memo(move || {
        let _ = revision();
        let query = shell.read().query(PAGE);
        store
            .threads(&query, chrono::Utc::now())
            .map(|page| page.items)
            .unwrap_or_default()
    });

    rsx! {
        style { {STYLE} }
        div { class: "app",
            nav { class: "places",
                for (index, place) in shell.read().places.iter().enumerate() {
                    button {
                        key: "{place.name}",
                        class: if index == shell.read().selected { "place on" } else { "place" },
                        onclick: move |_| shell.write().select(index),
                        "{place.name}"
                    }
                }
            }
            section { class: "list",
                input {
                    class: "search",
                    placeholder: "Search all mail",
                    value: "{shell.read().search}",
                    oninput: move |e| shell.write().search = e.value(),
                }
                if threads().is_empty() {
                    p { class: "empty", "Nothing here." }
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
                                                let store = use_context::<Arc<SqliteStore>>();
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
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
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
        for message in messages {
            article { key: "{message.id}",
                header {
                    strong { "{from_name(&message)}" }
                    span { "{address(&message)}" }
                    time { "{stamp(&message)}" }
                }
                // The raw HTML part is not stored separately yet, so the text part is what
                // renders. When it is, `reading` already takes it and sanitizes per policy.
                match reading(&message.body, None, policy) {
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
    // The store is taken inside each handler rather than here: a handler runs long after this
    // render, and the context it needs is the one live at that moment.
    let Some(editing) = shell.read().composing.clone() else {
        return rsx! {};
    };

    rsx! {
        div { class: "composer",
            header { class: "composer-head",
                strong { "{editing.subject}" }
                button {
                    class: "ghost",
                    onclick: move |_| shell.write().close_composer(),
                    "Close"
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
                },
            }
            div { class: "composer-actions",
                button {
                    onclick: move |_| {
                        let store = use_context::<Arc<SqliteStore>>();
                        // Cloned out of the signal in its own statement: the read guard ends
                        // here, so the handler can write a notice back afterwards. It also
                        // reads what is in the fields *now* rather than at last render.
                        let current = shell.read().composing.clone();
                        let saved = persist(&store, current.as_ref());
                        match saved {
                            Ok(_) => {
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
                        let store = use_context::<Arc<SqliteStore>>();
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
.images { font: inherit; font-size: 12px; padding: 4px 10px; border: 1px solid var(--edge); border-radius: 999px; background: none; color: inherit; cursor: pointer; margin-bottom: 8px; }
"#;
