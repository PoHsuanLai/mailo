//! The "Search the server automatically" switch in the Space editor, against a temporary config
//! directory only.

use crate::server_search::{self, Automatic};
use crate::ui::app::App;
use crate::ui::fixtures::{Seen, Work, click, dispatching, rebuild_into, work};
use dioxus::dioxus_core::VirtualDom;

fn opened(built: &Work) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    let seen = click(&mut dom, seen.one("aria-label", "Edit the Work Space"));
    (dom, seen)
}

#[tokio::test]
async fn the_switch_is_off_until_turned_on_and_keeps_the_setting() {
    let built = work();
    let config = &built.dirs.config;
    assert_eq!(
        server_search::load(config),
        Automatic::Off,
        "off unless turned on"
    );
    let (mut dom, seen) = opened(&built);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("a search&#39;s list ends with a button")
            || page.contains("a search's list ends with a button"),
        "{page}"
    );

    let segments = seen.after(
        "aria-label",
        "Search the server automatically",
        "aria-pressed",
    );
    click(&mut dom, segments[0]);
    assert_eq!(
        server_search::load(config),
        Automatic::On,
        "On was not kept"
    );
    click(&mut dom, segments[1]);
    assert_eq!(
        server_search::load(config),
        Automatic::Off,
        "Off was not kept"
    );
}
