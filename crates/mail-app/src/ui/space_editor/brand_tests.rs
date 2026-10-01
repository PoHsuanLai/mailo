//! The brand logo switch in the Space editor, against a temporary config directory only.

use crate::bimi::{self, Setting};
use crate::ui::app::App;
use crate::ui::fixtures::{Seen, Work, click, dispatching, drain_seen, rebuild_into, work};
use dioxus::dioxus_core::VirtualDom;

/// The window on the Work Space, with the editor open.
fn opened(built: &Work) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    // The sheet is drawn by the render after the click that asked for it.
    let seen =
        click(&mut dom, seen.one("aria-label", "Edit the Work Space")).merge(drain_seen(&mut dom));
    (dom, seen)
}

#[tokio::test]
async fn the_switch_is_off_until_turned_on_and_keeps_the_setting() {
    let built = work();
    let config = &built.dirs.config;
    assert_eq!(bimi::load(config), Setting::Off, "off unless turned on");
    let (mut dom, seen) = opened(&built);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Off: no logo is looked up or fetched."),
        "{page}"
    );

    let segments = seen.after("aria-label", "Show brand logos (BIMI)", "aria-checked");
    click(&mut dom, segments[0]);
    assert_eq!(bimi::load(config), Setting::On, "On was not kept");
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("mark certificate vouches for the logo"),
        "{page}"
    );
    assert!(
        page.contains("No mark verifying authority"),
        "with no roots installed, nothing says why no logo shows:\n{page}"
    );

    click(&mut dom, segments[1]);
    assert_eq!(bimi::load(config), Setting::Off, "Off was not kept");
}
