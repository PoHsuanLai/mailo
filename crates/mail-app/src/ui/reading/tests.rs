use super::blocks::{MessageView, iframe_mounts, reset_iframe_mounts};
use super::{Reader, initial};
use crate::ui::fixtures::{
    click, dispatching, held_and_remote, reader_markup, realistic, rebuild_into, seeded,
    thread_like,
};
use crate::view::{Reading, Shell};
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_domain::*;
use mail_mime::{RemoteImages, SanitizePolicy};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

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
    // `body{index}`, not `m{index}`: the seeded store already holds `m1@example.test`,
    // and a second message under that key is the same message to the store.
    let (store, dir) = seeded();
    let thread = ThreadId::generate();
    let mut fetched = Vec::new();
    for (index, (subject, bytes)) in parts.iter().enumerate() {
        let raw = store.blobs().put(&store.connection(), bytes).unwrap();
        let message = Message {
            id: MessageId::generate(),
            thread,
            account: crate::ui::fixtures::ACCOUNT,
            key: MessageKey::Rfc(format!("body{index}@example.test")),
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
            rfc_message_id: Some(format!("body{index}@example.test")),
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
                uidl: format!("u{index}"),
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
    (store, thread, dir)
}

fn blocks_of(raw: &str, images: RemoteImages) -> mail_mime::Document {
    let safe = mail_mime::sanitize(
        raw,
        SanitizePolicy {
            remote_images: RemoteImages::Allowed,
            version: SanitizePolicy::CURRENT.version,
        },
    );
    mail_mime::from_html(&safe, &[], images)
}

#[component]
fn ShowBlocks(document: mail_mime::Document) -> Element {
    let reading = Reading::Blocks {
        document,
        blocked_remote: false,
        html: None,
        fetches: Vec::new(),
    };
    let original = use_signal(std::collections::HashMap::new);
    let quotes = use_signal(super::blocks::OpenQuotes::new);
    let shell = use_signal(Shell::default);
    let message_id = use_hook(MessageId::generate);
    rsx! {
        MessageView { message_id, reading, original, quotes, shell }
    }
}

fn rendered_blocks(document: mail_mime::Document) -> String {
    let mut dom = VirtualDom::new_with_props(ShowBlocks, ShowBlocksProps { document });
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom)
}

/// A one-line-per-block sketch of rendered markup.
///
/// Text inside `span`/`strong`/`em` belongs to the block that contains it.
/// A block that contains another block emits its own line, then the inner ones.
fn sketch_markup(html: &str) -> String {
    let nodes = elements(html);
    let mut out = String::new();
    sketch_nodes(&nodes, &mut out);
    out
}

#[derive(Clone)]
enum Piece {
    Text(String),
    Node(Node),
}

#[derive(Clone)]
struct Node {
    label: String,
    interesting: bool,
    pieces: Vec<Piece>,
}

fn elements(html: &str) -> Vec<Node> {
    let mut stack: Vec<Node> = vec![Node {
        label: String::new(),
        interesting: false,
        pieces: Vec::new(),
    }];
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        let text = &rest[..start];
        if let Some(top) = stack.last_mut() {
            let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !flat.is_empty() {
                top.pieces.push(Piece::Text(flat));
            }
        }
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else { break };
        let tag = &after[..end];
        rest = &after[end + 1..];
        if tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.split_whitespace().next().unwrap_or(name);
            if let Some(done) = stack.pop() {
                if stack.is_empty() {
                    stack.push(done);
                    break;
                }
                let _ = name;
                if let Some(parent) = stack.last_mut() {
                    parent.pieces.push(Piece::Node(done));
                }
            }
            continue;
        }
        let name = tag.split_whitespace().next().unwrap_or("");
        let class = tag
            .split("class=\"")
            .nth(1)
            .and_then(|value| value.split('"').next())
            .unwrap_or("");
        let interesting_class = class.split_whitespace().find(|token| {
            token.starts_with("b-") || *token == "num" || *token == "lang" || *token == "fold"
        });
        let label = interesting_class
            .unwrap_or(
                if matches!(name, "li" | "th" | "td" | "dt" | "dd" | "img" | "hr") {
                    name
                } else {
                    ""
                },
            )
            .to_owned();
        let void = tag.ends_with('/') || matches!(name, "img" | "hr" | "br");
        let node = Node {
            interesting: !label.is_empty(),
            label,
            pieces: Vec::new(),
        };
        if void {
            if let Some(parent) = stack.last_mut() {
                parent.pieces.push(Piece::Node(node));
            }
        } else {
            stack.push(node);
        }
    }
    while stack.len() > 1 {
        if let Some(done) = stack.pop()
            && let Some(parent) = stack.last_mut()
        {
            parent.pieces.push(Piece::Node(done));
        }
    }
    stack
        .pop()
        .map(|root| pieces_nodes(&root.pieces))
        .unwrap_or_default()
}

