//! Group and Row Properties, applied to the loaded page only, and the list toolbar's ⋯ that
//! holds them with Sync Now; and the list title's menu, the way to where mail is stored.
//!
//! Neither of the first asks the store for a different query.

use ds::prelude::*;
use std::collections::BTreeMap;

use super::menu::anchor_at;
use super::press::on_primary;
use crate::ui::copy;
use crate::ui::fetching::use_status;
use crate::ui::view::{PageGroup, PageMenu, PageParts, Place, Reach, RowPart, Shell};
use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::host::measure::{Anchor, MountedRef};
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::ThreadSummary;

/// One band of the loaded page.
pub(super) struct Band {
    pub title: Option<String>,
    pub threads: Vec<ThreadSummary>,
}

/// Group `threads` the way the page menu says. The input order is the list order.
pub(super) fn group_page<Tz: TimeZone>(
    threads: Vec<ThreadSummary>,
    by: PageGroup,
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
        PageGroup::Date => date_bands(threads, now, zone),
    }
}

pub(super) fn by_key(
    threads: Vec<ThreadSummary>,
    key: impl Fn(&ThreadSummary) -> String,
) -> Vec<Band> {
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

pub(super) fn date_bands<Tz: TimeZone>(
    threads: Vec<ThreadSummary>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<Band>
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
        (copy::headers::TODAY, today),
        (copy::headers::YESTERDAY, yesterday),
        (copy::headers::THIS_WEEK, week),
        (copy::headers::OLDER, older),
    ])
}

pub(super) fn titled<const N: usize>(bands: [(&str, Vec<ThreadSummary>); N]) -> Vec<Band> {
    bands
        .into_iter()
        .filter(|(_, threads)| !threads.is_empty())
        .map(|(title, threads)| Band {
            title: Some(title.to_owned()),
            threads,
        })
        .collect()
}

/// What a row of the list's ⋯ asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum More {
    /// Band the loaded page by date, or stop.
    GroupByDate,
    /// Show or hide a part of every row, by its key in [`PageParts::part_mut`].
    Part(&'static str),
    /// Fetch what the list shows.
    Sync,
}

fn check(on: bool) -> Check {
    if on { Check::On } else { Check::Off }
}

/// The ⋯'s rows: Group by Date with a check while the page is banded, then the snippets' and
/// the labels' Show or Hide (a pick switches it and leaves the menu up, so both can be), then
/// Sync Now with what the sync mark says under it. `sync` is `None` while the sidebar, which
/// holds the mark, is shown, and with only local folders in view.
pub(super) fn more_items(
    group: PageGroup,
    parts: PageParts,
    sync: Option<String>,
) -> Vec<MenuItem<More>> {
    let toggle = |key: &'static str, part: RowPart, show: &str, hide: &str| {
        MenuItem::new(More::Part(key), if part.shown() { hide } else { show })
            .with_after(AfterPick::KeepOpen)
    };
    let mut rows = vec![
        MenuItem::new(More::GroupByDate, copy::menus::GROUP_BY_DATE)
            .with_image(MenuImage::Icon(Icon::Group))
            .with_check(check(group == PageGroup::Date)),
        MenuItem::Separator,
        toggle(
            "snippet",
            parts.snippet,
            copy::menus::SHOW_SNIPPETS,
            copy::menus::HIDE_SNIPPETS,
        ),
        toggle(
            "chips",
            parts.chips,
            copy::menus::SHOW_LABELS,
            copy::menus::HIDE_LABELS,
        ),
    ];
    if let Some(said) = sync {
        let item = MenuItem::new(More::Sync, copy::menus::SYNC_NOW)
            .with_image(MenuImage::Icon(Icon::Refresh));
        rows.push(MenuItem::Separator);
        rows.push(match said.is_empty() {
            true => item,
            false => item.with_subtitle(said),
        });
    }
    rows
}

