//! Every button that draws only its icon says what it does on hover, as a Mac control's help
//! tag does. Under quire's `Ds` a titled button's tip is written as its `aria-description` (the
//! web's AXHelp: what a screen reader reads after the name), unless a `Tooltip` wraps it, whose
//! text is then the tip. A tip that equals the button's name is written as `data-tip` instead of
//! a description, which would be read twice.

use super::app::App;
use super::fixtures::{dispatching, drain_seen, key, realistic};
use dioxus::prelude::*;

/// An icon-only button's opening tag, and whether the element just before it is a quire
/// `Tooltip`'s wrapper.
struct IconButton<'a> {
    tag: &'a str,
    wrapped: bool,
}

/// The buttons in `page` that draw no words of their own.
fn icon_buttons(page: &str) -> Vec<IconButton<'_>> {
    let parts: Vec<&str> = page.split("<button").collect();
    parts
        .windows(2)
        .filter_map(|pair| {
            let (tag, _) = pair[1].split_once('>')?;
            let before = pair[0].rsplit('<').next().unwrap_or_default();
            Some(IconButton {
                tag,
                wrapped: before.contains(r#"class="ds-hover-target""#),
            })
        })
        .filter(|button| button.tag.contains(r#"data-image="only""#))
        .collect()
}

/// Whether `tag` is a button whose tip is its own name: quire leaves its `aria-description` out
/// (it would be read twice) and writes the tip as `data-tip` instead.
fn tip_is_name(tag: &str) -> bool {
    tag.split_once(r#"data-tip=""#)
        .and_then(|(_, rest)| rest.split_once('"'))
        .is_some_and(|(tip, _)| !tip.trim().is_empty())
}

/// The non-empty `aria-description` of `tag`, if it has one.
fn description(tag: &str) -> Option<&str> {
    let (_, rest) = tag.split_once(r#"aria-description=""#)?;
    let (text, _) = rest.split_once('"')?;
    Some(text).filter(|text| !text.trim().is_empty())
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
    assert!(
        buttons.len() >= 8,
        "too few icon buttons to be the window: {}",
        buttons.len()
    );
    let bare: Vec<&str> = buttons
        .iter()
        .filter(|button| {
            !button.wrapped && description(button.tag).is_none() && !tip_is_name(button.tag)
        })
        .map(|button| button.tag)
        .collect();
    assert!(
        bare.is_empty(),
        "icon buttons with no tip:\n{}",
        bare.join("\n")
    );
    // A Tooltip's own text is the tip there: the button under it says nothing more.
    let doubled: Vec<&str> = buttons
        .iter()
        .filter(|button| button.wrapped && description(button.tag).is_some())
        .map(|button| button.tag)
        .collect();
    assert!(
        doubled.is_empty(),
        "icon buttons with two tips:\n{}",
        doubled.join("\n")
    );
}
