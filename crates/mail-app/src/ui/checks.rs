//! What the receiving server checked about a sender, as the reader's head and the sender card
//! show it: one line under the name, "SPF pass · DKIM pass · DMARC pass — checked by …".
//!
//! The results are in the stored raw message ([`crate::auth`]), so finding them reads a blob.
//! That is done off the thread that draws, once per message and body, and remembered: the reader
//! and the card ask for the same message and the second finds it already known. A message whose
//! body is not here shows nothing — fetching it to find out would be a POP3 `RETR`, which marks
//! it read.

use crate::auth::{Standing, sentence, standing};
use dioxus::prelude::*;
use ds::{Glyph, Icon, IconSize};
use mail_domain::{BlobId, MessageId};
use mail_mime::AuthResults;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// How many messages' answers to keep. An answer is a few short strings.
const KEPT: usize = 128;

type Known = Vec<(MessageId, BlobId, Option<AuthResults>)>;

static CACHE: std::sync::Mutex<Known> = std::sync::Mutex::new(Vec::new());

/// The answer already found for `message` with body `raw`, if one has been. `Some(None)` is a
/// message that was read and has no field this client believes.
fn cached(message: MessageId, raw: BlobId) -> Option<Option<AuthResults>> {
    let cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache
        .iter()
        .find(|(id, body, _)| *id == message && *body == raw)
        .map(|(_, _, results)| results.clone())
}

/// [`cached`], else read from the stored bytes and remembered. Blocking: runs on a blocking
/// thread.
fn lookup(store: &SqliteStore, message: MessageId, raw: BlobId) -> Option<AuthResults> {
    if let Some(had) = cached(message, raw) {
        return had;
    }
    let results = store
        .message(message)
        .ok()
        .and_then(|message| crate::auth::results_of(store, &message));
    let mut cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache.retain(|(id, _, _)| *id != message);
    cache.push((message, raw, results.clone()));
    if cache.len() > KEPT {
        cache.remove(0);
    }
    results
}

/// The glyph a standing is drawn with.
fn glyph(standing: Standing) -> Icon {
    match standing {
        Standing::Passed => Icon::Check,
        Standing::Failed => Icon::X,
        Standing::Unsure => Icon::Key,
    }
}

/// The line, once the answer is known; nothing before, and nothing for a message without a
/// believed field. Keyed by its parent on the message and its body, so a body arriving asks
/// again and one message's answer is never drawn under another.
#[component]
pub(in crate::ui) fn SenderChecks(message: MessageId, body: Option<BlobId>) -> Element {
    let mut known = use_signal(move || body.and_then(|raw| cached(message, raw)));
    let _look = use_resource(move || {
        let store = consume_context::<Arc<SqliteStore>>();
        async move {
            let Some(raw) = body else {
                return;
            };
            if known.peek().is_some() {
                return;
            }
            let results = tokio::task::spawn_blocking(move || lookup(&store, message, raw))
                .await
                .ok()
                .flatten();
            known.set(Some(results));
        }
    });
    let Some(Some(results)) = known() else {
        return rsx! {};
    };
    let standing = standing(&results);
    let said = sentence(&results);
    rsx! {
        div {
            class: "sender-checks",
            "data-standing": standing.word(),
            Glyph { icon: glyph(standing), size: IconSize::Small }
            span { "{said}" }
        }
    }
}
