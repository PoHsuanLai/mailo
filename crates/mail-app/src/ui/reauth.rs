//! Asking the person to sign in again.
//!
//! One account that needs the person is told once, in a dialog when the window opens, instead of
//! by a mark on every surface that happens to list it. The dialog asks about every account the
//! links say is waiting ([`crate::ui::fetching::needs_you`]), once they have all said how they
//! stand. Sign In goes to the browser or the Add Account window. Later puts the dialog off for
//! the rest of the session: it comes back for an account only when that account recovers and
//! needs the person again, or when the sidebar foot's sync mark is pressed. A lone account is
//! asked in quire's alert; several are asked in a sheet with a row, and a Sign In, for each.
//!
//! The one other thing only the person can answer is asked the same way, after any sign-in: a
//! work or school Microsoft account whose organisation switched IMAP off is offered Microsoft
//! Graph, which signs it in again for Graph in the browser. Everything else a connection can
//! suffer it retries by itself, and the sidebar foot's mark says while it does.

use crate::ui::common::in_card;
use crate::ui::copy;
use crate::ui::data::account_rows;
use crate::ui::fetching::{Fetching, Need, Needing};
use crate::ui::press::{SheetClose, on_primary};
use dioxus::prelude::*;
use ds::components::content::label::{Label, LabelStyle};
use ds::components::lists::row::row::Outline;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::components::overlays::sheet_attach::Attach;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_core::SqliteStore;
use mail_core::fetch::Link;
use mail_core::sync::report::Offer;
use porter_core::AccountId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The button that signs in.
pub(in crate::ui) const SIGN_IN: &str = "Sign In";

/// The button that puts the question off.
const LATER: &str = "Later";

/// What the dialog says under its title.
const MESSAGE: &str = "New mail stops arriving until you do.";

/// The title when several accounts are asked about.
const MANY: &str = "Some accounts need you to sign in again";

/// The title when one account is.
fn one(name: &str) -> String {
    format!("Sign in to {name} again?")
}

/// The accounts to ask about: those that need the person and were not put off.
fn to_ask(needs: &[(AccountId, Need)], put_off: &BTreeSet<AccountId>) -> Vec<(AccountId, Need)> {
    needs
        .iter()
        .filter(|(id, _)| !put_off.contains(id))
        .cloned()
        .collect()
}

/// Which of `put_off` stay put off: those still waiting for the person, or trying again, since
/// a try that fails sends them back to waiting and is not a new reason to ask. An account that
/// has recovered is forgotten, so that it is asked about if it ever needs the person again.
fn still_put_off(
    put_off: &BTreeSet<AccountId>,
    links: &[(AccountId, Link)],
) -> BTreeSet<AccountId> {
    links
        .iter()
        .filter(|(id, link)| {
            put_off.contains(id)
                && (Need::of(link).is_some() || matches!(link, Link::Syncing { .. }))
        })
        .map(|(id, _)| id.clone())
        .collect()
}

/// `accounts` taken out of `put_off`, so the prompt asks about them again. Whether there was one
/// to ask about.
fn asked_again(put_off: &mut BTreeSet<AccountId>, accounts: &[AccountId]) -> bool {
    put_off.retain(|id| !accounts.contains(id));
    !accounts.is_empty()
}

/// The dialog's state, shared by the window. Copy, so a handler can take it.
#[derive(Clone, Copy)]
pub(in crate::ui) struct Reauth {
    /// The accounts the person said Later about, or began signing in.
    put_off: Signal<BTreeSet<AccountId>>,
    /// The accounts whose sign-in has begun, with the revision it began at: when the revision
    /// moves, the Add Account window changed something, and the account's link is told.
    signing: Signal<BTreeMap<AccountId, u64>>,
    /// Whether the links have all said how they stand, once; it stays so.
    settled: Signal<bool>,
    /// The accounts offered Microsoft Graph that the person answered, either way, this session.
    answered: Signal<BTreeSet<AccountId>>,
    revision: Signal<u64>,
}

