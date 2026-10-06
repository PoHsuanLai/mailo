//! Mail fetching in the window: every account's [`Link`], and the one loop that moves them.
//!
//! [`Fetching`] is what the window shares, provided once by `App`. It holds a link for each
//! account that has a server, the [`Operation`] a spinner reads while its pass runs, and where
//! each folder opened on demand stands. Everything that happens to them is an [`Event`] sent
//! with [`Fetching::send`]; [`run`] is the only thing that receives them, steps the link with
//! [`mail_core::fetch::step`] and does what it asks (a pass, a cancel, a timer). What the screen
//! says is derived from the links and not stored: see [`Fetching::status`].

mod delegate;
mod external;
mod face;
mod folder;
mod line;
mod live;
mod marks;
mod outbox;
mod pass;
mod run;
mod scope;
mod start;
mod status;

use crate::ui::view::Shell;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use ds::motion::detail::operation::Operation;
use mail_core::fetch::{Event, FolderFetch, Link, Trigger};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub(in crate::ui) use delegate::Delegate;
pub(in crate::ui) use face::{CANNOT_LOAD, FIRST_SYNC, HasRows, ListFace, list_face};
pub(in crate::ui) use folder::opened;
pub(in crate::ui) use line::{AccountLine, Remedy, Standing, account_line};
pub(in crate::ui) use marks::{Mark, account_mark_local, folder_mark, sync_availability};
pub(in crate::ui) use pass::Passer;
pub(in crate::ui) use status::{StatusLine, Tone, status_line, thousandths};

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
            self.send(account.clone(), Event::Start(trigger));
        }
    }

    /// Ask every account for a pass.
    pub(in crate::ui) fn sync_all(&self, trigger: Trigger) {
        let all: Vec<AccountId> = self.links.peek().keys().cloned().collect();
        self.sync(&all, trigger);
    }

    /// The credential of `account` was replaced.
    pub(in crate::ui) fn signed_in(&self, account: AccountId) {
        self.send(account, Event::SignedIn);
    }

    /// Every link. Read in a component, it redraws when any of them moves.
    /// One account's link, or `None` for one with nothing to fetch.
    pub(in crate::ui) fn link(&self, account: AccountId) -> Option<Link> {
        self.links.read().get(&account).cloned()
    }

    /// What `account`'s pass is doing, for a spinner: running from the moment it starts.
    /// The accounts `shell` is showing that have something to fetch.
    pub(in crate::ui) fn in_view(&self, shell: &Shell) -> Vec<AccountId> {
        let with_links: BTreeSet<AccountId> = self.links.read().keys().cloned().collect();
        scope::in_view(&with_links, shell)
    }

    /// Whether any account `shell` is showing is fetching, which is when Sync waits.
    pub(in crate::ui) fn busy_in(&self, shell: &Shell) -> bool {
        let links = self.links.read();
        self.in_view(shell)
            .iter()
            .any(|id| links.get(id).is_some_and(Link::is_busy))
    }

    /// Every account's link, in no order a person would know. Read in a component, it redraws
    /// when any of them moves.
    pub(in crate::ui) fn all_links(&self) -> Vec<(AccountId, Link)> {
        self.links
            .read()
            .iter()
            .map(|(id, link)| (id.clone(), link.clone()))
            .collect()
    }

    /// The links of the accounts in view, with whom they belong to.
    pub(in crate::ui) fn links_in_view(&self, shell: &Shell) -> Vec<(AccountId, Link)> {
        let links = self.links.read();
        self.in_view(shell)
            .into_iter()
            .filter_map(|id| links.get(&id).map(|link| (id, link.clone())))
            .collect()
    }

    /// The operation to draw for the accounts in view: running while any of them is.
    pub(in crate::ui) fn op_in_view(&self, shell: &Shell) -> Operation {
        let ops = self.ops.read();
        self.in_view(shell)
            .iter()
            .filter_map(|id| ops.get(id).copied())
            .find(|op| matches!(op, Operation::Running(_)))
            .unwrap_or_default()
    }

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
        mail_core::sync::due::intervals(&store)
    });
    let links = use_signal({
        let store = store.clone();
        move || start::initial(&store, &accounts.peek(), Utc::now())
    });
    let delegate = try_consume_context::<Delegate>().unwrap_or_else(Delegate::server);
    let doors = try_consume_context::<external::Doors>().unwrap_or_else(external::Doors::server);
    external::use_external_changes(revision, store.clone(), doors);
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
            delegate: delegate.clone(),
            every: accounts.peek().iter().cloned().collect(),
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
#[cfg(test)]
mod ui_tests;
