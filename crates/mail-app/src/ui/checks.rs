//! What the receiving server checked about a sender, as the reader's head shows it beside the
//! name: a small check when the sender is who they say, a plain warning when they may not be,
//! and nothing when nothing could be told. The method names stay out of the window; the raw
//! results are in Show Original for whoever wants them.
//!
//! The results are in the stored raw message ([`mail_core::auth`]), so finding them reads a blob.
//! That is done off the thread that draws, once per message and body, and remembered in the
//! app's [`Looks`](mail_core::message::Looks): the reader and the card ask for the same message
//! and the second finds it already known. A message whose body is not here shows nothing —
//! fetching it to find out would be a POP3 `RETR`, which marks it read.

use dioxus::prelude::*;
use ds::prelude::*;
use ds::style::icon::render::Glyph;
use mail_core::SqliteStore;
use mail_core::auth::{Standing, standing};
use mail_domain::{BlobId, MessageId};
use std::sync::Arc;

/// The line, once the answer is known; nothing before, and nothing for a message without a
/// believed field. Keyed by its parent on the message and its body, so a body arriving asks
/// again and one message's answer is never drawn under another.
#[component]
pub(in crate::ui) fn SenderChecks(message: MessageId, body: Option<BlobId>) -> Element {
    let looks = super::reading::use_looks();
    let mut known = use_signal({
        let looks = looks.clone();
        move || body.and_then(|raw| looks.checks_cached(message, raw))
    });
    let _look = use_resource(move || {
        let store = consume_context::<Arc<SqliteStore>>();
        let looks = looks.clone();
        async move {
            let Some(raw) = body else {
                return;
            };
            if known.peek().is_some() {
                return;
            }
            let results = tokio::task::spawn_blocking(move || looks.checks(&store, message, raw))
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
    rsx! {
        span {
            class: "sender-checks",
            "data-standing": standing.word(),
            match standing {
                Standing::Passed => rsx! {
                    Tooltip { text: "Verified Sender",
                        span { class: "sender-mark", aria_label: "Verified sender",
                            Glyph { icon: Icon::Check, size: IconSize::Compact }
                        }
                    }
                },
                Standing::Failed => rsx! {
                    span { class: "sender-mark",
                        Glyph { icon: Icon::TriangleAlert, size: IconSize::Compact }
                    }
                    span { "May not be from this sender" }
                },
                Standing::Unsure => rsx! {},
            }
        }
    }
}
