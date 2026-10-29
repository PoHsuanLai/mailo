//! The Rules page of Settings: the accounts in a list on the left, and on the right the chosen
//! one's rules, in the order they run, each a page of its own pushed over the list; and the
//! Vacation Reply page, Settings' next: the same accounts, and the chosen one's reply
//! ([`vacation`]).
//!
//! The search bar's "Rules Settings" opens Settings on it. Everything it writes goes through the
//! same rows `mailo rules` and `mailo vacation` use — [`work`] and [`away`] are those questions
//! and writes as functions; the parts only draw them. One account at a time, because a rule acts
//! on one account's labels and folders, and a server runs one account's script.
//!
//! Server work is never a step the person takes: every save (a rule kept, switched, moved or
//! deleted, the vacation reply kept) puts the account's script on its server as `mailo sieve
//! push` does ([`server`]), and only a failure is said, at the head of the page. An account
//! whose server runs no rules keeps them here, and nothing is said.
//!
//! The rule pages are quire's `PaneStack`; which are shown is the window's `Shell` (`rules.path`).

pub(in crate::ui) mod apply;
mod away;
mod conditions;
mod editor;
mod list;
mod server;
mod vacation;
mod when;
pub(in crate::ui) mod work;

use chrono::Utc;
use dioxus::prelude::*;
use ds::prelude::*;
use ds::root::common::Common;

use super::common::Told;
use super::data::{AccountRow, account_rows};
use super::files::{Phase, run};
use crate::ui::copy;
use crate::ui::settings_window::account_split::{AccountSplit, NoAccountsPage, shown};
use crate::ui::view::{RulesAt, RulesPage as Showing, Shell};
use editor::RulePage;
use list::RulesList;
use mail_client::rules::server::AwayServer;
use mail_domain::Rule;
use porter_core::AccountId;
use server::{AwaySetter, Pusher, put, reach, reach_away, set_away};
pub(in crate::ui) use vacation::VacationPage;

/// What the last act on the page came to, when it failed: said in a dialog until it is closed.
pub(super) type Said = Signal<Option<Result<String, String>>>;

/// The stack's element id: its back buttons are `rules-pane-back-root` and `-detail`.
const STACK: &str = "rules-pane";

/// The element id of the row that opens `at`, where the keyboard goes back to.
pub(super) fn row_id(at: &RulesAt) -> Option<String> {
    match at {
        RulesAt::List => None,
        RulesAt::Rule(Some(id)) => Some(format!("rules-pane-open-{id}")),
        RulesAt::Rule(None) => Some("rules-pane-new".to_owned()),
    }
}

/// Push `at` over the list.
pub(super) fn open(mut shell: Signal<Shell>, at: RulesAt) {
    shell.write().rules.path = PanePath::new(RulesAt::List).pushed(at);
}

/// Back to the list.
pub(super) fn back(mut shell: Signal<Shell>) {
    let back = shell.peek().rules.path.popped();
    shell.write().rules.path = back;
}

/// The page: the accounts in a list ([`super::settings_window::account_split`]), and beside it the
/// chosen account's rules, its rule's page or a new rule's pushed over them.
#[component]
pub(in crate::ui) fn RulesPage(shell: Signal<Shell>) -> Element {
    let client = crate::ui::live::client();
    let rows = account_rows(&client);
    let mut said: Said = use_signal(|| None);
    // A rule or vacation reply kept here that its server did not take, said apart from a write
    // that did not happen at all.
    let unpushed: Said = use_signal(|| None);
    // A rule applied to the mail already here, as it runs: started as its page saves it and goes
    // back, so it is held here, and the list draws its bar.
    let phase = use_signal(|| Phase::Ready);
    let running = use_signal(String::new);
    let applied = use_callback(move |rule: Rule| apply(rule, phase, running));
    // Every save ends here: the account's script goes to its server, off the thread that draws,
    // and only a failure is said. Made here, so a push outlives the rule's page it was saved on.
    let pushed = use_callback(move |row: AccountRow| push(row, unpushed));
    if rows.is_empty() {
        return rsx! { NoAccountsPage { title: copy::titles::RULES } };
    }
    let held = shell.read().rules.account.clone();
    let chosen = shown(&rows, held.as_ref()).cloned();
    rsx! {
        AccountSplit {
            pane: STACK,
            label: copy::titles::RULES,
            rows: rows.clone(),
            chosen: chosen.as_ref().map(|row| row.id.clone()),
            onpick: move |id: AccountId| {
                said.set(None);
                let mut shell = shell;
                shell.write().rules = Showing { account: Some(id), ..Showing::default() };
            },
            if let Some(row) = chosen {
                AccountRules { key: "{row.id}", shell, row, said, pushed, phase, running, applied }
            }
        }
        Told { title: copy::titles::NOT_SAVED, said }
        Told { title: copy::titles::NOT_ON_SERVER, said: unpushed }
    }
}

