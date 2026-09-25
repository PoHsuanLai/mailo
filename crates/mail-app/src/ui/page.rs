//! Group and Properties, applied to the loaded page only.
//!
//! Neither asks the store for a different query. The menu title says so.

use std::collections::BTreeMap;

use super::menu::{Floating, MenuItem, Right, Tile};
use super::press::on_primary;
use crate::view::{PageGroup, PageMenu, PageParts, Shell};
use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use ds::{Icon, MenuKind, MountedRef};
use mail_domain::{LabelId, ReadState, ThreadSummary};

/// One band of the loaded page.
pub(super) struct Band {
    pub title: Option<String>,
    pub threads: Vec<ThreadSummary>,
}

/// Group `threads` the way the page menu says. The input order is the list order.
pub(super) fn group_page<Tz: TimeZone>(
    threads: Vec<ThreadSummary>,
    by: PageGroup,
    names: &BTreeMap<LabelId, String>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<Band>
where
    Tz::Offset: std::fmt::Display,
{
    match by {
        PageGroup::None => vec![Band {
            title: None,
            threads,
        }],
        PageGroup::Sender => by_key(threads, |thread| {
            thread
                .from
                .name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| thread.from.email.clone())
        }),
        PageGroup::Date => date_bands(threads, now, zone),
        PageGroup::Label => by_key(threads, |thread| {
            thread
                .labels
                .iter()
                .find_map(|id| names.get(id).cloned())
                .unwrap_or_else(|| "No label".to_owned())
        }),
        PageGroup::Unread => {
            let mut unread = Vec::new();
            let mut read = Vec::new();
            for thread in threads {
                if thread.read == ReadState::Unread {
                    unread.push(thread);
                } else {
                    read.push(thread);
                }
            }
            titled([("Unread", unread), ("Read", read)])
        }
    }
}

fn by_key(threads: Vec<ThreadSummary>, key: impl Fn(&ThreadSummary) -> String) -> Vec<Band> {
    let mut order: Vec<String> = Vec::new();
    let mut bands: BTreeMap<String, Vec<ThreadSummary>> = BTreeMap::new();
    for thread in threads {
        let name = key(&thread);
        if !bands.contains_key(&name) {
            order.push(name.clone());
        }
        bands.entry(name).or_default().push(thread);
    }
    order
        .into_iter()
        .map(|title| Band {
            threads: bands.remove(&title).unwrap_or_default(),
            title: Some(title),
        })
        .collect()
}

fn date_bands<Tz: TimeZone>(threads: Vec<ThreadSummary>, now: DateTime<Utc>, zone: &Tz) -> Vec<Band>
where
    Tz::Offset: std::fmt::Display,
{
    let mut today = Vec::new();
    let mut yesterday = Vec::new();
    let mut week = Vec::new();
    let mut older = Vec::new();
    let today_date = now.with_timezone(zone).date_naive();
    for thread in threads {
        let days = today_date
            .signed_duration_since(thread.last_date.with_timezone(zone).date_naive())
            .num_days();
        if days <= 0 {
            today.push(thread);
        } else if days == 1 {
            yesterday.push(thread);
        } else if days < 7 {
            week.push(thread);
        } else {
            older.push(thread);
        }
    }
    titled([
        ("Today", today),
        ("Yesterday", yesterday),
        ("This week", week),
        ("Older", older),
    ])
}

fn titled<const N: usize>(bands: [(&str, Vec<ThreadSummary>); N]) -> Vec<Band> {
    bands
        .into_iter()
        .filter(|(_, threads)| !threads.is_empty())
        .map(|(title, threads)| Band {
            title: Some(title.to_owned()),
            threads,
        })
        .collect()
}

fn group_items(current: PageGroup) -> Vec<MenuItem> {
    [
        (PageGroup::None, "None", "One list"),
        (PageGroup::Sender, "Sender", "By who sent it"),
        (
            PageGroup::Date,
            "Date",
            "Today, yesterday, this week, older",
        ),
        (PageGroup::Label, "Label", "By the first label"),
        (PageGroup::Unread, "Unread", "Unread, then the rest"),
    ]
    .into_iter()
    .map(|(group, name, help)| MenuItem {
        key: group_key(group).to_owned(),
        tile: if group == PageGroup::None {
            Tile::Glyph('–')
        } else {
            Tile::Icon(Icon::Group)
        },
        name: name.to_owned(),
        help: Some(help.to_owned()),
        right: Right::Check(current == group),
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    })
    .collect()
}

fn group_key(group: PageGroup) -> &'static str {
    match group {
        PageGroup::None => "none",
        PageGroup::Sender => "sender",
        PageGroup::Date => "date",
        PageGroup::Label => "label",
        PageGroup::Unread => "unread",
    }
}

fn group_from(key: &str) -> Option<PageGroup> {
    Some(match key {
        "none" => PageGroup::None,
        "sender" => PageGroup::Sender,
        "date" => PageGroup::Date,
        "label" => PageGroup::Label,
        "unread" => PageGroup::Unread,
        _ => return None,
    })
}

