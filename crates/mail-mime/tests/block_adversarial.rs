//! The sanitizer's payloads, read again as blocks, plus the stack cap.
//!
//! `PAYLOADS` lives inside `sanitize_adversarial`'s test function, and that
//! file is not this session's to edit. The strings are loaded from it so a
//! new payload there is a new payload here, rather than a list copied by hand.

#[path = "block/mod.rs"]
mod support;

use mail_mime::{Flowed, ImgSrc, Reached, RemoteImages, SafeUrl, from_html, from_text, sanitize};
use std::thread::Builder;

#[test]
fn payloads_leave_no_off_scheme_url_and_no_remote_image_when_blocked() {
    let payloads = payloads_from_the_sanitizer_suite();
    assert!(
        payloads.len() >= 20,
        "failed to import PAYLOADS, got {}",
        payloads.len()
    );
    for (name, payload) in &payloads {
        for images in [RemoteImages::Blocked, RemoteImages::Allowed] {
            let safe = sanitize(payload, support::policy(images));
            let doc = from_html(&safe, &[], images);
            for url in support::urls(&doc) {
                assert!(
                    matches!(url.scheme(), "http" | "https" | "mailto"),
                    "{name} ({images:?}) produced a {} URL",
                    url.scheme()
                );
                assert_eq!(
                    SafeUrl::parse(url.as_str()),
                    Some(url.clone()),
                    "{name} URL did not round-trip"
                );
            }
            for src in support::images(&doc) {
                match src {
                    ImgSrc::Remote(_) => assert!(
                        images == RemoteImages::Allowed,
                        "{name}: remote image under Blocked"
                    ),
                    ImgSrc::Inline(uri) => {
                        let ok = [
                            "data:image/png;",
                            "data:image/jpeg;",
                            "data:image/gif;",
                            "data:image/webp;",
                        ]
                        .iter()
                        .any(|prefix| uri.as_str().starts_with(prefix));
                        assert!(
                            ok,
                            "{name}: inline image off the allowlist: {}",
                            uri.as_str()
                        );
                    }
                    ImgSrc::Blocked { host } => {
                        assert!(
                            !host.contains('/'),
                            "{name}: blocked host contains a path: {host}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn deep_html_stays_on_a_small_stack() {
    let nested = "<div>".repeat(8_000) + "kept" + &"</div>".repeat(8_000);
    let safe = sanitize(&nested, support::policy(RemoteImages::Blocked));
    let doc = on_small_stack(move || from_html(&safe, &[], RemoteImages::Blocked));
    assert_eq!(doc.reached, Reached::Depth, "{}", support::sketch(&doc));
    assert!(
        support::plain(&doc).contains("kept"),
        "deep tags dropped their text: {}",
        support::sketch(&doc)
    );
}

#[test]
fn deep_quotes_stay_on_a_small_stack() {
    let raw = ">".repeat(8_000) + " kept\n";
    let doc = on_small_stack(move || from_text(&raw, Flowed::Fixed));
    assert_eq!(doc.reached, Reached::Depth, "{}", support::sketch(&doc));
    assert!(
        support::plain(&doc).contains("kept"),
        "deep quotes dropped their text: {}",
        support::sketch(&doc)
    );
}

fn on_small_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    Builder::new()
        .name("block-cap".into())
        .stack_size(256 * 1024)
        .spawn(work)
        .expect("spawn")
        .join()
        .expect("the walker overflowed a 256 KiB stack")
}

/// Second strings of the `PAYLOADS` table in the sibling sanitizer test.
fn payloads_from_the_sanitizer_suite() -> Vec<(String, String)> {
    let src = include_str!("sanitize_adversarial.rs");
    // The type is `&[(&str, &str)]`, so the first `&[` is not the array.
    let start = src
        .find("const PAYLOADS")
        .and_then(|at| src[at..].find("= &[").map(|rel| at + rel + 4))
        .expect("PAYLOADS array");
    let body = &src[start..];
    let end = body.find("];").expect("end of PAYLOADS");
    parse_pairs(&body[..end])
}

fn parse_pairs(mut src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    while let Some(at) = src.find('(') {
        src = &src[at + 1..];
        let Some((name, rest)) = parse_string(src) else {
            break;
        };
        src = rest.trim_start().trim_start_matches(',').trim_start();
        let Some((payload, rest)) = parse_string(src) else {
            break;
        };
        src = rest;
        out.push((name, payload));
    }
    out
}

fn parse_string(src: &str) -> Option<(String, &str)> {
    let src = src.trim_start();
    if let Some(rest) = src.strip_prefix("r#\"") {
        let end = rest.find("\"#")?;
        return Some((rest[..end].to_owned(), &rest[end + 2..]));
    }
    if let Some(rest) = src.strip_prefix("r##\"") {
        let end = rest.find("\"##")?;
        return Some((rest[..end].to_owned(), &rest[end + 3..]));
    }
    let rest = src.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\\' => {
                let (_, escaped) = chars.next()?;
                out.push(match escaped {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '0' => '\0',
                    other => other,
                });
            }
            '"' => return Some((out, &rest[index + 1..])),
            _ => out.push(ch),
        }
    }
    None
}
