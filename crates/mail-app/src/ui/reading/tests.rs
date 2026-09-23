use super::{Reader, initial};
use crate::ui::fixtures::{
    click, dispatching, held_and_remote, reader_markup, realistic, rebuild_into, thread_like,
};
use crate::view::Shell;
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_domain::ThreadId;

#[test]
fn the_avatar_is_the_first_character_not_the_first_byte() {
    // `校園` is three bytes per character. Indexing the bytes and casting would not be `校`.
    const CASES: &[(&str, &str)] = &[
        ("Ada", "A"),
        ("github", "G"),
        ("校園資訊網路中心", "校"),
        ("émail", "É"),
        ("", ""),
    ];
    let mut failures = Vec::new();
    for (name, expect) in CASES {
        let got = initial(name);
        if got != *expect {
            failures.push(format!("{name:?}: got {got:?}, want {expect:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn text_of(markup: &str, class: &str) -> String {
    let needle = format!(r#"class="{class}">"#);
    let Some((_, rest)) = markup.split_once(&needle) else {
        panic!("no {class} in:\n{markup}");
    };
    let end = rest.find('<').unwrap_or(rest.len());
    rest[..end].to_owned()
}

/// Nothing wrapped the iframe. A `<div` between the article and the frame is a new parent,
/// and a new parent reloads the document.
fn no_div_between_article_and_iframe(markup: &str) {
    let Some(start) = markup.find("<article") else {
        panic!("no article:\n{markup}");
    };
    let Some(rel) = markup[start..].find("<iframe") else {
        panic!("no iframe:\n{markup}");
    };
    let between = &markup[start..start + rel];
    assert!(
        !between.contains("<div"),
        "a div between article and its iframe reparents the frame:\n{between}"
    );
}

#[tokio::test]
async fn the_reader_does_not_offer_to_load_images_a_message_does_not_have() {
    // The offer sat above every conversation in the mailbox — plain text included — because
    // nothing asked whether anything had been blocked. A control that is always on screen is
    // furniture, and this one asks the user to make network requests on a sender's behalf.
    let (store, _dir) = realistic();
    let thread = thread_like(&store, "rust-lang");
    let markup = reader_markup(store, thread);

    assert!(
        markup.contains("Tracking issue"),
        "the body is there: {markup}"
    );
    assert!(
        !markup.contains("Load remote images"),
        "offered to load images for a message that has none:\n{markup}"
    );
    assert!(
        !markup.contains("Show images"),
        "offered to load images for a message that has none:\n{markup}"
    );
    assert!(
        !markup.contains("Remote images blocked"),
        "a consent bar for a message with nothing remote:\n{markup}"
    );
    assert_eq!(text_of(&markup, "reader-av"), "G", "{markup}");
    assert_eq!(text_of(&markup, "reader-from"), "GitHub", "{markup}");
    assert!(
        markup.contains("sandboxed frame · no scripts, no same-origin"),
        "an HTML message did not say it was sandboxed:\n{markup}"
    );
    assert!(
        markup.contains("sandbox=\"\""),
        "the frame is not sandboxed:\n{markup}"
    );
    no_div_between_article_and_iframe(&markup);
}

#[tokio::test]
async fn the_reader_offers_to_load_images_when_it_blocked_some() {
    // And the other direction, so the gate is not simply "never".
    let (store, _dir) = realistic();
    let thread = thread_like(&store, "receipt");
    let markup = reader_markup(store, thread);

    assert!(
        markup.contains("Show images"),
        "a tracking pixel was blocked and nothing said so:\n{markup}"
    );
    assert!(
        markup.contains("Remote images blocked"),
        "the offer does not say what agreeing does:\n{markup}"
    );
    assert!(
        !markup.contains("track.stripe.test"),
        "the blocked URL reached the document:\n{markup}"
    );
    no_div_between_article_and_iframe(&markup);
}

#[tokio::test]
async fn a_cjk_sender_is_named_by_its_first_character() {
    let (store, _dir) = realistic();
    let thread = thread_like(&store, "校園");
    let markup = reader_markup(store, thread);
    assert_eq!(
        text_of(&markup, "reader-av"),
        "校",
        "the avatar took a byte, not a character:\n{markup}"
    );
}

#[component]
fn Open(thread: ThreadId) -> Element {
    let shell = use_signal(Shell::default);
    rsx! { Reader { thread, shell } }
}

fn consent_buttons(markup: &str) -> usize {
    let Some(at) = markup.find(r#"class="consent""#) else {
        panic!("no consent bar:\n{markup}");
    };
    let rest = &markup[at..];
    let end = rest.find("<article").unwrap_or(rest.len());
    rest[..end].matches("<button").count()
}

#[tokio::test]
async fn showing_images_removes_the_consent_button() {
    // Counted before and after the click. The bar stays, so "no button" is 0, not an
    // empty page, and the blocked URL is what the click is allowed to reveal.
    dispatching();
    let (store, _dir) = realistic();
    let thread = thread_like(&store, "receipt");
    let mut dom = VirtualDom::new_with_props(Open, OpenProps { thread }).with_root_context(store);
    let seen = rebuild_into(&mut dom);
    let before = dioxus_ssr::render(&dom);
    assert_eq!(
        consent_buttons(&before),
        1,
        "the blocked bar should have one Show images button:\n{before}"
    );
    assert!(
        !before.contains("track.stripe.test"),
        "the URL was on the page before consent:\n{before}"
    );

    let id = seen.one("aria-label", "Show images");
    click(&mut dom, id);
    let after = dioxus_ssr::render(&dom);
    assert_eq!(
        consent_buttons(&after),
        0,
        "consent left the Show images button in place:\n{after}"
    );
    assert!(
        after.contains("Showing remote images from stripe.test"),
        "the bar did not say whose images are loading:\n{after}"
    );
    assert!(
        after.contains("track.stripe.test"),
        "consent did not let the image through:\n{after}"
    );
}

/// The `<li>` elements of the attachment list, each as rendered.
///
/// Taken from that list alone. A `contains("Download")` over the whole page would pass for
/// any screen that mentioned the word.
fn attachment_items(markup: &str) -> Vec<&str> {
    let Some((_, rest)) = markup.split_once(r#"<ul class="attachments">"#) else {
        panic!("the reader drew no attachment list:\n{markup}");
    };
    let Some((list, _)) = rest.split_once("</ul>") else {
        panic!("the attachment list was not closed:\n{markup}");
    };
    let mut items = Vec::new();
    let mut rest = list;
    while let Some(at) = rest.find("<li>") {
        let from = &rest[at..];
        let Some(close) = from.find("</li>") else {
            panic!("an attachment row was not closed:\n{markup}");
        };
        items.push(&from[..close + "</li>".len()]);
        rest = &from[close + "</li>".len()..];
    }
    items
}

fn row_has(item: &str, name: &str, size: &str, action: &str) -> Result<(), String> {
    let name_cell = format!(r#"class="name">{name}</span>"#);
    let size_cell = format!(r#"class="size mono">{size}</span>"#);
    let button = format!(">{action}</button>");
    let mut missing = Vec::new();
    for needle in [
        name_cell.as_str(),
        size_cell.as_str(),
        button.as_str(),
        "m16 6-8.41",
    ] {
        if !item.contains(needle) {
            missing.push(needle.to_owned());
        }
    }
    if item.contains('📎') {
        missing.push("emoji paperclip".to_owned());
    }
    if item.matches("<button").count() != 1 {
        missing.push(format!("{} buttons", item.matches("<button").count()));
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("missing {} in {item}", missing.join(", ")))
    }
}

#[tokio::test]
async fn a_held_part_offers_save_and_a_remote_part_offers_download() {
    let (store, _dir) = held_and_remote();
    let thread = thread_like(&store, "quarterly");
    let markup = reader_markup(store, thread);
    let items = attachment_items(&markup);
    assert_eq!(items.len(), 2, "expected two rows:\n{markup}");
    let mut failures = Vec::new();
    for (item, name, size, action) in [
        (items[0], "notes.txt", "1.5 kB", "Save"),
        (items[1], "report.pdf", "up to 5.0 MB", "Download"),
    ] {
        if let Err(why) = row_has(item, name, size, action) {
            failures.push(why);
        }
    }
    assert!(
        failures.is_empty(),
        "the rows are not Save for the held part and Download for the remote one:\n{}",
        failures.join("\n")
    );
    assert!(
        !markup.contains("frame-note"),
        "a plain-text message claimed a sandboxed frame:\n{markup}"
    );
}
