use super::icon::{Glyph, Icon};
use super::text::{Kept, address, attachment_rows, from_name, stamp};
use crate::view::{Peek, Reading, Shell};
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
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
    // rather than at ingest — storing sanitized HTML would freeze today's sanitizer into every
    // row, and storing the unsanitized part would duplicate bytes we already have.
    // Each message resolved all the way to what the pane should draw, before the view tree.
    // `reading` sanitizes; `embed_inline` then resolves `cid:` inside what the sanitizer
    // allowed, in that order, because the sanitizer must judge the message's own URLs and not
    // a `data:` URI we substituted for one.
    let shown: Vec<(Message, Reading, bool)> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .map(|message| {
            let reading = crate::reader::render(&store, &message, policy);
            // Once the reader is allowed to fetch, the display pass blocks nothing, so it can
            // no longer say whether this message had a remote image. The blocked policy can:
            // that is the same question the offer was answered with.
            let remote = match &reading {
                Reading::Html {
                    blocked_remote: true,
                    ..
                } => true,
                Reading::Html { .. } if showing => matches!(
                    crate::reader::render(&store, &message, mail_mime::SanitizePolicy::CURRENT),
                    Reading::Html {
                        blocked_remote: true,
                        ..
                    }
                ),
                _ => false,
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
    let any_html = shown
        .iter()
        .any(|(_, reading, _)| matches!(reading, Reading::Html { .. }));

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
            if any_html {
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
