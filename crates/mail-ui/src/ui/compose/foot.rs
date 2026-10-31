//! The composer's foot: Attach, ⋯, then Send ▾ at the right edge, one filled button.
//!
//! Every choice about how the message goes out is one step away rather than a row drawn on every
//! message (docs/design-language.md §2.1): Send ▾ holds Send Later, Remind Me If No Reply and Ask
//! for Read Receipt; ⋯ holds Security, Priority, Request Delivery Receipt, Show Plain Text, Save
//! as Template…, Keep for Later and Delete Draft. A choice that is not the default is then said by its row (`props.rs`), whose
//! menu sets it back. What the menus list and what a pick does are pure ([`send_items`], [`pick_send`],
//! [`more_items`], [`pick_more`]), so the tests can say them without a window.

use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use ds::components::controls::button_model::{Answers, Bezel, ImagePosition};
use ds::components::menus::item::item::MenuItem as Line;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_ui_model::copy;

use super::super::menu::{anchor_at, menu_items};
use super::super::menus::{snooze_help, when_words};
use super::later::{PICK_KEY as LATER_PICK, PICK_LABEL, PickTime};
use super::page::{Float, Fold, Page, When};
use super::protection;
use super::receipt;
use super::remind::{PickRemind, pick_remind, remind_choices};
use super::templates::{self, SAVE_KEY};
use crate::ui::actions::{self, Own};
use crate::ui::press::on_primary;
use mail_domain::{DeliveryReport, Priority, ReceiptRequest};

/// Send ▾'s keys: a Send Later choice is `later:` and the Sends key, a reminder `remind:` and its
/// key, and the receipt [`receipt::KEY`].
const LATER: &str = "later:";
const REMIND: &str = "remind:";

/// ⋯'s keys. A Security choice is `security:` and the protection's key.
const SECURITY: &str = "security:";
/// A Priority choice is `priority:` and [`priority_key`].
const PRIORITY: &str = "priority:";
const DELIVERY: &str = "delivery";
const PLAIN: &str = "plain";
const KEEP: &str = "keep";
const DELETE: &str = "delete";

/// What Send ▾ is called, by its tip and to assistive technology. Not "Send": that is the button.
pub(in crate::ui) const SEND_OPTIONS: &str = copy::aria::SEND_OPTIONS;
/// And the foot's ⋯.
pub(in crate::ui) const MORE: &str = copy::aria::MORE;

/// Send ▾: Send Later ▸, Remind Me If No Reply ▸, and Ask for Read Receipt, each checked as the
/// page has it.
pub(in crate::ui) fn send_items<Tz: TimeZone>(
    page: &Page,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: mail_client::when::Clock,
) -> Vec<Line<String>>
where
    Tz::Offset: std::fmt::Display,
{
    let prefixed = |prefix: &'static str, items: Vec<Line<String>>| -> Vec<Line<String>> {
        items
            .into_iter()
            .map(|item| item.map(&|key: String| format!("{prefix}{key}")))
            .collect()
    };
    let mut later: Vec<Line<String>> = When::ALL
        .into_iter()
        .filter(|when| *when != When::Now)
        .filter_map(|when| {
            let at = when.due(now, zone)?;
            let item = Line::new(when.key().to_owned(), when.label(clock))
                .with_hint(snooze_help(at, zone, clock))
                .with_check(if page.when == when {
                    Check::On
                } else {
                    Check::Off
                });
            Some(item)
        })
        .collect();
    let (hint, on) = match page.when {
        When::At(at) => (Some(when_words(at, now, zone, clock)), Check::On),
        _ => (None, Check::Off),
    };
    let mut pick = Line::new(LATER_PICK.to_owned(), PICK_LABEL).with_check(on);
    if let Some(hint) = hint {
        pick = pick.with_hint(hint);
    }
    later.push(pick);
    let remind = menu_items("", &remind_choices(page.remind, now, zone, clock), false);
    let asked = if page.receipt == ReceiptRequest::Requested {
        Check::On
    } else {
        Check::Off
    };
    vec![
        Line::Submenu {
            title: copy::menus::SEND_LATER.to_owned(),
            image: Some(MenuImage::Icon(Icon::Clock)),
            availability: Availability::Enabled,
            children: prefixed(LATER, later),
        },
        Line::Submenu {
            title: copy::menus::REMIND_IF_NO_REPLY.to_owned(),
            image: Some(MenuImage::Icon(Icon::Bell)),
            availability: Availability::Enabled,
            children: prefixed(REMIND, remind),
        },
        Line::new(receipt::KEY.to_owned(), receipt::ASK).with_check(asked),
    ]
}

