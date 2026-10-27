//! Push relay on an account's page in Settings: shown for a Microsoft 365 account that reads
//! through Graph and for no other, kept when Return is pressed, and refused under the field, which
//! is marked invalid, for an address that cannot be a relay's. Against the fixture's store only:
//! nothing is registered or subscribed from a page.

use crate::ui::copy::errors::{RELAY_EXTRAS, RELAY_NOT_HTTPS};
use crate::ui::copy::fields::account::{PUSH_RELAY, PUSH_RELAY_EXAMPLE};
use crate::ui::data::account_rows;
use crate::ui::fixtures::{Work, chord, drain_seen, type_into, work};
use dioxus::html::Modifiers;
use mail_core::sync::graph_push;
use mail_domain::Incoming;
use porter_core::AccountId;

const ADDRESS: &str = "poh@acme.example";

fn account(built: &Work) -> AccountId {
    account_rows(&built.store)
        .into_iter()
        .find(|row| row.address == ADDRESS)
        .map(|row| row.id)
        .unwrap_or_else(|| panic!("the fixture has no {ADDRESS}"))
}

/// The fixture's account, reading through Graph.
fn through_graph(built: &Work) -> AccountId {
    let row = account_rows(&built.store)
        .into_iter()
        .find(|row| row.address == ADDRESS)
        .unwrap_or_else(|| panic!("the fixture has no {ADDRESS}"));
    let mut plan = row.plan;
    plan.incoming = Incoming::Graph;
    built.store.set_account_plan(row.id.clone(), &plan).unwrap();
    row.id
}

#[tokio::test]
async fn only_an_account_that_reads_through_graph_has_a_push_relay() {
    let built = work();
    let (dom, _) = crate::ui::settings_window::account::tests::pushed(&built);
    let page = dioxus_ssr::render(&dom);
    assert!(!page.contains(PUSH_RELAY), "{page}");

    through_graph(&built);
    let (dom, _) = crate::ui::settings_window::account::tests::pushed(&built);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains(PUSH_RELAY), "{page}");
}

#[tokio::test]
async fn a_relay_is_kept_on_return_and_one_that_cannot_be_is_refused() {
    let built = work();
    let id = through_graph(&built);
    assert_eq!(account(&built), id);
    let (mut dom, seen) = crate::ui::settings_window::account::tests::pushed(&built);
    let field = seen.one("aria-placeholder", PUSH_RELAY_EXAMPLE);

    type_into(&mut dom, field, "https://relay.example.test/");
    chord(&mut dom, "Enter", Modifiers::empty(), field);
    drain_seen(&mut dom);
    assert_eq!(
        graph_push::relay(&built.store, id.clone()).as_deref(),
        Some("https://relay.example.test")
    );

    // Not HTTPS: said under the field, which is marked invalid, and what was kept stays.
    type_into(&mut dom, field, "http://relay.example.test");
    chord(&mut dom, "Enter", Modifiers::empty(), field);
    let seen = drain_seen(&mut dom);
    assert_eq!(seen.all("aria-invalid", "true"), [field]);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(RELAY_NOT_HTTPS),
        "the refusal is not said: {page}"
    );
    assert!(
        !page.contains(crate::ui::copy::titles::NOT_SAVED),
        "said in a dialog: {page}"
    );
    assert_eq!(
        graph_push::relay(&built.store, id.clone()).as_deref(),
        Some("https://relay.example.test")
    );

    // Each refusal in its own words.
    type_into(&mut dom, field, "https://relay.example.test/?a=b");
    chord(&mut dom, "Enter", Modifiers::empty(), field);
    drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains(RELAY_EXTRAS), "{page}");

    // Typing again takes the mark away.
    let seen = type_into(&mut dom, field, "https://relay.example.test/a");
    let seen = seen.merge(drain_seen(&mut dom));
    assert_eq!(seen.all("aria-invalid", "false"), [field]);

    // Emptied: polled again.
    type_into(&mut dom, field, "");
    chord(&mut dom, "Enter", Modifiers::empty(), field);
    drain_seen(&mut dom);
    assert_eq!(graph_push::relay(&built.store, id), None);
}
