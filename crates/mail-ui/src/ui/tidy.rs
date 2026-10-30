//! Tidy Up…: archive the read conversations the Inbox has kept longer than an age chosen.
//!
//! Opened from the sidebar's menu or ⌘K. The sheet lists what its button archives, in the Space shown:
//! read, in the Inbox, and last written to before the age chosen, which a pop-up picks. A
//! conversation someone marked to keep in view is left be: starred, pinned, snoozed, or waiting on
//! a reply. What the sheet lists is what its button archives ([`candidates`] is both), as one
//! gesture with one undo.

use crate::ui::common::in_card;
use crate::ui::copy;
use crate::ui::press::{SheetClose, available, on_primary};
use crate::ui::view::Shell;
use chrono::{DateTime, TimeDelta, Utc};
use dioxus::prelude::*;
use ds::components::content::avatar::{
    AvatarFace, AvatarShape, AvatarSize, AvatarTone, person_hue,
};
use ds::components::content::label::{Label, LabelStyle};
use ds::components::controls::button_model::Answers;
use ds::components::fields::field_row::FieldRow;
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
use ds::components::menus::item::item::MenuItem;
use ds::components::menus::pop_up_button::PopUpButton;
use ds::components::overlays::sheet_attach::Attach;
use ds::prelude::*;
use mail_client::Client;
use mail_client::when::Clock;
use mail_domain::*;

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
    client: &Client,
    scope: Option<Filter>,
    age: Age,
    now: DateTime<Utc>,
) -> Vec<ThreadSummary> {
    let filter = Filter::And(vec![
        mail_client::mailbox_filter::for_role(MailboxRole::Inbox),
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
        filter: mail_client::conversation::scoped(scope, filter),
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
    while let Ok(page) = client.stored().threads(&query, now) {
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
fn run(shell: Signal<Shell>, threads: &[ThreadId]) {
    let client = crate::ui::live::client();
    close(shell);
    crate::ui::motion::act_kind_all(&client, shell, threads, OpKind::Archive);
}

/// One conversation Tidy Up would archive, drawn as a row like the app's other lists: the
/// sender's face, the subject, who sent it, and when.
fn row(summary: &ThreadSummary, now: DateTime<Utc>, clock: Clock) -> ListItem<String> {
    let subject = mail_domain::subject_or_none(&summary.subject).into_owned();
    let face = AvatarFace {
        initial: summary
            .from
            .shown()
            .chars()
            .next()
            .map_or('?', |c| c.to_ascii_uppercase()),
        size: AvatarSize::Size28,
        tone: AvatarTone::Person(person_hue(&summary.from.email)),
        shape: AvatarShape::Round,
    };
    ListItem::row(
        summary.id.to_string(),
        subject.clone(),
        rsx! {
            Row {
                leading: RowLeading::Avatar(face),
                title: subject,
                detail: Some(TextLine::from(summary.from.shown().to_owned())),
                accessory: Accessory::Text(mail_client::when::listed(summary.last_date, now, &chrono::Local, clock)),
                size: RowSize::Settings,
            }
        },
    )
}

/// The sheet, over the window while it is open: the age in a form row, then what the button
/// archives as a grouped list, laid out as a Settings page is.
#[component]
pub(in crate::ui) fn TidySheet(shell: Signal<Shell>) -> Element {
    // What the sheet lists, read off the thread that draws: it pages through every old
    // conversation. Read again when the age, the Space, an archive or a sync changes it, and the
    // last list stays until the next is in.
    let listed = use_resource(move || {
        let _ = crate::ui::live::moved_for(mail_client::Topics::SUMMARIES);
        let age = shell.read().tidying;
        let scope = shell.read().account_filter();
        let client = crate::ui::live::client();
        let now = crate::ui::clock::now();
        async move {
            let age = age?;
            tokio::task::spawn_blocking(move || candidates(&client, scope, age, now))
                .await
                .inspect_err(|why| log::warn!("tidy up: the list was not read: {why}"))
                .ok()
        }
    });
    let Some(age) = shell.read().tidying else {
        return rsx! {};
    };
    let now = crate::ui::clock::now();
    let clock = crate::ui::clock::hours();
    // Not known until the first read is in: neither "nothing to archive" nor a count to press.
    let known = listed.read().clone().flatten();
    let reading = known.is_none();
    let found = known.unwrap_or_default();
    let ids: Vec<ThreadId> = found.iter().map(|summary| summary.id).collect();
    let count = found.len();
    let ages: Vec<MenuItem<Age>> = Age::ALL
        .iter()
        .map(|one| MenuItem::new(*one, one.label()))
        .collect();
    let mut rows: Vec<ListItem<String>> = found
        .iter()
        .take(SHOWN)
        .map(|one| row(one, now, clock))
        .collect();
    if count > SHOWN {
        let more = copy::confirm::tidy_more(count - SHOWN);
        rows.push(ListItem::row(
            "more".to_owned(),
            more.clone(),
            rsx! { Row { title: more, size: RowSize::Settings } },
        ));
    }
    rsx! {
        Sheet {
            label: copy::titles::TIDY_UP,
            attach: Attach::Window,
            common: in_card(),
            onclose: move |()| close(shell),
            div { class: "tidy-sheet",
                div { class: "tidy-title",
                    Label { text: copy::titles::TIDY_UP, style: LabelStyle::Title }
                }
                Form {
                    FormSection {
                        FieldRow { label: copy::fields::OLDER_THAN,
                            PopUpButton::<Age> {
                                items: ages,
                                value: Some(age),
                                common: Common { aria_label: Some(copy::fields::OLDER_THAN.to_owned()), ..Common::default() },
                                onpick: move |one: Age| {
                                    let mut shell = shell;
                                    shell.write().tidying = Some(one);
                                },
                            }
                        }
                    }
                    if reading {
                    } else if count == 0 {
                        EmptyState { title: copy::confirm::TIDY_NOTHING }
                    } else {
                        FormSection {
                            title: Some(copy::headers::TO_ARCHIVE.to_owned()),
                            footer: Some(copy::confirm::TIDY_SKIPS.to_owned()),
                            List::<String> { label: copy::headers::TO_ARCHIVE, items: rows, style: ListStyle::Grouped }
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
                        onclick: on_primary(move || run(shell, &ids)),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