/// Provide [`Reauth`] for the window. Once, from `App`, after `use_fetching`.
pub(in crate::ui) fn use_reauth(revision: Signal<u64>) -> Reauth {
    let fetching = try_consume_context::<Fetching>();
    let needing = try_consume_context::<Needing>();
    let said_to = needing.clone();
    let put_off = use_signal(BTreeSet::new);
    let signing = use_signal(BTreeMap::new);
    let settled = use_signal(|| false);
    let answered = use_signal(BTreeSet::new);
    let reauth = use_context_provider(|| Reauth {
        put_off,
        signing,
        settled,
        answered,
        revision,
    });
    // The first moment nobody is still at the beginning: from then on it can be said who waits.
    use_effect(move || {
        let Some(fetching) = fetching else { return };
        let mut settled = settled;
        if !*settled.peek() && fetching.settled() {
            settled.set(true);
        }
    });
    // Who waits is said to the windows that have no links, and an account that recovered is
    // forgotten by Later.
    use_effect(move || {
        let Some(fetching) = fetching else { return };
        let links = fetching.all_links();
        if let Some(needing) = &said_to {
            needing.say(crate::ui::fetching::needs_you(&links));
        }
        let mut put_off = put_off;
        let kept = still_put_off(&put_off.peek(), &links);
        if kept != *put_off.peek() {
            put_off.set(kept);
        }
    });
    // A sign-in begun in the Settings window is waited for here, where the links are.
    use_future(move || {
        let heard = needing.as_ref().map(Needing::signing);
        async move {
            let Some(mut heard) = heard else { return };
            while heard.changed().await.is_ok() {
                let began = heard.borrow_and_update().clone();
                if let Some(account) = began {
                    reauth.began(account);
                }
            }
        }
    });
    // The Add Account window signed an account in, or changed its settings: it moves the shared
    // revision when it does, and a window closed with nothing added leaves the sign-in waiting.
    use_effect(move || {
        let now = revision();
        let Some(fetching) = fetching else { return };
        let mut signing = signing;
        let done: Vec<AccountId> = signing
            .peek()
            .iter()
            .filter(|(_, began)| **began != now)
            .map(|(id, _)| id.clone())
            .collect();
        if done.is_empty() {
            return;
        }
        signing.write().retain(|_, began| *began == now);
        for account in done {
            fetching.signed_in(account);
        }
    });
    reauth
}

impl Reauth {
    /// The accounts to ask about now: none until the links have settled.
    pub(in crate::ui) fn asked(self) -> Vec<(AccountId, Need)> {
        let Some(fetching) = try_consume_context::<Fetching>() else {
            return Vec::new();
        };
        if !(self.settled)() {
            return Vec::new();
        }
        to_ask(&fetching.needs(), &self.put_off.read())
    }

    /// The account to offer Microsoft Graph now, once no sign-in is asked: one whose last pass
    /// broke on Microsoft refusing IMAP and left that offer, not yet answered.
    pub(in crate::ui) fn graph(self) -> Option<AccountId> {
        let fetching = try_consume_context::<Fetching>()?;
        if !(self.settled)() || !self.asked().is_empty() {
            return None;
        }
        let answered = self.answered.read();
        fetching
            .all_links()
            .into_iter()
            .filter(|(id, link)| {
                matches!(link, Link::Broken { .. })
                    && fetching.offer(id) == Some(Offer::UseGraph)
                    && !answered.contains(id)
            })
            .map(|(id, _)| id)
            .min()
    }

    /// Whether the window is asking the person something: a sign-in, or Microsoft Graph.
    pub(in crate::ui) fn asking(self) -> bool {
        !self.asked().is_empty() || self.graph().is_some()
    }

    /// Later: the accounts asked about now are not asked about again this session, until they
    /// recover; with no sign-in asked, the account offered Microsoft Graph is not offered it again
    /// this session.
    pub(in crate::ui) fn later(self) {
        let asked: Vec<AccountId> = self.asked().into_iter().map(|(id, _)| id).collect();
        if asked.is_empty() {
            if let Some(account) = self.graph() {
                let mut answered = self.answered;
                answered.write().insert(account);
            }
            return;
        }
        let mut put_off = self.put_off;
        put_off.write().extend(asked);
    }

