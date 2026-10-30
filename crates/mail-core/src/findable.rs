//! What a search outside mailo may find of the mail without asking it: the recent conversations
//! and the people in the address book, each as a title, a subtitle and the words that find it.
//!
//! A desktop's launcher searches as the person types, and cannot start mail for every key. So the
//! front end hands a copy of these to the desktop while it runs (`mail_intents::index`), and
//! the desktop matches them itself. Mail's own search box stays the way to search the words of the
//! mail: nothing here carries a body or a snippet.
//!
//! Plain values over the store, worded by nobody: a subject is the subject as written, and the one
//! word this module supplies is [`mail_domain::NO_SUBJECT`], what the window shows for a
//! conversation with none ([`subject_or_none`]). [`changes`] says what moved between two readings, so a front end hands
//! on only that.

use crate::error::CoreError;
use crate::notify::{Own, own_addresses};
use chrono::{DateTime, Utc};
use mail_domain::{
    Address, Filter, LabelId, MailboxRole, PageReq, Property, Query, Sort, SortDir, ThreadSummary,
    subject_or_none,
};
use mail_store::{SqliteStore, Store, StoreError};
use std::collections::{BTreeMap, HashMap};

/// The most conversations handed on, newest first.
///
/// Each one is a few hundred bytes at the desktop, which keeps them in memory and on disk and
/// matches every one of them at each key typed. Five thousand is about a year of a busy person's
/// mail, which is what a launcher is asked for; older mail is found through mail's own search.
pub const CONVERSATIONS: usize = 5_000;

/// The most people handed on, in the address book's own order (those written to most and most
/// recently first). Only those the composer offers: never the person's own addresses, and never a
/// list or no-reply sender they have not written to.
pub const CONTACTS: usize = 2_000;

/// How many conversations are read from the store at a time.
const PAGE: u32 = 500;

/// How many of the other people a conversation's subtitle names.
const NAMED: usize = 5;

/// What a findable thing is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// A conversation, keyed by its thread id.
    Conversation,
    /// A person in the address book, keyed by their address.
    Contact,
}

/// One thing a search outside mailo may find.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Findable {
    pub kind: Kind,
    /// The thread id, or the contact's address: the key the desktop's intents name it by.
    pub key: String,
    /// A conversation's subject, or a person's name (their address when they gave none).
    pub title: String,
    /// The other people in a conversation, latest sender first; a person's address.
    pub subtitle: String,
    /// The words that find it beyond those: addresses, names, label names, an organisation.
    pub keywords: Vec<String>,
    /// When it last changed: a conversation's newest message, the last mail to or from a person.
    pub updated: DateTime<Utc>,
}

impl Findable {
    /// What it is known by: its kind and its key.
    pub fn id(&self) -> (Kind, String) {
        (self.kind, self.key.clone())
    }
}

/// Every findable thing, by what it is known by.
pub type Findables = BTreeMap<(Kind, String), Findable>;

/// What moved between two readings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Changes {
    /// What is new, or differs.
    pub upserts: Vec<Findable>,
    /// What is gone: it left the store, the bound, or the places that are kept.
    pub removals: Vec<(Kind, String)>,
}

impl Changes {
    /// Whether nothing moved.
    pub fn is_empty(&self) -> bool {
        self.upserts.is_empty() && self.removals.is_empty()
    }
}

/// What `after` says that `before` did not.
pub fn changes(before: &Findables, after: &Findables) -> Changes {
    Changes {
        upserts: after
            .iter()
            .filter(|(id, now)| before.get(*id) != Some(*now))
            .map(|(_, now)| now.clone())
            .collect(),
        removals: before
            .keys()
            .filter(|id| !after.contains_key(*id))
            .cloned()
            .collect(),
    }
}

/// Everything to hand on: the [`CONVERSATIONS`] newest conversations and the [`CONTACTS`] first
/// people. `now` decides which snoozes are due, as it does for any listing.
pub fn read(store: &SqliteStore, now: DateTime<Utc>) -> Result<Findables, CoreError> {
    let own = own_addresses(store)?;
    let mut all = conversations(store, &own, CONVERSATIONS, now)?;
    all.extend(contacts(store, CONTACTS)?);
    Ok(all.into_iter().map(|found| (found.id(), found)).collect())
}

/// Every conversation that is not only in Trash or Spam, or in both and nowhere else. One that
/// has a message anywhere else, or that is in no folder at all (a label's), is kept.
fn kept() -> Filter {
    let away = [MailboxRole::Trash, MailboxRole::Spam];
    let elsewhere = MailboxRole::ALL
        .into_iter()
        .filter(|role| !away.contains(role))
        .map(Filter::InMailbox)
        .collect();
    Filter::Not(Box::new(Filter::And(vec![
        Filter::Or(away.into_iter().map(Filter::InMailbox).collect()),
        Filter::Not(Box::new(Filter::Or(elsewhere))),
    ])))
}

