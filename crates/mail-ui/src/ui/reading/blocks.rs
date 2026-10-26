//! The sandboxed frame displaying the message body.

use dioxus::prelude::*;
use mail_domain::MessageId;

/// The sandboxed frame. `tag` is the frame's `data-frame-tag` (`Holder::tag`): on Blitz the
/// network reads it to know which reader drew the frame and which message's consented images it
/// may fetch, and an untagged frame fetches none (`ui/original/net.rs`).
#[component]
pub(super) fn Sandbox(
    html: String,
    #[props(default)] tag: Option<String>,
    /// A plain-text message's frame, which has no ground of its own (`FrameBody::Present`).
    #[props(default)]
    plain: bool,
    /// An earlier message's frame, as tall as what it holds (`super::fit`); the last message's
    /// takes the room left instead.
    #[props(default)]
    fit: bool,
) -> Element {
    let height = use_signal(|| None::<f32>);
    // A sender's page wider than the frame is drawn smaller to fit it (`super::fit::shrink`); a
    // plain-text message wraps to the frame and is never wider.
    let zoom = use_signal(|| 1.0_f32);
    let html = super::fit::zoomed(&html, zoom());
    let style = match (fit, height()) {
        (true, Some(px)) => format!("height: {px}px;"),
        _ => String::new(),
    };
    rsx! {
        iframe {
            class: "html",
            "data-plain": plain,
            "data-fit": fit,
            style,
            onmounted: move |event: MountedEvent| {
                if fit {
                    super::fit::follow(event.data(), height);
                }
                if !plain {
                    super::fit::shrink(event.data(), zoom);
                }
            },
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
    #[props(default)] plain: bool,
    #[props(default)] fit: bool,
) -> Element {
    rsx! {
        Sandbox {
            html,
            plain,
            fit,
            tag: holder.map(|holder| holder.tag(message_id)),
        }
    }
}
