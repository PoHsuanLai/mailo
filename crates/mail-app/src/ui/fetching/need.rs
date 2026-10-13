//! Which accounts need the person, read from the links and nowhere else.
//!
//! The dialog that asks on launch, the account tiles, the list's failure, Settings' account page
//! and the Connections sheet all say "sign in again" about the same accounts. They agree because
//! each asks [`needs_you`] of the same links, instead of keeping a state of its own.

use dioxus::prelude::*;
use mail_core::fetch::Link;
use porter_core::AccountId;
use std::sync::Arc;
use tokio::sync::watch;

/// What an account is waiting for from the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// The sign-in was refused: the password or the browser sign-in is to be given again.
    SignIn,
    /// The desktop no longer lets Mail use the account: Add Account is the one way back.
    Allow,
}

impl Need {
    /// What `link` is waiting for, if it is waiting for the person.
    pub fn of(link: &Link) -> Option<Need> {
        match link {
            Link::NeedsSignIn { .. } => Some(Need::SignIn),
            Link::NeedsAllow { .. } => Some(Need::Allow),
            _ => None,
        }
    }
}

/// The accounts whose mail waits on the person, in the order of `links`, and for what.
pub fn needs_you(links: &[(AccountId, Link)]) -> Vec<(AccountId, Need)> {
    links
        .iter()
        .filter_map(|(id, link)| Need::of(link).map(|need| (id.clone(), need)))
        .collect()
}

/// Whether every account's first pass has told what it found: no link is still at its
/// beginning or on its way from there. Before that, "needs nothing" only means "not known yet".
pub fn settled(links: &[(AccountId, Link)]) -> bool {
    links.iter().all(|(_, link)| !beginning(link))
}

fn beginning(link: &Link) -> bool {
    match link {
        Link::Fresh => true,
        Link::Syncing { after, .. } => beginning(after),
        _ => false,
    }
}

/// Who needs the person, as a root context every window shares, since only the first window
/// has the links: it says here who is waiting, and a Settings window says here whom it is
/// signing in, so that the first window tries that account again when the sign-in is done.
#[derive(Clone)]
pub struct Needing {
    waiting: Arc<watch::Sender<Vec<(AccountId, Need)>>>,
    signing: Arc<watch::Sender<Option<AccountId>>>,
}

impl Default for Needing {
    fn default() -> Self {
        Needing {
            waiting: Arc::new(watch::channel(Vec::new()).0),
            signing: Arc::new(watch::channel(None).0),
        }
    }
}

impl std::fmt::Debug for Needing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Needing")
            .field(&*self.waiting.borrow())
            .finish()
    }
}

impl Needing {
    /// Say who is waiting now. Heard only when it is not what was said before.
    pub(in crate::ui) fn say(&self, waiting: Vec<(AccountId, Need)>) {
        self.waiting.send_if_modified(|held| {
            let changed = *held != waiting;
            if changed {
                *held = waiting;
            }
            changed
        });
    }

    /// Who is waiting, as last said.
    pub(in crate::ui) fn waiting(&self) -> Vec<(AccountId, Need)> {
        self.waiting.borrow().clone()
    }

    /// What a window hears each change of who is waiting through.
    pub(in crate::ui) fn heard(&self) -> watch::Receiver<Vec<(AccountId, Need)>> {
        self.waiting.subscribe()
    }

    /// A sign-in of `account` is starting in another window.
    pub(in crate::ui) fn signing_in(&self, account: AccountId) {
        self.signing.send_replace(Some(account));
    }

    /// What the first window hears each sign-in started elsewhere through.
    pub(in crate::ui) fn signing(&self) -> watch::Receiver<Option<AccountId>> {
        self.signing.subscribe()
    }
}

/// Who is waiting, for a window that has no links of its own (Settings): the shared
/// [`Needing`], followed as it changes. Nobody, without one.
pub(in crate::ui) fn use_needs() -> Signal<Vec<(AccountId, Need)>> {
    let needing = try_consume_context::<Needing>();
    let mut needs = use_signal(|| needing.as_ref().map(Needing::waiting).unwrap_or_default());
    use_future(move || {
        let heard = needing.as_ref().map(Needing::heard);
        async move {
            let Some(mut heard) = heard else { return };
            while heard.changed().await.is_ok() {
                let now = heard.borrow_and_update().clone();
                needs.set(now);
            }
        }
    });
    needs
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use mail_core::fetch::{First, Live, Pause, Step};
    use mail_domain::id::account_id_from_uuid;

    fn id(n: u128) -> AccountId {
        account_id_from_uuid(uuid::Uuid::from_u128(n))
    }

    fn sign_in() -> Link {
        Link::NeedsSignIn {
            why: "x".into(),
            first: First::No,
        }
    }

    fn allow() -> Link {
        Link::NeedsAllow {
            why: "x".into(),
            first: First::No,
        }
    }

    fn current() -> Link {
        Link::Current {
            at: Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap(),
            trouble: vec![],
            live: Live::Polling,
        }
    }

    fn syncing(after: Link) -> Link {
        Link::Syncing {
            first: First::Yes,
            step: Step::Folders,
            count: None,
            after: Box::new(after),
        }
    }

    #[test]
    fn only_a_refused_sign_in_or_a_withdrawn_permission_needs_the_person() {
        let broken = Link::Broken {
            why: "x".into(),
            first: First::No,
        };
        let offline = Link::Waiting {
            until: Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap(),
            why: Pause::Unreachable,
            failures: 1,
            first: First::No,
        };
        let cases = [
            (vec![], vec![]),
            (vec![(id(1), current())], vec![]),
            (vec![(id(1), broken), (id(2), offline)], vec![]),
            (vec![(id(1), sign_in())], vec![(id(1), Need::SignIn)]),
            (
                vec![(id(1), sign_in()), (id(2), current()), (id(3), allow())],
                vec![(id(1), Need::SignIn), (id(3), Need::Allow)],
            ),
        ];
        for (links, want) in cases {
            assert_eq!(needs_you(&links), want, "{links:?}");
        }
    }

    #[test]
    fn the_links_have_settled_once_none_is_at_its_beginning() {
        let cases = [
            (vec![], true),
            (vec![(id(1), current())], true),
            (vec![(id(1), sign_in())], true),
            (vec![(id(1), Link::Fresh)], false),
            (vec![(id(1), syncing(Link::Fresh))], false),
            (vec![(id(1), syncing(current()))], true),
            (vec![(id(1), sign_in()), (id(2), Link::Fresh)], false),
        ];
        for (links, want) in cases {
            assert_eq!(settled(&links), want, "{links:?}");
        }
    }
}
