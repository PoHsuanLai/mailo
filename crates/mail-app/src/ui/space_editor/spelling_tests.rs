//! The spelling switch in the Space editor, against a temporary config directory and a
//! temporary, empty dictionary directory only.

use crate::spelling::{self, Setting};
use crate::ui::app::App;
use crate::ui::compose::Dictionaries;
use crate::ui::fixtures::{Seen, Work, click, dispatching, rebuild_into, work};
use dioxus::dioxus_core::VirtualDom;
use ds_native::spell::SpellConfig;

/// The window on the Work Space, with the editor open, checking against `books`.
fn opened(built: &Work, books: &std::path::Path) -> (VirtualDom, Seen) {
    dispatching();
    let dictionaries = Dictionaries {
        config: SpellConfig {
            dictionaries: vec![books.to_path_buf()],
            user: books.join("learned"),
        },
        languages: vec![ds::Lang::parse("en_US").unwrap_or_else(|why| panic!("{why}"))],
    };
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone())
        .with_root_context(dictionaries);
    let seen = rebuild_into(&mut dom);
    let seen = click(&mut dom, seen.one("aria-label", "Edit the Work Space"));
    (dom, seen)
}

#[tokio::test]
async fn the_switch_keeps_the_setting_and_says_when_there_is_no_dictionary() {
    let built = work();
    let books = tempfile::tempdir().unwrap_or_else(|why| panic!("{why}"));
    let config = &built.dirs.config;
    assert_eq!(spelling::load(config), Setting::On, "on unless turned off");
    let (mut dom, seen) = opened(&built, books.path());
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("No spelling dictionaries were found"),
        "nothing says why nothing is marked:\n{page}"
    );

    let segments = seen.after("aria-label", "Check spelling", "aria-pressed");
    click(&mut dom, segments[1]);
    assert_eq!(spelling::load(config), Setting::Off, "Off was not kept");
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Messages are not checked for spelling."),
        "{page}"
    );
    assert!(
        !page.contains("No spelling dictionaries were found"),
        "a missing dictionary matters only while spelling is on"
    );

    click(&mut dom, segments[0]);
    assert_eq!(spelling::load(config), Setting::On, "On was not kept");
}

#[tokio::test]
async fn with_a_dictionary_for_the_language_nothing_is_missing() {
    let built = work();
    let books = tempfile::tempdir().unwrap_or_else(|why| panic!("{why}"));
    for file in ["en_US.aff", "en_US.dic"] {
        std::fs::write(books.path().join(file), "").unwrap_or_else(|why| panic!("{why}"));
    }
    let (dom, _) = opened(&built, books.path());
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Misspelt words in a message are underlined"),
        "{page}"
    );
    assert!(!page.contains("dictionar"), "{page}");
}
