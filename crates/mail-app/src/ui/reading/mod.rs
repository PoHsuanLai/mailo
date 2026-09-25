mod attachments;
mod blocks;
mod find_bar;
mod found;
mod image;
#[cfg(feature = "native")]
mod remote;
mod spans;
mod table;

use super::press::on_primary;
use super::text::{address, attachment_rows, from_name, stamp};
use crate::view::{Peek, Reading, Shell};
use attachments::Attachments;
use blocks::MessageView;
use dioxus::prelude::*;
use ds::{Glyph, Icon, IconButton, IconButtonVariant, Switch};
pub(super) use find_bar::open_find;
use find_bar::{FindBar, marking};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(test)]
thread_local! {
    static READER_MOUNTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// How many times [`Reader`] has mounted on this thread.
///
/// A re-render leaves it alone. A component that was thrown away and built again increments
/// it, which is the difference a peek-mode change must not make.
#[cfg(test)]
pub(super) fn reader_mounts() -> u32 {
    READER_MOUNTS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(super) fn reset_reader_mounts() {
    READER_MOUNTS.with(|mounts| mounts.set(0));
}

/// The first character of `name`, uppercased.
///
/// `chars`, not bytes: a CJK name's first byte is not a letter.
fn initial(name: &str) -> String {
    match name.chars().next() {
        Some(c) => c.to_uppercase().collect(),
        None => String::new(),
    }
}

/// Reader / Original. A span, so it can sit in the header without becoming the
/// iframe's parent.
#[component]
fn ViewSwitch(message_id: MessageId, mut original: Signal<HashMap<MessageId, bool>>) -> Element {
    let showing = original.read().get(&message_id) == Some(&true);
    let reader = "Reader";
    let original_label = "Original";
    rsx! {
        span { class: "view-switch", role: "group", aria_label: "How to show this message",
            ds::Button {
                variant: ds::ButtonVariant::Mini,
                label: reader,
                aria_label: reader.to_owned(),
                pressed: if showing { ds::Switch::Off } else { ds::Switch::On },
                onclick: on_primary(move || {
                    original.write().insert(message_id, false);
                }),
            }
            ds::Button {
                variant: ds::ButtonVariant::Mini,
                label: original_label,
                aria_label: original_label.to_owned(),
                pressed: if showing { ds::Switch::On } else { ds::Switch::Off },
                onclick: on_primary(move || {
                    original.write().insert(message_id, true);
                }),
            }
        }
    }
}

fn sender_initial(message: &Message) -> String {
    let named = message.from.name.as_deref().filter(|name| !name.is_empty());
    initial(named.unwrap_or(message.from.email.as_str()))
}

/// The host a remote image would report the open to.
fn host_of(email: &str) -> &str {
    match email.rsplit_once('@') {
        Some((_, host)) if !host.is_empty() => host,
        _ => email,
    }
}

fn show_images() -> &'static str {
    "Show images"
}

fn peek_tool(peek: Peek, current: Peek, icon: Icon, mut shell: Signal<Shell>) -> Element {
    let label = peek.label();
    let pressed = if current == peek {
        Switch::On
    } else {
        Switch::Off
    };
    rsx! {
        IconButton {
            variant: IconButtonVariant::Tool,
            icon,
            label: label.to_owned(),
            pressed,
            onclick: move |_| shell.write().peek = peek,
        }
    }
}

/// The key [`super::unsubscribe::Leave`] is mounted under: the thread and what each of its
/// messages holds, so a body arriving asks again and another thread never shows this one's answer.
fn leave_key(thread: ThreadId, bodies: &super::unsubscribe::Bodies) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bodies.hash(&mut hasher);
    format!("{thread}-{:x}", hasher.finish())
}

