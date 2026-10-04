//! Hover state for the window: link previews and shared clipboard helper.

mod link;

pub(super) use link::{LinkPill, link_out, link_over, url_spans};

use dioxus::prelude::*;
use mail_core::trust::Destination;

/// Put an address or text on the clipboard.
pub(in crate::ui) fn copy(text: &str) {
    crate::ui::host::Host::copy(text);
}

/// The hover state the window shares: link previews in the reader.
#[derive(Clone, Copy)]
pub(super) struct Hover {
    /// The link under the pointer in the reader, if any. Instant: no timer.
    pub link: Signal<Option<Destination>>,
}

/// Make the hover state for the window. Called once, from `App`.
pub(super) fn use_hover() -> Hover {
    let hover = use_context_provider(|| Hover {
        link: Signal::new(None),
    });
    use_frame_pill(hover);
    hover
}

/// On Blitz, a link in an Original frame is reported by quire as the pointer crosses it, outside
/// any component (`ui/original/links.rs`): this sets the same `link` a Reader view link sets, read
/// through the same honesty check, and clears it as the pointer leaves. One report per crossing,
/// so nothing to debounce.
fn use_frame_pill(mut hover: Hover) {
    let pill = use_hook(try_consume_context::<super::original::FramePill>);
    use_future(move || {
        let pill = pill.clone();
        async move {
            let Some(mut pill) = pill else {
                return;
            };
            while let Some(pointed) = pill.next().await {
                hover
                    .link
                    .set(pointed.map(|link| mail_core::trust::destination(&link.text, &link.href)));
            }
        }
    });
}

/// The window's hover state, when there is a window around the caller.
pub(super) fn hover() -> Option<Hover> {
    try_consume_context::<Hover>()
}
