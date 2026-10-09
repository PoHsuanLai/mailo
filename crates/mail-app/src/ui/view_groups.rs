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
    use mail_domain::id::new_account_id;
    use mail_domain::*;

    fn thread(subject: &str, read: ReadState, star: Star, labels: Vec<LabelId>) -> ThreadSummary {
        ThreadSummary {
            id: ThreadId::generate(),
            account: new_account_id(),
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

    /// A view groups its rows into named bands, keeping the list's order inside each band. A
    /// label band is named for the label and its absence, a mailbox band uses the sidebar's
    /// names, a band with nothing in it is not drawn, and the page menu's grouping is still the
    /// page menu's.
    #[test]
    fn a_view_groups_its_rows_into_named_bands() {
        use ReadState::*;
        use Star::*;
        let travel = LabelId::generate();
        let work = LabelId::generate();
        let labels = BTreeMap::from([(travel, "Travel".to_owned()), (work, "Work".to_owned())]);
        let starred_and_read = || {
            vec![
                thread("a", Read, Starred, vec![]),
                thread("b", Unread, Unstarred, vec![]),
                thread("c", Unread, Starred, vec![]),
            ]
        };
        let mut archived = thread("old", Read, Unstarred, vec![]);
        archived.mailboxes = MailboxSet::only(MailboxRole::Archive);
        type Row = (
            &'static str,
            Vec<ThreadSummary>,
            Grouping,
            &'static [(&'static str, &'static [&'static str])],
        );
        let cases: [Row; 6] = [
            (
                "by star",
                starred_and_read(),
                Grouping::Saved(GroupKey::Star),
                &[("Starred", &["a", "c"]), ("Not starred", &["b"])],
            ),
            (
                "by read",
                starred_and_read(),
                Grouping::Saved(GroupKey::Read),
                &[("Unread", &["b", "c"]), ("Read", &["a"])],
            ),
            (
                "by a label",
                vec![
                    thread("flight", Read, Unstarred, vec![travel]),
                    thread("report", Read, Unstarred, vec![work]),
                ],
                Grouping::Saved(GroupKey::Label(travel)),
                &[("Travel", &["flight"]), ("Not Travel", &["report"])],
            ),
            (
                "an empty band",
                vec![thread("a", Read, Unstarred, vec![])],
                Grouping::Saved(GroupKey::Read),
                &[("Read", &["a"])],
            ),
            (
                "by where it is",
                vec![thread("new", Read, Unstarred, vec![]), archived],
                Grouping::Saved(GroupKey::Mailbox),
                &[("Inbox", &["new"]), ("Archive", &["old"])],
            ),
            (
                "the page menu's",
                vec![thread("a", Unread, Unstarred, vec![])],
                Grouping::Page(PageGroup::Unread),
                &[("Unread", &["a"])],
            ),
        ];
        for (name, threads, grouping, want) in cases {
            let bands = group_list(threads, &grouping, &labels, Utc::now(), &Utc);
            let want: Vec<(String, Vec<String>)> = want
                .iter()
                .map(|(title, subjects)| {
                    (
                        (*title).to_owned(),
                        subjects.iter().map(|one| (*one).to_owned()).collect(),
                    )
                })
                .collect();
            assert_eq!(titles(&bands), want, "{name}");
        }
    }
}
