//! Mail fetching in the window: every account's [`Link`], as the one scheduler moves them.
//!
//! [`Fetching`] is what the window shares, provided once by `App`. It holds a link for each
//! account that has a server, the [`Operation`] a spinner reads while its pass runs, and where
//! each folder opened on demand stands. Everything that happens to them is an [`Event`] sent
//! with [`Fetching::send`]; the scheduler (`mail_core::schedule`, the loop `mailo watch` runs
//! too) is the only thing that receives them, steps the link and does what it asks (a pass, a
//! cancel, a timer), and reports each change to [`host`], which writes it into the signals the
//! window reads. What the screen says is derived from the links and not stored: see
//! [`Fetching::status`].

mod delegate;
mod external;
mod face;
mod folder;
mod host;
mod live;
mod marks;
mod need;
mod outbox;
mod pass;
mod scope;
mod status;

use crate::ui::view::Shell;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use ds::motion::detail::operation::Operation;
use mail_core::SqliteStore;
use mail_core::fetch::{Event, FolderFetch, Link, Trigger};
use mail_core::schedule::{self, End, Note, Scheduler};
use mail_core::sync::fresh::Pace;
use mail_core::sync::report::Offer;
use porter_core::AccountId;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub(in crate::ui) use delegate::Delegate;
pub(in crate::ui) use face::{CANNOT_LOAD, HasRows, ListFace, list_face};
pub(in crate::ui) use folder::opened;
pub(in crate::ui) use marks::sync_availability;
pub(in crate::ui) use need::{Need, Needing, needs_you, use_needs};
pub(in crate::ui) use pass::Passer;
pub(in crate::ui) use status::{StatusLine, Tone, status_line, thousandths};

/// The window's mail fetching. Copy, so a handler can take it.
#[derive(Clone, Copy)]
pub(in crate::ui) struct Fetching {
    links: Signal<BTreeMap<AccountId, Link>>,
    /// What each account's pass is doing, for a spinner: read by the surfaces of the next wave.
    ops: Signal<BTreeMap<AccountId, Operation>>,
    folders: Signal<BTreeMap<(AccountId, String), FolderFetch>>,
    /// What each account's last pass offered beside its failure.
    offers: Signal<BTreeMap<AccountId, Offer>>,
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

    /// Ask every account for a pass, as the tests start one.
    #[cfg(test)]
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

