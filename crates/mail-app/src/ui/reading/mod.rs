mod blocks;
mod spans;
mod table;

use super::icon::{Glyph, Icon};
use super::text::{Kept, address, attachment_rows, from_name, stamp};
use crate::view::{Peek, Reading, Shell};
use blocks::MessageView;
use dioxus::prelude::*;
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
            button {
                r#type: "button",
                aria_label: "{reader}",
                aria_pressed: if showing { "false" } else { "true" },
                onclick: move |_| { original.write().insert(message_id, false); },
                "Reader"
            }
            button {
                r#type: "button",
                aria_label: "{original_label}",
                aria_pressed: if showing { "true" } else { "false" },
                onclick: move |_| { original.write().insert(message_id, true); },
                "Original"
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
    let pressed = if current == peek { "true" } else { "false" };
    rsx! {
        button {
            class: "tool",
            r#type: "button",
            aria_label: "{label}",
            aria_pressed: "{pressed}",
            onclick: move |_| shell.write().peek = peek,
            Glyph { icon, class: None }
        }
    }
}

#[component]
pub(super) fn Reader(thread: ThreadId, shell: Signal<Shell>) -> Element {
    let store = use_context::<Arc<SqliteStore>>();
    // Where the last attachment went, or why it did not. Cleared by opening another
    // conversation, because this component is rebuilt for each one.
    let mut saved = use_signal(|| None::<String>);
    // Which attachment is being fetched, if one is. That part's button stays disabled until
    // the fetch ends, so a second click cannot start a second download of it.
    let mut downloading = use_signal(|| None::<(MessageId, usize)>);
    // Which HTML message is showing its Original frame. Keyed by message, so
    // opening another one does not carry the choice over. The frame itself is
    // not created and destroyed with this flag.
    let original = use_signal(HashMap::<MessageId, bool>::new);
    // Quotes the reader has unfolded, keyed by message and path.
    let quotes = use_signal(blocks::OpenQuotes::new);
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
            let reading = crate::reader::render(&store, &message, policy);
            // Once the reader is allowed to fetch, the display pass blocks nothing, so it can
            // no longer say whether this message had a remote image. The blocked policy can:
            // that is the same question the offer was answered with.
            let remote = if reading.blocked_remote() {
                true
            } else if showing && reading.frame_html().is_some() {
                crate::reader::render(&store, &message, mail_mime::SanitizePolicy::CURRENT)
                    .blocked_remote()
            } else {
                false
            };
            (message, reading, remote)
        })
        .collect();

    let subject = loaded.summary.subject.clone();
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

    rsx! {
        div { class: "reader-head",
            div { class: "head-row",
                span { class: "spacer" }
                div { class: "bar-tools",
                    {peek_tool(Peek::Side, peek, Icon::Panel, shell)}
                    {peek_tool(Peek::Center, peek, Icon::Square, shell)}
                    {peek_tool(Peek::Full, peek, Icon::Maximize, shell)}
                }
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
                }
            }
        }
        div { class: "reader-body",
            if let Some(where_it_went) = saved() {
                // Where it went, named. A file saved somewhere the user cannot point at is a file
                // they have lost, and this pane's previous answer was to print a command to run.
                p { class: "notice", "{where_it_went}" }
            }
            if let Some(host) = from_host {
                div { class: "consent",
                    Glyph { icon: Icon::X, class: None }
                    span {
                        if showing {
                            "Showing remote images from {host}"
                        } else {
                            "Remote images blocked — loading them tells the sender you opened this"
                        }
                    }
                    if !showing {
                        button {
                            class: "images",
                            r#type: "button",
                            aria_label: "{show_images()}",
                            onclick: move |_| shell.write().show_remote_images = true,
                            "{show_images()}"
                        }
                    }
                }
            }
            for (message, reading, _) in shown {
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
                    // What is attached, if anything. Save writes a part that is already here;
                    // Download fetches one still on the server and then writes it. The name is
                    // the one the file will be written under. There is no file chooser: it lands
                    // in the downloads directory, and the notice says where.
                    if !attachment_rows(&message).is_empty() {
                        ul { class: "attachments",
                            for row in attachment_rows(&message) {
                                li { key: "{row.index}",
                                    Glyph { icon: Icon::Paperclip, class: None }
                                    span { class: "name", "{row.name}" }
                                    span { class: "size mono", "{row.size}" }
                                    button {
                                        class: "mini",
                                        disabled: downloading() == Some((message.id, row.index)),
                                        onclick: {
                                            let id = message.id;
                                            let index = row.index;
                                            let name = row.name.clone();
                                            let kept = row.kept;
                                            move |_| match kept {
                                                Kept::Here => {
                                                    let store = consume_context::<Arc<SqliteStore>>();
                                                    let where_to = crate::attach::downloads_dir();
                                                    saved.set(Some(
                                                        match crate::attach::save(
                                                            &store, id, index, &where_to,
                                                        ) {
                                                            Ok(path) => {
                                                                format!("Saved to {}", path.display())
                                                            }
                                                            Err(why) => why,
                                                        },
                                                    ));
                                                }
                                                Kept::OnServer => {
                                                    if downloading() == Some((id, index)) {
                                                        return;
                                                    }
                                                    saved.set(Some(format!("Downloading {name}…")));
                                                    downloading.set(Some((id, index)));
                                                    // Cloned out of the context into the blocking
                                                    // thread: the fetch outlives this click, and
                                                    // `fetch_part` holds the store for the whole
                                                    // download.
                                                    let store_arc =
                                                        consume_context::<Arc<SqliteStore>>();
                                                    let dir = crate::attach::downloads_dir();
                                                    spawn(async move {
                                                        // `spawn_blocking`, not this task:
                                                        // `fetch_part` opens sockets and builds its
                                                        // own runtime, and `Runtime::block_on`
                                                        // inside an async context panics.
                                                        let done = tokio::task::spawn_blocking(move || {
                                                            crate::attach::fetch_and_save(
                                                                &store_arc,
                                                                id,
                                                                index,
                                                                &dir,
                                                                |section| {
                                                                    crate::sync::fetch_part(
                                                                        &store_arc,
                                                                        id,
                                                                        section,
                                                                        chrono::Utc::now(),
                                                                    )
                                                                },
                                                            )
                                                        })
                                                        .await;
                                                        let sentence = match done {
                                                            Ok(Ok(sentence) | Err(sentence)) => sentence,
                                                            Err(error) => format!(
                                                                "The download stopped before it finished: {error}"
                                                            ),
                                                        };
                                                        saved.set(Some(sentence));
                                                        downloading.set(None);
                                                    });
                                                }
                                            }
                                        },
                                        if downloading() == Some((message.id, row.index)) {
                                            "Downloading…"
                                        } else {
                                            match row.kept {
                                                Kept::Here => "Save",
                                                Kept::OnServer => "Download",
                                            }
                                        }
                                    }
                                }
                            }
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
                        }
                    }
                }
            }
            if any_frame {
                div { class: "frame-note",
                    Glyph { icon: Icon::Key, class: None }
                    span { "sandboxed frame · no scripts, no same-origin" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