/// The list toolbar's ⋯: what the list offers beyond New Message and Search, folded into one
/// menu so the header is its title and three icons. Grouping and the rows' parts apply to the
/// loaded page only. Sync Now is the sidebar's sync mark's press, so it is here only while the
/// sidebar is hidden; `syncs` is false with only local folders in view.
#[component]
pub(super) fn ListMore(shell: Signal<Shell>, syncs: bool, side_hidden: Signal<bool>) -> Element {
    let open = shell.read().page_menu == PageMenu::More;
    let mut tool = use_signal(|| None::<MountedRef>);
    let line = use_status(shell);
    let items = if open {
        let sync = (syncs && side_hidden()).then(|| line.map(|line| line.text).unwrap_or_default());
        more_items(shell.read().group, shell.read().parts, sync)
    } else {
        Vec::new()
    };
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            size: ControlSize::Large,
            image: ImagePosition::Only,
            label: copy::aria::MORE,
            title: Some(copy::tooltips::MORE.to_owned()),
            icon: Some(IconSource::Glyph(Icon::Ellipsis)),
            shown: if open { Shown::Visible } else { Shown::Hidden },
            common: Common {
                mounted: Some(EventHandler::new(move |event: MountedEvent| {
                    tool.set(Some(MountedRef(event.data())));
                })),
                ..Common::default()
            },
            onclick: on_primary(move || {
                let next = if shell.peek().page_menu == PageMenu::More {
                    PageMenu::Closed
                } else {
                    PageMenu::More
                };
                shell.write().page_menu = next;
            }),
        }
        if open {
            Menu::<More> {
                placement: MenuPlacement::Popup,
                anchor: anchor_at(tool()),
                items,
                common: Common {
                    aria_label: Some(copy::aria::LIST_OPTIONS.to_owned()),
                    ..Common::default()
                },
                onpick: move |pick: More| match pick {
                    More::GroupByDate => {
                        let mut written = shell.write();
                        written.group = match written.group {
                            PageGroup::Date => PageGroup::None,
                            PageGroup::None => PageGroup::Date,
                        };
                        written.page_menu = PageMenu::Closed;
                    }
                    // The menu stays up for the next toggle (`AfterPick::KeepOpen`).
                    More::Part(key) => {
                        if let Some(part) = shell.write().parts.part_mut(key) {
                            *part = (*part).toggle();
                        }
                    }
                    More::Sync => {
                        shell.write().page_menu = PageMenu::Closed;
                        crate::ui::fetching::sync_now(&shell.read());
                    }
                },
                onclose: move |()| shell.write().page_menu = PageMenu::Closed,
            }
        }
    }
}

/// The list title's menu: the places where mail is stored that the sidebar leaves out (Sent,
/// Drafts, Spam, Trash), each by its index among `places`, with a check on the one shown.
pub(super) fn title_items(places: &[Place], selected: usize) -> Vec<MenuItem<usize>> {
    places
        .iter()
        .enumerate()
        .filter(|(_, place)| place.reach == Reach::TitleMenu)
        .map(|(index, place)| {
            MenuItem::new(index, place.name.clone()).with_check(check(index == selected))
        })
        .collect()
}

