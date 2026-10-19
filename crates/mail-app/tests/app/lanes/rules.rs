//! Lane 10: a rule. Settings → Rules makes "from Edsger → Archive" on New Rule…'s page; a
//! matching message arrives; Run on Existing Mail, from the rule's right-click menu, files it,
//! and the main window's inbox follows.

use ds_harness::Harness;
use mail_core::SqliteStore;
use std::sync::Arc;

use super::hands::{click, scroll_to, type_text, until};
use super::look::inbox_at;
use super::window::{Window, hours_ago, row_of};

const ARRIVING: &str = "Agenda for Monday";

/// In Settings → Rules: a rule archiving whatever Edsger sends, made on the page New Rule…
/// pushes, and kept by Save Rule, which goes back to the list.
fn make_the_rule(settings: &mut Harness) {
    click(settings, "[*|aria-label=\"New Rule\u{2026}\"]");
    until(settings, "the rule's page opens", |h| {
        h.count("input[*|aria-label=\"Condition\"]") == 1
    });
    click(settings, "input[*|aria-label=\"Name\"]");
    type_text(settings, "Edsger");
    click(settings, "input[*|aria-label=\"Condition\"]");
    type_text(settings, "from:edsger@example.test");
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
        ".ds-field-row[*|aria-label=\"Do\"] .ds-button",
    );
    click(settings, ".ds-field-row[*|aria-label=\"Do\"] .ds-button");
    super::hands::menu_item(settings, "Archive");
    scroll_to(settings, ".settings-scroll", "[*|aria-label=\"Save Rule\"]");
    click(settings, "[*|aria-label=\"Save Rule\"]");
    until(settings, "the list shows the rule", |h| {
        h.count("input[*|aria-label=\"Condition\"]") == 0
            && h.text_of(".settings-page")
                .is_some_and(|page| page.contains("Edsger"))
    });
}

/// In Settings → Rules: run the rule on the mail already here from its row's right-click menu,
/// and wait for it to end.
fn run_the_rule(settings: &mut Harness, store: &SqliteStore) {
    use super::drive::Drive as _;
    let row = "[*|role=\"option\"][*|id^=\"rules-pane-open-\"]";
    until(settings, "the rule's row is drawn", |h| h.count(row) == 1);
    let at = super::hands::centre(settings, row);
    settings.press(at, ds::base::press::PointerButton::Secondary);
    super::hands::menu_item(settings, "Run on Existing Mail");
    // A run says nothing when it ends: what it did is in the mail, and its bar goes.
    until(settings, "the rule ran to its end", |h| {
        h.count(".rules-progress") == 0
            && !inbox_at(store, chrono::Utc::now()).contains(&ARRIVING.to_owned())
    });
    assert_eq!(settings.count(".ds-alert"), 0, "the run said it failed");
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
