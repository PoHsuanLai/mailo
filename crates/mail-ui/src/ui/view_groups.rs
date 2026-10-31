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
use std::borrow::Borrow;
use std::collections::BTreeMap;

/// Group `threads` the way the list is grouped now ([`crate::ui::view::Shell::grouping`]).
pub(super) fn group_list<T: Borrow<ThreadSummary>, Tz: TimeZone>(
    threads: Vec<T>,
    by: &Grouping,
    names: &BTreeMap<LabelId, String>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<Band<T>>
where
    Tz::Offset: std::fmt::Display,
{
    match by {
        Grouping::Page(page) => group_page(threads, *page, now, zone),
        Grouping::Saved(key) => saved_bands(threads, key, names, now, zone),
    }
}

fn saved_bands<T: Borrow<ThreadSummary>, Tz: TimeZone>(
    threads: Vec<T>,
    key: &GroupKey,
    names: &BTreeMap<LabelId, String>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<Band<T>>
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
            Property::From | Property::Sender => by_key(threads, |t| t.from.shown().to_owned()),
            Property::Subject => by_key(threads, |t| t.subject.clone()),
            Property::Attachments => two(threads, "With attachments", "Without", |t| {
                matches!(t.attachments, Attachments::Present { .. })
            }),
            Property::Pin => two(threads, "Pinned", "Not pinned", |t| {
                matches!(t.pin, Pin::Rank(_))
            }),
            // The sorts by read state and by star band as those groupings do.
            Property::Unread => saved_bands(threads, &GroupKey::Read, names, now, zone),
            Property::Starred => saved_bands(threads, &GroupKey::Star, names, now, zone),
            // A summary carries no size, so there is nothing to band by: one list.
            Property::Size => vec![Band {
                title: None,
                threads,
            }],
        },
    }
}

/// The ones `is` holds for under `yes`, then the rest under `no`; an empty band is not drawn.
fn two<T: Borrow<ThreadSummary>>(
    threads: Vec<T>,
    yes: &str,
    no: &str,
    is: impl Fn(&ThreadSummary) -> bool,
) -> Vec<Band<T>> {
    let (held, rest): (Vec<_>, Vec<_>) = threads.into_iter().partition(|t| is(t.borrow()));
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
            subject: subject.to_owned(),
            from: Address {
                name: None,
                email: "a@example.test".to_owned(),
            },
            last_date: Utc::now(),
            read,
            star,
            labels,
            ..mail_domain::testing::summary()
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
                &[
                    (mail_ui_model::copy::fields::STARRED, &["a", "c"]),
                    (mail_ui_model::copy::fields::NOT_STARRED, &["b"]),
                ],
            ),
            (
                "by read",
                starred_and_read(),
                Grouping::Saved(GroupKey::Read),
                &[
                    (mail_ui_model::copy::headers::UNREAD, &["b", "c"]),
                    (mail_ui_model::copy::headers::READ, &["a"]),
                ],
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
                &[(mail_ui_model::copy::headers::READ, &["a"])],
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
                Grouping::Page(PageGroup::Date),
                &[(mail_ui_model::copy::headers::TODAY, &["a"])],
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
