//! Mail fetching in the window: every account's [`Link`], and the one loop that moves them.
//!
//! [`Fetching`] is what the window shares, provided once by `App`. It holds a link for each
//! account that has a server, the [`Operation`] a spinner reads while its pass runs, and where
//! each folder opened on demand stands. Everything that happens to them is an [`Event`] sent
//! with [`Fetching::send`]; [`run`] is the only thing that receives them, steps the link with
//! [`crate::fetch::step`] and does what it asks (a pass, a cancel, a timer). What the screen
//! says is derived from the links and not stored: see [`Fetching::status`].
//!
//! This replaces the one `SyncState` that the poll loop, both Sync buttons and folder opening
//! used to share, whose single "running" was every account's and whose single failure ended
//! the poll for all of them.

mod folder;
mod outbox;
mod pass;
mod run;
mod scope;
mod start;

use crate::fetch::{Event, FolderFetch, Link, StatusLine, Trigger, status_line};
use crate::view::Shell;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use ds::motion::detail::operation::Operation;
use mail_domain::AccountId;
use mail_store::SqliteStore;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub(in crate::ui) use folder::opened;
pub(in crate::ui) use pass::Passer;

/// What reaches the runner.
pub(super) enum Note {
    /// Something happened to an account's link.
    Event(AccountId, Event),
    /// The accounts that have a server, with how often each wants a pass, are these now.
    Accounts(Vec<(AccountId, Duration)>),
}

/// The window's mail fetching. Copy, so a handler can take it.
#[derive(Clone, Copy)]
pub(in crate::ui) struct Fetching {
    links: Signal<BTreeMap<AccountId, Link>>,
    /// What each account's pass is doing, for a spinner: read by the surfaces of the next wave.
    #[allow(dead_code)]
    ops: Signal<BTreeMap<AccountId, Operation>>,
    folders: Signal<BTreeMap<(AccountId, String), FolderFetch>>,
    tx: Signal<UnboundedSender<Note>>,
}

impl Fetching {
    /// Tell an account's link that `event` happened. Never blocks, and is ignored for an account
    /// that has no link or whose link has nothing to do with it.
    pub(in crate::ui) fn send(&self, account: AccountId, event: Event) {
        let _ = self.tx.peek().send(Note::Event(account, event));
    }

    /// Ask each of `accounts` for a pass. A link already running one ignores it.
    pub(in crate::ui) fn sync(&self, accounts: &[AccountId], trigger: Trigger) {
        for account in accounts {
            self.send(*account, Event::Start(trigger));
        }
    }

    /// Ask every account for a pass.
    #[allow(dead_code)] // The banners and the sidebar of the next wave are its readers.
    pub(in crate::ui) fn sync_all(&self, trigger: Trigger) {
        let all: Vec<AccountId> = self.links.peek().keys().copied().collect();
        self.sync(&all, trigger);
    }

    /// The credential of `account` was replaced.
    #[allow(dead_code)] // The banners and the sidebar of the next wave are its readers.
    pub(in crate::ui) fn signed_in(&self, account: AccountId) {
        self.send(account, Event::SignedIn);
    }

    /// Every link. Read in a component, it redraws when any of them moves.
    #[allow(dead_code)] // The banners and the sidebar of the next wave are its readers.
    pub(in crate::ui) fn links(&self) -> BTreeMap<AccountId, Link> {
        self.links.read().clone()
    }

    /// One account's link, or `None` for one with nothing to fetch.
    pub(in crate::ui) fn link(&self, account: AccountId) -> Option<Link> {
        self.links.read().get(&account).cloned()
    }

    /// What `account`'s pass is doing, for a spinner: running from the moment it starts.
    #[allow(dead_code)] // The banners and the sidebar of the next wave are its readers.
    pub(in crate::ui) fn op(&self, account: AccountId) -> Operation {
        self.ops.read().get(&account).copied().unwrap_or_default()
    }

    /// The accounts `shell` is showing that have something to fetch.
    pub(in crate::ui) fn in_view(&self, shell: &Shell) -> Vec<AccountId> {
        let with_links: BTreeSet<AccountId> = self.links.read().keys().copied().collect();
        scope::in_view(&with_links, shell)
    }

    /// Whether any account `shell` is showing is fetching, which is when Sync waits.
    pub(in crate::ui) fn busy_in(&self, shell: &Shell) -> bool {
        let links = self.links.read();
        self.in_view(shell)
            .iter()
            .any(|id| links.get(id).is_some_and(Link::is_busy))
    }

    /// What to say about the accounts `shell` is showing, at `now`.
    pub(in crate::ui) fn status(&self, shell: &Shell, now: DateTime<Utc>) -> StatusLine {
        let links = self.links.read();
        let shown: Vec<&Link> = self
            .in_view(shell)
            .iter()
            .filter_map(|id| links.get(id))
            .collect();
        status_line(&shown, now, &chrono::Local)
    }
}

/// Sync what `shell` is showing, from the toolbar button and the command menu alike.
///
/// Does nothing while any of it is already fetching: a second press is the same request. Every
/// folder fetched on opening is fetched again the next time it is opened.
pub(in crate::ui) fn sync_now(shell: &Shell) {
    let Some(fetching) = try_consume_context::<Fetching>() else {
        return;
    };
    if fetching.busy_in(shell) {
        return;
    }
    fetching.forget_folders();
    fetching.sync(&fetching.in_view(shell), Trigger::Manual);
}

/// Provide [`Fetching`] for the window and start the loop that serves it. Once, from `App`.
///
/// `revision` moves when a pass has stored something, and when an account comes or goes.
pub(in crate::ui) fn use_fetching(revision: Signal<u64>) -> Fetching {
    let store = consume_context::<Arc<SqliteStore>>();
    let accounts = use_memo(move || {
        let _ = revision();
        let store = consume_context::<Arc<SqliteStore>>();
        crate::sync::due::intervals(&store)
    });
    let links = use_signal({
        let store = store.clone();
        move || start::initial(&store, &accounts.peek(), Utc::now())
    });
    let ops = use_signal(BTreeMap::new);
    let folders = use_signal(BTreeMap::new);
    let channel = use_hook(|| {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Note>();
        (tx, Rc::new(RefCell::new(Some(rx))))
    });
    let tx = use_signal(|| channel.0.clone());
    let fetching = use_context_provider(|| Fetching {
        links,
        ops,
        folders,
        tx,
    });
    // The accounts changing is the runner's to hear, since it owns what to do about it.
    use_effect(move || {
        let _ = tx.peek().send(Note::Accounts(accounts.read().clone()));
    });
    let inbox: Rc<RefCell<Option<UnboundedReceiver<Note>>>> = channel.1.clone();
    let _runner = use_future(move || {
        let received = inbox.borrow_mut().take();
        let runner = run::Runner {
            links,
            ops,
            folders,
            revision,
            tx: tx.peek().clone(),
            store: store.clone(),
            passer: try_consume_context::<Passer>().unwrap_or_else(Passer::server),
            every: accounts.peek().iter().copied().collect(),
        };
        async move {
            if let Some(received) = received {
                runner.run(received).await;
            }
        }
    });
    fetching
}

#[cfg(test)]
mod tests;