/// The list's title, whose right click is the menu of the places one step away
/// ([`title_items`]).
#[component]
pub(super) fn ListTitle(shell: Signal<Shell>, pages: Signal<u32>, children: Element) -> Element {
    let mut at = use_signal(|| None::<Point>);
    let items = match at() {
        Some(_) => title_items(&shell.read().places, shell.read().selected),
        None => Vec::new(),
    };
    rsx! {
        div {
            class: "list-title",
            oncontextmenu: move |event: MouseEvent| {
                event.prevent_default();
                let point = event.client_coordinates();
                at.set(Some(Point { x: Px(point.x as f32), y: Px(point.y as f32) }));
            },
            {children}
        }
        if let Some(point) = at() {
            Menu::<usize> {
                placement: MenuPlacement::default(),
                anchor: Anchor::Point(point),
                items,
                common: Common {
                    aria_label: Some(copy::aria::GO_TO.to_owned()),
                    ..Common::default()
                },
                onpick: move |index: usize| {
                    at.set(None);
                    shell.write().select(index);
                    pages.set(1);
                },
                onclose: move |()| at.set(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::view::RowPart;
    use chrono::TimeZone;
    use ds::prelude::{AfterPick, Check, MenuItem};
    use mail_domain::id::new_account_id;
    use mail_domain::*;

    fn thread(name: &str, email: &str, days_ago: i64, unread: bool) -> ThreadSummary {
        let now = Utc.with_ymd_and_hms(2026, 9, 23, 15, 0, 0).unwrap();
        ThreadSummary {
            id: ThreadId::generate(),
            account: new_account_id(),
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
            mute: Mute::Unmuted,
            follow_up: mail_domain::FollowUp::Inactive,
        }
    }

    /// The ⋯'s rows as a person reads them: a submenu as `Name ▸ [children]`, a checked item
    /// with `✓`, a rule as `-`, an item's second line after `/`.
    fn read(rows: &[MenuItem<More>]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                MenuItem::Item {
                    title, check, text, ..
                } => {
                    let mark = if *check == Some(Check::On) { "✓" } else { "" };
                    match &text.subtitle {
                        Some(under) => format!("{mark}{title} / {under}"),
                        None => format!("{mark}{title}"),
                    }
                }
                MenuItem::Submenu {
                    title, children, ..
                } => format!("{title} ▸ [{}]", read(children).join(", ")),
                MenuItem::Separator => "-".to_owned(),
                MenuItem::Header(title) | MenuItem::Info { title, .. } => title.clone(),
            })
            .collect()
    }

    /// The ⋯ holds Group by Date with its check, the snippets' and labels' Show or Hide, and
    /// Sync Now under a rule, saying how fresh the mail is, only where it is offered (the
    /// sidebar hidden, something to sync).
    #[test]
    fn the_more_menu_holds_group_by_date_the_parts_and_sync_now() {
        let parts = PageParts::default();
        let cases: [(PageGroup, Option<&str>, &[&str]); 4] = [
            (
                PageGroup::None,
                None,
                &["Group by Date", "-", "Hide Snippets", "Hide Labels"],
            ),
            (
                PageGroup::Date,
                None,
                &["✓Group by Date", "-", "Hide Snippets", "Hide Labels"],
            ),
            (
                PageGroup::None,
                Some(""),
                &[
                    "Group by Date",
                    "-",
                    "Hide Snippets",
                    "Hide Labels",
                    "-",
                    "Sync Now",
                ],
            ),
            (
                PageGroup::None,
                Some("Not updated since 14:05"),
                &[
                    "Group by Date",
                    "-",
                    "Hide Snippets",
                    "Hide Labels",
                    "-",
                    "Sync Now / Not updated since 14:05",
                ],
            ),
        ];
        for (group, sync, want) in cases {
            let rows = read(&more_items(group, parts, sync.map(str::to_owned)));
            assert_eq!(rows, want, "{group:?} {sync:?}");
        }
        // A part hidden is offered back, and each part's pick leaves the menu up.
        let hidden = PageParts {
            snippet: RowPart::Hidden,
            chips: RowPart::Hidden,
        };
        let rows = more_items(PageGroup::None, hidden, None);
        assert_eq!(read(&rows)[2..], ["Show Snippets", "Show Labels"]);
        assert!(rows[2..].iter().all(|row| matches!(
            row,
            MenuItem::Item {
                after: AfterPick::KeepOpen,
                ..
            }
        )));
    }

    /// The page menu's grouping: dates fall into the four bands, and no grouping is one band
    /// with no header.
    #[test]
    fn group_page_bands() {
        let now = Utc.with_ymd_and_hms(2026, 9, 23, 15, 0, 0).unwrap();
        type Row = (
            &'static str,
            Vec<ThreadSummary>,
            PageGroup,
            &'static [Option<&'static str>],
        );
        let cases: [Row; 2] = [
            (
                "by date",
                vec![
                    thread("today", "a@b.c", 0, true),
                    thread("yesterday", "a@b.c", 1, true),
                    thread("week", "a@b.c", 3, false),
                    thread("old", "a@b.c", 20, false),
                ],
                PageGroup::Date,
                &[
                    Some("Today"),
                    Some("Yesterday"),
                    Some("This week"),
                    Some("Older"),
                ],
            ),
            (
                "none",
                vec![thread("a", "a@b.c", 0, true)],
                PageGroup::None,
                &[None],
            ),
        ];
        for (name, threads, group, want) in cases {
            let bands = group_page(threads, group, now, &Utc);
            let titles: Vec<Option<&str>> =
                bands.iter().map(|band| band.title.as_deref()).collect();
            assert_eq!(titles, want, "{name}");
        }
    }
}
