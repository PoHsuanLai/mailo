//! What can be asked: the store's facts, as values.
//!
//! Each is a fact about the mail and nothing more. Which of them a caller puts together, and what
//! it calls the result, is the caller's: a list of places is the accounts' folders and labels and
//! the saved views, and a number beside one is a [`Counts`] of the filter it lists.

use crate::{ClientError, Query, Topics};
use chrono::{DateTime, Utc};
use mail_domain::{AccountPlan, Filter, Folder, Incoming, Label, ServerLabels, View};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

/// Every account, oldest first, with its mailboxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Accounts;

/// One account and its mailboxes, as the store holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub address: String,
    /// How the account reaches its server, or `None` when the stored plan could not be read.
    pub plan: Option<AccountPlan>,
    /// Mail kept on this computer and never synced.
    pub local: bool,
    /// What the person calls the account, in place of its address.
    pub description: Option<String>,
    /// Whether the person paused its syncing.
    pub paused: bool,
    pub mailboxes: Mailboxes,
}

/// Whether an account has mailboxes of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mailboxes {
    /// POP3, or local mail: one mailbox, and nothing to name. Its labels are its own, and are
    /// all the organisation it has.
    One { labels: Vec<Label> },
    Many {
        /// The server's last listing. Empty until the first sync has asked.
        folders: Vec<Folder>,
        labels: Vec<Label>,
        /// Whether the server presents its labels as mailboxes, as Gmail does. Before the first
        /// connection nothing is known, and nothing is assumed.
        server_labels: ServerLabels,
    },
}

impl Account {
    /// The account's labels, whatever its mailboxes.
    pub fn labels(&self) -> &[Label] {
        match &self.mailboxes {
            Mailboxes::Many { labels, .. } | Mailboxes::One { labels } => labels,
        }
    }
}

impl Query for Accounts {
    type Output = Vec<Account>;

    const READS: Topics = Topics::ACCOUNTS
        .union(Topics::FOLDERS)
        .union(Topics::LABELS);

    /// A plan that does not parse is read as a server's, so the account still has its folders;
    /// an account whose folders or labels cannot be read has none, and the others still do.
    fn run(&self, store: &SqliteStore, _now: DateTime<Utc>) -> Result<Vec<Account>, ClientError> {
        Ok(store
            .list_accounts()?
            .into_iter()
            .map(|account| {
                // A plan that does not parse is no plan here, and the account keeps its folders.
                let plan = account
                    .plan
                    .inspect_err(|why| log::warn!("an account's plan could not be read: {why}"))
                    .ok();
                let incoming = plan.as_ref().map(|plan| &plan.incoming);
                let caps = incoming.map(Incoming::caps);
                let local = caps.is_some_and(|caps| !caps.has_server);
                let mailboxes = match caps {
                    Some(caps) if !caps.lists_folders => Mailboxes::One {
                        labels: or_none(store.labels(account.id.clone()), "labels"),
                    },
                    _ => Mailboxes::Many {
                        folders: or_none(store.folders(account.id.clone()), "folders"),
                        labels: or_none(store.labels(account.id.clone()), "labels"),
                        server_labels: account
                            .caps
                            .and_then(Result::ok)
                            .map_or(ServerLabels::LocalOnly, |caps| caps.labels),
                    },
                };
                let prefs = store.account_prefs(account.id.clone()).unwrap_or_default();
                Account {
                    description: prefs.description,
                    paused: prefs.syncing == mail_store::Syncing::Paused,
                    id: account.id,
                    address: account.address,
                    plan,
                    local,
                    mailboxes,
                }
            })
            .collect())
    }
}

fn or_none<T>(read: Result<Vec<T>, mail_store::StoreError>, what: &str) -> Vec<T> {
    read.unwrap_or_else(|why| {
        log::warn!("an account's {what} could not be read: {why}");
        Vec::new()
    })
}

/// The saved views, in the order they were first kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Views;

impl Query for Views {
    type Output = Vec<View>;

    const READS: Topics = Topics::VIEWS;

    fn run(&self, store: &SqliteStore, _now: DateTime<Utc>) -> Result<Vec<View>, ClientError> {
        Ok(store.views()?)
    }
}

/// How many conversations each filter matches, all counted in one moment of the database.
///
/// Answered filter by filter, beside the filter, so an answer is never read against another
/// list of filters by position.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Counts(pub Vec<Filter>);

/// Each filter asked, and how many it matches. `None` where the count could not be made: no
/// number rather than a wrong one, and said in the log.
pub type Counted = Vec<(Filter, Option<u64>)>;

