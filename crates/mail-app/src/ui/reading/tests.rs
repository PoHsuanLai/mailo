use super::{Reader, initial};
use crate::ui::fixtures::{
    click, dispatching, held_and_remote, reader_markup, realistic, rebuild_into, seeded,
    thread_like,
};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

#[test]
fn the_avatar_is_the_first_character_not_the_first_byte() {
    // `校園` is three bytes per character. Indexing the bytes and casting would not be `校`.
    const CASES: &[(&str, char)] = &[
        ("Ada", 'A'),
        ("github", 'G'),
        ("校園資訊網路中心", '校'),
        ("émail", 'É'),
        ("", '?'),
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
    // The avatar is quire's `span.ds-avatar`, the sender's name its first headline `Label`.
    let needles: Vec<String> = match class {
        // Quire's avatar, or the letter the brand logo falls back to.
        "reader-av" => vec![
            r#"class="ds-avatar""#.to_owned(),
            r#"class="reader-av""#.to_owned(),
        ],
        "reader-from" => vec![
            r#"data-style="headline""#.to_owned(),
            r#"class="reader-from""#.to_owned(),
        ],
        other => vec![format!(r#"class="{other}""#)],
    };
    let Some((_, rest)) = needles.iter().find_map(|needle| markup.split_once(needle)) else {
        panic!("no {class} in:\n{markup}");
    };
    let rest = rest.split_once('>').map_or(rest, |(_, after)| after);
    let end = rest.find('<').unwrap_or(rest.len());
    rest[..end].to_owned()
}

/// Nothing wrapped the iframe. A `<div` between the article and the frame is a new parent,
/// and a new parent reloads the document.
fn no_div_between_article_and_iframe(markup: &str) {
    let Some(start) = markup.find("<article") else {
        panic!("no article:\n{markup}");
    };
    // The head holds the Reader / Original switch, which is quire's `div`: the frame's parent is
    // what must not change, so what is measured is what follows the head.
    let start = start + markup[start..].find("</header>").unwrap_or(0);
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
    rsx! { ds::prelude::Ds { appearance: ds::prelude::Appearance::default(), material: ds::prelude::Material::Window, Reader { thread, shell } } }
}

fn consent_buttons(markup: &str) -> usize {
    let Some(at) = markup.find(r#"class="ds-inline-banner""#) else {
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
    let Some((_, rest)) = markup.split_once(r#"class="attachments""#) else {
        panic!("the reader drew no attachment list:\n{markup}");
    };
    // A quire `List` of `Row`s: one `ds-list-item` each, the list ending where the next
    // section of the reader starts.
    let list = rest.split("</article>").next().unwrap_or(rest);
    let mut items: Vec<&str> = list.split(r#"<div class="ds-list-item""#).skip(1).collect();
    // What follows the last row is the rest of the message, not part of it.
    if let Some(last) = items.last_mut() {
        *last = last.split("<iframe").next().unwrap_or(last);
    }
    items
}

fn row_has(item: &str, name: &str, size: &str, action: &str) -> Result<(), String> {
    let name_cell = format!(r#"class="ds-row-title ds-truncate">{name}</b>"#);
    let size_cell = format!(r#"class="ds-row-detail ds-truncate">{size}</small>"#);
    // quire's Button sets its label in a span.
    let button = format!(">{action}</span></button>");
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

pub(super) fn html_message(subject: &str, html: &str) -> Vec<u8> {
    format!(
        "From: Ada <ada@example.test>\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n"
    )
    .into_bytes()
}

pub(super) fn text_message(subject: &str, content_type: &str, body: &str) -> Vec<u8> {
    format!(
        "From: Ada <ada@example.test>\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\n\
         Content-Type: {content_type}\r\n\r\n{body}"
    )
    .into_bytes()
}

/// One thread whose messages are `parts`: `(subject, raw)`.
///
/// The directory is part of the return so the store's files outlive the call.
pub(super) fn thread_of(
    parts: &[(&str, Vec<u8>)],
) -> (Arc<SqliteStore>, ThreadId, tempfile::TempDir) {
    let (store, dir) = seeded();
    let thread = add_thread(&store, "body", parts);
    (store, thread, dir)
}

/// A new thread of `parts` in `store`, its messages keyed `{tag}{index}@example.test`.
pub(super) fn add_thread(store: &SqliteStore, tag: &str, parts: &[(&str, Vec<u8>)]) -> ThreadId {
    let thread = ThreadId::generate();
    add_to(store, thread, tag, parts);
    thread
}

/// `parts` arriving in `thread`, keyed `{tag}{index}@example.test`: a tag no other call used.
pub(super) fn add_to(store: &SqliteStore, thread: ThreadId, tag: &str, parts: &[(&str, Vec<u8>)]) {
    // `body{index}`, not `m{index}`: the seeded store already holds `m1@example.test`,
    // and a second message under that key is the same message to the store.
    let mut fetched = Vec::new();
    for (index, (subject, bytes)) in parts.iter().enumerate() {
        let raw = store.blobs().put(&store.connection(), bytes).unwrap();
        let message = Message {
            id: MessageId::generate(),
            thread,
            account: crate::ui::fixtures::ACCOUNT,
            key: MessageKey::Rfc(format!("{tag}{index}@example.test")),
            date: chrono::Utc::now() - chrono::TimeDelta::try_hours(index as i64).unwrap(),
            from: Address {
                name: Some("Ada".to_owned()),
                email: "ada@example.test".to_owned(),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: (*subject).to_owned(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some(format!("{tag}{index}@example.test")),
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            body: Body::Present {
                text: Some((*subject).to_owned()),
                raw,
            },
            attachments: vec![],
        };
        fetched.push(Fetched {
            remote: RemoteRef::Pop {
                uidl: format!("{tag}-u{index}"),
            },
            key: message.key.clone(),
            raw,
            message,
        });
    }
    store
        .ingest(
            crate::ui::fixtures::ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: crate::ui::fixtures::ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: Some(SyncCursor::Pop),
                messages: fetched,
                flags: vec![],
                labels: vec![],
                label_names: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
}

pub(super) fn iframe_srcdoc(page: &str) -> String {
    let Some(at) = page.find("<iframe") else {
        panic!("no iframe:\n{page}");
    };
    let tag = &page[at..];
    let Some(end) = tag.find('>') else {
        panic!("iframe tag was not closed:\n{tag}");
    };
    let open = &tag[..end];
    let key = "srcdoc=\"";
    let Some(start) = open.find(key) else {
        panic!("iframe has no srcdoc: {open}");
    };
    open[start + key.len()..]
        .split('"')
        .next()
        .unwrap_or("")
        .to_owned()
}

#[tokio::test]
async fn the_frame_renders_html_body_in_sandboxed_iframe() {
    let html = "<p>Hello <b>world</b></p><script>alert('xss')</script>";
    let (store, thread, _dir) = thread_of(&[("html test", html_message("html test", html))]);
    let markup = reader_markup(store, thread);
    assert!(
        markup.contains("<iframe"),
        "no iframe in reader markup:\n{markup}"
    );
    assert!(
        markup.contains("sandbox=\"\""),
        "iframe is not sandboxed:\n{markup}"
    );
    let srcdoc = iframe_srcdoc(&markup);
    assert!(
        srcdoc.contains("Hello"),
        "body missing from srcdoc: {srcdoc}"
    );
    assert!(
        !srcdoc.contains("alert"),
        "script was not sanitized: {srcdoc}"
    );
}

#[tokio::test]
async fn plain_text_body_is_rendered_in_sandboxed_iframe() {
    let text =
        "Hello Ada,\n\nThis is a plain-text email with special characters: <>&.\n\nBest,\nSam";
    let (store, thread, _dir) = thread_of(&[(
        "plain test",
        text_message("plain test", "text/plain; charset=utf-8", text),
    )]);
    let markup = reader_markup(store, thread);
    assert!(
        markup.contains("<iframe"),
        "no iframe in reader markup:\n{markup}"
    );
    assert!(
        markup.contains("sandbox=\"\""),
        "iframe is not sandboxed:\n{markup}"
    );
    let srcdoc = iframe_srcdoc(&markup);
    assert!(
        srcdoc.contains("Hello Ada"),
        "text missing from srcdoc: {srcdoc}"
    );
    assert!(
        srcdoc.contains("&#38;lt;&#38;gt;&#38;amp;"),
        "special characters not escaped: {srcdoc}"
    );
    assert!(
        srcdoc.contains("color-scheme: light dark"),
        "color scheme styling missing: {srcdoc}"
    );
}

#[tokio::test]
async fn blocked_remote_images_trigger_consent_banner() {
    let html = "<p>Look at this:</p><img src=\"https://remote.example/pixel.png\">";
    let (store, thread, _dir) = thread_of(&[("tracker test", html_message("tracker test", html))]);
    let markup = reader_markup(store, thread);
    assert!(
        markup.contains("Remote images blocked"),
        "banner missing:\n{markup}"
    );
    assert!(
        markup.contains("Show images"),
        "show images button missing:\n{markup}"
    );
    let srcdoc = iframe_srcdoc(&markup);
    assert!(
        !srcdoc.contains("remote.example/pixel.png"),
        "remote image was not blocked in srcdoc:\n{srcdoc}"
    );
}
