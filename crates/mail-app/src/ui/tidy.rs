//! Tidy Up…: archive the read conversations the Inbox has kept longer than an age chosen.
//!
//! Opened from the sidebar's menu. The sheet lists what its button archives, in the Space shown:
//! read, in the Inbox, and last written to before the age chosen. A conversation someone marked
//! to keep in view is left be: starred, pinned, snoozed, or waiting on a reply. What the sheet
//! counts is what its button archives ([`candidates`] is both), as one gesture with one undo.

use crate::ui::common::in_card;
use crate::ui::copy;
use crate::ui::press::{SheetClose, available, on_primary};
use crate::ui::view::Shell;
use chrono::{DateTime, TimeDelta, Utc};
use dioxus::prelude::*;
use ds::components::content::label::{Label, LabelStyle};
use ds::components::controls::button_model::Answers;
use ds::components::controls::segmented::Tracking;
use ds::components::fields::field_row::{FieldGroup, FieldRow, RowLayout};
use ds::components::lists::row::row::Outline;
use ds::components::overlays::sheet_attach::Attach;
use ds::prelude::*;
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use std::sync::Arc;

/// How many conversations one page of the search reads.
const PAGE: u32 = 500;
/// How many of the conversations the sheet names; the count says the rest.
const SHOWN: usize = 5;

/// How long a read conversation sits in the Inbox before Tidy Up offers to archive it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Age {
    OneWeek,
    #[default]
    TwoWeeks,
    OneMonth,
    ThreeMonths,
}

impl Age {
    pub const ALL: [Age; 4] = [Age::OneWeek, Age::TwoWeeks, Age::OneMonth, Age::ThreeMonths];

    fn span(self) -> TimeDelta {
        TimeDelta::days(match self {
            Age::OneWeek => 7,
            Age::TwoWeeks => 14,
            Age::OneMonth => 30,
            Age::ThreeMonths => 91,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Age::OneWeek => copy::controls::ONE_WEEK,
            Age::TwoWeeks => copy::controls::TWO_WEEKS,
            Age::OneMonth => copy::controls::ONE_MONTH,
            Age::ThreeMonths => copy::controls::THREE_MONTHS,
        }
    }
}

/// The conversations Tidy Up archives, newest first: read, in the Inbox of the accounts `scope`
/// names (`None` is every account), last written to more than `age` before `now`, and neither
/// starred, pinned, snoozed nor waiting on a reply.
pub(in crate::ui) fn candidates(
    store: &SqliteStore,
    scope: Option<Filter>,
    age: Age,
    now: DateTime<Utc>,
) -> Vec<ThreadSummary> {
    let filter = Filter::And(vec![
        mail_core::place::place_filter(MailboxRole::Inbox),
        Filter::Read(ReadState::Read),
        Filter::Not(Box::new(Filter::Starred(Star::Starred))),
        Filter::Not(Box::new(Filter::Pinned)),
        Filter::Not(Box::new(Filter::Snoozed)),
        Filter::Date(DateRange {
            from: None,
            to: Some(now - age.span()),
        }),
    ]);
    let mut query = Query {
        filter: mail_core::view::scoped(scope, filter),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: PAGE,
        },
    };
    let mut found = Vec::new();
    while let Ok(page) = store.threads(&query, now) {
        let last = page.items.is_empty();
        found.extend(
            page.items
                .into_iter()
                .filter(|summary| summary.follow_up == FollowUp::Inactive),
        );
        match page.next {
            Some(next) if !last => query.page.after = Some(next),
            _ => break,
        }
    }
    found
}

/// Open the sheet, at the age it starts from.
pub(in crate::ui) fn open(mut shell: Signal<Shell>) {
    shell.write().tidying = Some(Age::default());
}

/// Close the sheet, archiving nothing, and give the keyboard back to the window.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().tidying = None;
    crate::ui::host::Host::focus_app();
}

/// The sheet's button: archive what it listed, as one gesture, then close.
fn run(shell: Signal<Shell>, revision: Signal<u64>) {
    let Some(age) = shell.peek().tidying else {
        return;
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let scope = shell.peek().account_filter();
    let threads: Vec<ThreadId> = candidates(&store, scope, age, crate::ui::clock::now())
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    close(shell);
    crate::ui::motion::act_kind_all(&store, shell, revision, &threads, OpKind::Archive);
}

/// The sheet, over the window while it is open.
#[component]
pub(in crate::ui) fn TidySheet(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let Some(age) = shell.read().tidying else {
        return rsx! {};
    };
    // Read so an archive or a sync elsewhere redraws the count.
    let _ = revision();
    let store = consume_context::<Arc<SqliteStore>>();
    let found = candidates(
        &store,
        shell.read().account_filter(),
        age,
        crate::ui::clock::now(),
    );
    let count = found.len();
    let ages: Vec<Choice<Age>> = Age::ALL
        .iter()
        .map(|one| Choice::new(*one, one.label()))
        .collect();
    rsx! {
        Sheet {
            label: copy::titles::TIDY_UP,
            attach: Attach::Window,
            common: in_card(),
            onclose: move |()| close(shell),
            div { class: "sheet-form",
                Label { text: copy::titles::TIDY_UP, style: LabelStyle::Title }
                FieldGroup {
                    FieldRow {
                        label: copy::fields::OLDER_THAN,
                        layout: RowLayout::Form,
                        SegmentedControl::<Age> {
                            label: copy::fields::OLDER_THAN.to_owned(),
                            choices: ages,
                            tracking: Tracking::SelectOne(age),
                            onchange: move |one: Age| {
                                let mut shell = shell;
                                shell.write().tidying = Some(one);
                            },
                        }
                    }
                }
                Label { text: copy::confirm::tidy_up(count) }
                if count > 0 {
                    div { class: "tidy-list",
                        for summary in found.iter().take(SHOWN) {
                            Row {
                                key: "{summary.id}",
                                title: if summary.subject.trim().is_empty() {
                                    copy::empty::NO_SUBJECT.to_owned()
                                } else {
                                    summary.subject.clone()
                                },
                                outline: Outline::None,
                            }
                        }
                    }
                }
                div { class: "sheet-actions",
                    SheetClose { label: copy::controls::CANCEL, on_close: move |()| close(shell) }
                    Button {
                        answers: Answers::Return,
                        label: copy::controls::archive_count(count),
                        availability: available(count > 0),
                        common: crate::ui::sidebar::tagged("answer", "archive"),
                        onclick: on_primary(move || run(shell, revision)),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
