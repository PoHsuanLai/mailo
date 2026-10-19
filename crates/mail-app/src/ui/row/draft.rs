//! A draft, as a row of Drafts (and of Scheduled): who it is to, its subject, and a time.
//!
//! A draft being written says when it was last touched; a click opens the composer on it, the
//! row stays selected while the composer is open, and its menu holds Delete Draft. One held to
//! leave at a time says when it leaves; a click opens its menu, which holds Cancel Sending: the
//! row has nothing to open, and taking a message back is never one press. Either menu is also its
//! ⋯ and its right click, and each pick has an undo.

use super::super::text::draft_state;
use crate::ui::compose::{cancel_scheduled, delete_stored_draft};
use crate::ui::copy;
use crate::ui::menus::when_words;
use crate::ui::view::Shell;
use chrono::{Local, Utc};
use dioxus::prelude::*;
use ds::base::press::Press;
use ds::base::vocab::RowState;
use ds::components::app::row_more::RowMore;
use ds::components::app::thread_row::ThreadRow;
use ds::host::measure::Anchor;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::{DataAttr, DataName};
use mail_core::{SqliteStore, Store};
use mail_domain::{Draft, SendState};
use std::sync::Arc;

/// What a draft's menu offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DraftPick {
    Delete,
    Cancel,
}

/// The menu over a draft: Cancel Sending while it is held for a time, Delete Draft otherwise.
fn menu_items(held: bool) -> Vec<MenuItem<DraftPick>> {
    vec![if held {
        MenuItem::new(DraftPick::Cancel, copy::menus::CANCEL_SENDING)
            .with_image(MenuImage::Icon(Icon::X))
    } else {
        MenuItem::new(DraftPick::Delete, copy::menus::DELETE_DRAFT)
            .with_image(MenuImage::Icon(Icon::Trash))
    }]
}

/// `draft` as a row.
#[component]
pub(in crate::ui) fn DraftRow(
    draft: Draft,
    shell: Signal<Shell>,
    mut revision: Signal<u64>,
) -> Element {
    let id = draft.id;
    let held_at = match draft.state {
        SendState::Scheduled { at } => Some(at),
        _ => None,
    };
    let mut menu = use_signal(|| None::<Anchor>);
    // The focus inside the row shows its ⋯, as the pointer over it does (`MailRow`).
    let mut more_shown = use_signal(|| None::<Shown>);
    let subject = if draft.subject.trim().is_empty() {
        copy::empty::NO_SUBJECT.to_owned()
    } else {
        draft.subject.clone()
    };
    let who = mail_core::compose::addresses::join_addresses(&draft.to);
    let who = if who.is_empty() {
        copy::empty::NO_RECIPIENT.to_owned()
    } else {
        who
    };
    let (time, snippet) = match held_at {
        Some(at) => (when_words(at, Utc::now(), &Local), None),
        None => (
            mail_core::when::listed(draft.updated, Utc::now(), &Local),
            shell
                .read()
                .parts
                .snippet
                .shown()
                .then(|| TextLine::from(draft_state(&draft.state))),
        ),
    };
    let open = shell.read().composing.as_ref().map(|open| open.draft) == Some(id);
    let state = RowState {
        selection: if open {
            Selection::Selected
        } else {
            Selection::Unselected
        },
        ..RowState::default()
    };
    // Where a test, and a hover card, find the message: a held one by its subject, any other by
    // the label its press reads as.
    let common = match held_at {
        Some(_) => Common {
            data: DataName::parse("scheduled")
                .map(|name| vec![DataAttr::new(name, subject.clone())])
                .unwrap_or_default(),
            ..Common::default()
        },
        None => Common {
            aria_label: Some(copy::aria::open_draft(&subject)),
            ..Common::default()
        },
    };
    let more = rsx! {
        RowMore {
            expanded: if matches!(menu(), Some(Anchor::Rect(_))) { Shown::Visible } else { Shown::Hidden },
            shown: more_shown(),
            onclick: move |rect: Rect| menu.set(Some(Anchor::Rect(rect))),
        }
    };
    rsx! {
        div { key: "{id}", class: "row", role: "none",
            onfocusin: move |_| more_shown.set(Some(Shown::Visible)),
            onfocusout: move |_| more_shown.set(None),
            oncontextmenu: move |event: MouseEvent| {
                event.prevent_default();
                let at = event.client_coordinates();
                menu.set(Some(Anchor::Point(Point { x: Px(at.x as f32), y: Px(at.y as f32) })));
            },
            ThreadRow {
                state,
                name: who,
                via: None,
                subject: TextLine::from(subject.clone()),
                snippet,
                time,
                tags: rsx! {},
                star: None,
                strip: None,
                more: Some(more),
                onclick: move |press: Press| {
                    if held_at.is_some() {
                        menu.set(Some(Anchor::Point(press.at)));
                        return;
                    }
                    let store = consume_context::<Arc<SqliteStore>>();
                    if let Ok(draft) = store.draft(id) {
                        shell.write().compose(&draft);
                    }
                },
                common,
            }
            if let Some(anchor) = menu() {
                Menu::<DraftPick> {
                    placement: MenuPlacement::default(),
                    anchor,
                    items: menu_items(held_at.is_some()),
                    common: Common {
                        aria_label: Some(copy::aria::DRAFT_MENU.to_owned()),
                        ..Common::default()
                    },
                    onpick: move |pick: DraftPick| {
                        menu.set(None);
                        match pick {
                            DraftPick::Delete => delete_stored_draft(shell, revision, id),
                            // A refusal is said in the toast; the message stays as it was.
                            DraftPick::Cancel => {
                                if cancel_scheduled(shell, id).is_ok() {
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
