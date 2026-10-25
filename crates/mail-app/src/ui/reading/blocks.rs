//! The sandboxed frame displaying the message body.

use dioxus::core::Task;
use dioxus::prelude::*;
use mail_domain::MessageId;
use std::rc::Rc;

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
    // The reader's text size (`reading.text_zoom`), read here so a change draws the frame again;
    // a frame with no window around it (a test's) is at its own size.
    let settings = use_hook(try_consume_context::<Signal<crate::settings::MailSettings>>);
    let text = settings.map_or(1.0, |settings| settings.read().reading.text_zoom.factor());
    let height = use_signal(|| None::<f32>);
    // The page is drawn at the text size, and a sender's page that would then be wider than the
    // frame smaller, to fit it (`super::fit::shrink`); a plain-text message wraps to the frame
    // and is never wider.
    let mut zoom = use_signal(|| text);
    let mut mounted = use_signal(|| None::<Rc<MountedData>>);
    let mut reading = use_signal(Vec::<Task>::new);
    // Measure the frame again from the start, for the text size `text`: what was being measured
    // for another size stops, so it cannot set the zoom back.
    let mut measure = move |text: f32| {
        for task in reading.write().drain(..) {
            task.cancel();
        }
        let Some(frame) = mounted.peek().clone() else {
            return;
        };
        let mut started = Vec::new();
        if fit {
            started.push(super::fit::follow(frame.clone(), height));
        }
        if !plain {
            started.push(super::fit::shrink(frame, zoom, text));
        }
        reading.set(started);
    };
    // The size the frame was last measured for: a new one starts the page at it and measures
    // again.
    let mut measured = use_signal(|| text);
    use_effect(use_reactive!(|text| {
        if (*measured.peek() - text).abs() >= 0.001 {
            measured.set(text);
            zoom.set(text);
            measure(text);
        }
    }));
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
                mounted.set(Some(event.data()));
                measure(text);
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
