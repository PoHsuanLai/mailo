//! Hover previews: wait for intent, stay warm, never mark read, never fetch.
//!
//! The rules are quire's hover hub's (450 ms to open, none while warm, 150 ms of grace after
//! the pointer leaves, warm for 400 ms after a close); this is the window's half. A hook (a
//! row, a sender's name, a time, a pinned tile, a Today tab) reports the pointer to quire's
//! [`HoverDriver`], and one `HoverCard` is drawn for whatever the hub says is open.
//!
//! **Nothing here writes.** A card reads what the store already holds — the thread row, the
//! messages' stored text, the sender history — and never marks anything read, never loads a
//! remote image and never downloads a body. A POP3 `RETR` marks the message read on the
//! server, which is why a body that is not here says "not downloaded" rather than going to get it.

mod cards;
mod link;
mod sender;

pub(super) use cards::HoverLayer;
pub(super) use link::{LinkPill, link_out, link_over, url_spans};
pub(super) use sender::copy;

use crate::trust::Destination;
use dioxus::prelude::*;
use ds::{
    HoverAnchor, HoverDriver, HoverEvent, HoverHub, HoverKey, HoverKind, MountedRef, Point, Px,
    Rect, Size,
};
use mail_domain::ThreadId;

/// Where a card can be asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hook {
    /// A row: the thread card, beside the list.
    Thread(ThreadId),
    /// The sender's name in a row.
    Sender(ThreadId),
    /// The time in a row.
    Time(ThreadId),
    /// A pinned person or search in the sidebar, by position.
    Pin(usize),
    /// A Today tab.
    Today(ThreadId),
}

impl Hook {
    /// quire's key for it: the same words the hook's element names itself by (`data-hc`).
    fn key(self) -> HoverKey {
        HoverKey(match self {
            Hook::Thread(id) => format!("thread:{id}"),
            Hook::Sender(id) => format!("sender:{id}"),
            Hook::Time(id) => format!("time:{id}"),
            Hook::Pin(index) => format!("pin:{index}"),
            Hook::Today(id) => format!("today:{id}"),
        })
    }

    /// Which of quire's cards it opens, which is where the card is placed.
    fn kind(self) -> HoverKind {
        match self {
            Hook::Thread(_) => HoverKind::Thread,
            Hook::Sender(_) => HoverKind::Sender,
            Hook::Time(_) => HoverKind::Tip,
            Hook::Pin(_) | Hook::Today(_) => HoverKind::Side,
        }
    }

    /// The hook a key names, read back.
    fn of(key: &HoverKey) -> Option<Hook> {
        let (kind, rest) = key.0.split_once(':')?;
        match kind {
            "pin" => rest.parse().ok().map(Hook::Pin),
            _ => {
                let id = ThreadId::from_uuid(rest.parse::<uuid::Uuid>().ok()?);
                match kind {
                    "thread" => Some(Hook::Thread(id)),
                    "sender" => Some(Hook::Sender(id)),
                    "time" => Some(Hook::Time(id)),
                    "today" => Some(Hook::Today(id)),
                    _ => None,
                }
            }
        }
    }
}

/// The hover state the window shares beyond quire's hub. Signals, so it is `Copy` and every
/// hook holds it.
#[derive(Clone, Copy)]
pub(super) struct Hover {
    /// The link under the pointer in the reader, if any. Instant: no timer.
    pub link: Signal<Option<Destination>>,
    /// quire's driver, as the card layer inside the root found it: the window's keys are heard
    /// above the root, where the hub is not in context. Not a signal: nothing redraws for it.
    driver: CopyValue<Option<HoverDriver>>,
}

/// Make the hover state for the window. Called once, from `App`.
pub(super) fn use_hover() -> Hover {
    let hover = use_context_provider(|| Hover {
        link: Signal::new(None),
        driver: CopyValue::new(None),
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
                    .set(pointed.map(|link| crate::trust::destination(&link.text, &link.href)));
            }
        }
    });
}

/// The window's hover state, when there is a window around the caller.
pub(super) fn hover() -> Option<Hover> {
    try_consume_context::<Hover>()
}

/// quire's hover driver, when the caller is inside a quire root (a component drawn on its own
/// in a test is not). Call it from a component body, as a hook.
pub(super) fn use_driver() -> Option<HoverDriver> {
    let inside = use_hook(|| try_consume_context::<HoverHub>().is_some());
    if inside {
        Some(ds::use_hover_intent())
    } else {
        None
    }
}

/// Where a part of a row is, from the pointer event that entered it: its top-left corner and
/// one line's height, which is all a sender card or a time tip is placed against (below it).
pub(super) fn line_at(event: &Event<PointerData>) -> HoverAnchor {
    let client = event.client_coordinates();
    let offset = event.element_coordinates();
    HoverAnchor::Rect(Rect {
        origin: Point {
            x: Px((client.x - offset.x) as f32),
            y: Px((client.y - offset.y) as f32),
        },
        size: Size {
            width: Px(0.0),
            height: Px(LINE),
        },
    })
}

/// A row part's line height, in pixels.
const LINE: f32 = 16.0;

/// An element's anchor once it has mounted; before that (a document with no renderer) the card
/// opens unplaced.
pub(super) fn element(mounted: Option<MountedRef>) -> HoverAnchor {
    mounted.map_or(HoverAnchor::Unplaced, HoverAnchor::Element)
}

/// The pointer came to rest on `hook`, placed against `anchor`.
pub(super) fn over(driver: Option<HoverDriver>, hook: Hook, anchor: HoverAnchor) {
    if let Some(driver) = driver {
        driver.over(hook.key(), hook.kind(), anchor);
    }
}

/// The pointer left the hook.
pub(super) fn out(driver: Option<HoverDriver>) {
    if let Some(driver) = driver {
        driver.out();
    }
}

/// A press: whatever card is showing goes at once.
pub(super) fn press(driver: Option<HoverDriver>) {
    if let Some(driver) = driver {
        driver.press();
    }
}

/// Close whatever card is showing, from outside the root (a drag starting, an action in a
/// card): the press quire's hub hears.
pub(super) fn dismiss() {
    if let Some(state) = hover() {
        press(*state.driver.peek());
    }
}

/// The card layer hands the root's driver up, so the window's keys can reach it.
fn keep_driver(driver: Option<HoverDriver>) {
    if let (Some(state), Some(driver)) = (hover(), driver)
        && state.driver.peek().is_none()
    {
        let mut kept = state.driver;
        kept.set(Some(driver));
    }
}

/// Space opens the thread whose card is showing in a centred peek; Esc closes the card.
/// Returns whether the key was the hover's.
pub(super) fn key(name: &str, mut shell: Signal<crate::view::Shell>) -> bool {
    let Some(driver) = hover().and_then(|state| *state.driver.peek()) else {
        return false;
    };
    let hub = driver.hub();
    let Some(hook) = hub.open().and_then(|(key, _)| Hook::of(&key)) else {
        return false;
    };
    match (name, hook) {
        (" ", Hook::Thread(id)) => {
            // quire's Space: the card closes, warm, as it turns into the peek.
            hub.feed(HoverEvent::SpaceKey);
            // A card in its close grace is not open to Space: it goes at once instead.
            if hub.open().is_some() {
                driver.press();
            }
            let mut write = shell.write();
            write.peek = crate::view::Peek::CENTER;
            write.open(id);
            true
        }
        ("Escape", _) => {
            driver.press();
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod intent_tests;
#[cfg(test)]
mod render;
#[cfg(test)]
mod tests;
