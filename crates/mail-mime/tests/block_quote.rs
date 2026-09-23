//! Attribution, in six languages, and the three lines that must not match.

#[path = "block/mod.rs"]
mod support;

use mail_mime::Flowed;
use support::{html, sketch, text};

#[test]
fn attribution_in_six_languages_and_the_lines_that_are_not() {
    let cases: &[(&str, &str, bool)] = &[
        ("english", "On Monday, Ada wrote:", true),
        ("french", "Le 3 mars, Bea a écrit :", true),
        ("german", "Am Dienstag schrieb Cy:", true),
        ("spanish", "El lunes, Dee escribió:", true),
        ("portuguese", "Na segunda, Eve escreveu:", true),
        ("chinese", "Ada 写道：", true),
        ("rewrote", "I rewrote the parser:", false),
        ("wrote inside a url", "https://example.test/wrote", false),
        ("dash rule", "----------", false),
    ];
    for (name, line, expect) in cases {
        let raw = format!("<p>{line}</p><blockquote><p>inner</p></blockquote>");
        let doc = html(&raw);
        let got = sketch(&doc);
        let matched = got.contains("@ ");
        assert_eq!(matched, *expect, "{name}\n{got}");
        if *expect {
            assert!(
                got.contains(line.trim_end_matches([':', '：', ' '])) || got.contains(line),
                "{name} attribution text missing: {got}"
            );
        }
    }
}

#[test]
fn a_reply_chain_nests_three_deep() {
    let doc = html(include_str!("fixtures/block/reply.html"));
    let got = sketch(&doc);
    let quotes = got.matches("quote\n").count();
    assert_eq!(quotes, 3, "{got}");
    assert!(got.contains("@ On Tue, Ada wrote:"), "{got}");
    assert!(got.contains("@ On Mon, Bea wrote:"), "{got}");
    assert!(got.contains("@ On Sun, Cy wrote:"), "{got}");
    assert!(got.contains("The original note."), "{got}");
}

#[test]
fn text_quotes_nest_and_take_the_line_before_them() {
    let raw = "\
On Monday, Ada wrote:
> On Tuesday, Bea wrote:
> > the original
";
    let doc = text(raw, Flowed::Fixed);
    let got = sketch(&doc);
    assert_eq!(got.matches("quote\n").count(), 2, "{got}");
    assert!(got.contains("@ On Monday, Ada wrote:"), "{got}");
    assert!(got.contains("@ On Tuesday, Bea wrote:"), "{got}");
    assert!(got.contains("the original"), "{got}");
}