    /// What `account`'s last pass offered beside its failure, if anything: the sign-in
    /// question's Use Microsoft Graph (`reauth`).
    pub(in crate::ui) fn offer(&self, account: &AccountId) -> Option<Offer> {
        self.offers.read().get(account).copied()
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

    /// The accounts waiting for the person to sign in again or to allow Mail again. Read in a
    /// component, it redraws when any link moves.
    pub(in crate::ui) fn needs(&self) -> Vec<(AccountId, Need)> {
        needs_you(&self.all_links())
    }

    /// Those of [`Fetching::needs`] that `shell` is showing.
    pub(in crate::ui) fn needs_in_view(&self, shell: &Shell) -> Vec<(AccountId, Need)> {
        needs_you(&self.links_in_view(shell))
    }

    /// Whether every account has had its first pass say how it stands, so that "needs nothing"
    /// is known and not only not yet asked.
    pub(in crate::ui) fn settled(&self) -> bool {
        need::settled(&self.all_links())
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

    /// What `account`'s freshness is judged by besides its link: when the store last recorded a
    /// finished pass of it, by this window or by a watch beside it, and what its server allows.
    /// Read from the store each time, as cheap as two lookups by key: a watch's passes move
    /// nothing in this window's links.
    pub(in crate::ui) fn pace(&self, account: &AccountId) -> Pace {
        match try_consume_context::<Arc<SqliteStore>>() {
            Some(store) => Pace::of(&store, account.clone()),
            None => Pace::of_watch(&mail_domain::WatchMode::Poll {
                every: mail_core::sync::due::IDLE,
            }),
        }
    }

    /// `account`'s own status line, what its tile's mark says: `None` for an account with
    /// nothing to fetch (local folders).
    pub(in crate::ui) fn account_line(
        &self,
        account: &AccountId,
        now: DateTime<Utc>,
    ) -> Option<StatusLine> {
        let link = self.links.read().get(account).cloned()?;
        Some(status_line(
            &[(&link, self.pace(account))],
            now,
            &chrono::Local,
            crate::ui::clock::hours(),
        ))
    }

    pub(in crate::ui) fn status(&self, shell: &Shell, now: DateTime<Utc>) -> StatusLine {
        let links = self.links.read();
        let shown: Vec<(&Link, Pace)> = self
            .in_view(shell)
            .iter()
            .filter_map(|id| links.get(id).map(|link| (link, self.pace(id))))
            .collect();
        status_line(&shown, now, &chrono::Local, crate::ui::clock::hours())
    }
}

/// What Sync says when what it would fetch is already being fetched.
pub(in crate::ui) const ALREADY_SYNCING: &str = "Already checking for mail.";

/// Sync what `shell` is showing, from the toolbar button and the command menu alike.
///
/// While any of it is already fetching a second press is the same request, and starts nothing;
/// it says so, because a press that does nothing visible reads as a button that is broken. Every
/// folder fetched on opening is fetched again the next time it is opened.
pub(in crate::ui) fn sync_now(shell: &Shell) {
    let Some(fetching) = try_consume_context::<Fetching>() else {
        return;
    };
    if fetching.busy_in(shell) {
        crate::ui::motion::tell(
            ALREADY_SYNCING.to_owned(),
            crate::ui::motion::Follow::Nothing,
        );
        return;
    }
    fetching.forget_folders();
    fetching.sync(&fetching.in_view(shell), Trigger::Manual);
}

/// Sync `account` alone, from its tile's menu, as [`sync_now`] does for what is shown: a second
/// ask while it fetches starts nothing, and says so.
pub(in crate::ui) fn sync_account(account: AccountId) {
    let Some(fetching) = try_consume_context::<Fetching>() else {
        return;
    };
    if fetching
        .link(account.clone())
        .is_some_and(|link| link.is_busy())
    {
        crate::ui::motion::tell(
            ALREADY_SYNCING.to_owned(),
            crate::ui::motion::Follow::Nothing,
        );
        return;
    }
    fetching.forget_folders();
    fetching.sync(&[account], Trigger::Manual);
}

/// How often the status line is read again, so "5 minutes ago" moves on. A minute is the finest
/// the words go.
const STATUS_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

/// What the accounts `shell` shows are doing, as [`Fetching::status`] says it, read again every
/// [`STATUS_EVERY`]. `None` in a window with no fetching. A hook: the component that calls it is
/// the one a tick of the clock draws again, so it is called by the small parts that say the
/// line (the sidebar's sync mark, the list's bar and its ⋯), never by the list.
pub(in crate::ui) fn use_status(shell: Signal<Shell>) -> Option<StatusLine> {
    let mut tick = use_signal(|| 0u32);
    use_future(move || async move {
        loop {
            ds::base::time::clock::sleep(STATUS_EVERY).await;
            tick += 1;
        }
    });
    let _ = tick();
    let fetching = try_consume_context::<Fetching>()?;
    Some(fetching.status(&shell.read(), crate::ui::clock::now()))
}

/// Say which accounts came since the window last looked, once it has looked: an account added in
/// its own window (or by `mailo account add`) is otherwise seen only as the sidebar changing.
fn announce_added(
    known: &RefCell<Option<BTreeSet<AccountId>>>,
    accounts: &[(AccountId, std::time::Duration)],
) {
    let now: BTreeSet<AccountId> = accounts.iter().map(|(id, _)| id.clone()).collect();
    let before = known.replace(Some(now.clone()));
    let Some(before) = before else {
        return;
    };
    let store = consume_context::<Arc<SqliteStore>>();
    for row in crate::ui::data::account_rows(&store) {
        if now.contains(&row.id) && !before.contains(&row.id) {
            crate::ui::motion::tell(
                crate::ui::copy::toasts::account_added(&row.address),
                crate::ui::motion::Follow::Nothing,
            );
        }
    }
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
        move || schedule::initial(&store, &accounts.peek(), Utc::now())
    });
    let delegate = try_consume_context::<Delegate>().unwrap_or_else(Delegate::server);
    external::use_external_changes(revision, store.clone());
    let ops = use_signal(BTreeMap::new);
    let folders = use_signal(BTreeMap::new);
    let offers = use_signal(BTreeMap::new);
    let channel = use_hook(|| {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Note>();
        (tx, Rc::new(RefCell::new(Some(rx))))
    });
    let tx = use_signal(|| channel.0.clone());
    let fetching = use_context_provider(|| Fetching {
        links,
        ops,
        folders,
        offers,
        tx,
    });
    // The accounts changing is the runner's to hear, since it owns what to do about it.
    let known = use_hook(|| Rc::new(RefCell::new(None::<BTreeSet<AccountId>>)));
    use_effect(move || {
        let now: Vec<(AccountId, std::time::Duration)> = accounts.read().clone();
        announce_added(&known, &now);
        let _ = tx.peek().send(Note::Accounts(now));
    });
    let inbox: Rc<RefCell<Option<UnboundedReceiver<Note>>>> = channel.1.clone();
    let _runner = use_future(move || {
        let received = inbox.borrow_mut().take();
        let host = host::WindowHost {
            store: store.clone(),
            passer: try_consume_context::<Passer>().unwrap_or_else(Passer::server),
            listener: try_consume_context::<live::Listener>()
                .unwrap_or_else(live::Listener::server),
            delegate: delegate.clone(),
            clock: try_consume_context::<crate::ui::clock::WallClock>(),
            links,
            ops,
            folders,
            offers,
            revision,
        };
        let every = accounts.peek().iter().cloned().collect();
        let initial = links.peek().clone();
        let (store, tx) = (store.clone(), tx.peek().clone());
        async move {
            if let Some(received) = received {
                Scheduler::new(&host, store, initial, every, tx)
                    .run(received, End::Never)
                    .await;
            }
        }
    });
    fetching
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod ui_tests;
