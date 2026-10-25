//! Settings › Signatures over the reference fixture: a signature's page pushed from its row,
//! Delete Signature said in a toast whose Undo puts it back, and a change the store refuses said
//! under the words.

use super::super::tests::opened_on;
use super::row_id;
use crate::ui::copy;
use crate::ui::fixtures::{Seen, Work, click, drain_seen, settle_quiet, type_into, work};
use crate::ui::view::SettingsPage;
use dioxus::dioxus_core::VirtualDom;
use mail_core::signature;
use mail_domain::Signature;

/// A signature called "Work" on the fixture's first address that sends, made before the window
/// opens.
fn work_signature(built: &Work) -> Signature {
    let (identity, _) = signature::every(&built.store)
        .unwrap()
        .into_iter()
        .next()
        .expect("the fixture sends from an address");
    signature::create(&built.store, identity.id, "Work").unwrap()
}

/// The Settings window on Signatures, with `made`'s page pushed from its row.
fn pushed(built: &Work, made: &Signature) -> (VirtualDom, Seen) {
    let (mut dom, seen) = opened_on(built, SettingsPage::Signatures);
    let seen = click(&mut dom, seen.one("id", &row_id(made.id))).merge(drain_seen(&mut dom));
    (dom, seen)
}

/// Delete Signature deletes at once and says so in a toast that offers Undo, which puts the
/// signature back as it was.
#[tokio::test]
async fn a_deleted_signature_is_said_with_an_undo_that_puts_it_back() {
    let built = work();
    let made = work_signature(&built);
    let (mut dom, seen) = pushed(&built, &made);
    click(
        &mut dom,
        seen.one("aria-label", &copy::aria::delete_signature("Work")),
    );
    let _ = drain_seen(&mut dom);
    assert!(built.store.signature(made.id).is_err(), "not deleted");

    // The toast is quire's, laid out a frame after the push.
    let mut page = String::new();
    for _ in 0..8 {
        settle_quiet(&mut dom).await;
        page = dioxus_ssr::render(&dom);
        if page.contains("ds-toast-action") {
            break;
        }
    }
    assert!(
        page.contains(&copy::toasts::signature_deleted("Work"))
            && page.contains("class=\"ds-toast-action\""),
        "the delete put up no toast with an Undo:\n{page}"
    );

    // What the Undo puts back.
    crate::ui::motion::Restore::Signature(made.clone())
        .put_back(&built.store)
        .unwrap();
    assert_eq!(built.store.signature(made.id).unwrap(), made);
}

/// A name the store will not keep (the signature went from under the page) is said under the
/// words, rather than lost in silence.
#[tokio::test]
async fn a_name_the_store_refuses_is_said_on_the_page() {
    let built = work();
    let made = work_signature(&built);
    let (mut dom, seen) = pushed(&built, &made);
    assert!(!dioxus_ssr::render(&dom).contains(copy::errors::NOT_KEPT_HERE));

    built.store.delete_signature(made.id).unwrap();
    let name = seen.one("aria-placeholder", mail_domain::UNNAMED_SIGNATURE);
    type_into(&mut dom, name, "Home");
    let _ = drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains(copy::errors::NOT_KEPT_HERE), "{page}");
}
