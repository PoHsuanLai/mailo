//! A saved view's grouping, drawn as the list's bands.
//!
//! The Group menu's bands are [`super::page`]'s; a view's [`GroupKey`] says more than the menu
//! can (starred, a label's yes and no, where a conversation is), so it has its own here, built
//! from the same pieces. Like the menu's, the bands are over the rows loaded, in list order.

use super::page::{Band, by_key, date_bands, group_page, titled};
use crate::ui::view::Grouping;
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{
    Attachments, GroupKey, LabelId, MailboxRole, Pin, Property, ReadState, Star, ThreadSummary,
};
use std::collections::BTreeMap;

/// Group `threads` the way the list is grouped now ([`crate::ui::view::Shell::grouping`]).
pub(super) fn group_list<Tz: TimeZone>(
    threads: Vec<ThreadSummary>,
    by: &Grouping,
    names: &BTreeMap<LabelId, String>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<Band>
where
    Tz::Offset: std::fmt::Display,
{
    match by {
        Grouping::Page(page) => group_page(threads, *page, names, now, zone),
        Grouping::Saved(key) => saved_bands(threads, key, names, now, zone),
    }
}

fn saved_bands<Tz: TimeZone>(
    threads: Vec<ThreadSummary>,
    key: &GroupKey,
    names: &BTreeMap<LabelId, String>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<Band>
where
    Tz::Offset: std::fmt::Display,
{
    match key {
        GroupKey::Read => two(threads, "Unread", "Read", |t| t.read == ReadState::Unread),
        GroupKey::Star => two(threads, "Starred", "Not starred", |t| {
            t.star == Star::Starred
        }),
        GroupKey::Label(id) => {
            let name = names
                .get(id)
                .cloned()
                .unwrap_or_else(|| "Labelled".to_owned());
            let not = format!("Not {name}");
            two(threads, &name, &not, |t| t.labels.contains(id))
        }
        GroupKey::Mailbox => by_key(threads, |t| where_it_is(t).to_owned()),
        GroupKey::Property(property) => match property {
            Property::Date => date_bands(threads, now, zone),
            Property::From | Property::Sender => by_key(threads, |t| {
                t.from
                    .name
                    .clone()
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| t.from.email.clone())
            }),
            Property::Subject => by_key(threads, |t| t.subject.clone()),
            Property::Attachments => two(threads, "With attachments", "Without", |t| {
                matches!(t.attachments, Attachments::Present { .. })
            }),
            Property::Pin => two(threads, "Pinned", "Not pinned", |t| {
                matches!(t.pin, Pin::Rank(_))
            }),
            // A summary carries no size, so there is nothing to band by: one list.
            Property::Size => vec![Band {
                title: None,
                threads,
            }],
        },
    }
}

/// The ones `is` holds for under `yes`, then the rest under `no`; an empty band is not drawn.
fn two(
    threads: Vec<ThreadSummary>,
    yes: &str,
    no: &str,
    is: impl Fn(&ThreadSummary) -> bool,
) -> Vec<Band> {
    let (held, rest): (Vec<_>, Vec<_>) = threads.into_iter().partition(|t| is(t));
    titled([(yes, held), (no, rest)])
}

/// The first place a conversation is, in the sidebar's order.
fn where_it_is(thread: &ThreadSummary) -> &'static str {
    [
        (MailboxRole::Inbox, "Inbox"),
        (MailboxRole::Archive, "Archive"),
        (MailboxRole::Sent, "Sent"),
        (MailboxRole::Drafts, "Drafts"),
        (MailboxRole::Spam, "Spam"),
        (MailboxRole::Trash, "Trash"),
    ]
    .into_iter()
    .find(|(role, _)| thread.mailboxes.contains(*role))
    .map_or("Elsewhere", |(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::view::PageGroup;
    use mail_domain::*;

    fn thread(subject: &str, read: ReadState, star: Star, labels: Vec<LabelId>) -> ThreadSummary {
        ThreadSummary {
            id: ThreadId::generate(),
            account: AccountId::generate(),
            subject: subject.to_owned(),
            snippet: String::new(),
            from: Address {
                name: None,
                email: "a@example.test".to_owned(),
            },
            participants: Vec::new(),
            recipients: Vec::new(),
            last_date: Utc::now(),
            message_count: 1,
            read,
            star,
            mailboxes: MailboxSet::only(MailboxRole::Inbox),
            labels,
            attachments: Attachments::None,
            snooze: Snooze::Inactive,
            pin: Pin::Unpinned,
            mute: Mute::Unmuted,
            follow_up: mail_domain::FollowUp::Inactive,
        }
    }

    fn titles(bands: &[Band]) -> Vec<(String, Vec<String>)> {
        bands
            .iter()
            .map(|band| {
                (
                    band.title.clone().unwrap_or_default(),
                    band.threads.iter().map(|t| t.subject.clone()).collect(),
                )
            })
            .collect()
    }

    fn grouped(
        threads: Vec<ThreadSummary>,
        key: GroupKey,
        names: &BTreeMap<LabelId, String>,
    ) -> Vec<(String, Vec<String>)> {
        titles(&group_list(
            threads,
            &Grouping::Saved(key),
            names,
            Utc::now(),
            &Utc,
        ))
    }

    fn s(list: &[&str]) -> Vec<String> {
        list.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn a_view_grouped_by_star_or_read_keeps_the_list_order_inside_each_band() {
        use ReadState::*;
        use Star::*;
        let threads = || {
            vec![
                thread("a", Read, Starred, vec![]),
                thread("b", Unread, Unstarred, vec![]),
                thread("c", Unread, Starred, vec![]),
            ]
        };
        let none = BTreeMap::new();
        assert_eq!(
            grouped(threads(), GroupKey::Star, &none),
            vec![
                ("Starred".into(), s(&["a", "c"])),
                ("Not starred".into(), s(&["b"]))
            ]
        );
        assert_eq!(
            grouped(threads(), GroupKey::Read, &none),
            vec![
                ("Unread".into(), s(&["b", "c"])),
                ("Read".into(), s(&["a"]))
            ]
        );
    }

    #[test]
    fn a_view_grouped_by_a_label_names_it_and_its_absence() {
        let travel = LabelId::generate();
        let other = LabelId::generate();
        let names = BTreeMap::from([(travel, "Travel".to_owned()), (other, "Work".to_owned())]);
        let threads = vec![
            thread("flight", ReadState::Read, Star::Unstarred, vec![travel]),
            thread("report", ReadState::Read, Star::Unstarred, vec![other]),
        ];
        assert_eq!(
            grouped(threads, GroupKey::Label(travel), &names),
            vec![
                ("Travel".into(), s(&["flight"])),
                ("Not Travel".into(), s(&["report"]))
            ]
        );
    }

    #[test]
    fn a_band_with_nothing_in_it_is_not_drawn() {
        let threads = vec![thread("a", ReadState::Read, Star::Unstarred, vec![])];
        assert_eq!(
            grouped(threads, GroupKey::Read, &BTreeMap::new()),
            vec![("Read".into(), s(&["a"]))]
        );
    }

    #[test]
    fn a_view_grouped_by_where_it_is_uses_the_sidebars_names() {
        let mut archived = thread("old", ReadState::Read, Star::Unstarred, vec![]);
        archived.mailboxes = MailboxSet::only(MailboxRole::Archive);
        let threads = vec![
            thread("new", ReadState::Read, Star::Unstarred, vec![]),
            archived,
        ];
        assert_eq!(
            grouped(threads, GroupKey::Mailbox, &BTreeMap::new()),
            vec![
                ("Inbox".into(), s(&["new"])),
                ("Archive".into(), s(&["old"]))
            ]
        );
    }

    #[test]
    fn the_page_menu_grouping_is_still_the_page_menus() {
        let threads = vec![thread("a", ReadState::Unread, Star::Unstarred, vec![])];
        let bands = group_list(
            threads,
            &Grouping::Page(PageGroup::Unread),
            &BTreeMap::new(),
            Utc::now(),
            &Utc,
        );
        assert_eq!(titles(&bands), vec![("Unread".into(), s(&["a"]))]);
    }
}