/// The `limit` newest conversations [`kept`] keeps, newest first.
pub fn conversations(
    store: &SqliteStore,
    own: &Own,
    limit: usize,
    now: DateTime<Utc>,
) -> Result<Vec<Findable>, StoreError> {
    let labels: HashMap<LabelId, String> = store
        .list_accounts()?
        .into_iter()
        .map(|account| store.labels(account.id))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .map(|label| (label.id, label.name))
        .collect();
    let mut found = Vec::new();
    let mut after = None;
    while found.len() < limit {
        let page = store.threads(
            &Query {
                filter: kept(),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq { after, limit: PAGE },
            },
            now,
        )?;
        let room = limit - found.len();
        found.extend(
            page.items
                .iter()
                .take(room)
                .map(|summary| conversation(summary, own, &labels)),
        );
        match page.next {
            Some(next) if !page.items.is_empty() => after = Some(next),
            _ => break,
        }
    }
    Ok(found)
}

/// The people of `summary` other than the person, latest sender first, each once.
fn others<'a>(summary: &'a ThreadSummary, own: &Own) -> Vec<&'a Address> {
    let everyone = std::iter::once(&summary.from)
        .chain(summary.participants.iter().rev())
        .chain(summary.recipients.iter().rev());
    let mut seen: Vec<String> = Vec::new();
    everyone
        .filter(|address| !own.includes(&address.email))
        .filter(|address| {
            let email = address.email.trim().to_ascii_lowercase();
            let new = !email.is_empty() && !seen.contains(&email);
            seen.push(email);
            new
        })
        .collect()
}

fn conversation(summary: &ThreadSummary, own: &Own, labels: &HashMap<LabelId, String>) -> Findable {
    let others = others(summary, own);
    let subtitle = match others.as_slice() {
        // A note to oneself has nobody else in it: it says who sent it.
        [] => summary.from.shown().to_owned(),
        some => some
            .iter()
            .take(NAMED)
            .map(|address| address.shown())
            .collect::<Vec<_>>()
            .join(", "),
    };
    let mut keywords: Vec<String> = Vec::new();
    let words = others
        .iter()
        .flat_map(|address| [Some(address.email.as_str()), address.display_name()])
        .flatten()
        .chain(
            summary
                .labels
                .iter()
                .filter_map(|id| labels.get(id))
                .map(String::as_str),
        );
    for word in words {
        if !keywords.iter().any(|kept| kept == word) {
            keywords.push(word.to_owned());
        }
    }
    Findable {
        kind: Kind::Conversation,
        key: summary.id.to_string(),
        title: subject_or_none(&summary.subject).into_owned(),
        subtitle,
        keywords,
        updated: summary.last_date,
    }
}

/// The first `limit` people the composer offers, in the address book's order. A person an address
/// book's card names has that card's other addresses and its organisation as words that find them.
pub fn contacts(store: &SqliteStore, limit: usize) -> Result<Vec<Findable>, StoreError> {
    let mut cards: HashMap<String, (Vec<String>, Option<String>)> = HashMap::new();
    for book in store.address_books()? {
        for card in book.cards.values() {
            let org = mail_pim::vcard::parse(&card.vcard)
                .into_iter()
                .next()
                .and_then(|read| read.org.into_iter().next())
                .filter(|org| !org.trim().is_empty());
            for address in &card.addresses {
                let others = card.addresses.iter().filter(|a| *a != address).cloned();
                let entry = cards.entry(address.clone()).or_default();
                entry.0.extend(others);
                if entry.1.is_none() {
                    entry.1.clone_from(&org);
                }
            }
        }
    }
    Ok(store
        .contacts()?
        .into_iter()
        .filter(mail_store::Contact::offered)
        .take(limit)
        .map(|contact| {
            let name = contact
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty());
            // A person no card names has no other words.
            let (mut keywords, org) = cards.remove(&contact.address).unwrap_or_default();
            keywords.extend(org);
            keywords.dedup();
            Findable {
                kind: Kind::Contact,
                title: name.unwrap_or(&contact.address).to_owned(),
                subtitle: contact.address.clone(),
                keywords,
                updated: contact
                    .written
                    .last
                    .max(contact.received.last)
                    .unwrap_or(DateTime::UNIX_EPOCH),
                key: contact.address,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests;
