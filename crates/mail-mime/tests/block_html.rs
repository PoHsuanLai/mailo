//! One case per tag mapping, compared by a sketch, plus the allowlist closure.

#[path = "block/mod.rs"]
mod support;

use mail_mime::{
    Block, ImgSrc, LINK_REL, LINK_TARGET, RemoteImages, SafeUrl, from_html, is_mapped, mapped_tags,
    sanitize,
};
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

#[test]
fn an_unknown_tag_keeps_its_subtree() {
    let doc = html("<main><section><p>kept</p>inner</section></main>");
    let text = support::plain(&doc);
    assert!(
        text.contains("kept") && text.contains("inner"),
        "unknown tag dropped its subtree: {text}"
    );
}

#[test]
fn javascript_is_not_a_url_and_links_keep_rel_target() {
    assert!(
        SafeUrl::parse("javascript:alert(1)").is_none(),
        "javascript: must not be constructible as a SafeUrl"
    );
    assert!(
        SafeUrl::parse("JavaScript:alert(1)").is_none(),
        "javascript: must not be constructible as a SafeUrl"
    );
    assert_eq!(
        LINK_REL, "noopener noreferrer",
        "the renderer rel contract was dropped"
    );
    assert_eq!(
        LINK_TARGET, "_blank",
        "the renderer target contract was dropped"
    );
    let safe = sanitize(
        r#"<p><a href="https://example.test/doc">doc</a></p>"#,
        support::policy(RemoteImages::Blocked),
    );
    assert!(
        safe.as_str().contains("noopener noreferrer"),
        "sanitizer rel missing, the renderer has nothing to copy: {}",
        safe.as_str()
    );
    assert!(
        safe.as_str().contains(r#"target="_blank""#),
        "sanitizer target missing, the renderer has nothing to copy: {}",
        safe.as_str()
    );
    let doc = from_html(&safe, &[], RemoteImages::Blocked);
    let urls = support::urls(&doc);
    assert_eq!(urls.len(), 1, "the href was lost: {}", sketch(&doc));
    assert_eq!(urls[0].scheme(), "https");
    let attacked = html(r#"<p><a href="javascript:alert(1)">x</a></p>"#);
    assert!(
        support::urls(&attacked).is_empty(),
        "javascript: became a link: {}",
        sketch(&attacked)
    );
}

#[test]
fn inline_and_blocked_images() {
    let raw = include_str!("fixtures/block/images.html");
    let parts = [part("logo@example.test", "image/png", PNG)];
    let blocked = html_images(raw, &parts, RemoteImages::Blocked);
    let imgs = support::images(&blocked);
    assert_eq!(imgs.len(), 2, "{}", sketch(&blocked));
    match &imgs[0] {
        ImgSrc::Inline(uri) => {
            assert!(
                uri.as_str().starts_with("data:image/png;base64,"),
                "{}",
                uri.as_str()
            );
        }
        other => panic!("cid image was {other:?}"),
    }
    match &imgs[1] {
        ImgSrc::Blocked { host } => assert_eq!(host, "pixels.example"),
        other => panic!("remote image was {other:?}"),
    }
    let allowed = html_images(raw, &parts, RemoteImages::Allowed);
    match &support::images(&allowed)[1] {
        ImgSrc::Remote(url) => assert_eq!(url.scheme(), "https"),
        other => panic!("allowed image was {other:?}"),
    }
}

#[test]
fn blocked_images_name_their_host() {
    let raw = include_str!("fixtures/block/images.html");
    let doc = html_images(raw, &[], RemoteImages::Blocked);
    let dump = format!("{doc:?}");
    assert!(
        dump.contains("pixels.example"),
        "the host was not recorded: {dump}"
    );
    assert!(
        !dump.contains("beacon-9f3"),
        "the blocked URL's path leaked into the document: {dump}"
    );
    assert!(
        !dump.contains("track/"),
        "the blocked URL's path leaked into the document: {dump}"
    );
    assert!(
        support::images(&doc)
            .iter()
            .all(|src| !matches!(src, ImgSrc::Remote(_))),
        "a remote URL survived blocking"
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