fn part_items(parts: PageParts) -> Vec<MenuItem> {
    [
        ("snippet", "Snippet", parts.snippet),
        ("provider", "Provider", parts.provider),
        ("chips", "Chips", parts.chips),
        ("time", "Time", parts.time),
    ]
    .into_iter()
    .map(|(key, name, part)| MenuItem {
        key: key.to_owned(),
        tile: Tile::Icon(Icon::Columns),
        name: name.to_owned(),
        help: None,
        right: Right::Check(part.shown()),
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    })
    .collect()
}

/// The Group and Properties buttons, and whichever menu is open.
#[component]
pub(super) fn PageMenus(shell: Signal<Shell>) -> Element {
    let open = shell.read().page_menu;
    let group = shell.read().group;
    let parts = shell.read().parts;
    let mut group_button = use_signal(|| None::<MountedRef>);
    let mut parts_button = use_signal(|| None::<MountedRef>);
    rsx! {
        ds::Button {
            variant: ds::ButtonVariant::Mini,
            label: "Group",
            icon: Icon::Group,
            aria_label: "Group".to_owned(),
            mounted: move |event: MountedEvent| group_button.set(Some(MountedRef(event.data()))),
            expanded: if open == PageMenu::Group { ds::Expanded::Open } else { ds::Expanded::Closed },
            onclick: on_primary(move || {
                let next = if shell.peek().page_menu == PageMenu::Group {
                    PageMenu::Closed
                } else {
                    PageMenu::Group
                };
                shell.write().page_menu = next;
            }),
        }
        ds::Button {
            variant: ds::ButtonVariant::Mini,
            label: "Properties",
            icon: Icon::Columns,
            aria_label: "Properties".to_owned(),
            mounted: move |event: MountedEvent| parts_button.set(Some(MountedRef(event.data()))),
            expanded: if open == PageMenu::Properties { ds::Expanded::Open } else { ds::Expanded::Closed },
            onclick: on_primary(move || {
                let next = if shell.peek().page_menu == PageMenu::Properties {
                    PageMenu::Closed
                } else {
                    PageMenu::Properties
                };
                shell.write().page_menu = next;
            }),
        }
        if open == PageMenu::Group {
            Floating {
                kind: MenuKind::Rich,
                anchor: group_button(),
                title: "This page".to_owned(),
                items: group_items(group),
                on_pick: move |key: String| {
                    if let Some(group) = group_from(&key) {
                        shell.write().group = group;
                        shell.write().page_menu = PageMenu::Closed;
                    }
                },
                on_close: move |_| shell.write().page_menu = PageMenu::Closed,
            }
        }
        if open == PageMenu::Properties {
            // A checklist: each pick shows or hides a part, and the menu stays open.
            Floating {
                kind: MenuKind::Rich,
                anchor: parts_button(),
                title: "This page".to_owned(),
                items: part_items(parts),
                dismiss: ds::PickDismiss::Stay,
                on_pick: move |key: String| {
                    if let Some(part) = shell.write().parts.part_mut(&key) {
                        *part = (*part).toggle();
                    }
                },
                on_close: move |_| shell.write().page_menu = PageMenu::Closed,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mail_domain::*;

    fn thread(name: &str, email: &str, days_ago: i64, unread: bool) -> ThreadSummary {
        let now = Utc.with_ymd_and_hms(2026, 9, 23, 15, 0, 0).unwrap();
        ThreadSummary {
            id: ThreadId::generate(),
            account: AccountId::generate(),
            subject: name.to_owned(),
            snippet: String::new(),
            from: Address {
                name: Some(name.to_owned()),
                email: email.to_owned(),
            },
            participants: Vec::new(),
            recipients: Vec::new(),
            last_date: now - chrono::TimeDelta::try_days(days_ago).unwrap_or_default(),
            message_count: 1,
            read: if unread {
                ReadState::Unread
            } else {
                ReadState::Read
            },
            star: Star::Unstarred,
            mailboxes: MailboxSet::only(MailboxRole::Inbox),
            labels: Vec::new(),
            attachments: Attachments::None,
            snooze: Snooze::Inactive,
            pin: Pin::Unpinned,
        }
    }

    #[test]
    fn dates_fall_into_the_four_bands() {
        let now = Utc.with_ymd_and_hms(2026, 9, 23, 15, 0, 0).unwrap();
        let threads = vec![
            thread("today", "a@b.c", 0, true),
            thread("yesterday", "a@b.c", 1, true),
            thread("week", "a@b.c", 3, false),
            thread("old", "a@b.c", 20, false),
        ];
        let bands = group_page(threads, PageGroup::Date, &BTreeMap::new(), now, &Utc);
        let titles: Vec<&str> = bands
            .iter()
            .filter_map(|band| band.title.as_deref())
            .collect();
        assert_eq!(titles, ["Today", "Yesterday", "This week", "Older"]);
    }

    #[test]
    fn none_has_no_header() {
        let now = Utc::now();
        let bands = group_page(
            vec![thread("a", "a@b.c", 0, true)],
            PageGroup::None,
            &BTreeMap::new(),
            now,
            &Utc,
        );
        assert_eq!(bands.len(), 1);
        assert!(bands[0].title.is_none());
    }
}
