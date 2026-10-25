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
    pub mailboxes: Mailboxes,
}

/// Whether an account has mailboxes of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mailboxes {
    /// POP3, or local mail: one mailbox, and nothing to name. Local mail's labels are its own.
    One,
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
    /// The account's labels: none for one mailbox.
    pub fn labels(&self) -> &[Label] {
        match &self.mailboxes {
            Mailboxes::Many { labels, .. } => labels,
            Mailboxes::One => &[],
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
                let plan = account.plan.ok();
                let incoming = plan.as_ref().map(|plan| &plan.incoming);
                let local = matches!(incoming, Some(Incoming::Local));
                let mailboxes = match incoming {
                    Some(Incoming::Pop3 { .. } | Incoming::Local) => Mailboxes::One,
                    _ => Mailboxes::Many {
                        folders: or_none(store.folders(account.id.clone()), "folders"),
                        labels: or_none(store.labels(account.id.clone()), "labels"),
                        server_labels: account
                            .caps
                            .and_then(Result::ok)
                            .map_or(ServerLabels::LocalOnly, |caps| caps.labels),
                    },
                };
                Account {
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