/// `revision` is the window's, moved when leaving a list queues a message; a reader drawn on its
/// own has none.
#[component]
pub(super) fn Reader(
    thread: ThreadId,
    shell: Signal<Shell>,
    revision: Option<Signal<u64>>,
    children: Element,
) -> Element {
    let store = use_context::<Arc<SqliteStore>>();
    // Where the last attachment went, or why it did not. Cleared by opening another
    // conversation, because this component is rebuilt for each one.
    let saved = use_signal(|| None::<String>);
    // Which attachment is being fetched, if one is. That part's button stays disabled until
    // the fetch ends, so a second click cannot start a second download of it.
    let downloading = use_signal(|| None::<(MessageId, usize)>);
    // Which HTML message is showing its Original frame. Keyed by message, so
    // opening another one does not carry the choice over. The frame itself is
    // not created and destroyed with this flag.
    let original = use_signal(HashMap::<MessageId, bool>::new);
    // Quotes the reader has unfolded, keyed by message and path.
    let quotes = use_signal(blocks::OpenQuotes::new);
    // Moved when an OpenPGP or S/MIME message has been opened and has a body of its own to show,
    // so the messages are drawn again with it. The opening itself happens in `pgp::Seal`, off
    // this thread; here it is only looked up.
    let landed = use_signal(|| 0u64);
    let _ = landed();
    // The window's consent to remote images, and this reader's claim on it; closing the reader
    // takes back what it granted.
    let consent = use_hook(|| {
        try_consume_context::<super::original::Consent>().map(|consent| {
            let holder = consent.holder();
            (consent, holder)
        })
    });
    {
        let consent = consent.clone();
        use_drop(move || {
            if let Some((consent, holder)) = consent {
                consent.release(holder);
            }
        });
    }
    // The Reader view's remote images on Blitz, which mailo fetches itself: held to this
    // render's thread and consent, and fetched by `remote::Fetcher`'s effect (`remote.rs`).
    #[cfg(feature = "native")]
    {
        let pictures = use_context_provider(remote::Pictures::new);
        pictures.hold(thread, shell.read().show_remote_images);
    }
    #[cfg(test)]
    use_hook(|| {
        READER_MOUNTS.with(|mounts| mounts.set(mounts.get().saturating_add(1)));
    });
    let Ok(loaded) = store.thread(thread) else {
        return rsx! {
            div { class: "reader-empty",
                p { "That conversation is gone." }
            }
        };
    };
    let policy = shell.read().policy();
    let showing = shell.read().show_remote_images;
    let peek = shell.read().peek;
    // The HTML part is not a column: it lives inside the stored raw message, which is the only
    // copy that is byte-for-byte what the server sent. Parsed here, once per render of a thread,
    // rather than at ingest — storing the blocks would freeze today's limits into every row.
    // Images resolve in that walk. The frame, when there is one, is the sanitized markup.
    let shown: Vec<(Message, Reading, bool)> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .map(|message| {
            // A protected message opened already shows what it was opened to; everything else,
            // and one not opened yet, shows what is stored.
            let render = |policy| {
                super::pgp::reading(&message, policy)
                    .unwrap_or_else(|| crate::reader::render(&store, &message, policy))
            };
            let reading = render(policy);
            // Once the reader is allowed to fetch, the display pass blocks nothing, so it can
            // no longer say whether this message had a remote image. The blocked policy can:
            // that is the same question the offer was answered with.
            let remote = if reading.blocked_remote() {
                true
            } else if showing && reading.frame_html().is_some() {
                render(mail_mime::SanitizePolicy::CURRENT).blocked_remote()
            } else {
                false
            };
            (message, reading, remote)
        })
        .collect();

    // The consent, as the Original frames' network reads it (`ui/original`): written here, in the
    // render, so a frame that reloads with the consented markup finds it already granted, and
    // taken back by the same render that stops showing the images (open, select, close all
    // clear `show_remote_images`). Only the window on Blitz provides one; the webview's frames
    // are held by the markup alone.
    if let Some((consent, holder)) = &consent {
        let allowed = showing.then(|| {
            shown
                .iter()
                .map(|(message, reading, _)| (message.id, reading.frame_fetches().to_vec()))
                .collect()
        });
        consent.hold(*holder, thread, allowed);
    }

    // What the list lookup is a function of. Ids and blob ids only: nothing here reads a blob.
    let bodies: super::unsubscribe::Bodies = shown
        .iter()
        .map(|(message, _, _)| (message.id, message.body.raw()))
        .collect();
    let leave_key = leave_key(thread, &bodies);
    // An encrypted message's subject travels inside it; the outside says `...`.
    let subject = shown
        .iter()
        .find(|(message, _, _)| message.subject == loaded.summary.subject)
        .and_then(|(message, _, _)| super::pgp::subject(message))
        .unwrap_or_else(|| loaded.summary.subject.clone());
    let meta = shown.last().map(|(message, _, _)| {
        (
            sender_initial(message),
            from_name(message),
            address(message),
            stamp(message),
        )
    });
    let from_host = shown
        .iter()
        .rev()
        .find(|(_, _, remote)| *remote)
        .map(|(message, _, _)| host_of(&message.from.email).to_owned());
    let any_frame = shown
        .iter()
        .any(|(_, reading, _)| reading.frame_html().is_some());
    // Ctrl F's marks, or the list search's while no find is open. Blocks only: the frame is
    // never read and never marked.
    let (highlight, problem) = marking(&shell.read());
    let finding = shell.read().find.clone();
    let documents: Vec<_> = shown
        .iter()
        .map(|(_, reading, _)| reading.document())
        .collect();
    let (founds, total) = found::find_in(&documents, &highlight, finding.as_ref());
    // On Blitz, what of this render mailo fetches itself: the consented remote images it draws.
    #[cfg(feature = "native")]
    let fetcher = {
        let wanted = if showing {
            remote::wanted(documents.iter().flatten().copied())
        } else {
            Vec::new()
        };
        rsx! { remote::Fetcher { thread, wanted } }
    };
    #[cfg(not(feature = "native"))]
    let fetcher = rsx! {};
    let invalid = problem.is_some();
    // A protected message opened to a body lists what is attached inside it; its stored parts
    // are its wrapping.
    let attached: Vec<_> = shown
        .iter()
        .map(|(message, _, _)| {
            super::pgp::attachments(message).unwrap_or_else(|| attachment_rows(message))
        })
        .collect();

    rsx! {
        {fetcher}
        div { class: "reader-head",
            div { class: "head-row",
                if finding.is_some() {
                    FindBar { shell, total, invalid }
                } else {
                    span { class: "spacer" }
                }
                div { class: "bar-tools",
                    if let Some(revision) = revision {
                        super::move_to::MoveTool { thread, shell, revision }
                    }
                    super::print::PrintTool { thread }
                    {peek_tool(Peek::Side, peek, Icon::Panel, shell)}
                    {peek_tool(Peek::CENTER, peek, Icon::Square, shell)}
                    {peek_tool(Peek::FULL, peek, Icon::Maximize, shell)}
                }
            }
            if let Some(why) = problem {
                pre { class: "find-err mono", "{why}" }
            }
            h2 { "{subject}" }
            if let Some((initial, from, addr, when)) = meta {
                div { class: "reader-meta",
                    div { class: "reader-av", "{initial}" }
                    div {
                        div { class: "reader-from", "{from}" }
                        div { class: "mono reader-addr", "{addr}" }
                        div { class: "mono when", "{when}" }
                    }
                    // Its own template, so the key is that template's root key and a new one
                    // remounts it: rsx reads a key only on a template's root node, and one on a
                    // nested component is dropped, in a release build and a debug one alike.
                    {rsx! { super::unsubscribe::Leave { key: "{leave_key}", thread, bodies: bodies.clone(), revision } }}
                }
            }
            // Under the head, where a question about this message belongs. Keyed like Leave.
            {rsx! { super::receipt::Receipts { key: "{leave_key}", bodies } }}
        }
        div { class: "reader-body",
            if let Some(where_it_went) = saved() {
                // Where it went, named. A file saved somewhere the user cannot point at is a file
                // they have lost, and this pane's previous answer was to print a command to run.
                p { class: "notice", "{where_it_went}" }
            }
            if let Some(host) = from_host {
                div { class: "consent",
                    Glyph { icon: Icon::X }
                    span {
                        if showing {
                            "Showing remote images from {host}"
                        } else {
                            "Remote images blocked — loading them tells the sender you opened this"
                        }
                    }
                    if !showing {
                        ds::Button {
                            variant: ds::ButtonVariant::Mini,
                            label: show_images(),
                            aria_label: show_images(),
                            // The press takes the button away, and on Blitz the keyboard with
                            // it (quire focuses the pressed button a frame later, gone or not):
                            // it is handed back to the window, as a closing panel hands it back.
                            onclick: on_primary(move || {
                                shell.write().show_remote_images = true;
                                super::host::Host::focus_app();
                            }),
                        }
                    }
                }
            }
            for (((message, reading, _), found), attached) in shown.into_iter().zip(founds).zip(attached) {
                article { key: "{message.id}", class: "frame",
                    header {
                        strong { "{from_name(&message)}" }
                        span { class: "mono", "{address(&message)}" }
                        time { class: "mono", "{stamp(&message)}" }
                        if reading.frame_html().is_some() {
                            // Offered wherever there is a frame, not only for
                            // `Reading::Layout`: the frame is mounted for every HTML body,
                            // and Original is the escape hatch when the blocks got a
                            // message wrong. The mockup offers it on the receipt too.
                            // A span, not a div: a div between the article and its iframe
                            // is a new parent, and a new parent reloads the frame.
                            // The labels are computed so a test can find the control: a
                            // literal attribute never appears in the render mutations.
                            ViewSwitch { message_id: message.id, original }
                        }
                    }
                    // What the message's OpenPGP or S/MIME says, and its passphrase field. A
                    // sibling before the frame, like the invitation under it, for the same reason.
                    super::pgp::Seal {
                        key: "{message.id}-{message.body.raw():?}",
                        message: message.id,
                        body: message.body.raw(),
                        landed,
                    }
                    // A calendar invitation, drawn by this window and never inside the sender's
                    // HTML. A sibling before the frame, not its parent: it lands after the first
                    // paint, and inserting a sibling does not move the iframe. Keyed on the
                    // message and its body, so a body arriving asks again.
                    super::invite::Invitation {
                        key: "{message.id}-{message.body.raw():?}",
                        message: message.id,
                        body: message.body.raw(),
                    }
                    // What is attached, if anything: what was sent, or what a protected
                    // message was opened to holds. Keyed like the seal, so a body arriving asks
                    // again.
                    if !attached.is_empty() {
                        Attachments {
                            key: "{message.id}-{message.body.raw():?}",
                            message: message.id,
                            body: message.body.raw(),
                            rows: attached,
                            saved,
                            downloading,
                        }
                    }
                    // The iframe, when this message has one, is the first element MessageView
                    // draws, and it is drawn on every render. Toggling Reader / Original changes
                    // a class. Conditionally rendering the iframe would reload it: a new parent,
                    // or a frame that was not in the tree, re-runs the document, loses scroll,
                    // and re-fetches anything just consented to.
                    if matches!(reading, Reading::NotFetched) {
                        p { class: "pending", "Body not downloaded yet." }
                    } else {
                        MessageView {
                            message_id: message.id,
                            reading: reading.clone(),
                            original,
                            quotes,
                            shell,
                            found,
                        }
                    }
                }
            }
            if any_frame {
                div { class: "frame-note",
                    Glyph { icon: Icon::Key, size: ds::IconSize::Small }
                    span { "sandboxed frame · no scripts, no same-origin" }
                }
            }
            // An inline reply, after every frame so no iframe gains a new parent.
            {children}
        }
    }
}

/// The Original frame as the reader draws it, in mailo's stylesheet, and nothing else: for the
/// guarantee tests on Blitz (`tests/native_frame.rs`), which put markup in it that the sanitizer
/// would never have let through. A test binary of its own, because a Blitz document replaces the
/// process's event converter, which the webview fixtures in this crate's unit tests rely on.
#[cfg(feature = "native")]
#[component]
pub fn OriginalFrame(html: String) -> Element {
    rsx! {
        style { {ds::stylesheet()} }
        style { {super::style::STYLE} }
        div { class: "reader-body",
            article { class: "frame",
                blocks::Sandbox { html, concealed: false }
            }
        }
    }
}

#[cfg(test)]
mod find_tests;
#[cfg(test)]
mod tests;
