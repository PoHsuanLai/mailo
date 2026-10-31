//! Group and Row Properties, applied to the loaded page only, and the list toolbar's ⋯ that
//! holds them with Sort By, Organise by Conversation and Sync Now.
//!
//! Neither Group nor Row Properties asks the client for a different query; Sort By does, and is
//! kept: on the saved view it sorts, or under the place's key in `settings.toml`. Organise by
//! Conversation does too: it is the place's own choice ([`super::organise`]), and a message to a
//! row is another list.

use ds::prelude::*;
use std::collections::BTreeMap;

use crate::ui::fetching::use_status;
use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::host::measure::MountedRef;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::{Property, Sort, SortDir, ThreadSummary, Threading};
use mail_ui_kit::menu::anchor_at;
use mail_ui_kit::press::on_primary;
use mail_ui_kit::view::{PageGroup, PageMenu, PageParts, RowPart, Shell, SortKept};
use mail_ui_model::copy;
use std::borrow::Borrow;

/// One band of the loaded page: its rows, each a conversation's summary or something that holds
/// one (a message to a row holds its own, beside which message it is).
pub(super) struct Band<T = ThreadSummary> {
    pub title: Option<String>,
    pub threads: Vec<T>,
}

/// Group `threads` the way the page menu says. The input order is the list order.
pub(super) fn group_page<T: Borrow<ThreadSummary>, Tz: TimeZone>(
    threads: Vec<T>,
    by: PageGroup,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<Band<T>>
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

pub(super) fn by_key<T: Borrow<ThreadSummary>>(
    threads: Vec<T>,
    key: impl Fn(&ThreadSummary) -> String,
) -> Vec<Band<T>> {
    let mut order: Vec<String> = Vec::new();
    let mut bands: BTreeMap<String, Vec<T>> = BTreeMap::new();
    for thread in threads {
        let name = key(thread.borrow());
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

pub(super) fn date_bands<T: Borrow<ThreadSummary>, Tz: TimeZone>(
    threads: Vec<T>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<Band<T>>
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
            .signed_duration_since(thread.borrow().last_date.with_timezone(zone).date_naive())
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

pub(super) fn titled<T, const N: usize>(bands: [(&str, Vec<T>); N]) -> Vec<Band<T>> {
    bands
        .into_iter()
        .filter(|(_, threads)| !threads.is_empty())
        .map(|(title, threads)| Band {
            title: Some(title.to_owned()),
            threads,
        })
        .collect()
}

/// The keys Sort By offers, in its order, and the saved-view sheet's Sort by with it. Sender is
/// [`Property::From`], the one a saved view already says.
pub(in crate::ui) const SORT_KEYS: [Property; 6] = [
    Property::Date,
    Property::From,
    Property::Subject,
    Property::Size,
    Property::Unread,
    Property::Starred,
];

/// What a row of the list's ⋯ asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum More {
    /// Sort by this key, the way it first runs ([`Property::natural`]).
    SortBy(Property),
    /// Run the key sorted by this way.
    SortDir(SortDir),
    /// Band the loaded page by date, or stop.
    GroupByDate,
    /// List conversations, or a message to a row, in the place shown ([`super::organise`]).
    Conversations,
    /// Show or hide a part of every row, by its key in [`PageParts::part_mut`].
    Part(&'static str),
    /// Fetch what the list shows.
    Sync,
}

fn check(on: bool) -> Check {
    if on { Check::On } else { Check::Off }
}

/// Whether a list sorted by `property` is sorted by the offered `key`: Sender stands for both
/// of the sender's properties.
pub(in crate::ui) fn sorts_by(key: Property, property: Property) -> bool {
    key == property || (key == Property::From && property == Property::Sender)
}

/// Sort By's rows: each key, the one sorted by checked, then the two ways it runs in its own
/// words, the one it runs checked.
fn sort_items(sort: Sort) -> Vec<MenuItem<More>> {
    let mut rows: Vec<MenuItem<More>> = SORT_KEYS
        .into_iter()
        .map(|key| {
            MenuItem::new(More::SortBy(key), copy::menus::sort_key(key))
                .with_check(check(sorts_by(key, sort.property)))
        })
        .collect();
    rows.push(MenuItem::Separator);
    let [up, down] = copy::menus::sort_dirs(sort.property);
    rows.extend(
        [(SortDir::Asc, up), (SortDir::Desc, down)].map(|(dir, name)| {
            MenuItem::new(More::SortDir(dir), name).with_check(check(sort.dir == dir))
        }),
    );
    rows
}

/// The ⋯'s rows: Sort By where the list can be sorted (`sort` is `None` where it cannot), Group
/// by Date with a check while the page is banded, Organise by Conversation with a check while the
/// place lists conversations (`organise` is `None` where the place offers no choice), then the
/// snippets' and the labels' Show or Hide (a pick switches it and leaves the menu up, so both can
/// be), then Sync Now with what the sync mark says under it. `sync` is `None` while the sidebar,
/// which holds the mark, is shown, and with only local folders in view.
pub(super) fn more_items(
    group: PageGroup,
    sort: Option<Sort>,
    organise: Option<Threading>,
    parts: PageParts,
    sync: Option<String>,
) -> Vec<MenuItem<More>> {
    let toggle = |key: &'static str, part: RowPart, show: &str, hide: &str| {
        MenuItem::new(More::Part(key), if part.shown() { hide } else { show })
            .with_after(AfterPick::KeepOpen)
    };
    let mut rows: Vec<MenuItem<More>> = sort
        .map(|sort| MenuItem::Submenu {
            title: copy::menus::SORT_BY.to_owned(),
            image: None,
            availability: Availability::Enabled,
            children: sort_items(sort),
        })
        .into_iter()
        .collect();
    rows.extend(
        [MenuItem::new(More::GroupByDate, copy::menus::GROUP_BY_DATE)
            .with_image(MenuImage::Icon(Icon::Group))
            .with_check(check(group == PageGroup::Date))],
    );
    if let Some(threading) = organise {
        rows.push(
            MenuItem::new(More::Conversations, copy::menus::ORGANISE_BY_CONVERSATION)
                .with_check(check(threading == Threading::Threaded)),
        );
    }
    rows.extend([
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
    ]);
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

/// What the list menu changed, kept in `settings.toml` so the next window opens the same way.
/// Shown at once whatever the write does; a window with nowhere to write keeps it until it closes.
fn kept(edit: impl FnOnce(&mut mail_ui_model::settings::WindowSettings)) {
    if let Err(why) = mail_ui_kit::prefs::change(|settings| edit(&mut settings.window)) {
        log::warn!("the list's look was not kept: {why}");
    }
}

/// Sort the list by `sort`, and keep the choice: a saved view kept again in the client, which
/// every window reads after `revision` moves; any other place's under its key in
/// `settings.toml`, beside the window's other choices. The list shows the new order at once; the
/// write runs off the drawing thread, and a choice that cannot be kept puts the list back in the
/// order it had and says why.
pub(super) fn sort_list(mut shell: Signal<Shell>, sort: Sort) {
    let previous = shell.peek().sort();
    let Some(kept) = shell.write().sort_by(sort) else {
        return;
    };
    let motion = mail_ui_kit::motion::motion();
    let write: std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>>>> =
        match &kept {
            SortKept::View(view) => Box::pin(mail_ui_kit::motion::kept(
                mail_client::act::Keep::View(view.clone()),
            )),
            SortKept::Place(key, sort) => {
                let (key, sort) = (key.clone(), *sort);
                Box::pin(mail_ui_kit::prefs::change_later(move |settings| {
                    settings
                        .window
                        .sorts
                        .insert(key, mail_ui_model::settings::ListOrder(sort));
                }))
            }
        };
    spawn(async move {
        if let Err(why) = write.await {
            shell.write().unsort(&kept, previous);
            mail_ui_kit::motion::tell_through(motion, copy::status::sort_not_kept(&why));
        }
    });
}

/// The list toolbar's ⋯: what the list offers beyond New Message and Search, folded into one
/// menu so the header is its title and four icons. Sort By asks the client for the list in
/// another order; grouping and the rows' parts are kept as settings ([`kept`]), so every window
/// opens with them. Sync Now is the sidebar's sync mark's press, so it is here only while the
/// sidebar is hidden; `syncs` is false with only local folders in view.
#[component]
pub(super) fn ListMore(shell: Signal<Shell>, syncs: bool, side_hidden: Signal<bool>) -> Element {
    let open = shell.read().page_menu == PageMenu::More;
    let mut tool = use_signal(|| None::<MountedRef>);
    let line = use_status(shell);
    let items = if open {
        let sync = (syncs && side_hidden()).then(|| line.map(|line| line.text).unwrap_or_default());
        let organise = shell.read().organises().then(|| shell.read().threading());
        let shown = shell.read();
        let sort = shown.sortable().then(|| shown.sort());
        more_items(shown.page_group(), sort, organise, shown.parts, sync)
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
                    More::SortBy(property) => {
                        shell.write().page_menu = PageMenu::Closed;
                        let sort = Sort { property, dir: property.natural() };
                        sort_list(shell, sort);
                    }
                    More::SortDir(dir) => {
                        shell.write().page_menu = PageMenu::Closed;
                        let property = shell.peek().sort().property;
                        sort_list(shell, Sort { property, dir });
                    }
                    More::GroupByDate => {
                        let group = {
                            let mut written = shell.write();
                            written.group = match written.group {
                                PageGroup::Date => PageGroup::None,
                                PageGroup::None => PageGroup::Date,
                            };
                            written.page_menu = PageMenu::Closed;
                            written.group
                        };
                        kept(|window| window.group_by_date = group.into());
                    }
                    More::Conversations => {
                        shell.write().page_menu = PageMenu::Closed;
                        super::organise::turn(shell);
                    }
                    // The menu stays up for the next toggle (`AfterPick::KeepOpen`).
                    More::Part(key) => {
                        let parts = {
                            let mut written = shell.write();
                            if let Some(part) = written.parts.part_mut(key) {
                                *part = (*part).toggle();
                            }
                            written.parts
                        };
                        kept(|window| parts.keep(window));
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use ds::prelude::{AfterPick, Check, MenuItem};
    use mail_ui_kit::view::RowPart;

    use mail_domain::*;

    fn thread(name: &str, email: &str, days_ago: i64, unread: bool) -> ThreadSummary {
        let now = Utc.with_ymd_and_hms(2026, 9, 23, 15, 0, 0).unwrap();
        ThreadSummary {
            subject: name.to_owned(),
            from: Address {
                name: Some(name.to_owned()),
                email: email.to_owned(),
            },
            last_date: now - chrono::TimeDelta::try_days(days_ago).unwrap_or_default(),
            read: if unread {
                ReadState::Unread
            } else {
                ReadState::Read
            },
            ..mail_domain::testing::summary()
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
                MenuItem::Swatches(row) => row.title.clone(),
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
                &[
                    mail_ui_model::copy::menus::GROUP_BY_DATE,
                    "-",
                    mail_ui_model::copy::menus::HIDE_SNIPPETS,
                    mail_ui_model::copy::menus::HIDE_LABELS,
                ],
            ),
            (
                PageGroup::Date,
                None,
                &[
                    "✓Group by Date",
                    "-",
                    mail_ui_model::copy::menus::HIDE_SNIPPETS,
                    mail_ui_model::copy::menus::HIDE_LABELS,
                ],
            ),
            (
                PageGroup::None,
                Some(""),
                &[
                    mail_ui_model::copy::menus::GROUP_BY_DATE,
                    "-",
                    mail_ui_model::copy::menus::HIDE_SNIPPETS,
                    mail_ui_model::copy::menus::HIDE_LABELS,
                    "-",
                    mail_ui_model::copy::menus::SYNC_NOW,
                ],
            ),
            (
                PageGroup::None,
                Some("Not updated since 14:05"),
                &[
                    mail_ui_model::copy::menus::GROUP_BY_DATE,
                    "-",
                    mail_ui_model::copy::menus::HIDE_SNIPPETS,
                    mail_ui_model::copy::menus::HIDE_LABELS,
                    "-",
                    "Sync Now / Not updated since 14:05",
                ],
            ),
        ];
        for (group, sync, want) in cases {
            let rows = read(&more_items(
                group,
                None,
                None,
                parts,
                sync.map(str::to_owned),
            ));
            assert_eq!(rows, want, "{group:?} {sync:?}");
        }
        // Where the place offers it, Organise by Conversation follows Group by Date, checked
        // while the place lists conversations.
        for (threading, want) in [
            (Threading::Threaded, "✓Organise by Conversation"),
            (
                Threading::Single,
                mail_ui_model::copy::menus::ORGANISE_BY_CONVERSATION,
            ),
        ] {
            let rows = read(&more_items(
                PageGroup::None,
                None,
                Some(threading),
                parts,
                None,
            ));
            assert_eq!(
                rows[..3],
                [mail_ui_model::copy::menus::GROUP_BY_DATE, want, "-"],
                "{threading:?}"
            );
        }
        // A part hidden is offered back, and each part's pick leaves the menu up.
        let hidden = PageParts {
            snippet: RowPart::Hidden,
            chips: RowPart::Hidden,
        };
        let rows = more_items(PageGroup::None, None, None, hidden, None);
        assert_eq!(
            read(&rows)[2..],
            [
                mail_ui_model::copy::menus::SHOW_SNIPPETS,
                mail_ui_model::copy::menus::SHOW_LABELS
            ]
        );
        assert!(rows[2..].iter().all(|row| matches!(
            row,
            MenuItem::Item {
                after: AfterPick::KeepOpen,
                ..
            }
        )));
    }

    /// Where the list can be sorted the ⋯ opens on Sort By: the keys, the one sorted by checked,
    /// then the two ways that key runs in its own words.
    #[test]
    fn sort_by_lists_the_keys_and_the_ways_the_chosen_one_runs() {
        let parts = PageParts::default();
        let cases: [(Sort, &str); 3] = [
            (
                mail_ui_kit::view::NEWEST_FIRST,
                "Sort By ▸ [✓Date, Sender, Subject, Size, Unread, Starred, -, Oldest First, ✓Newest First]",
            ),
            (
                Sort {
                    property: Property::Subject,
                    dir: SortDir::Asc,
                },
                "Sort By ▸ [Date, Sender, ✓Subject, Size, Unread, Starred, -, ✓A to Z, Z to A]",
            ),
            (
                // A saved view sorted by `Sender` is the menu's Sender.
                Sort {
                    property: Property::Sender,
                    dir: SortDir::Desc,
                },
                "Sort By ▸ [Date, ✓Sender, Subject, Size, Unread, Starred, -, A to Z, ✓Z to A]",
            ),
        ];
        for (sort, want) in cases {
            let rows = read(&more_items(PageGroup::None, Some(sort), None, parts, None));
            assert_eq!(rows[0], want, "{sort:?}");
            assert_eq!(rows[1], mail_ui_model::copy::menus::GROUP_BY_DATE);
        }
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
                    Some(mail_ui_model::copy::headers::TODAY),
                    Some(mail_ui_model::copy::headers::YESTERDAY),
                    Some(mail_ui_model::copy::headers::THIS_WEEK),
                    Some(mail_ui_model::copy::headers::OLDER),
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
