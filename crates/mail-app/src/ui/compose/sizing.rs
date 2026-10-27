//! How large a message is, and how large its pictures go: Picture Size at the attachment strip,
//! as Apple Mail offers it, and the warning, before Send, of a message larger than its account's
//! server takes.
//!
//! The draft's files are weighed off the thread that draws whenever what it carries or the account
//! it goes from changes (`mail_core::compose::weigh`), at every picture size at once, so choosing
//! one is only reading a number. The choice is the page's: the draft keeps every picture as it
//! was attached, and the smaller copies are made as Send freezes the message.

use dioxus::prelude::*;
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::Bezel;
use ds::components::overlays::inline_banner::InlineBanner;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use mail_core::compose::{Fit, PictureSize, SendLimit, Weight, fit};
use mail_domain::human_size;

use super::super::menu::{Floating, MenuItem, Right};
use super::page::{Float, Guard, Page};
use crate::ui::copy;
use crate::ui::press::on_primary;

/// What the page's files were found to weigh, and the most its account sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Measured {
    pub weight: Weight,
    pub limit: SendLimit,
}

/// Weigh the page's draft whenever what it carries or where it goes from changes, and keep the
/// answer on the page. A draft that carries nothing is not weighed, and the page says nothing.
pub(super) fn use_weight(mut page: Signal<Page>) {
    // A memo, so a keystroke does not weigh again; only another file or another account does.
    let carried = use_memo(move || {
        let read = page.read();
        (read.draft, read.from.clone(), read.attached.clone())
    });
    let weighed = use_resource(move || {
        let (draft, from, attached) = carried();
        let client = crate::ui::live::client();
        async move {
            if attached.is_empty() {
                return None;
            }
            tokio::task::spawn_blocking(move || {
                // A draft that is gone has nothing to weigh, and the page goes with it.
                let stored = client.stored().draft(draft).ok()?;
                Some(Measured {
                    weight: client.compose().weigh(&stored),
                    limit: client.compose().send_limit(&from),
                })
            })
            .await
            // A weighing that stopped says nothing: the page is as it was before files.
            .ok()
            .flatten()
        }
    });
    use_effect(move || {
        let measured = weighed.read().clone().flatten();
        if page.peek().weight != measured {
            page.write().weight = measured;
        }
    });
}

/// Whether the page's message is over its limit at the picture size chosen: the [`Fit::Over`]
/// that says by how much, or `None` when it fits or has not been weighed.
pub(super) fn over(page: &Page) -> Option<Fit> {
    let measured = page.weight.as_ref()?;
    match fit(&measured.weight, page.pictures, measured.limit) {
        Fit::Fits => None,
        over @ Fit::Over { .. } => Some(over),
    }
}

/// What Picture Size calls a size.
pub(in crate::ui) fn name(size: PictureSize) -> &'static str {
    match size {
        PictureSize::Small => copy::menus::PICTURES_SMALL,
        PictureSize::Medium => copy::menus::PICTURES_MEDIUM,
        PictureSize::Large => copy::menus::PICTURES_LARGE,
        PictureSize::Actual => copy::menus::PICTURES_ACTUAL,
    }
}

/// A size's key in the menu.
fn key(size: PictureSize) -> &'static str {
    match size {
        PictureSize::Small => "pictures:small",
        PictureSize::Medium => "pictures:medium",
        PictureSize::Large => "pictures:large",
        PictureSize::Actual => "pictures:actual",
    }
}

/// Send the page's pictures at `size`. What Send stopped for the size is asked again at the next
/// Send, against the new one.
pub(in crate::ui) fn choose(page: &mut Page, size: PictureSize) {
    page.pictures = size;
    if page.guard == Guard::TooLarge {
        page.guard = Guard::Clear;
    }
    page.float = Float::Closed;
}

/// Picture Size's menu: each size, checked if chosen, with what the message comes to at it.
pub(in crate::ui) fn items(measured: &Measured, chosen: PictureSize) -> Vec<MenuItem> {
    PictureSize::ALL
        .into_iter()
        .map(|size| MenuItem {
            help: Some(human_size(measured.weight.against(size, measured.limit))),
            right: Right::Check(size == chosen),
            ..MenuItem::plain(key(size), name(size))
        })
        .collect()
}

