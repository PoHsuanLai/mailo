//! Lane 10: a rule. Settings → Rules makes "from Edsger → Archive"; a matching message arrives;
//! running the rule on existing mail files it, and the main window's inbox follows.

use ds_harness::{Harness, Query};
use mail_store::SqliteStore;
use std::sync::Arc;

use super::hands::{click, scroll_to, type_text, until};
use super::look::inbox_at;
use super::window::{Window, hours_ago, row_of};

const ARRIVING: &str = "Agenda for Monday";

/// In Settings → Rules: a rule archiving whatever Edsger sends.
fn make_the_rule(settings: &mut Harness) {
    click(settings, "[*|aria-label=\"New rule\"]");
    until(settings, "the rule's form opens", |h| {
        h.count("input[*|aria-label=\"Condition\"]") == 1
    });
    click(settings, "input[*|aria-label=\"Name\"]");
    type_text(settings, "Edsger");
    click(settings, "input[*|aria-label=\"Condition\"]");
    type_text(settings, "from:edsger@example.test");
    until(settings, "the condition matches his conversation", |h| {
        h.text_of(".settings-page")
            .is_some_and(|page| page.contains("1 conversation"))
    });
    scroll_to(
        settings,
        ".settings-scroll",
        ".ds-field-row[*|aria-label=\"Do\"] .ds-button",
    );
    click(settings, ".ds-field-row[*|aria-label=\"Do\"] .ds-button");
    super::hands::menu_item(settings, "Archive");
    scroll_to(
        settings,
        ".settings-scroll",
        "[*|aria-label=\"Save New rule\"]",
    );
    click(settings, "[*|aria-label=\"Save New rule\"]");
    until(settings, "the page says it kept the rule", |h| {
        h.text_of(".settings-page")
            .is_some_and(|page| page.contains("Kept the rule"))
    });
}

/// In Settings → Rules: run the rule on the mail already here, and wait for it to end.
fn run_the_rule(settings: &mut Harness, store: &SqliteStore) {
    scroll_to(
        settings,
        ".settings-scroll",
        "[*|aria-label=\"Run Edsger on existing mail\"]",
    );
    click(settings, "[*|aria-label=\"Run Edsger on existing mail\"]");
    until(settings, "the page says the rule ran", |h| {
        h.text_of(".settings-page")
            .is_some_and(|page| page.contains("Ran \u{201c}Edsger\u{201d}"))
    });
    assert!(
        !inbox_at(store, chrono::Utc::now()).contains(&ARRIVING.to_owned()),
        "the rule ran and left the message in the inbox"
    );
}

#[test]
fn a_rule_made_in_settings_files_the_mail_it_matches() {
    let mut window = Window::open(|_| {});
    window.in_settings("Rules", make_the_rule);

    // A message from Edsger arrives, as a sync stores it. A rule runs in the sync that brings
    // mail (`mail_store::rules::at_arrival`); nothing here syncs, so it lands in the inbox.
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

    // Run on existing mail: both of Edsger's conversations leave the inbox.
    let store = Arc::clone(&window.store);
    window.in_settings("Rules", |settings| run_the_rule(settings, &store));
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
