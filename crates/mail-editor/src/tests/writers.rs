//! The two writers: `format=flowed` plain text as a table, and HTML by example.

use super::*;

#[test]
fn flowed_table() {
    let long = format!("{} {}", "a".repeat(60), "b".repeat(20));
    let cjk = "字".repeat(80);
    let cases: Vec<(&str, Doc, String)> = vec![
        (
            "stuff From",
            Doc::from_text("From the office"),
            " From the office\n".into(),
        ),
        (
            "stuff >",
            Doc::from_text("> not a quote"),
            " > not a quote\n".into(),
        ),
        (
            "stuff a leading space",
            Doc::from_text("  indented"),
            "   indented\n".into(),
        ),
        (
            "no stuffing mid-line",
            Doc::from_text("a > b From c"),
            "a > b From c\n".into(),
        ),
        (
            "soft wrap at 72 keeps the breaking space",
            Doc::from_text(long.as_str()),
            format!("{} \n{}\n", "a".repeat(60), "b".repeat(20)),
        ),
        (
            "a wrapped line that starts with From is stuffed",
            Doc::from_text(format!("{} From here", "a".repeat(70))),
            format!("{} \n From here\n", "a".repeat(70)),
        ),
        (
            "trailing spaces do not make a hard line soft",
            doc_of(vec![
                plain(ParaKind::Paragraph, "one  \ntwo "),
                plain(ParaKind::Paragraph, "three"),
            ]),
            "one\ntwo\n\nthree\n".into(),
        ),
        (
            "quote",
            doc_of(vec![plain(ParaKind::Quote, "hello")]),
            "> hello\n".into(),
        ),
        (
            "quote's own space is the stuffing",
            doc_of(vec![plain(ParaKind::Quote, ">x\n\nFrom y")]),
            "> >x\n>\n> From y\n".into(),
        ),
        (
            "the quoted original says when, then who, as the reply opened it",
            doc_of(vec![Node::Object(Object::QuotedMessage {
                who: "Ada Lovelace".to_owned(),
                when: "Wed, 07 Oct 2026 at 21:02".to_owned(),
                body: vec![mail_mime::Block::Paragraph {
                    spans: vec![mail_mime::Span::Text("See you there.".to_owned())],
                    dir: mail_mime::Dir::Auto,
                    style: mail_mime::ParaStyle::default(),
                }],
            })]),
            "On Wed, 07 Oct 2026 at 21:02, Ada Lovelace wrote:\n> See you there.\n".into(),
        ),
        (
            "bullet",
            doc_of(vec![plain(ParaKind::Bullet, "item")]),
            "- item\n".into(),
        ),
        (
            "numbered",
            doc_of(vec![
                plain(ParaKind::Numbered, "a"),
                plain(ParaKind::Numbered, "b"),
            ]),
            "1. a\n2. b\n".into(),
        ),
        (
            "numbering restarts after a break in the list",
            doc_of(vec![
                plain(ParaKind::Numbered, "a"),
                plain(ParaKind::Paragraph, "then"),
                plain(ParaKind::Numbered, "b"),
            ]),
            "1. a\n\nthen\n\n1. b\n".into(),
        ),
        (
            "todo",
            doc_of(vec![
                plain(ParaKind::Todo(Check::Open), "a"),
                plain(ParaKind::Todo(Check::Done), "b"),
            ]),
            "[ ] a\n[x] b\n".into(),
        ),
        (
            "an empty item does not end in a space",
            doc_of(vec![plain(ParaKind::Bullet, "")]),
            "-\n".into(),
        ),
        (
            "heading is a text line",
            doc_of(vec![plain(ParaKind::Heading(Level::One), "Title")]),
            "Title\n".into(),
        ),
        (
            "signature separator is not flowed",
            doc_of(vec![
                plain(ParaKind::Paragraph, "Bye"),
                Node::Object(Object::Signature),
                plain(ParaKind::Paragraph, "Ann"),
            ]),
            "Bye\n\n-- \n\nAnn\n".into(),
        ),
        (
            "a paragraph that says -- is not a separator",
            Doc::from_text("-- "),
            "--\n".into(),
        ),
        (
            "code is stuffed",
            doc_of(vec![plain(ParaKind::Code, "From x\n  y")]),
            " From x\n   y\n".into(),
        ),
        (
            "code is not wrapped",
            doc_of(vec![plain(ParaKind::Code, &"c ".repeat(50))]),
            format!("{}\n", "c ".repeat(50).trim_end()),
        ),
        (
            "CJK is not broken, and gains no space",
            Doc::from_text(cjk.as_str()),
            format!("{cjk}\n"),
        ),
        (
            "CJK wraps at the spaces it has",
            Doc::from_text(format!("{} {}", "字".repeat(40), "語".repeat(40))),
            format!("{} \n{}\n", "字".repeat(40), "語".repeat(40)),
        ),
        (
            "a letter and its combining accent are one column",
            Doc::from_text(format!("{} {}", "e\u{301}".repeat(70), "b")),
            format!("{} b\n", "e\u{301}".repeat(70)),
        ),
    ];
    for (name, doc, expected) in cases {
        assert_eq!(to_flowed(&doc), expected, "{name}");
    }
}

