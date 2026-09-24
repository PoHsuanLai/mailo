//! The notifications switch in the Space editor, against a temporary config directory only.

use crate::notify::{self, Setting};
use crate::ui::app::App;
use crate::ui::fixtures::{Seen, Work, click, dispatching, rebuild_into, work};
use crate::ui::sidebar::tests::{buttons_in, pressed};
use dioxus::dioxus_core::VirtualDom;

/// The window on the Work Space, with the editor open. `Work` owns the directories.
fn opened(built: &Work) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    let seen = click(&mut dom, seen.one("aria-label", "Edit the Work Space"));
    (dom, seen)
}

/// The Notifications group's pressed button, as the page draws it.
fn shown(page: &str) -> Vec<String> {
    const OPEN: &str = "class=\"seg\" role=\"group\" aria-label=\"Notifications\"";
    let at = page
        .find(OPEN)
        .unwrap_or_else(|| panic!("no Notifications switch in:\n{page}"));
    let tail = &page[at + OPEN.len()..];
    let body = &tail[..tail.find("</div>").unwrap_or(tail.len())];
    let buttons = buttons_in(&format!("<div class=\"seg\"{body}</div>"), "seg");
    pressed(&buttons, |button| button.text.as_str())
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn the_switch_reads_and_writes_the_setting_the_watch_reads() {
    let built = work();
    let config = &built.dirs.config;
    assert_eq!(notify::load(config), Setting::On, "on unless turned off");
    let (mut dom, seen) = opened(&built);
    assert_eq!(shown(&dioxus_ssr::render(&dom)), ["On"]);

    click(&mut dom, seen.one("data-v", "Off"));
    assert_eq!(notify::load(config), Setting::Off, "Off was not kept");
    let page = dioxus_ssr::render(&dom);
    assert_eq!(shown(&page), ["Off"]);
    assert!(
        page.contains("mailo watch keeps new mail to itself."),
        "{page}"
    );

    // The buttons are keyed, so the ids the first paint recorded still name them.
    click(&mut dom, seen.one("data-v", "On"));
    assert_eq!(notify::load(config), Setting::On, "On was not kept");
}

#[tokio::test]
async fn the_switch_opens_on_what_was_kept() {
    let built = work();
    notify::save(&built.dirs.config, Setting::Off).unwrap_or_else(|why| panic!("{why}"));
    let (dom, _) = opened(&built);
    assert_eq!(shown(&dioxus_ssr::render(&dom)), ["Off"]);
}
