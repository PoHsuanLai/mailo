//! The sandboxed frame displaying the message body.

use dioxus::prelude::*;
use mail_domain::MessageId;

/// The sandboxed frame. `tag` is the frame's `data-frame-tag` (`Holder::tag`): on Blitz the
/// network reads it to know which reader drew the frame and which message's consented images it
/// may fetch, and an untagged frame fetches none (`ui/original/net.rs`).
#[component]
pub(super) fn Sandbox(html: String, #[props(default)] tag: Option<String>) -> Element {
    rsx! {
        iframe {
            class: "html",
            "data-frame-tag": tag,
            "sandbox": "",
            srcdoc: "{html}",
            title: "The message as the sender laid it out",
        }
    }
}

/// One message body, drawn as the sandboxed frame.
#[component]
pub(super) fn MessageView(
    message_id: MessageId,
    html: String,
    #[props(default)] holder: Option<crate::ui::original::Holder>,
) -> Element {
    rsx! {
        Sandbox {
            html,
            tag: holder.map(|holder| holder.tag(message_id)),
        }
    }
}
