//! Mail fetching in the window: every account's [`Link`], as the one scheduler moves them.
//!
//! The client fetches (`mail_client::fetch`): it runs the one scheduler (`mail_core::schedule`,
//! the loop `mailo watch` runs too), which holds every account's link and does what each asks (a
//! pass, a cancel, a timer). [`Fetching`] is what the window shares of it, provided once by
//! `App`: the links, where each folder opened on demand stands, and the [`Operation`] a spinner
//! reads while a pass runs, followed into signals. Everything that happens to a link is an
//! [`Event`] sent with [`Fetching::send`]. What reaches a server is the window's to lend
//! ([`seams`]), so a test lends its own. What the screen says is derived from the links and not
//! stored: see [`Fetching::status`].

mod delegate;
mod external;
mod face;
mod folder;
mod host;
mod live;
mod marks;
mod need;
mod pass;
mod scope;
mod status;

use crate::ui::view::Shell;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use ds::motion::detail::operation::Operation;
use mail_client::Client;
use mail_client::fetch::{Fetched, Seams};
use mail_core::SqliteStore;
use mail_core::fetch::{Event, FolderFetch, Link, Trigger};
use mail_core::schedule::Timing;
use mail_core::sync::fresh::Pace;
use mail_core::sync::report::Offer;
use porter_core::AccountId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

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
    /// What each account's pass is doing, for a spinner.
    ops: Signal<BTreeMap<AccountId, Operation>>,
    folders: Signal<BTreeMap<(AccountId, String), FolderFetch>>,
    /// What each account's last pass offered beside its failure.
    offers: Signal<BTreeMap<AccountId, Offer>>,
    client: Signal<Client>,
}