#[test]
fn flowed_lines_are_short_and_soft_only_where_wrapped() {
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(10);
    let flowed = to_flowed(&Doc::from_text(text.trim_end()));
    let lines: Vec<&str> = flowed.lines().collect();
    assert!(lines.len() > 1);
    for line in &lines[..lines.len() - 1] {
        assert!(line.ends_with(' '), "soft line {line:?}");
        assert!(line.chars().count() <= 72, "long line {line:?}");
    }
    assert!(
        !lines[lines.len() - 1].ends_with(' '),
        "the last line is hard"
    );
    // Unflowing (delsp=no: join soft lines, keep the space) gives the paragraph back.
    assert_eq!(lines.concat(), text.trim_end());
}

#[test]
fn a_line_over_998_octets_is_cut_between_clusters() {
    let cjk = "字".repeat(400); // 1200 octets, no space anywhere.
    let flowed = to_flowed(&Doc::from_text(cjk.as_str()));
    let lines: Vec<&str> = flowed.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|line| line.len() <= 998));
    assert!(lines.iter().all(|line| !line.ends_with(' ')));
    assert_eq!(lines.concat(), cjk);
}

#[test]
fn html_is_escaped_and_carries_no_style() {
    let mut linked = marks_of(&[Mark::Bold]);
    linked.link = Some(url());
    let doc = doc_of(vec![
        Node::para(
            ParaKind::Paragraph,
            vec![
                Run::new("<script>alert('x')</script> & \"", Marks::new()),
                Run::new("link", linked),
            ],
        ),
        plain(ParaKind::Code, "a < b\nc"),
        plain(ParaKind::Paragraph, "line\nbreak"),
        Node::Object(Object::Divider),
    ]);
    let html = to_html(&doc);
    assert_eq!(
        html,
        "<p>&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &quot;\
         <a href=\"https://example.com/a\"><strong>link</strong></a></p>\
         <pre>a &lt; b\nc</pre><p>line<br>break</p><hr>"
    );
}

#[test]
fn html_round_trip_examples_and_what_is_dropped() {
    let mut bold_link = marks_of(&[Mark::Bold]);
    bold_link.link = Some(url());
    let doc = doc_of(vec![
        Node::para(
            ParaKind::Paragraph,
            vec![
                Run::new("Hello ", Marks::new()),
                Run::new("world", bold_link),
            ],
        ),
        plain(ParaKind::Heading(Level::Two), "Title"),
        plain(ParaKind::Bullet, "one"),
        plain(ParaKind::Bullet, "two"),
        plain(ParaKind::Todo(Check::Open), "buy milk"),
        plain(ParaKind::Todo(Check::Done), "ship"),
        plain(ParaKind::Quote, "said"),
        plain(ParaKind::Code, "a < b & c"),
    ]);
    let back = round_trip(&doc);
    assert_eq!(canon(&back), canon(&doc), "{}", to_html(&doc));
    assert_eq!(
        kinds(&back),
        "p|h2|ul|ul|todo|done|quote|code",
        "to-dos survive"
    );

    let fancy = doc_of(vec![Node::para(
        ParaKind::Paragraph,
        vec![
            Run::new(
                "hi ",
                marks_of(&[Mark::Bold, Mark::Underline, Mark::Strike]),
            ),
            Run::new("there", Marks::new()),
        ],
    )]);
    let back = round_trip(&fancy);
    let Node::Para { runs, .. } = &back.nodes[0] else {
        panic!("paragraph");
    };
    assert!(runs[0].marks.has(Mark::Bold));
    assert!(
        !runs[0].marks.has(Mark::Underline),
        "underline does not survive from_html"
    );
    assert!(
        !runs[0].marks.has(Mark::Strike),
        "strike does not survive from_html"
    );

    assert!(
        round_trip(&Doc::blank()).nodes.is_empty(),
        "an empty paragraph is dropped by from_html"
    );
}

