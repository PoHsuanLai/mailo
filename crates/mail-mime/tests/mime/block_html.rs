//! One case per tag mapping, compared by a sketch, plus the allowlist closure.

use crate::block as support;

use mail_mime::{Block, ImgSrc, RemoteImages, is_mapped, mapped_tags};
use support::{html, html_images, part, sketch};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-pretend-";

#[test]
fn mappings_match_their_sketch() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "paragraph",
            "<p>Hello</p>",
            "letter nothing -\np ltr Hello\n",
        ),
        (
            "strong and emphasis",
            "<p>a <b>b</b> <em>c</em></p>",
            "letter nothing -\np ltr a *b* _c_\n",
        ),
        (
            "link",
            r#"<p>see <a href="https://example.test/doc">the doc</a></p>"#,
            "letter nothing -\np ltr see [the doc](https://example.test/doc)\n",
        ),
        ("heading", "<h2>Title</h2>", "letter nothing -\nh2 Title\n"),
        (
            "list",
            "<ul><li>one</li><li>two</li></ul>",
            "letter nothing -\nul\n  item\n    p ltr one\n  item\n    p ltr two\n",
        ),
        (
            "a space opening a span after a word",
            "<p><span>Bring</span><span> the</span><span> pen</span> now</p>",
            "letter nothing -\np ltr Bring the pen now\n",
        ),
        (
            "a list written beside its items belongs to the item before it",
            "<ul><li>a</li><ul><li>b</li></ul><li>c</li></ul>",
            "letter nothing -\nul\n  item\n    p ltr a\n    ul\n      item\n        p ltr b\n  item\n    p ltr c\n",
        ),
        (
            "ordered",
            "<ol><li>one</li></ol>",
            "letter nothing -\nol\n  item\n    p ltr one\n",
        ),
        (
            "quote",
            "<p>On Monday, Ada wrote:</p><blockquote><p>hi</p></blockquote>",
            "letter nothing -\nquote\n  @ On Monday, Ada wrote:\n  p ltr hi\n",
        ),
        (
            "code",
            r#"<pre lang="rust">fn main() {}</pre>"#,
            "letter nothing -\ncode rust fn main() {}\n",
        ),
        (
            "inline code",
            "<p>call <code>main</code></p>",
            "letter nothing -\np ltr call `main`\n",
        ),
        (
            "table",
            "<table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td></tr></table>",
            "letter nothing -\ntable\n  head a | b\n  row 1 | 2\n",
        ),
        (
            "rule",
            "<p>above</p><hr><p>below</p>",
            "letter nothing -\np ltr above\nrule\np ltr below\n",
        ),
        (
            "break",
            "<p>one<br>two</p>",
            "letter nothing -\np ltr one⏎two\n",
        ),
        (
            "unknown keeps its subtree",
            "<main><p>kept</p><custom>inner</custom></main>",
            "letter nothing -\np ltr kept\np ltr inner\n",
        ),
        (
            "div is a block boundary",
            "<div><p>one</p><p>two</p></div>",
            "letter nothing -\np ltr one\np ltr two\n",
        ),
        (
            "button",
            r#"<p><a href="https://example.test/issue">Read the issue</a></p>"#,
            "letter nothing -\nbutton Read the issue https://example.test/issue\n",
        ),
        (
            "direction from the first strong character",
            "<p>שלום</p><p>Hello</p>",
            "letter nothing -\np rtl שלום\np ltr Hello\n",
        ),
        (
            "bdo dir wins when the text has no strong character",
            r#"<p><bdo dir="rtl">123</bdo></p>"#,
            "letter nothing -\np rtl 123\n",
        ),
    ];
    for (name, raw, expect) in cases {
        let got = sketch(&html(raw));
        assert_eq!(&got, expect, "{name}");
    }
}

#[test]
fn every_default_ammonia_tag_is_mapped() {
    let live: std::collections::BTreeSet<_> =
        ammonia::Builder::new().clone_tags().into_iter().collect();
    let listed: std::collections::BTreeSet<_> = mapped_tags().iter().copied().collect();
    assert_eq!(listed, live, "the walker and ammonia's allowlist disagree");
    for tag in &live {
        assert!(is_mapped(tag), "{tag} is unmapped");
        let raw = format!("before<{tag}>kept</{tag}>after");
        let doc = html(&raw);
        let text = support::plain(&doc);
        assert!(
            text.contains("before") && text.contains("after"),
            "{tag} dropped the text around it: {text}"
        );
        // Void tags have no body. Every other tag must keep `kept`.
        if !matches!(*tag, "area" | "br" | "col" | "hr" | "img" | "wbr") {
            assert!(
                text.contains("kept"),
                "{tag} dropped its subtree: {text}\n{}",
                sketch(&doc)
            );
        }
    }
}

/// The images fixture through each policy: a cid image is inlined, a remote one is blocked down
/// to its host (no path in the document) or kept when allowed.
#[test]
fn the_images_fixture_inline_blocked_and_allowed() {
    let raw = include_str!("../fixtures/block/images.html");
    let parts = [part("logo@example.test", "image/png", PNG)];

    let blocked = html_images(raw, &parts, RemoteImages::Blocked);
    let imgs = support::images(&blocked);
    assert_eq!(imgs.len(), 2, "blocked: {}", sketch(&blocked));
    match &imgs[0] {
        ImgSrc::Inline(uri) => {
            assert!(
                uri.as_str().starts_with("data:image/png;base64,"),
                "blocked: {}",
                uri.as_str()
            );
        }
        other => panic!("blocked: cid image was {other:?}"),
    }
    match &imgs[1] {
        ImgSrc::Blocked { host } => assert_eq!(host, "pixels.example"),
        other => panic!("blocked: remote image was {other:?}"),
    }

    let allowed = html_images(raw, &parts, RemoteImages::Allowed);
    match &support::images(&allowed)[1] {
        ImgSrc::Remote(url) => assert_eq!(url.scheme(), "https"),
        other => panic!("allowed: image was {other:?}"),
    }

    let no_parts = html_images(raw, &[], RemoteImages::Blocked);
    let dump = format!("{no_parts:?}");
    assert!(
        dump.contains("pixels.example"),
        "no parts: the host was not recorded: {dump}"
    );
    assert!(
        !dump.contains("beacon-9f3"),
        "no parts: the blocked URL's path leaked into the document: {dump}"
    );
    assert!(
        !dump.contains("track/"),
        "no parts: the blocked URL's path leaked into the document: {dump}"
    );
    assert!(
        support::images(&no_parts)
            .iter()
            .all(|src| !matches!(src, ImgSrc::Remote(_))),
        "no parts: a remote URL survived blocking"
    );
}

#[test]
fn a_layout_table_flattens_and_a_data_table_does_not() {
    let doc = html(
        "<table><tr><td><h1>Title</h1><p>Hello</p></td></tr></table>\
         <table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>",
    );
    let got = sketch(&doc);
    assert!(
        got.contains("h1 Title") && got.contains("p ltr Hello"),
        "layout table swallowed its blocks: {got}"
    );
    assert!(
        got.contains("row a | b") && got.contains("row c | d"),
        "data table was flattened: {got}"
    );
    assert!(
        !matches!(doc.blocks.first(), Some(Block::Table { .. })),
        "the outer layout table was kept as a table: {got}"
    );
}