/// One account's rules: its list at the root, a rule's page over it.
#[component]
fn AccountRules(
    shell: Signal<Shell>,
    row: AccountRow,
    said: Said,
    pushed: Callback<AccountRow>,
    phase: Signal<Phase>,
    running: Signal<String>,
    applied: Callback<Rule>,
) -> Element {
    let path = shell.read().rules.path.clone();
    let named = row.clone();
    let title = use_callback(move |at: RulesAt| match at {
        RulesAt::List => named.shown(),
        RulesAt::Rule(None) => copy::titles::NEW_RULE.to_owned(),
        RulesAt::Rule(Some(id)) => {
            let client = crate::ui::live::client();
            client
                .stored()
                .rules(named.id.clone())
                .ok()
                .and_then(|rules| rules.into_iter().find(|rule| rule.id == id))
                .map_or_else(|| named.shown(), |rule| rule.name)
        }
    });
    let account = row.clone();
    let page = use_callback(move |at: RulesAt| {
        let row = account.clone();
        match at {
            RulesAt::List => rsx! { RulesList { shell, row, said, pushed, phase, running } },
            RulesAt::Rule(id) => {
                rsx! { RulePage { key: "{id:?}", shell, row, id, said, pushed, applied } }
            }
        }
    });
    let opener = use_callback(|at: RulesAt| row_id(&at));
    rsx! {
        PaneStack::<RulesAt> {
            path,
            title,
            page,
            opener: Some(opener),
            on_back: move |()| back(shell),
            common: Common { id: Some(STACK.to_owned()), ..Common::default() },
        }
    }
}

/// Apply `rule` to the mail already here, as `mailo rules run` does, its bar in `phase`; the
/// window's lists follow once it ends.
fn apply(rule: Rule, phase: Signal<Phase>, mut running: Signal<String>) {
    let client = crate::ui::live::client();
    let of = client
        .rules_manage()
        .conversations(rule.account.clone(), Utc::now());
    running.set(rule.name.clone());
    run(
        phase,
        of,
        move |report| work::run(&client, &rule, Utc::now(), report),
        // What it wrote draws the window again: nothing more to do after.
        || {},
    );
}

/// Put `row`'s rules and vacation reply on its server, when it has one that runs them: on a
/// blocking thread, saying only a failure, in `said`.
fn push(row: AccountRow, mut said: Said) {
    if let Err(why) = reach(&row.plan) {
        log::debug!("{}: no server to put the rules on: {why}", row.address);
        return;
    }
    let client = crate::ui::live::client();
    let push = try_consume_context::<Pusher>()
        .unwrap_or_else(Pusher::server)
        .0;
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || put(&client, &row, &push, Utc::now())).await;
        let failed = match done {
            Ok(Ok(_)) => return,
            Ok(Err(why)) => why,
            Err(error) => error.to_string(),
        };
        said.set(Some(Err(copy::errors::not_on_server(&failed))));
    });
}

/// Put `row`'s vacation reply where its server keeps one: with its rules in its Sieve script
/// ([`push`]), or set on its JMAP or Graph server. On a blocking thread, saying only a failure,
/// in `said`.
fn push_away(row: AccountRow, mut said: Said) {
    match reach_away(&row.plan) {
        Ok(AwayServer::Sieve) => return push(row, said),
        Ok(AwayServer::Jmap | AwayServer::Graph) => {}
        // Nowhere to put one: the page offers no switch for such an account.
        Err(why) => {
            log::debug!("{}: no reply to put on a server: {why}", row.address);
            return;
        }
    }
    let client = crate::ui::live::client();
    let set = try_consume_context::<AwaySetter>()
        .unwrap_or_else(AwaySetter::server)
        .0;
    spawn(async move {
        let done =
            tokio::task::spawn_blocking(move || set_away(&client, &row, &set, Utc::now())).await;
        let failed = match done {
            Ok(Ok(())) => return,
            Ok(Err(why)) => why,
            Err(error) => error.to_string(),
        };
        said.set(Some(Err(copy::errors::not_on_server(&failed))));
    });
}

#[cfg(test)]
mod away_tests;
#[cfg(test)]
pub(in crate::ui) mod page_tests;
#[cfg(test)]
mod work_tests;
