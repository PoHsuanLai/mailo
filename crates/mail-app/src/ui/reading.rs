use super::text::{Kept, address, attachment_rows, from_name, stamp};
use crate::view::{Reading, Shell};
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

#[component]
pub(super) fn Reader(thread: ThreadId, shell: Signal<Shell>) -> Element {
    let store = use_context::<Arc<SqliteStore>>();
    // Where the last attachment went, or why it did not. Cleared by opening another
    // conversation, because this component is rebuilt for each one.
    let mut saved = use_signal(|| None::<String>);
    // Which attachment is being fetched, if one is. That part's button stays disabled until
    // the fetch ends, so a second click cannot start a second download of it.
    let mut downloading = use_signal(|| None::<(MessageId, usize)>);
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
        if let Some(where_it_went) = saved() {
            // Where it went, named. A file saved somewhere the user cannot point at is a file
            // they have lost, and this pane's previous answer was to print a command to run.
            p { class: "notice", "{where_it_went}" }
        }
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
                // What is attached, if anything. Save writes a part that is already here;
                // Download fetches one still on the server and then writes it. The name is
                // the one the file will be written under. There is no file chooser: it lands
                // in the downloads directory, and the notice says where.
                if !attachment_rows(&message).is_empty() {
                    ul { class: "attachments",
                        for row in attachment_rows(&message) {
                            li { key: "{row.index}",
                                span { class: "paperclip", "📎" }
                                span { class: "name", "{row.name}" }
                                span { class: "size", "{row.size}" }
                                button {
                                    class: "ghost",
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
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::fixtures::{held_and_remote, reader_markup, realistic, thread_like};

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

    /// The `<li>` elements of the attachment list, each as rendered.
    ///
    /// Taken from that list alone. A `contains("Download")` over the whole page would pass for
    /// any screen that mentioned the word.
    fn attachment_items(markup: &str) -> Vec<&str> {
        let Some((_, rest)) = markup.split_once(r#"<ul class="attachments">"#) else {
            panic!("the reader drew no attachment list:\n{markup}");
        };
        let Some((list, _)) = rest.split_once("</ul>") else {
            panic!("the attachment list was not closed:\n{markup}");
        };
        let mut items = Vec::new();
        let mut rest = list;
        while let Some(at) = rest.find("<li>") {
            let from = &rest[at..];
            let Some(close) = from.find("</li>") else {
                panic!("an attachment row was not closed:\n{markup}");
            };
            items.push(&from[..close + "</li>".len()]);
            rest = &from[close + "</li>".len()..];
        }
        items
    }

    #[tokio::test]
    async fn a_held_part_offers_save_and_a_remote_part_offers_download() {
        let (store, _dir) = held_and_remote();
        let thread = thread_like(&store, "quarterly");
        let markup = reader_markup(store, thread);
        assert_eq!(
            attachment_items(&markup),
            [
                r#"<li><span class="paperclip">📎</span><span class="name">notes.txt</span><span class="size">1.5 kB</span><button class="ghost">Save</button></li>"#,
                r#"<li><span class="paperclip">📎</span><span class="name">report.pdf</span><span class="size">up to 5.0 MB</span><button class="ghost">Download</button></li>"#,
            ],
            "the rows are not Save for the held part and Download for the remote one:\n{markup}"
        );
    }
}
