//! An image shown in a draft's body goes out inside the HTML's `multipart/related`, under the
//! `Content-ID` the HTML names it by (RFC 2387), and reads back as the message's own image.
//!
//! The rows are the shapes a draft can take: images alone, with files, in order, deleted from the
//! body, with no HTML to show them, and with a `Content-ID` that cannot be written as it is.

use crate::support::{draft, sender};
use mail_domain::{BlobId, Draft, Inline, PendingAttachment};
use mail_mime::{Disclosure, INLINE_BUDGET, build, embed_inline, parse};
use mail_parser::{MessageParser, MimeHeaders, PartType};

/// One of the draft's parts: its name, type and `Content-ID`.
struct Part {
    name: &'static str,
    mime: &'static str,
    cid: Option<&'static str>,
}

struct Case {
    name: &'static str,
    html: Option<&'static str>,
    parts: &'static [Part],
    /// The message's MIME tree: a multipart as `subtype(children)`, a leaf as its type.
    expect: &'static str,
    /// The `Content-ID`s that went out inline, in order.
    inline: &'static [&'static str],
}

const PHOTO: Part = Part {
    name: "photo.png",
    mime: "image/png",
    cid: Some("img1@mailo"),
};

const CASES: &[Case] = &[
    Case {
        name: "one image the body shows",
        html: Some("<p>Look</p><img alt=\"\" src=\"cid:img1@mailo\">"),
        parts: &[PHOTO],
        expect: "alternative(text/plain, related(text/html, image/png))",
        inline: &["img1@mailo"],
    },
    Case {
        name: "an image and a file",
        html: Some("<img alt=\"\" src=\"cid:img1@mailo\">"),
        parts: &[
            PHOTO,
            Part {
                name: "plan.pdf",
                mime: "application/pdf",
                cid: None,
            },
        ],
        expect: "mixed(alternative(text/plain, related(text/html, image/png)), application/pdf)",
        inline: &["img1@mailo"],
    },
    Case {
        name: "two images keep their order",
        html: Some("<img src=\"cid:img1@mailo\"><p>and</p><img src=\"cid:img2@mailo\">"),
        parts: &[
            PHOTO,
            Part {
                name: "cat.jpg",
                mime: "image/jpeg",
                cid: Some("img2@mailo"),
            },
        ],
        expect: "alternative(text/plain, related(text/html, image/png, image/jpeg))",
        inline: &["img1@mailo", "img2@mailo"],
    },
    Case {
        name: "an image deleted from the body is not sent",
        html: Some("<p>Never mind</p>"),
        parts: &[PHOTO],
        expect: "alternative(text/plain, text/html)",
        inline: &[],
    },
    Case {
        name: "a reference that only starts with the id is not the image",
        html: Some("<img src=\"cid:img1@mailo.other\">"),
        parts: &[PHOTO],
        expect: "alternative(text/plain, text/html)",
        inline: &[],
    },
    Case {
        name: "with no HTML to show it the image goes as a file",
        html: None,
        parts: &[PHOTO],
        expect: "mixed(text/plain, image/png)",
        inline: &[],
    },
    Case {
        name: "a Content-ID that would break its header goes as a file",
        html: Some("<img src=\"cid:img1@mailo\">"),
        parts: &[Part {
            name: "photo.png",
            mime: "image/png",
            cid: Some("img1@mailo>\r\nBcc: x@example.test"),
        }],
        expect: "mixed(alternative(text/plain, text/html), image/png)",
        inline: &[],
    },
];

/// The bytes of a tiny PNG: what is checked is that they arrive, not that they draw.
fn png() -> Vec<u8> {
    b"\x89PNG\r\n\x1a\nnot really a picture".to_vec()
}