impl Counts {
    /// The count `counted` has for `filter`, if it counted it.
    pub fn of(counted: &Counted, filter: &Filter) -> Option<u64> {
        counted
            .iter()
            .find(|(asked, _)| asked == filter)
            .and_then(|(_, n)| *n)
    }
}

impl Query for Counts {
    type Output = Counted;

    /// The clock too: a snoozed conversation comes back when its time comes.
    const READS: Topics = Topics::SUMMARIES.union(Topics::LABELS).union(Topics::CLOCK);

    fn run(&self, store: &SqliteStore, now: DateTime<Utc>) -> Result<Counted, ClientError> {
        Ok(self
            .0
            .iter()
            .map(|filter| {
                let n = store
                    .count(filter, now)
                    .inspect_err(|why| log::warn!("a count failed: {why}"))
                    // Said above; no number rather than a wrong one.
                    .ok();
                (filter.clone(), n)
            })
            .collect())
    }
}

/// What Favourites shows of a Space's pins: each pin's unread count, and the pinned conversations
/// themselves, most recently pinned first, all in one reading.
///
/// A pin is a filter the caller built (a person, a search); `scope` narrows all of them to a
/// Space's accounts, and `shown` is the most pinned conversations wanted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pins {
    pub scope: Option<Filter>,
    pub pins: Vec<Filter>,
    pub shown: u32,
}

/// What [`Pins`] read: the unread count of each pin, in the order asked, and the pinned
/// conversations. A count that could not be made is nothing, and said in the log.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pinned {
    pub unread: Vec<u64>,
    pub threads: Vec<mail_domain::ThreadSummary>,
}

impl Query for Pins {
    type Output = Pinned;

    const READS: Topics = Topics::SUMMARIES.union(Topics::LABELS).union(Topics::CLOCK);

    fn run(&self, store: &SqliteStore, now: DateTime<Utc>) -> Result<Pinned, ClientError> {
        use mail_domain::{PageReq, Property, Query as Asked, ReadState, Sort, SortDir};
        let unread = self
            .pins
            .iter()
            .map(|pin| {
                let mut parts = vec![Filter::Read(ReadState::Unread), pin.clone()];
                parts.extend(self.scope.clone());
                store
                    .count(&Filter::And(parts), now)
                    .inspect_err(|why| log::warn!("a pin could not be counted: {why}"))
                    .unwrap_or(0)
            })
            .collect();
        let pinned = Filter::Pinned;
        let filter = match self.scope.clone() {
            Some(scope) => Filter::And(vec![scope, pinned]),
            None => pinned,
        };
        let threads = store
            .threads(
                &Asked {
                    filter,
                    sort: Sort {
                        property: Property::Pin,
                        dir: SortDir::Desc,
                    },
                    page: PageReq {
                        after: None,
                        limit: self.shown,
                    },
                },
                now,
            )?
            .items;
        Ok(Pinned { unread, threads })
    }
}

/// The drafts of some accounts, every kind: what Drafts lists, and from which Scheduled and
/// whatever shows a count of it are told. A draft written, sent, held or taken back changes it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DraftsOf(pub Vec<AccountId>);

impl Query for DraftsOf {
    fn bucket(&self) -> u64 {
        crate::live::bucket_of(&self.0)
    }

    type Output = Vec<mail_domain::Draft>;

    const READS: Topics = Topics::DRAFTS.union(Topics::OUTBOX);

    fn run(&self, store: &SqliteStore, _now: DateTime<Utc>) -> Result<Self::Output, ClientError> {
        let mut out = Vec::new();
        for account in &self.0 {
            out.extend(store.drafts(account.clone())?);
        }
        Ok(out)
    }
}

mod conversation;
mod conversations;
mod paces;
mod summaries;
pub use conversation::{Conversation, Messages};
pub use conversations::{Conversations, Found, How, Search};
pub use paces::Paces;
pub use summaries::Summaries;

// A search as typed, read into a filter.
pub use mail_core::query::{Known, Token, is_either, named, parse_with, tokens, unnamed};

/// The people marked as VIPs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VipList;

impl Query for VipList {
    type Output = mail_core::vip::Vips;

    const READS: Topics = Topics::CONTACTS.union(Topics::SUMMARIES);

    fn run(
        &self,
        store: &SqliteStore,
        _now: DateTime<Utc>,
    ) -> Result<mail_core::vip::Vips, ClientError> {
        Ok(mail_core::vip::read(store))
    }
}
