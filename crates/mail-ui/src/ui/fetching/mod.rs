//! Mail fetching in the window: every account's [`Link`], as the one scheduler moves them.
//!
//! The client fetches (`mail_client::fetch`): it runs the one scheduler (`mail_client::schedule`,
//! the loop `mailo watch` runs too), which holds every account's link and does what each asks (a
//! pass, a cancel, a timer). [`Fetching`] is what the window shares of it, provided once by
//! `App`: the links, where each folder opened on demand stands, and the [`Operation`] a spinner
//! reads while a pass runs, followed into signals. Everything that happens to a link is an
//! [`Event`] sent with [`Fetching::send`]. What reaches a server is the window's to lend
//! ([`seams`]), so a test lends its own. What the screen says is derived from the links and not
//! stored: see [`Fetching::status`].

mod delegate;
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
use mail_client::fetch::{Announce, Fetched, Seams, Speaking};
use mail_client::fetch::{Event, FolderFetch, Link, Trigger};
use mail_client::schedule::Timing;
use mail_client::sync::fresh::Pace;
use mail_client::sync::report::Offer;
use mail_client::{Client, Snapshot};
use porter_core::AccountId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(in crate::ui) use delegate::Delegate;
pub(in crate::ui) use face::{HasRows, ListFace, list_face};
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
    /// How fresh each account is judged to be besides its link, as the client keeps it read.
    paces: Memo<Snapshot<Vec<(AccountId, Pace)>>>,
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
    /// As the client has it read: a watch's passes move nothing in this window's links, and the
    /// store is not read here. An account not read yet is taken to poll at the default interval.
    pub(in crate::ui) fn pace(&self, account: &AccountId) -> Pace {
        self.paces
            .read()
            .value()
            .and_then(|paces| {
                paces
                    .iter()
                    .find(|(id, _)| id == account)
                    .map(|(_, pace)| *pace)
            })
            .unwrap_or_else(|| {
                Pace::of_watch(&mail_domain::WatchMode::Poll {
                    every: mail_client::sync::due::IDLE,
                })
            })
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
/// its own window (or by `mailo account add`) is otherwise seen only as the sidebar changing. As
/// the client keeps the accounts read; the first reading is only what was already there.
fn announce_added() {
    let accounts = crate::ui::live::use_live(|| mail_client::query::Accounts);
    let known = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(None::<BTreeSet<AccountId>>)));
    use_effect(move || {
        let Snapshot::Ready { value, .. } = &*accounts.read() else {
            return;
        };
        let rows = crate::ui::data::rows_of(value);
        let mut known = known.borrow_mut();
        if let Some(before) = known.as_ref() {
            for row in rows.iter().filter(|row| !before.contains(&row.id)) {
                crate::ui::motion::tell(
                    crate::ui::copy::toasts::account_added(&row.address),
                    crate::ui::motion::Follow::Nothing,
                );
            }
        }
        *known = Some(rows.iter().map(|row| row.id.clone()).collect());
    });
}

/// What the client's fetching reaches, as this window lends it: the servers, unless a test
/// provided its own pass, watch, folder fetch or word on who schedules. A pass that stored mail
/// says so through the window's notices, as `speaking` has them.
fn seams(client: &Client, speaking: Arc<std::sync::Mutex<Speaking>>) -> Seams {
    let pass = try_consume_context::<Passer>().unwrap_or_else(|| Passer::server(client));
    let listener =
        try_consume_context::<live::Listener>().unwrap_or_else(|| live::Listener::server(client));
    let delegate = try_consume_context::<Delegate>().unwrap_or_else(Delegate::server);
    let folder = try_consume_context::<crate::ui::folder_open::Fetcher>()
        .unwrap_or_else(|| crate::ui::folder_open::Fetcher::server(client));
    Seams {
        pass: pass.0,
        listen: listener.listen,
        pushers: listener.pushers,
        daemon: listener.daemon,
        schedule: delegate.0,
        folder: folder.0,
        config: try_consume_context::<crate::ui::appearance::WindowDirs>().map(|dirs| dirs.config),
        announce: try_consume_context::<crate::ui::follow_up::Notices>().map(|notices| Announce {
            notifier: notices.0,
            speaking,
        }),
        timing: Timing {
            first: listener.first,
            retry: listener.retry,
            ..Timing::default()
        },
    }
}

/// Provide [`Fetching`] for the window, and have the client fetch. Once, from `App`.
pub(in crate::ui) fn use_fetching() -> Fetching {
    let client = crate::ui::live::use_client();
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
        move || {
            std::rc::Rc::new(std::cell::RefCell::new(
                client.fetch(seams(&client, speaking)),
            ))
        }
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
    let paces = crate::ui::live::use_live(move || {
        mail_client::query::Paces(links.read().keys().cloned().collect())
    });
    let fetching = use_context_provider(|| Fetching {
        links,
        ops,
        folders,
        offers,
        client: held,
        paces,
    });
    announce_added();
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

/// Write what the client's fetching now is into the window's signals, each only if it moved.
/// Which accounts came is [`announce_added`]'s, from the accounts the client has read.
fn follow(fetching: Fetching, now: &Fetched) {
    let Fetching {
        mut links,
        mut ops,
        mut folders,
        mut offers,
        ..
    } = fetching;
    if *links.peek() != now.links {
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
