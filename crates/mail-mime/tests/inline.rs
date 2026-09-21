//! Inline images, and the media type that must never be echoed back.

use mail_domain::Inline;
use mail_mime::{INLINE_BUDGET, ParsedPart, embed_inline};

fn part(cid: &str, mime: &str, bytes: &[u8]) -> ParsedPart {
    ParsedPart {
        name: "logo".to_owned(),
        mime: mime.to_owned(),
        bytes: bytes.to_vec(),
        inline: Inline::Embedded {
            cid: cid.to_owned(),
        },
    }
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-pretend-this-is-an-image";

#[test]
fn an_inline_image_becomes_the_bytes_it_names() {
    let html = r#"<p>hi</p><img src="cid:logo@example">"#;
    let out = embed_inline(
        html,
        &[part("logo@example", "image/png", PNG)],
        INLINE_BUDGET,
    );

    assert!(
        out.starts_with("<p>hi</p><img src=\"data:image/png;base64,"),
        "{out}"
    );
    assert!(!out.contains("cid:"), "{out}");
    assert!(
        out.ends_with("\">"),
        "the tail after the URL was lost: {out}"
    );
}

#[test]
fn angle_brackets_around_the_id_are_tolerated() {
    // A Content-ID header carries them and some senders copy them into the reference.
    let out = embed_inline(
        r#"<img src="cid:<logo@example>">"#,
        &[part("<logo@example>", "image/png", PNG)],
        INLINE_BUDGET,
    );
    assert!(out.contains("data:image/png;base64,"), "{out}");
}

#[test]
fn a_reference_with_no_part_is_left_exactly_as_it_was() {
    // Substituting a placeholder would be the reader inventing content for a message.
    let html = r#"<img src="cid:missing@example">"#;
    assert_eq!(embed_inline(html, &[], INLINE_BUDGET), html);
}

#[test]
fn several_references_to_the_same_part_all_resolve() {
    let html = r#"<img src="cid:a@x"><img src="cid:a@x">"#;
    let out = embed_inline(
        html,
        &[part("a@x", "image/gif", b"gif-bytes")],
        INLINE_BUDGET,
    );
    assert_eq!(out.matches("data:image/gif;base64,").count(), 2, "{out}");
}

#[test]
fn the_declared_media_type_is_never_echoed_into_the_document() {
    // The security boundary. `data:text/html` in an href is script execution, and the media
    // type comes from the message. Anything not on the allowlist keeps its cid: and renders
    // broken, which is the safe failure.
    for hostile in [
        "text/html",
        "image/svg+xml",
        "application/javascript",
        "image/png\"><script>alert(1)</script>",
        "text/html;charset=utf-8",
    ] {
        let html = r#"<a href="cid:evil@x">click</a>"#;
        let out = embed_inline(html, &[part("evil@x", hostile, b"payload")], INLINE_BUDGET);
        assert_eq!(out, html, "{hostile:?} was embedded");
        assert!(!out.contains("data:"), "{hostile:?} produced a data URI");
        assert!(!out.contains("<script"), "{out}");
    }
}

#[test]
fn a_media_type_with_parameters_still_matches_on_its_essence() {
    let out = embed_inline(
        r#"<img src="cid:a@x">"#,
        &[part("a@x", "IMAGE/PNG; name=logo.png", PNG)],
        INLINE_BUDGET,
    );
    // Matched case-insensitively and ignoring parameters, but what is written is the
    // allowlist's own spelling.
    assert!(out.contains("data:image/png;base64,"), "{out}");
    assert!(!out.contains("name=logo.png"), "{out}");
}

#[test]
fn an_attached_part_is_not_an_inline_one() {
    // A `cid` that names an ordinary attachment must not be embedded: it was not referenced as
    // inline content, and treating it as such renders a document nobody composed.
    let mut attached = part("a@x", "image/png", PNG);
    attached.inline = Inline::Attached;
    let html = r#"<img src="cid:a@x">"#;
    assert_eq!(embed_inline(html, &[attached], INLINE_BUDGET), html);
}

#[test]
fn the_budget_stops_embedding_rather_than_truncating_an_image() {
    // A half-written data URI is a corrupt document. Past the budget the reference stays a
    // cid:, which is a broken image — exactly what it was before any of this existed.
    let big = vec![b'x'; 1024];
    let html = r#"<img src="cid:a@x"><img src="cid:b@x">"#;
    let parts = vec![
        part("a@x", "image/png", &big),
        part("b@x", "image/png", &big),
    ];

    // Room for one, not two.
    let out = embed_inline(html, &parts, 1500);
    assert_eq!(out.matches("data:").count(), 1, "{out}");
    assert_eq!(out.matches("cid:").count(), 1, "{out}");
}

#[test]
fn html_with_no_references_is_returned_untouched() {
    let html = "<p>nothing inline here</p>";
    assert_eq!(embed_inline(html, &[], INLINE_BUDGET), html);
}

#[test]
fn a_bare_cid_with_nothing_after_it_does_not_panic() {
    // Hostile input reaches this directly; the sanitizer does not repair truncated URLs.
    for odd in ["cid:", "cid:<>", "<img src=\"cid:", "cid::::", "cid:a@x"] {
        let _ = embed_inline(odd, &[part("a@x", "image/png", PNG)], INLINE_BUDGET);
    }
}

#[test]
fn a_zero_budget_embeds_nothing_and_keeps_the_document_intact() {
    let html = r#"<img src="cid:a@x">"#;
    assert_eq!(
        embed_inline(html, &[part("a@x", "image/png", PNG)], 0),
        html
    );
}
