//! Apply Rules on picked conversations: asks before it sends anything.
//!
//! The rules that reply or forward speak for the person. Applying them to mail the person picked
//! is their ask, but not their ask to send, so when a rule of an account in the picks would send,
//! an alert says so first and offers to apply the rules without the sending.

use super::work::apply_to;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::*;
use mail_client::Client;
use mail_client::rules::manage::Sending;
use mail_domain::ThreadId;
use mail_ui_model::copy;

/// Apply the rules to `threads`: at once when none of them would send, else ask first.
pub(in crate::ui) fn apply_or_ask(
    client: &Client,
    mut shell: Signal<Shell>,
    threads: Vec<ThreadId>,
) {
    if client.rules_manage().would_send(&threads) {
        shell.write().applying_rules = Some(threads);
    } else {
        say(&apply_to(
            client,
            &threads,
            Sending::Yes,
            chrono::Utc::now(),
        ));
    }
}

fn say(said: &str) {
    crate::ui::motion::tell(said.to_owned(), crate::ui::motion::Follow::Nothing);
}

/// Close the alert, applying nothing, and give the keyboard back to the window.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().applying_rules = None;
    crate::ui::host::Host::focus_app();
}

fn answer(shell: Signal<Shell>, sending: Sending) {
    let Some(threads) = shell.peek().applying_rules.clone() else {
        return;
    };
    close(shell);
    let client = crate::ui::live::client();
    say(&apply_to(&client, &threads, sending, chrono::Utc::now()));
}

/// The alert while it is open: its Cancel answers Escape.
#[component]
pub(in crate::ui) fn ApplyRulesAlert(shell: Signal<Shell>) -> Element {
    if shell.read().applying_rules.is_none() {
        return rsx! {};
    }
    rsx! {
        Alert {
            title: copy::confirm::APPLY_RULES_SENDS.to_owned(),
            message: Some(TextLine::from(copy::confirm::APPLY_RULES_SENDS_BODY)),
            style: AlertStyle::Warning,
            buttons: vec![
                AlertButton::new(
                    copy::controls::APPLY_AND_SEND,
                    AlertRole::Normal,
                    EventHandler::new(move |()| answer(shell, Sending::Yes)),
                ),
                AlertButton::new(
                    copy::controls::APPLY_WITHOUT_SENDING,
                    AlertRole::Normal,
                    EventHandler::new(move |()| answer(shell, Sending::No)),
                ),
                AlertButton::new(
                    copy::controls::CANCEL,
                    AlertRole::Cancel,
                    EventHandler::new(move |()| close(shell)),
                ),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::fixtures::{acct_account, seeded, thread_like};
    use mail_domain::{AfterMatch, Filter, Rule, RuleAction, RuleId, RuleState};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Clone)]
    struct Probe(Vec<ThreadId>, Arc<AtomicBool>);

    #[component]
    fn Asker() -> Element {
        let Probe(threads, asked) = consume_context::<Probe>();
        let shell = use_signal(Shell::default);
        use_hook(|| {
            apply_or_ask(&crate::ui::live::client(), shell, threads.clone());
            asked.store(shell.peek().applying_rules.is_some(), Ordering::SeqCst);
        });
        rsx! {}
    }

    fn with_rule(actions: Vec<RuleAction>) -> bool {
        let (store, _dir) = seeded();
        let thread = thread_like(&store, "hi");
        mail_store::Store::put_rule(
            &*store,
            &Rule {
                id: RuleId::generate(),
                account: acct_account(),
                name: "Rule".to_owned(),
                position: 1,
                state: RuleState::Enabled,
                filter: Filter::All,
                actions,
                after: AfterMatch::Continue,
                made: Default::default(),
                reply_days: 4,
            },
        )
        .unwrap();
        let asked = Arc::new(AtomicBool::new(false));
        let mut dom = VirtualDom::new(Asker)
            .with_root_context(crate::ui::fixtures::client(&store))
            .with_root_context(Probe(vec![thread], asked.clone()));
        dom.rebuild_in_place();
        asked.load(Ordering::SeqCst)
    }

    /// Rules that reply or forward are asked about before they send; the rest apply at once.
    #[test]
    fn apply_rules_asks_only_when_a_rule_would_send() {
        assert!(
            with_rule(vec![RuleAction::Reply("Away".to_owned())]),
            "a reply asks"
        );
        assert!(
            with_rule(vec![RuleAction::Forward("a@b.test".to_owned())]),
            "a forward asks"
        );
        assert!(!with_rule(vec![RuleAction::Archive]), "an archive does not");
        assert!(
            !with_rule(vec![RuleAction::Notify]),
            "a notification sends nothing"
        );
    }
}
