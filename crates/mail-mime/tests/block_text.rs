//! Flowed text, both `delsp` directions, and the monospace veto.

#[path = "block/mod.rs"]
mod support;

use mail_mime::{Block, Dir, Flowed};
use support::{sketch, text};

const LISTPOST: &str = include_str!("fixtures/block/listpost.txt");

#[test]
fn flowed_delsp_joins_or_keeps_the_space() {
    let cases: &[(&str, Flowed, &str)] = &[
        (
            "delsp no keeps the flow space",
            Flowed::Flowed { delsp: false },
            "This is a flowed paragraph that continues on the next line.",
        ),
        (
            "delsp yes deletes the flow space",
            Flowed::Flowed { delsp: true },
            "This is a flowed paragraph thatcontinues on the next line.",
        ),
    ];
    for (name, mode, expect) in cases {
        let doc = text(LISTPOST, *mode);
        let got = sketch(&doc);
        assert!(got.contains(expect), "{name}\n{got}");
    }
}

#[test]
fn cjk_delsp_does_not_insert_a_space_and_the_other_way_does() {
    let raw = include_str!("fixtures/block/cjk.txt");
    let yes = text(raw, Flowed::Flowed { delsp: true });
    let no = text(raw, Flowed::Flowed { delsp: false });
    let yes = sketch(&yes);
    let no = sketch(&no);
    assert!(
        yes.contains("日本語の文がここで折り返される。"),
        "delsp=yes mangled CJK: {yes}"
    );
    assert!(!yes.contains("ここで 折"), "delsp=yes kept a space: {yes}");
    assert!(
        no.contains("日本語の文がここで 折り返される。"),
        "delsp=no dropped the space: {no}"
    );
}

#[test]
fn stuffing_comes_off_and_a_stuffed_quote_is_a_quote() {
    let raw = " Hello\n > quoted line\n";
    let doc = text(raw, Flowed::Flowed { delsp: false });
    let got = sketch(&doc);
    assert!(
        got.contains("p ltr Hello\n"),
        "stuffing space stayed on the paragraph: {got}"
    );
    assert!(
        got.contains("quote\n") && got.contains("quoted line"),
        "stuffed quote was not unwrapped: {got}"
    );
}

#[test]
fn depth_change_ends_a_flowed_paragraph() {
    let raw = "still flowing \n> and then quoted\n";
    let doc = text(raw, Flowed::Flowed { delsp: false });
    let got = sketch(&doc);
    assert!(
        got.contains("p ltr still flowing\n"),
        "the flowed line ran into the quote: {got}"
    );
    assert!(got.contains("and then quoted"), "{got}");
}

#[test]
fn signature_is_the_last_marker() {
    let raw = "\
See the marker.

-- 
not the signature

More prose.

-- 
Ada
ada@example.test
";
    let doc = text(raw, Flowed::Fixed);
    let got = sketch(&doc);
    let sig_at = got.find("sig\n").expect(&got);
    let body = &got[..sig_at];
    let sig = &got[sig_at..];
    assert!(
        body.contains("not the signature") && body.contains("More prose"),
        "the first `-- ` swallowed the body: {got}"
    );
    assert!(
        sig.contains("Ada") && sig.contains("ada@example.test"),
        "the last `-- ` was not the signature: {got}"
    );
    assert!(
        !sig.contains("More prose"),
        "signature took the first `-- `: {got}"
    );
}

#[test]
fn a_quoted_signature_is_not_the_one_at_depth_zero() {
    let raw = "\
Thanks.

> -- 
> Quoted Person

-- 
Real Name
";
    let doc = text(raw, Flowed::Fixed);
    let got = sketch(&doc);
    assert!(got.contains("Quoted Person"), "{got}");
    let sig_at = got.find("sig\n").expect(&got);
    assert!(
        got[sig_at..].contains("Real Name"),
        "depth 0 signature was missed: {got}"
    );
    assert!(
        !got[sig_at..].contains("Quoted Person"),
        "the quoted `-- ` became the signature: {got}"
    );
}

#[test]
fn an_ascii_table_becomes_code_and_a_justified_note_does_not() {
    let doc = text(LISTPOST, Flowed::Flowed { delsp: false });
    let got = sketch(&doc);
    assert!(
        got.contains("code ") && got.contains("apples") && got.contains("pears"),
        "the ascii table was not code: {got}"
    );
    let code = doc.blocks.iter().any(|block| {
        matches!(block, Block::Code { text, .. } if text.contains("apples") && text.contains('|').then_some(()).is_none() && text.contains("qty"))
    });
    assert!(code, "table text was not a Code block: {got}");

    let note = text(include_str!("fixtures/block/justified.txt"), Flowed::Fixed);
    let got = sketch(&note);
    assert!(!got.contains("code "), "justified prose became code: {got}");
    assert!(
        matches!(note.blocks.first(), Some(Block::Paragraph { .. })),
        "{got}"
    );
}

#[test]
fn a_rust_block_is_code_with_its_language() {
    let doc = text(LISTPOST, Flowed::Flowed { delsp: false });
    let code = doc.blocks.iter().find_map(|block| match block {
        Block::Code { lang, text } if text.contains("fn main") => Some((lang, text)),
        _ => None,
    });
    let (lang, text) = code.unwrap_or_else(|| panic!("{}", sketch(&doc)));
    assert_eq!(lang.as_deref(), Some("rust"));
    assert!(text.contains("println!"), "{text}");
}

#[test]
fn hebrew_and_english_take_their_own_directions() {
    let doc = text(include_str!("fixtures/block/hebrew.txt"), Flowed::Fixed);
    let dirs: Vec<_> = doc
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph { dir, .. } => Some(*dir),
            _ => None,
        })
        .collect();
    assert_eq!(dirs, vec![Dir::Rtl, Dir::Ltr], "{}", sketch(&doc));
}

#[test]
fn fixed_text_does_not_flow_on_a_trailing_space() {
    let doc = text("alpha \nbeta\n", Flowed::Fixed);
    let got = sketch(&doc);
    assert!(got.contains("alpha") && got.contains("beta"), "{got}");
    // Fixed lines in one run join with a space, but the trailing flow-space
    // is not a second space, and nothing is deleted the way delsp would.
    assert!(!got.contains("alphabeta"), "{got}");
}
