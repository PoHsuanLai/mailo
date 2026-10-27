//! The sidebar as the client keeps it read: a commit nobody announced reaches it, and a label is
//! drawn only while its account is in scope.

use super::super::app::App;
use crate::ui::fixtures::empty;
use crate::ui::space::{Mail, built};
use dioxus::prelude::*;
use ds::prelude::SpaceLook;
use mail_domain::{LabelId, LabelOrigin};
use mail_store::testing::{seed_account, seed_label};
use porter_core::AccountId;
use std::time::{Duration, Instant};

fn account(n: u128) -> AccountId {
    mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(n))
}

fn drawn(page: &str, label: &str) -> bool {
    page.contains(&format!("data-place=\"{label}\""))
}

/// Render until `done` holds of the page, or fail saying `what` with the page.
async fn until(dom: &mut VirtualDom, what: &str, done: impl Fn(&str) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let _ = tokio::time::timeout(Duration::from_millis(20), dom.wait_for_work()).await;
        dom.render_immediate(&mut dioxus_core::NoOpMutations);
        let page = dioxus_ssr::render(dom);
        if done(&page) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: {page}");
    }
}

/// A window over two accounts' labels, in a Space of `shown`, with the client the program gives
/// every window: one reading on threads of its own.
fn window(
    shown: Vec<AccountId>,
) -> (
    VirtualDom,
    std::sync::Arc<mail_core::SqliteStore>,
    tempfile::TempDir,
) {
    let (store, dir) = empty();
    seed_account(&store, account(1), "one@example.test");
    seed_account(&store, account(2), "two@example.test");
    seed_label(
        &store,
        LabelId::generate(),
        account(1),
        "Travel",
        LabelOrigin::User,
    );
    seed_label(
        &store,
        LabelId::generate(),
        account(2),
        "Work",
        LabelOrigin::User,
    );
    let spaces = built(
        vec![("Mail".to_owned(), SpaceLook::default(), Mail::of(shown))],
        0,
    );
    let client = mail_client::Client::open(store.clone(), mail_client::ClientConfig::default());
    let mut dom = VirtualDom::new(App)
        .with_root_context(spaces)
        .with_root_context(client);
    dom.rebuild_in_place();
    (dom, store, dir)
}

/// An account removed by a commit no one tells the window of — the background watch's, or one
/// made through the store beside it — takes its label out of the sidebar.
#[tokio::test]
async fn an_account_removed_unannounced_takes_its_label_from_the_sidebar() {
    let (mut dom, store, _dir) = window(vec![account(1), account(2)]);
    until(&mut dom, "both labels drawn", |page| {
        drawn(page, "Travel") && drawn(page, "Work")
    })
    .await;

    store.remove_account(account(1)).unwrap();

    until(&mut dom, "Travel gone and Work kept", |page| {
        !drawn(page, "Travel") && drawn(page, "Work")
    })
    .await;
}

/// A Space whose accounts were all taken out draws no account's labels.
#[tokio::test]
async fn a_space_with_no_accounts_draws_no_labels() {
    let (mut dom, _store, _dir) = window(Vec::new());
    // Long enough for the client to have read the accounts, and the places to have their labels.
    until(&mut dom, "the places read", |page| {
        page.contains("data-place=\"Inbox\"")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    until(&mut dom, "no label drawn", |page| {
        !drawn(page, "Travel") && !drawn(page, "Work")
    })
    .await;
}

/// A Space of one account draws that account's label and not the other's.
#[tokio::test]
async fn a_space_of_one_account_draws_its_labels_only() {
    let (mut dom, _store, _dir) = window(vec![account(2)]);
    until(&mut dom, "Work drawn and Travel not", |page| {
        drawn(page, "Work") && !drawn(page, "Travel")
    })
    .await;
}