/// The end of the attachment strip: Picture Size while the draft carries a picture, what the
/// message comes to, and, while a picture goes smaller, that it goes without its location.
#[component]
pub(super) fn Sizing(page: Signal<Page>) -> Element {
    let mut at = use_signal(|| None::<MountedRef>);
    let read = page.read();
    let Some(measured) = read.weight.clone() else {
        return rsx! {};
    };
    let chosen = read.pictures;
    let open = read.float == Float::Pictures;
    drop(read);
    let pictures = measured.weight.pictures() > 0;
    let size = human_size(measured.weight.against(chosen, measured.limit));
    let label = copy::controls::picture_size(name(chosen));
    let menu = items(&measured, chosen);
    rsx! {
        if pictures {
            Button {
                bezel: Bezel::Inline,
                label: label.clone(),
                trailing: Some(Icon::ChevronDown),
                shown: Some(if open { Shown::Visible } else { Shown::Hidden }),
                onclick: on_primary(move || {
                    let next = if open { Float::Closed } else { Float::Pictures };
                    page.write().float = next;
                }),
                common: Common {
                    aria_label: Some(label),
                    mounted: Some(EventHandler::new(move |event: MountedEvent| at.set(Some(MountedRef(event.data()))))),
                    ..Common::default()
                },
            }
            if open {
                Floating {
                    placement: MenuPlacement::Popup,
                    anchor: at(),
                    title: copy::aria::PICTURE_SIZE.to_owned(),
                    items: menu,
                    on_pick: move |picked: String| {
                        let size = PictureSize::ALL.into_iter().find(|size| key(*size) == picked);
                        let mut write = page.write();
                        match size {
                            Some(size) => choose(&mut write, size),
                            None => write.float = Float::Closed,
                        }
                    },
                    on_close: move |_| {
                        if page.peek().float == Float::Pictures {
                            page.write().float = Float::Closed;
                        }
                    },
                }
            }
        }
        Label {
            text: copy::status::message_size(&size),
            role: LabelRole::Secondary,
            style: LabelStyle::Footnote,
        }
        // Only when a picture is made smaller: one that already fits goes as it was.
        if pictures && measured.weight.resizes_at(chosen) {
            Label {
                text: copy::status::PICTURES_WITHOUT_LOCATION.to_owned(),
                role: LabelRole::Secondary,
                style: LabelStyle::Footnote,
            }
        }
    }
}

/// The warning, before Send, of a message larger than its server takes: its size and the limit,
/// the way to the largest picture size that fits when one does, and, once Send was pressed and
/// stopped for it, Send Anyway.
#[component]
pub(super) fn TooLarge(page: Signal<Page>, on_anyway: EventHandler<()>) -> Element {
    let read = page.read();
    let Some(Fit::Over {
        size,
        limit,
        smaller,
    }) = over(&read)
    else {
        return rsx! {};
    };
    let stopped = read.guard == Guard::TooLarge;
    drop(read);
    let text = copy::status::too_large(&human_size(size), &human_size(limit.octets), limit.said);
    let smaller = smaller.map(|size| (size, copy::controls::use_pictures(name(size))));
    rsx! {
        InlineBanner {
            severity: Severity::Warn,
            icon: Some(Icon::Paperclip),
            text,
            actions: rsx! {
                if let Some((size, words)) = smaller {
                    Button {
                        label: words.clone(),
                        onclick: on_primary(move || choose(&mut page.write(), size)),
                        common: Common { aria_label: Some(words), ..Common::default() },
                    }
                }
                if stopped {
                    Button {
                        label: copy::controls::SEND_ANYWAY,
                        onclick: on_primary(move || on_anyway.call(())),
                        common: Common { aria_label: Some(copy::controls::SEND_ANYWAY.to_owned()), ..Common::default() },
                    }
                }
            },
        }
    }
}
