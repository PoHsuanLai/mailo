//! Lane 10: a rule. A message from Edsger arrives; Settings → Rules makes "from Edsger → Archive"
//! on New Rule…'s page, from its From row, with "Also apply" checked; saving files the message,
//! and the main window's inbox follows.

use ds_harness::Harness;
use mail_core::SqliteStore;
use std::sync::Arc;

use super::hands::{click, scroll_to, type_text, until};
use super::look::inbox_at;
use super::window::{Window, hours_ago, row_of};

const ARRIVING: &str = "Agenda for Monday";

/// The From row's field a new rule opens on.
const FROM: &str = "input[*|aria-label=\"From\"]";

/// In Settings → Rules: a rule archiving whatever Edsger sends, made on the page New Rule…
/// pushes, applied to the mail already here, and kept by Save Rule, which goes back to the list.
fn make_the_rule(settings: &mut Harness, store: &SqliteStore) {
    click(settings, "[*|aria-label=\"New Rule\u{2026}\"]");
    until(settings, "the rule's page opens", |h| h.count(FROM) == 1);
    click(settings, "input[*|aria-label=\"Name\"]");
    type_text(settings, "Edsger");
    click(settings, FROM);
    type_text(settings, "edsger@example.test");
    until(
        settings,
        "the condition reads, so Save Rule is offered",
        |h| {
            h.count("[*|aria-label=\"Save Rule\"][disabled]") == 0
                && h.count("[*|aria-label=\"Save Rule\"][*|aria-disabled=true]") == 0
        },
    );
    scroll_to(
        settings,
        ".settings-scroll",
        "[*|aria-label=\"Add an action\"]",
    );
    click(settings, "[*|aria-label=\"Add an action\"]");
    super::hands::menu_item(settings, "Archive");
    scroll_to(settings, ".settings-scroll", ".ds-toggle");
    click(settings, ".ds-toggle");
    until(settings, "Also apply is on", |h| {
        h.count(".ds-toggle[*|aria-checked=true]") == 1
    });
    scroll_to(settings, ".settings-scroll", "[*|aria-label=\"Save Rule\"]");
    click(settings, "[*|aria-label=\"Save Rule\"]");
    until(settings, "the list shows the rule", |h| {
        h.count(FROM) == 0
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
    assert!(!said.contains("Run"), "the run said it failed: {said:?}");
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
    window.in_settings("Rules", |settings| make_the_rule(settings, &store));
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
