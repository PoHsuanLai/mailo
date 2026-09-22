use super::text::{address, attached, from_name, stamp};
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
                // What is attached, if anything. Named and sized but not saved from here: a
                // file dialog is the one piece this pane cannot do, and a list that tells the
                // user a file exists — and what `mailo save` will call it — beats a message
                // that looks like it has nothing in it.
                if !attached(&message).is_empty() {
                    ul { class: "attachments",
                        for (index, item) in attached(&message) {
                            li { key: "{index}",
                                span { class: "paperclip", "📎" }
                                span { class: "name", "{item.0}" }
                                span { class: "size", "{item.1}" }
                                button {
                                    class: "ghost",
                                    onclick: {
                                        let id = message.id;
                                        move |_| {
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
                                    },
                                    "Save"
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
    use crate::ui::fixtures::{reader_markup, realistic, thread_like};

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
}
