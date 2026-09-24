//! Asking for a read receipt: the Sends menu's item, the row it shows, and the draft it saves.

use mail_store::Store;

use super::super::page::{Float, Saved};
use super::super::props::pick_sends;
use super::super::receipt::{ASK, KEY};
use super::*;
use crate::ui::fixtures::{ACCOUNT, click, seeded};

fn fresh_draft(store: &SqliteStore) -> Draft {
    crate::compose::draft_new(store, ACCOUNT, &[], "", "", Utc::now())
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
fn the_sends_menu_item_turns_the_request_on_and_off() {
    let mut page = page_of("");
    assert_eq!(page.receipt, ReceiptRequest::Unrequested);
    page.float = Float::Sends;

    pick_sends(&mut page, KEY);
    assert_eq!(page.receipt, ReceiptRequest::Requested);
    assert_eq!(page.saved, Saved::Dirty, "a change the autosave would miss");
    assert_eq!(page.float, Float::Closed);

    pick_sends(&mut page, KEY);
    assert_eq!(page.receipt, ReceiptRequest::Unrequested);
    // A when is still a when: the receipt item does not shadow one.
    pick_sends(&mut page, "tomorrow");
    assert_eq!(page.receipt, ReceiptRequest::Unrequested);
    assert_eq!(page.when, super::super::page::When::Tomorrow);
}

#[test]
fn the_draft_carries_the_request_both_ways() {
    let mut draft = draft_of("");
    draft.receipt = ReceiptRequest::Requested;
    let page = Page::of(&draft, Vec::new(), Vec::new());
    assert_eq!(page.receipt, ReceiptRequest::Requested);
    let mut off = page.clone();
    off.toggle_receipt();
    assert_eq!(
        off.apply_to(&draft, at(1)).receipt,
        ReceiptRequest::Unrequested
    );
}

#[tokio::test]
async fn asking_is_autosaved_shown_as_a_row_and_can_be_taken_back() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let (mut window, _seen) = Window::open(store.clone(), draft.clone(), None);
    let markup = window.render();
    assert!(!markup.contains("data-row=\"receipt\""), "{markup}");

    let mut page = window.page();
    window.dom.in_runtime(|| page.write().float = Float::Sends);
    let markup = window.render();
    assert!(
        markup.contains(ASK),
        "the Sends menu has no receipt item:\n{markup}"
    );

    window.dom.in_runtime(|| pick_sends(&mut page.write(), KEY));
    let markup = window.render();
    assert!(markup.contains("data-row=\"receipt\""), "{markup}");
    assert!(markup.contains("Asks for a read receipt"), "{markup}");
    saved_as(&mut window, &store, draft.id, ReceiptRequest::Requested).await;

    // Opened again from the store, the page still asks, and × stops it.
    let stored = store.draft(draft.id).unwrap_or_else(|why| panic!("{why}"));
    let (mut window, seen) = Window::open(store.clone(), stored, None);
    let markup = window.render();
    assert!(markup.contains("data-row=\"receipt\""), "{markup}");
    click(
        &mut window.dom,
        seen.one("aria-label", "Stop asking for a read receipt"),
    );
    let markup = window.render();
    assert!(!markup.contains("data-row=\"receipt\""), "{markup}");
    saved_as(&mut window, &store, draft.id, ReceiptRequest::Unrequested).await;
}