fn drafted(case: &Case) -> (Draft, Vec<(BlobId, Vec<u8>)>) {
    let mut bytes = Vec::new();
    let attachments = case
        .parts
        .iter()
        .map(|part| {
            let blob = BlobId::generate();
            bytes.push((blob, png()));
            PendingAttachment {
                name: part.name.to_owned(),
                mime: part.mime.to_owned(),
                blob,
                cid: part.cid.map(str::to_owned),
            }
        })
        .collect();
    let draft = Draft {
        text: "Look\r\n".to_owned(),
        html: case.html.map(str::to_owned),
        attachments,
        ..draft()
    };
    (draft, bytes)
}

/// The MIME tree of `raw`, as [`Case::expect`] writes it.
fn outline(raw: &[u8]) -> String {
    let parsed = MessageParser::default()
        .parse(raw)
        .expect("the built message parses");
    fn walk(parsed: &mail_parser::Message<'_>, index: usize) -> String {
        let part = &parsed.parts[index];
        let ct = part.content_type();
        let kind = ct.map_or("text".to_owned(), |ct| ct.ctype().to_ascii_lowercase());
        let sub = ct
            .and_then(|ct| ct.subtype())
            .map_or("plain".to_owned(), str::to_ascii_lowercase);
        match &part.body {
            PartType::Multipart(children) => {
                let inner: Vec<String> = children
                    .iter()
                    .map(|child| walk(parsed, *child as usize))
                    .collect();
                format!("{sub}({})", inner.join(", "))
            }
            _ => format!("{kind}/{sub}"),
        }
    }
    walk(&parsed, 0)
}

#[test]
fn an_image_in_the_body_goes_inside_the_html_s_related_part() {
    for case in CASES {
        let (draft, bytes) = drafted(case);
        let raw = build(
            &draft,
            &sender(None, "me@example.test"),
            None,
            &bytes,
            Disclosure::Full,
        )
        .unwrap_or_else(|err| panic!("{}: {err}", case.name));
        assert_eq!(outline(&raw), case.expect, "{}", case.name);

        // Each inline part says it is inline and names its Content-ID, and nothing else is.
        let parsed = MessageParser::default().parse(&raw).expect("parses");
        let inline: Vec<&str> = parsed
            .parts
            .iter()
            .filter(|part| part.content_id().is_some())
            .map(|part| {
                let disposition = part
                    .content_disposition()
                    .map(|cd| cd.ctype().to_ascii_lowercase());
                assert_eq!(disposition.as_deref(), Some("inline"), "{}", case.name);
                part.content_id().unwrap_or_default()
            })
            .collect();
        assert_eq!(inline, case.inline, "{}", case.name);
        let text = String::from_utf8_lossy(&raw);
        assert!(!text.contains("\r\nBcc:"), "{}: {text}", case.name);
    }
}

#[test]
fn an_image_sent_inline_reads_back_as_the_message_s_own_image() {
    let case = &CASES[0];
    let (draft, bytes) = drafted(case);
    let raw = build(
        &draft,
        &sender(None, "me@example.test"),
        None,
        &bytes,
        Disclosure::Full,
    )
    .expect("builds");
    let read = parse(&raw).expect("parses");
    let image = read
        .attachments
        .iter()
        .find(|part| {
            part.inline
                == Inline::Embedded {
                    cid: "img1@mailo".to_owned(),
                }
        })
        .expect("the image is the message's own, by its Content-ID");
    assert_eq!(image.bytes, png());
    let html = read.html.as_deref().expect("an HTML body");
    let shown = embed_inline(html, &read.attachments, INLINE_BUDGET);
    assert!(shown.contains("src=\"data:image/png;base64,"), "{shown}");
}

#[test]
fn an_image_deleted_from_the_body_needs_no_bytes() {
    // The draft keeps the part for an undo; the send neither carries nor asks for it.
    let (draft, _) = drafted(&CASES[3]);
    let raw = build(
        &draft,
        &sender(None, "me@example.test"),
        None,
        &[],
        Disclosure::Full,
    )
    .expect("a part the body no longer shows is not looked for");
    assert_eq!(outline(&raw), "alternative(text/plain, text/html)");
}