fn pieces_nodes(pieces: &[Piece]) -> Vec<Node> {
    pieces
        .iter()
        .filter_map(|piece| match piece {
            Piece::Node(node) => Some(node.clone()),
            Piece::Text(_) => None,
        })
        .collect()
}

fn sketch_nodes(nodes: &[Node], out: &mut String) {
    for node in nodes {
        if node.interesting {
            let text = own_text(node);
            if text.is_empty() {
                out.push_str(&node.label);
                out.push('\n');
            } else {
                out.push_str(&node.label);
                out.push(' ');
                out.push_str(&text);
                out.push('\n');
            }
        }
        sketch_nodes(&pieces_nodes(&node.pieces), out);
    }
}

fn own_text(node: &Node) -> String {
    let mut out = String::new();
    for piece in &node.pieces {
        let inner = match piece {
            Piece::Text(text) => text.clone(),
            Piece::Node(child) if !child.interesting => own_text(child),
            Piece::Node(_) => continue,
        };
        if inner.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&inner);
    }
    out
}

#[test]
fn every_block_renders_to_the_same_sketch() {
    // One case per variant the renderer has a shape for. The sketch is the
    // markup, not a second copy of the block tree.
    let cases: &[(&str, &str, &str)] = &[
        ("heading", "<h1>Title</h1>", "b-h1 Title\n"),
        (
            "paragraph",
            "<p>Hello <b>there</b></p>",
            "b-p Hello there\n",
        ),
        ("list", "<ul><li>One</li></ul>", "b-list\nli\nb-p One\n"),
        (
            "quote",
            "<p>Ada wrote:</p><blockquote><p>Inside</p></blockquote>",
            "b-quote Ada wrote:\nb-p Inside\n",
        ),
        (
            "code",
            "<pre lang=\"rust\">let x = 1;</pre>",
            "b-code let x = 1;\nlang rust\n",
        ),
        (
            "table",
            "<table><thead><tr><th>Name</th><th>Qty</th></tr></thead>\
             <tbody><tr><td>Ada</td><td>2</td></tr><tr><td>Bea</td><td>5</td></tr></tbody></table>",
            "b-table\nth Name\nth Qty\ntd Ada\nnum 2\ntd Bea\nnum 5\n",
        ),
        (
            "image",
            "<img src=\"https://pixels.example/banner.png\" alt=\"logo\" width=\"60\" height=\"40\">",
            "b-img Image from pixels.example — load images\n",
        ),
        (
            "button",
            "<p><a href=\"https://news.example/issue\">Read the issue</a></p>",
            "b-cta Read the issue\n",
        ),
        ("rule", "<hr>", "b-rule\n"),
    ];
    let mut failures = Vec::new();
    for (name, raw, expect) in cases {
        let document = blocks_of(raw, RemoteImages::Blocked);
        let markup = rendered_blocks(document);
        let got = sketch_markup(&markup);
        if got != *expect {
            failures.push(format!("{name}:\nGOT\n{got}WANT\n{expect}MARKUP\n{markup}"));
        }
    }

    let signed = mail_mime::from_text("Thanks.\n\n-- \nSam\n", mail_mime::Flowed::Fixed);
    let markup = rendered_blocks(signed);
    let got = sketch_markup(&markup);
    let expect = "b-p Thanks.\nb-sig\nb-p Sam\n";
    if got != expect {
        failures.push(format!(
            "signature:\nGOT\n{got}WANT\n{expect}MARKUP\n{markup}"
        ));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n---\n"));
}

#[tokio::test]
async fn a_flowed_reply_is_paragraphs_a_folded_quote_and_a_signature() {
    let body = "\
Thanks for the note. \r\n\
It can wrap.\r\n\
\r\n\
On Monday Ada wrote:\r\n\
> The list jumps when mail arrives.\r\n\
>\r\n\
> On Sunday Bea wrote:\r\n\
> > Can you look at page two?\r\n\
\r\n\
-- \r\n\
Sam\r\n\
sam@example.test\r\n";
    let (store, thread, _dir) = thread_of(&[(
        "re: cursors",
        text_message(
            "re: cursors",
            "text/plain; charset=utf-8; format=flowed",
            body,
        ),
    )]);
    let markup = reader_markup(store, thread);
    assert!(
        markup.contains("Thanks for the note."),
        "the reply paragraph is missing:\n{markup}"
    );
    assert!(
        markup.contains("earlier message"),
        "the quoted chain was not folded:\n{markup}"
    );
    assert!(
        markup.contains("b-sig"),
        "the signature was not demoted:\n{markup}"
    );
    assert!(
        !markup.contains("<pre"),
        "plain text was still a pre:\n{markup}"
    );
}

#[tokio::test]
async fn a_justified_newsletter_stays_text() {
    let body = include_str!("../../../../mail-mime/tests/fixtures/block/justified.txt");
    let (store, thread, _dir) = thread_of(&[(
        "the weekly",
        text_message(
            "the weekly",
            "text/plain; charset=utf-8; format=fixed",
            body,
        ),
    )]);
    let markup = reader_markup(store, thread);
    assert!(markup.contains("weekly note"), "{markup}");
    assert!(
        !markup.contains("b-code"),
        "justified prose became code:\n{markup}"
    );
    assert!(!markup.contains("<pre"), "it was still a pre:\n{markup}");
}

#[tokio::test]
async fn the_receipt_renders_the_button_before_the_facts() {
    let body = include_str!("../../../../mail-mime/tests/fixtures/block/receipt.html");
    let (store, thread, _dir) = thread_of(&[("order", html_message("order", body))]);
    let markup = reader_markup(store, thread);
    let blocks = markup
        .split_once("class=\"blocks\"")
        .map(|(_, rest)| rest)
        .unwrap_or_else(|| panic!("no blocks:\n{markup}"));
    let button = blocks
        .find("TRACK YOUR PARCEL")
        .unwrap_or_else(|| panic!("no button:\n{blocks}"));
    let facts = blocks
        .find("Order number")
        .unwrap_or_else(|| panic!("no facts:\n{blocks}"));
    assert!(
        button < facts,
        "the button did not come before the facts:\n{markup}"
    );
    assert!(markup.contains("b-receipt"), "{markup}");
    assert!(markup.contains("b-kv"), "{markup}");
}

#[tokio::test]
async fn the_original_tab_does_not_remount_the_iframe() {
    // Same technique as `changing_the_peek_does_not_remount_the_reader`: the
    // mount count and the srcdoc, taken before the click and after it.
    dispatching();
    reset_iframe_mounts();
    let body = include_str!("../../../../mail-mime/tests/fixtures/block/newsletter.html");
    let (store, thread, _dir) = thread_of(&[("issue", html_message("issue", body))]);
    #[component]
    fn Open(thread: ThreadId) -> Element {
        let shell = use_signal(Shell::default);
        rsx! { Reader { thread, shell } }
    }
    let shape = {
        let message = store
            .message(store.thread(thread).unwrap().messages[0])
            .unwrap();
        crate::reader::render(&store, &message, SanitizePolicy::CURRENT)
            .document()
            .map(|document| format!("{:?}", document.shape))
            .unwrap_or_else(|| "none".to_owned())
    };
    let mut dom = VirtualDom::new_with_props(Open, OpenProps { thread }).with_root_context(store);
    let seen = rebuild_into(&mut dom);
    let before = dioxus_ssr::render(&dom);
    assert!(
        before.contains("aria-label=\"Original\""),
        "a laid-out message offered no Original tab (shape {shape}):\n{before}"
    );
    assert!(
        before.contains("class=\"html is-hidden\""),
        "the frame was not mounted while the blocks were showing:\n{before}"
    );
    let srcdoc = iframe_srcdoc(&before);
    let mounted = iframe_mounts();
    assert!(mounted >= 1, "the frame never mounted");

    let id = seen.one("aria-label", "Original");
    click(&mut dom, id);
    let after = dioxus_ssr::render(&dom);
    assert_eq!(
        iframe_mounts(),
        mounted,
        "Original remounted the frame: {mounted} before, {} after",
        iframe_mounts()
    );
    assert_eq!(
        iframe_srcdoc(&after),
        srcdoc,
        "Original replaced the iframe's document"
    );
    assert!(
        after.contains("class=\"blocks is-hidden\""),
        "the blocks stayed visible on Original:\n{after}"
    );
    assert!(
        !after.contains("class=\"html is-hidden\""),
        "the frame stayed concealed on Original:\n{after}"
    );
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

/// The harness renders the reader inside an empty `.app`. Its grid has a column for the
/// sidebar and one for the list, so the reader would wrap into the 232px sidebar slot.
/// This page shows the reader alone, the width a centre peek gives it. The harness renders no
/// `.card`, so the last rule stands in for it: in the window the reader sits on the card's paper
/// and takes the paper's ink, never the frame's.
const READER_ONLY: &str = ".app { grid-template-columns: minmax(0, 1fr); } \
    .app > .places, .app > .list { display: none; } \
    .app > .reader { background: var(--surface); color: var(--ink); border-radius: var(--r-card); }";

pub(super) fn dump_page(name: &str, body: &str) {
    let look = crate::space::Space::default().look;
    let head = format!("<style>{READER_ONLY}</style>");
    for (suffix, scheme) in [("", ds::Scheme::Light), ("-dark", ds::Scheme::Dark)] {
        let column = format!("<div style=\"width: 760px; margin: 0 auto\">{body}</div>");
        let framed = crate::ui::fixtures::framed(&column, scheme, &look);
        crate::ui::fixtures::write_page(
            &format!("{name}{suffix}"),
            &crate::ui::fixtures::page(&framed, &head),
        );
    }
}

/// The reference fixture's rich messages, open, for a side-by-side with the mockup.
///
/// ```text
/// cargo test -p mail-app -- --ignored render_the_bodies_to_a_file
/// ```
#[tokio::test]
#[ignore = "writes target/bodies.html for a screenshot; run with --ignored"]
async fn render_the_bodies_to_a_file() {
    let letter = include_str!("../../../../mail-mime/tests/fixtures/block/letter.html");
    let reply = include_str!("../../../../mail-mime/tests/fixtures/block/reply.html");
    let receipt = include_str!("../../../../mail-mime/tests/fixtures/block/receipt.html");
    let newsletter = include_str!("../../../../mail-mime/tests/fixtures/block/newsletter.html");
    let hebrew = include_str!("../../../../mail-mime/tests/fixtures/block/hebrew.txt");
    let cjk = include_str!("../../../../mail-mime/tests/fixtures/block/cjk.txt");
    let blocked = "<p>The banner stayed on their server.</p>\
        <img src=\"https://cdn.example/banner.png\" alt=\"This Week in Rust banner\" width=\"600\" height=\"150\">";
    let (store, thread, _dir) = thread_of(&[
        ("a letter", html_message("a letter", letter)),
        ("the chain", html_message("the chain", reply)),
        ("the receipt", html_message("the receipt", receipt)),
        ("the newsletter", html_message("the newsletter", newsletter)),
        (
            "hebrew",
            text_message("hebrew", "text/plain; charset=utf-8", hebrew),
        ),
        (
            "cjk",
            text_message(
                "cjk",
                "text/plain; charset=utf-8; format=flowed; delsp=yes",
                cjk,
            ),
        ),
        ("blocked", html_message("blocked", blocked)),
    ]);
    let body = reader_markup(store, thread);
    dump_page("bodies", &body);
}