    /// Use Microsoft Graph for `account`: set it up again to read and send through Graph and sign
    /// it in for that in the browser (`mail_core::account::switch_to_graph`), then fetch with the
    /// new sign-in. It is not offered again while that runs, nor after.
    ///
    /// The sign-in waits on a browser, so it runs on a thread of its own, as the Add account
    /// sheet's does; what it came to is said, and a sign-in that worked is told to the account's
    /// link.
    pub(in crate::ui) fn use_graph(self, account: AccountId, name: String) {
        let (Some(store), Some(fetching)) = (
            try_consume_context::<Arc<SqliteStore>>(),
            try_consume_context::<Fetching>(),
        ) else {
            return;
        };
        let mut answered = self.answered;
        answered.write().insert(account.clone());
        let mut revision = self.revision;
        dioxus::core::spawn_forever(async move {
            let (done, answer) = tokio::sync::oneshot::channel();
            let switching = account.clone();
            std::thread::spawn(move || {
                let environment = crate::edge::environment();
                let secrets = crate::edge::secrets();
                let saved = crate::edge::clients().saved();
                let browse = |url: &str| {
                    if let Err(why) = webbrowser::open(url) {
                        log::warn!("the browser would not open the Microsoft sign-in: {why}");
                    }
                };
                let switched = crate::edge::block_on(mail_core::account::switch_to_graph(
                    &store,
                    switching,
                    chrono::Utc::now(),
                    mail_core::account::Credentials {
                        password: None,
                        saved: &saved,
                        secrets: secrets.as_ref(),
                        environment: &environment,
                        on_url: &browse,
                        signed: None,
                    },
                ));
                // The window that asked may have closed; then nobody is left to tell.
                let _ = done.send(switched.map(|added| added.outcome).map_err(String::from));
            });
            let said = match answer.await {
                Ok(Ok(mail_core::account::Outcome::SignedIn { .. })) => {
                    revision += 1;
                    fetching.signed_in(account);
                    copy::toasts::graph_signed_in(&name)
                }
                Ok(Ok(_)) => {
                    revision += 1;
                    copy::toasts::graph_waits(&name)
                }
                Ok(Err(why)) => copy::toasts::graph_refused(&name, &why),
                Err(_) => copy::toasts::graph_refused(&name, ""),
            };
            crate::ui::motion::tell(
                said.trim_end().to_owned(),
                crate::ui::motion::Follow::Nothing,
            );
        });
    }

    /// Sign `address`'s account in again: straight in the browser when its plan signs in with a redirect, else in the Add Account window. The
    /// account is not asked about again while it is being signed in. Call it from an event
    /// handler.
    pub(in crate::ui) fn sign_in(self, address: &str, need: Need) {
        let store = consume_context::<Arc<SqliteStore>>();
        let Some(account) = account_rows(&store)
            .into_iter()
            .find(|row| row.address == address)
            .map(|row| row.id)
        else {
            return;
        };
        self.began(account);
        match need {
            Need::SignIn => crate::ui::add_account::again(address.to_owned()),
            Need::Allow => crate::ui::add_account::allow_again(),
        }
    }

    /// Ask again about `accounts`, put off with Later: the foot's sync mark pressed while an
    /// account in view needs the person. Returns whether one of them is asked about.
    pub(in crate::ui) fn ask_again(self, accounts: &[AccountId]) -> bool {
        let mut put_off = self.put_off;
        asked_again(&mut put_off.write(), accounts)
    }

    /// `account`'s sign-in has begun.
    fn began(self, account: AccountId) {
        let (mut put_off, mut signing) = (self.put_off, self.signing);
        put_off.write().insert(account.clone());
        let revision = *self.revision.peek();
        signing.write().insert(account, revision);
    }
}

