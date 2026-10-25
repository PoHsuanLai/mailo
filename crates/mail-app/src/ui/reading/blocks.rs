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
    /// The message the frame draws, for ⌘F to look through (`super::find`), which a frame drawn
    /// with no reader around it is not.
    #[props(default)]
    message: Option<MessageId>,
) -> Element {
    let height = use_signal(|| None::<f32>);
    // A sender's page wider than the frame is drawn smaller to fit it (`super::fit::shrink`); a
    // plain-text message wraps to the frame and is never wider.
    let zoom = use_signal(|| 1.0_f32);
    let html = super::fit::zoomed(&html, zoom());
    let found = use_hook(try_consume_context::<super::find::Frames>);
    // A new document in the frame (a zoom to fit is one) holds none of what ⌘F marked.
    {
        let found = found.clone();
        use_effect(use_reactive!(|html| {
            // Read for its changes alone: each is a new document.
            let _ = html;
            if let Some(found) = &found {
                found.changed();
            }
        }));
    }
    {
        let found = found.clone();
        use_drop(move || {
            if let (Some(found), Some(message)) = (found, message) {
                found.gone(message);
            }
        });
    }
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
                if let (Some(found), Some(message)) = (&found, message) {
                    found.drawn(message, event.data());
                }
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
            message: Some(message_id),
        }
    }
}
