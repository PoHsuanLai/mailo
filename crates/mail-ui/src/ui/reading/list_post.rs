//! Whether the open conversation's newest message names its list's address (`List-Post`), for
//! the ⋯'s Reply to List.
//!
//! The address is in the stored raw message, so finding it reads a blob: once per conversation
//! and bodies, off the thread that draws, and only for the conversation the reader has open
//! ([`mail_core::message::Looks::post`]), as the unsubscribe lookout does for the way out.

use dioxus::prelude::*;
use mail_client::message::Bodies;
use mail_domain::ThreadId;

/// Finds it, and puts down `tag` in `found` when it is there. Draws nothing.
///
/// Keyed by the reader on the conversation and its bodies, and the answer is put down with that
/// key, so Reply to List is offered under the key it was found for and no other.
#[component]
pub(super) fn PostLookout(
    thread: ThreadId,
    bodies: Bodies,
    tag: String,
    found: Signal<Option<String>>,
) -> Element {
    let looks = super::use_looks();
    let _look = use_resource(move || {
        let bodies = bodies.clone();
        let looks = looks.clone();
        let tag = tag.clone();
        let client = mail_ui_kit::live::client();
        let mut found = found;
        async move {
            let post = match looks.post_cached(thread, &bodies) {
                Some(post) => post,
                None => tokio::task::spawn_blocking(move || {
                    client.looking(&looks).post(thread, &bodies)
                })
                .await
                // A lookup that panicked found nothing, which offers nothing.
                .ok()
                .flatten(),
            };
            found.set(post.map(|_| tag));
        }
    });
    rsx! {}
}