/// A reply quoted twice with a space between the marks is quoted to depth two, not one with a
/// `>` in its words.
#[test]
fn quote_marks_with_spaces_between_them_are_run_together() {
    use crate::plain_to_flowed;
    assert_eq!(
        plain_to_flowed("> > text  \n> >\n>> kept\n> plain"),
        ">> text\n>>\n>> kept\n> plain"
    );
}

/// Text not written by the editor is flowed text that reads back exactly as it was written.
#[test]
fn plain_text_reads_back_as_itself_through_flowed() {
    use crate::plain_to_flowed;
    use mail_mime::{Delsp, unflow};
    let long = "word ".repeat(30);
    let cases: &[(&str, &str, &str)] = &[
        (
            "trailing spaces are not soft breaks",
            "one  \ntwo",
            "one\ntwo",
        ),
        ("a leading space stays", " indented", " indented"),
        ("From is stuffed", "From here", "From here"),
        (
            "the separator keeps its space",
            "Hi\n-- \nAda",
            "Hi\n-- \nAda",
        ),
        (
            "a quote stays a quote",
            "On Monday:\n> said\n>\n> more",
            "On Monday:\n> said\n>\n> more",
        ),
        ("crlf lines", "a  \r\nb\r\n", "a\nb\n"),
        ("a long line is not cut", long.as_str(), long.trim_end()),
    ];
    for (name, written, read) in cases {
        let flowed = plain_to_flowed(written);
        assert!(
            flowed
                .split('\n')
                .map(|line| line.trim_end_matches('\r'))
                .all(|line| line == "-- " || !line.ends_with(' ')),
            "{name}: every line is hard: {flowed:?}"
        );
        assert_eq!(unflow(&flowed, Delsp::No), *read, "{name}");
    }
}

/// What the editor soft-wraps reads back as the one paragraph the author typed.
#[test]
fn the_editors_soft_breaks_read_back_joined() {
    use mail_mime::{Delsp, unflow};
    let para = "word ".repeat(40);
    let para = para.trim_end();
    let flowed = to_flowed(&Doc::from_text(para));
    assert!(flowed.lines().count() > 1, "{flowed:?}");
    assert_eq!(unflow(&flowed, Delsp::No), format!("{para}\n"));
}

#[test]
fn an_image_on_the_draft_is_written_by_its_content_id_and_reopens_by_it() {
    let image = |src: &str| {
        doc_of(vec![Node::Object(Object::Image {
            src: ImageRef::new(src),
            alt: "Garden".to_owned(),
        })])
    };
    let cases = [
        (
            "cid:img1@mailo",
            "<img alt=\"Garden\" src=\"cid:img1@mailo\">",
        ),
        (
            "https://example.com/a.png",
            "<img alt=\"Garden\" src=\"https://example.com/a.png\">",
        ),
        // A cid that would close the attribute is written with no source at all.
        ("cid:a\"onerror=\"x", "<img alt=\"Garden\">"),
        ("cid:", "<img alt=\"Garden\">"),
        ("", "<img alt=\"Garden\">"),
    ];
    for (src, expect) in cases {
        assert_eq!(to_html(&image(src)), expect, "{src}");
    }

    // Reopened with the draft's part, the image is that part again, by the same name.
    let part = mail_mime::ParsedPart {
        name: "photo.png".to_owned(),
        mime: "image/png".to_owned(),
        bytes: b"\x89PNG".to_vec(),
        inline: mail_domain::Inline::Embedded {
            cid: "img1@mailo".to_owned(),
        },
        remote: None,
    };
    let safe = sanitize(&to_html(&image("cid:img1@mailo")), SanitizePolicy::CURRENT);
    let parsed = from_html(&safe, &[part], RemoteImages::Blocked);
    assert_eq!(
        nodes_from_blocks(&parsed.blocks),
        vec![Node::Object(Object::Image {
            src: ImageRef::of_cid("img1@mailo"),
            alt: "Garden".to_owned(),
        })]
    );
}
