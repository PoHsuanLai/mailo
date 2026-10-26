//! Asking for a read receipt: Send ▾'s item, the row it shows while on, and the draft it saves.

use mail_core::Store;

use super::super::foot::pick_send;
use super::super::page::{Float, Saved};
use super::super::receipt::{ASK, KEY, OFF_KEY};
use super::*;
use crate::ui::fixtures::{acct_account, click, seeded};
use mail_core::SqliteStore;

fn fresh_draft(store: &SqliteStore) -> Draft {
    mail_core::compose::draft_new(store, acct_account(), &[], "", "", Utc::now())
        .unwrap_or_else(|why| panic!("a new draft: {why}"))
}

/// Let the page's autosave run until the stored draft asks as `want`, or give up.
async fn saved_as(window: &mut Window, store: &SqliteStore, draft: DraftId, want: ReceiptRequest) {
    for _ in 0..40 {
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            window.dom.wait_for_work(),
        )
        .await;
        window.dom.render_immediate(&mut NoOpMutations);
        let stored = store.draft(draft).unwrap_or_else(|why| panic!("{why}"));
        if stored.receipt == want {
            return;
        }
    }
    panic!("autosave never stored {want:?}");
}

#[test]
fn send_menu_item_turns_the_request_on_and_off() {
    let mut page = page_of("");
    assert_eq!(page.receipt, ReceiptRequest::Unrequested);
    page.float = Float::SendMenu;

    pick_send(&mut page, KEY);
    assert_eq!(page.receipt, ReceiptRequest::Requested);
    assert_eq!(page.saved, Saved::Dirty, "a change the autosave would miss");
    assert_eq!(page.float, Float::Closed);

    pick_send(&mut page, KEY);
    assert_eq!(page.receipt, ReceiptRequest::Unrequested);
    // A when is still a when: the receipt item does not shadow one.
    pick_send(&mut page, "later:tomorrow");
    assert_eq!(page.receipt, ReceiptRequest::Unrequested);
    assert_eq!(page.when, super::super::page::When::Tomorrow);
}

#[tokio::test]
async fn asking_is_autosaved_shown_as_a_row_and_can_be_set_back_to_off() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let (mut window, _seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    let markup = window.render();
    assert!(!markup.contains("data-row=\"receipt\""), "{markup}");

    let mut page = window.page();
    window
        .dom
        .in_runtime(|| page.write().float = Float::SendMenu);
    let markup = window.render();
    assert!(
        markup.contains(ASK),
        "Send ▾ has no receipt item:\n{markup}"
    );

    window.dom.in_runtime(|| pick_send(&mut page.write(), KEY));
    let markup = window.render();
    assert!(markup.contains("data-row=\"receipt\""), "{markup}");
    saved_as(&mut window, &store, draft.id, ReceiptRequest::Requested).await;

    // Opened again from the store, the page still asks, and the row's menu sets it Off.
    let stored = store.draft(draft.id).unwrap_or_else(|why| panic!("{why}"));
    let (mut window, seen) =
        Window::open(crate::ui::fixtures::client(&store.clone()), stored, None);
    let markup = window.render();
    assert!(markup.contains("data-row=\"receipt\""), "{markup}");
    click(
        &mut window.dom,
        seen.one("aria-label", "Receipt: Requested"),
    );
    let markup = window.render();
    assert!(
        markup.contains(">Off<"),
        "the row's menu has no Off:\n{markup}"
    );
    let mut page = window.page();
    window
        .dom
        .in_runtime(|| super::super::receipt::pick_row(&mut page.write(), OFF_KEY));
    let markup = window.render();
    assert!(!markup.contains("data-row=\"receipt\""), "{markup}");
    saved_as(&mut window, &store, draft.id, ReceiptRequest::Unrequested).await;
}