/// A pick from Send ▾. "Pick a Time…" opens its field by Send ▾, where the time is typed.
pub(in crate::ui) fn pick_send(page: &mut Page, key: &str) {
    if let Some(when) = key.strip_prefix(LATER) {
        super::props::pick_sends(page, when);
    } else if let Some(choice) = key.strip_prefix(REMIND) {
        pick_remind(page, choice);
    } else if key == receipt::KEY {
        page.toggle_receipt();
        page.float = Float::Closed;
    } else {
        page.float = Float::Closed;
    }
}

/// A priority's key under ⋯ › Priority, after [`PRIORITY`].
fn priority_key(priority: Priority) -> &'static str {
    match priority {
        Priority::High => "high",
        Priority::Normal => "normal",
        Priority::Low => "low",
    }
}

/// What ⋯ › Priority calls a priority.
fn priority_name(priority: Priority) -> &'static str {
    match priority {
        Priority::High => copy::menus::PRIORITY_HIGH,
        Priority::Normal => copy::menus::PRIORITY_NORMAL,
        Priority::Low => copy::menus::PRIORITY_LOW,
    }
}

/// The foot's ⋯: Security ▸ (None, OpenPGP ▸, S/MIME ▸), Priority ▸ (High, Normal, Low), Request
/// Delivery Receipt, Show Plain Text, Save as Template…, then Keep for Later (Esc) and Delete
/// Draft, which has an undo. Request Delivery Receipt is left out where the sending account has no
/// way to ask (`delivery_offered` is false), so the menu never offers what does nothing.
pub(in crate::ui) fn more_items(page: &Page, delivery_offered: bool) -> Vec<Line<String>> {
    let check = |on: bool| if on { Check::On } else { Check::Off };
    let mut security = Vec::new();
    let mut schemes: Vec<(&'static str, Vec<Line<String>>)> = Vec::new();
    for (key, scheme, name, on) in protection::choices(page.protection) {
        let item = Line::new(format!("{SECURITY}{key}"), name).with_check(check(on));
        match scheme {
            None => security.push(item),
            Some(scheme) => match schemes.iter_mut().find(|(named, _)| *named == scheme) {
                Some((_, items)) => items.push(item),
                None => schemes.push((scheme, vec![item])),
            },
        }
    }
    security.extend(schemes.into_iter().map(|(scheme, children)| Line::Submenu {
        title: scheme.to_owned(),
        image: None,
        availability: Availability::Enabled,
        children,
    }));
    let mut items = vec![
        Line::Submenu {
            title: copy::menus::SECURITY.to_owned(),
            image: Some(MenuImage::Icon(Icon::Key)),
            availability: Availability::Enabled,
            children: security,
        },
        Line::Submenu {
            title: copy::menus::PRIORITY.to_owned(),
            image: None,
            availability: Availability::Enabled,
            children: Priority::ALL
                .into_iter()
                .map(|priority| {
                    Line::new(
                        format!("{PRIORITY}{}", priority_key(priority)),
                        priority_name(priority),
                    )
                    .with_check(check(page.priority == priority))
                })
                .collect(),
        },
    ];
    if delivery_offered {
        items.push(
            Line::new(DELIVERY.to_owned(), copy::menus::REQUEST_DELIVERY_RECEIPT)
                .with_check(check(page.delivery == DeliveryReport::Requested)),
        );
    }
    items.extend([
        Line::new(PLAIN.to_owned(), copy::menus::SHOW_PLAIN_TEXT)
            .with_check(check(page.plain == Fold::Open)),
        Line::new(SAVE_KEY.to_owned(), copy::menus::SAVE_AS_TEMPLATE)
            .with_image(MenuImage::Icon(Icon::FilePen)),
        Line::Separator,
    ]);
    items.extend([
        Line::new(KEEP.to_owned(), copy::menus::KEEP_FOR_LATER)
            .with_image(MenuImage::Icon(Icon::Clock))
            .with_key(actions::escape()),
        Line::new(DELETE.to_owned(), copy::menus::DELETE_DRAFT)
            .with_image(MenuImage::Icon(Icon::Trash)),
    ]);
    items
}

/// What a pick from ⋯ leaves for the page to do with its desk and shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum More {
    /// Nothing more: the page changed, or the pick was nothing.
    Done,
    /// Keep for Later: park the draft, as Esc does.
    Keep,
    /// Delete Draft, with an undo.
    Delete,
}

/// A pick from ⋯, on the page alone.
pub(in crate::ui) fn pick_more(page: &mut Page, key: &str) -> More {
    page.float = Float::Closed;
    if let Some(choice) = key.strip_prefix(SECURITY) {
        protection::pick(page, choice);
        return More::Done;
    }
    if let Some(choice) = key.strip_prefix(PRIORITY) {
        if let Some(priority) = Priority::ALL
            .into_iter()
            .find(|priority| priority_key(*priority) == choice)
        {
            page.set_priority(priority);
        }
        return More::Done;
    }
    match key {
        DELIVERY => {
            page.toggle_delivery();
            More::Done
        }
        PLAIN => {
            page.plain = match page.plain {
                Fold::Open => Fold::Folded,
                Fold::Folded => Fold::Open,
            };
            More::Done
        }
        SAVE_KEY => {
            templates::pick(page, SAVE_KEY);
            More::Done
        }
        KEEP => More::Keep,
        DELETE => More::Delete,
        _ => More::Done,
    }
}

