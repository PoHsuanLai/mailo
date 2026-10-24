//! Lane 10: a rule. A message from Edsger arrives; Settings → Rules makes "from Edsger → Archive"
//! on New Rule…'s page, from its From row, with "Also apply" checked; saving files the message,
//! and the main window's inbox follows.

use ds_harness::Harness;
use mail_app::ui::copy;
use mail_core::SqliteStore;
use std::sync::Arc;

use super::hands::{click, scroll_to, type_text, until};
use super::look::inbox_at;
use super::window::{Window, hours_ago, row_of};

const ARRIVING: &str = "Agenda for Monday";

/// What carries `label` as its accessible name.
fn labelled(label: &str) -> String {
    format!("[*|aria-label=\"{label}\"]")
}

/// In Settings → Rules: a rule archiving whatever Edsger sends, made on the page New Rule…
/// pushes, applied to the mail already here, and kept by Save Rule, which goes back to the list.
fn make_the_rule(settings: &mut Harness, store: &SqliteStore) {
    // The From row's field, which a new rule opens on.
    let from = format!("input{}", labelled(copy::fields::FROM));
    let save = labelled(copy::aria::SAVE_RULE);
    let add = labelled(copy::aria::ADD_ACTION);
    click(settings, &labelled(copy::controls::NEW_RULE));
    until(settings, "the rule's page opens", |h| h.count(&from) == 1);
    click(settings, &format!("input{}", labelled(copy::fields::NAME)));
    type_text(settings, "Edsger");
    click(settings, &from);
    type_text(settings, "edsger@example.test");
    until(
        settings,
        "the condition reads, so Save Rule is offered",
        |h| {
            h.count(&format!("{save}[disabled]")) == 0
                && h.count(&format!("{save}[*|aria-disabled=true]")) == 0
        },
    );
    scroll_to(settings, ".settings-scroll", &add);
    click(settings, &add);
    super::hands::menu_item(settings, copy::menus::ARCHIVE);
    scroll_to(settings, ".settings-scroll", ".ds-toggle");
    click(settings, ".ds-toggle");
    until(settings, "Also apply is on", |h| {
        h.count(".ds-toggle[*|aria-checked=true]") == 1
    });
    scroll_to(settings, ".settings-scroll", &save);
    click(settings, &save);
    until(settings, "the list shows the rule", |h| {
        h.count(&from) == 0
            && h.text_of(".settings-page")
                .is_some_and(|page| page.contains("Edsger"))
    });
    // A run says nothing when it ends: what it did is in the mail, and its bar goes.
    until(settings, "the rule ran to its end", |h| {
        h.count(".rules-progress") == 0
            && !inbox_at(store, chrono::Utc::now()).contains(&ARRIVING.to_owned())
    });
    // The fixture's account has no password to reach its server with, so the save's push to it
    // may be refused in a dialog; a run that failed would say so in one of its own.
    let said = settings.text_of(".ds-alert").unwrap_or_default();
    assert!(
        !said.contains(&copy::titles::did_not_run("Edsger")),
        "the run said it failed: {said:?}"
    );
}

#[test]
fn a_rule_made_in_settings_files_the_mail_it_matches() {
    let mut window = Window::open(|_| {});

    // A message from Edsger arrives, as a sync stores it, before there is a rule.
    window.arrives(
        "agenda1",
        &format!(
            "From: Edsger Dijkstra <edsger@example.test>\r\nTo: Me <me@example.test>\r\n\
             Subject: {ARRIVING}\r\nDate: {}\r\nMessage-ID: <agenda1@example.test>\r\n\r\n\
             Items attached below.\r\n",
            hours_ago(0)
        ),
    );
    window.until("the message shows in the inbox", |h| {
        row_of(h, ARRIVING).is_some()
    });

    // The rule, applied to the mail already here: both of Edsger's conversations leave the inbox.
    let store = Arc::clone(&window.store);
    window.in_settings(copy::titles::RULES, |settings| {
        make_the_rule(settings, &store)
    });
    window.until("the main window's inbox follows", |h| {
        row_of(h, ARRIVING).is_none() && row_of(h, "Notes from the review").is_none()
    });
    assert_eq!(
        window.subjects(),
        [
            "Flight to the conference",
            "The invoice for September",
            "Lunch on Thursday"
        ]
    );
}

/// Block Sender, from the reader's ⋯: a rule that files the sender's mail in Spam, first in the
/// order, and the conversation in Spam now.
#[test]
fn block_sender_makes_a_rule_and_files_the_conversation_in_spam() {
    use mail_core::Store as _;
    use mail_domain::{Filter, MailboxRole, RuleAction, TextMatch};
    let mut window = Window::open(|_| {});
    let subject = super::window::INBOX[0].1;
    let sender = super::look::conversation(&window.store, subject).from.email;
    window.open_subject(subject);
    window.click(&format!(
        ".reader-head [*|aria-label=\"{}\"]",
        copy::aria::MORE
    ));
    window.menu_item(copy::menus::BLOCK_SENDER);
    window.until("the toast says the sender is blocked", |h| {
        h.text_of(".ds-toast-body") == Some(copy::toasts::blocked(&sender))
    });
    let store = window.store.clone();
    window.until("the conversation is in Spam", |_| {
        super::look::conversation(&store, subject)
            .mailboxes
            .contains(MailboxRole::Spam)
    });
    let rules = window.store.rules(super::window::account()).unwrap();
    let block = rules
        .iter()
        .find(|rule| rule.filter == Filter::From(TextMatch::Exact(sender.clone())))
        .expect("a rule for the sender");
    assert_eq!(block.actions, vec![RuleAction::Spam]);
    assert!(rules.iter().all(|rule| rule.position >= block.position));
}
