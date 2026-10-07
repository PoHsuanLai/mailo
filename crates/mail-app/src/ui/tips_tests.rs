//! Every button that draws only its icon says what it does on hover: its `title`, which quire
//! draws as the tooltip, as a Mac control's help tag is.

use super::app::App;
use super::fixtures::{dispatching, drain_seen, key, realistic};
use dioxus::prelude::*;

/// The opening tags of the buttons in `page` that draw no words of their own.
fn icon_buttons(page: &str) -> Vec<&str> {
    page.split("<button")
        .skip(1)
        .filter_map(|rest| rest.split_once('>').map(|(tag, _)| tag))
        .filter(|tag| tag.contains(r#"data-image="only""#))
        .collect()
}

#[tokio::test]
async fn every_icon_button_in_the_window_has_a_tip() {
    dispatching();
    let (store, _dir) = realistic();
    let mut dom = VirtualDom::new(App).with_root_context(store);
    dom.rebuild_in_place();
    // A conversation open, so the reader's toolbar is drawn beside the list's.
    key(&mut dom, "j");
    drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    let buttons = icon_buttons(&page);
    assert!(buttons.len() >= 8, "too few icon buttons to be the window: {}", buttons.len());
    let bare: Vec<&str> = buttons
        .into_iter()
        .filter(|tag| !tag.contains(" title=\""))
        .collect();
    assert!(bare.is_empty(), "icon buttons with no tip:\n{}", bare.join("\n"));
}
