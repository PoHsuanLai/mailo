//! A message held to leave at a time, as a row of the Scheduled place (and of Drafts): who it is
//! to, its subject and when it leaves.
//!
//! Its one action, Cancel Sending, is in its menu, with undo. A click opens that menu as a right
//! click does: the row has nothing to open, and taking a message back is never one press.

use crate::ui::compose::cancel_scheduled;
use crate::ui::copy;
use crate::ui::menus::when_words;
use crate::ui::view::Shell;
use chrono::{DateTime, Local, Utc};
use dioxus::prelude::*;
use ds::base::press::Press;
use ds::components::app::thread_row::ThreadRow;
use ds::host::measure::Anchor;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::{DataAttr, DataName};
use mail_domain::Draft;

/// What a scheduled message's menu offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScheduledPick {
    Cancel,
}

/// The menu over a scheduled message.
fn menu_items() -> Vec<MenuItem<ScheduledPick>> {
    vec![
        MenuItem::new(ScheduledPick::Cancel, copy::menus::CANCEL_SENDING)
            .with_image(MenuImage::Icon(Icon::X)),
    ]
}

/// `draft`, held to leave at `at`, as a row.
#[component]
pub(in crate::ui) fn ScheduledRow(
    draft: Draft,
    at: DateTime<Utc>,
    shell: Signal<Shell>,
    revision: Signal<u64>,
) -> Element {
    let mut menu = use_signal(|| None::<Point>);
    let id = draft.id;
    let subject = if draft.subject.trim().is_empty() {
        copy::empty::NO_SUBJECT.to_owned()
    } else {
        draft.subject.clone()
    };
    let who = mail_core::compose::addresses::join_addresses(&draft.to);
    let when = when_words(at, Utc::now(), &Local);
    rsx! {
        div { key: "{id}", class: "row", role: "none",
            oncontextmenu: move |event: MouseEvent| {
                event.prevent_default();
                let at = event.client_coordinates();
                menu.set(Some(Point { x: Px(at.x as f32), y: Px(at.y as f32) }));
            },
            ThreadRow {
                name: who,
                via: None,
                subject: TextLine::from(subject.clone()),
                snippet: None,
                time: when,
                tags: rsx! {},
                star: None,
                strip: None,
                more: None,
                onclick: move |press: Press| menu.set(Some(press.at)),
                // Where a test, and a hover card, find the message.
                common: Common {
                    data: DataName::parse("scheduled")
                        .map(|name| vec![DataAttr::new(name, subject.clone())])
                        .unwrap_or_default(),
                    ..Common::default()
                },
            }
            if let Some(point) = menu() {
                Menu::<ScheduledPick> {
                    placement: MenuPlacement::default(),
                    anchor: Anchor::Point(point),
                    items: menu_items(),
                    onpick: move |pick: ScheduledPick| {
                        menu.set(None);
                        match pick {
                            ScheduledPick::Cancel => {
                                // A refusal is said in the toast; the message stays as it was.
                                if cancel_scheduled(shell, id).is_ok() {
                                    let mut revision = revision;
                                    revision += 1;
                                }
                            }
                        }
                    },
                    onclose: move |()| menu.set(None),
                }
            }
        }
    }
}