/// Send, filled, and its ▾: the menu of how it goes out, and the time fields its picks open. The
/// fields hang from the ▾, whichever menu asked for them.
#[component]
pub(super) fn SendButton(page: Signal<Page>, on_send: EventHandler<()>) -> Element {
    let mut chevron = use_signal(|| None::<MountedRef>);
    let float = page.read().float.clone();
    let open = float == Float::SendMenu;
    let items = if open {
        send_items(
            &page.read(),
            super::super::clock::now(),
            &chrono::Local,
            super::super::clock::hours(),
        )
    } else {
        Vec::new()
    };
    rsx! {
        span { class: "c-send",
            Button {
                answers: Answers::Return,
                label: copy::controls::SEND,
                title: copy::controls::SEND.to_owned(),
                title_shortcut: actions::tip_own(Own::Send),
                icon: Icon::Send,
                onclick: on_primary(move || on_send.call(())),
                common: Common { aria_label: Some(copy::controls::SEND.to_owned()), ..Common::default() },
            }
            PullDownButton::<String> {
                face: PullDownFace::Chevron,
                bezel: Bezel::Toolbar,
                items,
                open: Some(if open { Shown::Visible } else { Shown::Hidden }),
                on_open_change: move |to: Shown| {
                    let mut page = page;
                    let now_open = page.peek().float == Float::SendMenu;
                    match (to, now_open) {
                        (Shown::Visible, _) => page.write().float = Float::SendMenu,
                        (Shown::Hidden, true) => page.write().float = Float::Closed,
                        // A pick already opened a time field: closing the menu leaves it.
                        (Shown::Hidden, false) => {}
                    }
                },
                // A window with no renderer (the tests) has no rect to hang the menu from.
                anchor: Some(anchor_at(chevron())),
                common: Common {
                    aria_label: Some(SEND_OPTIONS.to_owned()),
                    mounted: Some(EventHandler::new(move |event: MountedEvent| {
                        chevron.set(Some(MountedRef(event.data())));
                    })),
                    ..Common::default()
                },
                onpick: move |key: String| {
                    let mut page = page;
                    pick_send(&mut page.write(), &key);
                },
            }
        }
        match float {
            Float::PickTime(_) => rsx! { PickTime { page, anchor: chevron() } },
            Float::PickRemind(_) => rsx! { PickRemind { page, anchor: chevron() } },
            _ => rsx! {},
        }
    }
}

/// The foot's ⋯ and its menu. Keep for Later and Delete Draft reach past the page, so the page
/// hands in what they do.
#[component]
pub(super) fn MoreButton(
    page: Signal<Page>,
    on_keep: EventHandler<()>,
    on_delete: EventHandler<()>,
) -> Element {
    let mut at = use_signal(|| None::<MountedRef>);
    let open = page.read().float == Float::More;
    // Asked of the client again only when the page's From account changes.
    let delivery_offered = use_memo(move || {
        let client = crate::ui::live::client();
        client.compose().can_ask_delivery_report(&page.read().from)
    });
    let items = if open {
        more_items(&page.read(), delivery_offered())
    } else {
        Vec::new()
    };
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            size: ControlSize::Regular,
            image: ImagePosition::Only,
            icon: Some(IconSource::Glyph(Icon::Ellipsis)),
            label: MORE.to_owned(),
            title: Some(MORE.to_owned()),
            shown: Some(if open { Shown::Visible } else { Shown::Hidden }),
            common: Common {
                mounted: Some(EventHandler::new(move |event: MountedEvent| {
                    at.set(Some(MountedRef(event.data())));
                })),
                ..Common::default()
            },
            onclick: move |_| {
                let mut page = page;
                let next = if open { Float::Closed } else { Float::More };
                page.write().float = next;
            },
        }
        if open {
            Menu::<String> {
                placement: MenuPlacement::Popup,
                anchor: anchor_at(at()),
                items,
                common: Common {
                    aria_label: Some(copy::aria::MESSAGE_ACTIONS.to_owned()),
                    ..Common::default()
                },
                onpick: move |key: String| {
                    let mut page = page;
                    let act = pick_more(&mut page.write(), &key);
                    match act {
                        More::Done => {
                            if matches!(page.peek().float, Float::SaveTemplate(_)) {
                                crate::ui::host::Host::focus_after_task(".tpl-name input");
                            }
                        }
                        More::Keep => on_keep.call(()),
                        More::Delete => on_delete.call(()),
                    }
                },
                onclose: move |()| {
                    let mut page = page;
                    if page.peek().float == Float::More {
                        page.write().float = Float::Closed;
                    }
                },
            }
        }
    }
}