/// The dialog. Mounted always; draws nothing while there is nobody to ask about.
#[component]
pub(in crate::ui) fn ReauthPrompt() -> Element {
    let Some(reauth) = try_consume_context::<Reauth>() else {
        return rsx! {};
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let names: BTreeMap<AccountId, String> = account_rows(&store)
        .into_iter()
        .map(|row| (row.id.clone(), row.shown()))
        .collect();
    let asked: Vec<(AccountId, String, Need)> = reauth
        .asked()
        .into_iter()
        .filter_map(|(id, need)| names.get(&id).map(|name| (id, name.clone(), need)))
        .collect();
    match asked.as_slice() {
        [] => match reauth.graph() {
            Some(account) => {
                let name = names.get(&account).cloned().unwrap_or_default();
                let switching = name.clone();
                rsx! {
                    Alert {
                        title: copy::titles::use_graph(&name),
                        message: Some(TextLine::from(copy::errors::IMAP_REFUSED)),
                        style: AlertStyle::Warning,
                        buttons: vec![
                            AlertButton::new(
                                copy::controls::USE_GRAPH,
                                AlertRole::Normal,
                                EventHandler::new(move |()| {
                                    reauth.use_graph(account.clone(), switching.clone())
                                }),
                            ),
                            AlertButton::new(
                                LATER,
                                AlertRole::Cancel,
                                EventHandler::new(move |()| reauth.later()),
                            )
                            .with_common(crate::ui::sidebar::tagged("answer", "later")),
                        ],
                    }
                }
            }
            None => rsx! {},
        },
        [(_, name, need)] => {
            let (signing, need) = (name.clone(), *need);
            rsx! {
                Alert {
                    title: one(name),
                    message: Some(TextLine::from(MESSAGE)),
                    style: AlertStyle::Warning,
                    buttons: vec![
                        AlertButton::new(
                            SIGN_IN,
                            AlertRole::Normal,
                            EventHandler::new(move |()| reauth.sign_in(&signing, need)),
                        )
                        .with_common(Common {
                            aria_label: Some(format!("{SIGN_IN} to {name}")),
                            ..Common::default()
                        }),
                        AlertButton::new(
                            LATER,
                            AlertRole::Cancel,
                            EventHandler::new(move |()| reauth.later()),
                        )
                        .with_common(crate::ui::sidebar::tagged("answer", "later")),
                    ],
                }
            }
        }
        _ => rsx! {
            Sheet {
                label: MANY,
                attach: Attach::Window,
                common: in_card(),
                onclose: move |()| reauth.later(),
                div { class: "doctor",
                    Label { text: MANY, style: LabelStyle::Title }
                    Label { text: MESSAGE }
                    for (account, name, need) in asked.clone() {
                        Row {
                            key: "{account}",
                            leading: RowLeading::Icon(Icon::TriangleAlert),
                            title: name.clone(),
                            outline: Outline::None,
                            accessory: Accessory::Slot(rsx! {
                                Button {
                                    size: ControlSize::Small,
                                    label: SIGN_IN,
                                    common: Common {
                                        aria_label: Some(format!("{SIGN_IN} to {name}")),
                                        ..Common::default()
                                    },
                                    onclick: on_primary({
                                        let name = name.clone();
                                        move || reauth.sign_in(&name, need)
                                    }),
                                }
                            }),
                        }
                    }
                    div { class: "doctor-foot",
                        SheetClose { label: LATER, on_close: move |()| reauth.later() }
                    }
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use mail_core::fetch::{First, Live, Step};
    use mail_domain::id::account_id_from_uuid;

    fn id(n: u128) -> AccountId {
        account_id_from_uuid(uuid::Uuid::from_u128(n))
    }

    fn needing() -> Link {
        Link::NeedsSignIn {
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

    fn trying() -> Link {
        Link::Syncing {
            first: First::No,
            step: Step::Folders,
            count: None,
            after: Box::new(needing()),
        }
    }

    #[test]
    fn later_keeps_the_dialog_away_from_those_it_was_said_about() {
        let both = [(id(1), Need::SignIn), (id(2), Need::Allow)];
        let cases = [
            (vec![], both.to_vec()),
            (vec![id(1)], vec![(id(2), Need::Allow)]),
            (vec![id(1), id(2)], vec![]),
            (vec![id(3)], both.to_vec()),
        ];
        for (put_off, want) in cases {
            let put_off: BTreeSet<AccountId> = put_off.into_iter().collect();
            assert_eq!(to_ask(&both, &put_off), want, "{put_off:?}");
        }
    }

    /// Later puts the question away; the foot's mark, pressed, brings it back for the accounts
    /// in view, and only those.
    #[test]
    fn the_sync_mark_asks_again_about_an_account_put_off() {
        let both = [(id(1), Need::SignIn), (id(2), Need::SignIn)];
        let mut put_off: BTreeSet<AccountId> = [id(1), id(2)].into_iter().collect();
        assert!(
            to_ask(&both, &put_off).is_empty(),
            "Later kept nothing away"
        );
        assert!(asked_again(&mut put_off, &[id(1)]));
        assert_eq!(to_ask(&both, &put_off), [(id(1), Need::SignIn)]);
        assert!(
            !asked_again(&mut put_off, &[]),
            "a press with nobody waiting asked"
        );
        assert_eq!(put_off, [id(2)].into_iter().collect());
    }

    #[test]
    fn an_account_is_forgotten_by_later_once_it_has_recovered() {
        let put_off: BTreeSet<AccountId> = [id(1), id(2), id(3)].into_iter().collect();
        let links = [
            (id(1), needing()),
            (id(2), current()),
            (id(3), trying()),
            (id(4), needing()),
        ];
        let kept: BTreeSet<AccountId> = [id(1), id(3)].into_iter().collect();
        assert_eq!(still_put_off(&put_off, &links), kept);
    }
}