impl Fetching {
    /// Tell an account's link that `event` happened. Never blocks, and is ignored for an account
    /// that has no link or whose link has nothing to do with it.
    pub(in crate::ui) fn send(&self, account: AccountId, event: Event) {
        self.client.peek().tell(account, event);
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
fn announce_added(before: &BTreeMap<AccountId, Link>, now: &BTreeMap<AccountId, Link>) {
    if now.keys().all(|id| before.contains_key(id)) {
        return;
    }
    let store = consume_context::<Arc<SqliteStore>>();
    for row in crate::ui::data::account_rows(&store) {
        if now.contains_key(&row.id) && !before.contains_key(&row.id) {
            crate::ui::motion::tell(
                crate::ui::copy::toasts::account_added(&row.address),
                crate::ui::motion::Follow::Nothing,
            );
        }
    }
}

/// Whether the window's notices speak, and whose new mail they say: the settings as they stand,
/// handed to a pass that runs on a thread of the client's, which cannot read a signal.
#[derive(Debug, Clone, Copy)]
struct Speaking {
    on: bool,
    whom: mail_core::notify::Whom,
}

impl Default for Speaking {
    /// A window with no settings (a test's lone component) has them on, as the setting's default
    /// is.
    fn default() -> Self {
        Speaking {
            on: true,
            whom: mail_core::notify::Whom::Everyone,
        }
    }
}

/// Announce what each finished pass in `ends` stored, as `mailo watch` does
/// (`mail_core::sync::watcher::told`). A message is an arrival only on the pass that first
/// stores it, so a watch and a window running side by side seldom both announce one: only two
/// passes storing the same message at the same moment can.
fn announce(
    store: &SqliteStore,
    ends: &[mail_core::sync::report::PassEnd],
    notifier: &dyn mail_core::notify::Notifier,
    whom: mail_core::notify::Whom,
    at: DateTime<Utc>,
) {
    for end in ends {
        let mail_core::sync::report::PassEnd::Finished(report) = end else {
            continue;
        };
        if let Err(why) = mail_core::notify::announce(
            store,
            report.account.clone(),
            &report.counts.arrived,
            &report.counts.noted,
            notifier,
            whom,
            at,
        ) {
            log::warn!(
                "new mail for {} could not be announced: {why}",
                report.address
            );
        }
    }
}

/// What the client's fetching reaches, as this window lends it: the servers, unless a test
/// provided its own pass, watch, folder fetch or word on who schedules. A pass that stored mail
/// says so through the window's notices, as `speaking` has them.
fn seams(speaking: Arc<std::sync::Mutex<Speaking>>) -> Seams {
    let pass = try_consume_context::<Passer>().unwrap_or_else(Passer::server);
    let listener = try_consume_context::<live::Listener>().unwrap_or_else(live::Listener::server);
    let delegate = try_consume_context::<Delegate>().unwrap_or_else(Delegate::server);
    let folder = try_consume_context::<crate::ui::folder_open::Fetcher>()
        .unwrap_or_else(crate::ui::folder_open::Fetcher::server);
    let notices = try_consume_context::<crate::ui::follow_up::Notices>();
    let offline = try_consume_context::<crate::ui::appearance::WindowDirs>()
        .map(|dirs| mail_core::offline::load(&dirs.config));
    let ran = pass.0;
    let passing: mail_client::fetch::Pass = Arc::new(move |store, started, account, hooks| {
        let done = ran(store.clone(), started, account, hooks);
        let speaks = *speaking
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let (Some(notices), true, Ok(ends)) = (&notices, speaks.on, &done) {
            announce(&store, ends, notices.0.as_ref(), speaks.whom, started);
        }
        done
    });
    Seams {
        pass: passing,
        listen: listener.listen,
        pushers: listener.pushers,
        daemon: listener.daemon,
        schedule: delegate.0,
        folder: folder.0,
        every: Arc::new(move |store| match &offline {
            Some(offline) => mail_core::sync::due::intervals_with(store, offline),
            None => mail_core::sync::due::intervals(store),
        }),
        timing: Timing {
            first: listener.first,
            retry: listener.retry,
            ..Timing::default()
        },
    }
}

/// Provide [`Fetching`] for the window, and have the client fetch. Once, from `App`.
///
/// `revision` moves when the client says the store changed.
pub(in crate::ui) fn use_fetching(revision: Signal<u64>) -> Fetching {
    let client = crate::ui::live::use_client();
    external::use_external_changes(revision);
    // The first window to ask starts it: the program's client fetches on a thread of its own
    // from then on, and one that reads here hands back what this window runs.
    let speaking = use_hook(|| Arc::new(std::sync::Mutex::new(Speaking::default())));
    let settings = try_consume_context::<Signal<crate::settings::MailSettings>>();
    // What the notices say follows Settings, as it is changed.
    {
        let speaking = speaking.clone();
        use_effect(move || {
            if let Some(settings) = settings {
                let notifications = settings.read().notifications.clone();
                *speaking
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Speaking {
                    on: notifications.new_mail == crate::settings::NewMail::On,
                    whom: notifications.notify_for.into(),
                };
            }
        });
    }
    let run = use_hook({
        let client = client.clone();
        let speaking = speaking.clone();
        move || std::rc::Rc::new(std::cell::RefCell::new(client.fetch(seams(speaking))))
    });
    let first = client
        .fetched()
        .map(|fetched| fetched.borrow().clone())
        .unwrap_or_default();
    let links = use_signal(|| first.links.clone());
    let offers = use_signal(|| first.offers.clone());
    let folders = use_signal(|| first.folders.clone());
    let ops = use_signal(BTreeMap::new);
    let held = use_signal(|| client.clone());
    let fetching = use_context_provider(|| Fetching {
        links,
        ops,
        folders,
        offers,
        client: held,
    });
    let _running = use_future(move || {
        let run = run.borrow_mut().take();
        async move {
            if let Some(run) = run {
                run.await;
            }
        }
    });
    let _following = use_future(move || {
        let fetched = held.peek().fetched();
        async move {
            let Some(mut fetched) = fetched else {
                return;
            };
            loop {
                let now = fetched.borrow_and_update().clone();
                follow(fetching, &now);
                if fetched.changed().await.is_err() {
                    return;
                }
            }
        }
    });
    fetching
}

/// Write what the client's fetching now is into the window's signals, each only if it moved, and
/// say which accounts came.
fn follow(fetching: Fetching, now: &Fetched) {
    let Fetching {
        mut links,
        mut ops,
        mut folders,
        mut offers,
        ..
    } = fetching;
    if *links.peek() != now.links {
        announce_added(&links.peek(), &now.links);
        let next: BTreeMap<AccountId, Operation> = now
            .links
            .iter()
            .map(|(id, link)| {
                let before = ops.peek().get(id).copied().unwrap_or_default();
                (id.clone(), host::operation(before, link))
            })
            .collect();
        if *ops.peek() != next {
            ops.set(next);
        }
        links.set(now.links.clone());
    }
    if *offers.peek() != now.offers {
        offers.set(now.offers.clone());
    }
    if *folders.peek() != now.folders {
        folders.set(now.folders.clone());
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod ui_tests;
