//! The sender's CSS, kept for the reader's frame with everything that could reach the network
//! taken out.
//!
//! The frame's own network refuses what the reader did not consent to, so this is the second
//! wall, not the first: a renderer that fetched whatever CSS named would still be handed nothing
//! to fetch. What is removed:
//! - every backslash, so no escape (`u\72l(`, `\40import`) can spell a removed word, and every
//!   `<`, which CSS has no use for and which is the only way a sheet's text reads as markup to
//!   whatever parses it next;
//! - `@import` rules, which load another sheet;
//! - the functions that take a URL or a string that is one (`url()`, `image()`, `image-set()`,
//!   `-webkit-image-set()`, `cross-fade()`, `element()`), and `expression()` and `-moz-binding`,
//!   which an old engine ran as code, each replaced by `none`.
//!
//! Background images a sender laid out in CSS are therefore never shown, even after the reader
//! allows remote images: only an `<img>`'s address is on the consented list.

/// Functions whose arguments name something to fetch or run. Matched case-insensitively, at a
/// name boundary, before an opening parenthesis; each becomes `none`.
const FETCHING: &[&str] = &[
    "url",
    "image",
    "image-set",
    "-webkit-image-set",
    "cross-fade",
    "element",
    "expression",
];

/// Properties that are code in some engine: the whole value goes.
const RUNNING: &[&str] = &["-moz-binding", "behavior"];

/// `css` with nothing in it that fetches or runs.
pub(super) fn scrub(css: &str) -> String {
    let plain: String = css.chars().filter(|c| !matches!(c, '\\' | '<')).collect();
    let without_imports = drop_at_imports(&plain);
    let without_calls = drop_calls(&without_imports);
    drop_running(&without_calls)
}

/// Remove every `@import ... ;` (or to the end, when there is no semicolon).
fn drop_at_imports(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(at) = find_ci(rest, "@import") {
        out.push_str(&rest[..at]);
        rest = match rest[at..].find(';') {
            Some(end) => &rest[at + end + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Replace each fetching function call, arguments and all, with `none`.
fn drop_calls(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let mut i = 0;
    while i < css.len() {
        if let Some(len) = call_at(css, i) {
            out.push_str("none");
            i = close_of(bytes, i + len);
            continue;
        }
        let Some(c) = css[i..].chars().next() else {
            break;
        };
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// The length of a fetching function's name and its `(` when one starts at `i`.
fn call_at(css: &str, i: usize) -> Option<usize> {
    let boundary = css[..i]
        .chars()
        .next_back()
        .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    if !boundary {
        return None;
    }
    let rest = &css[i..];
    FETCHING.iter().find_map(|name| {
        let head = rest.get(..name.len())?;
        if !head.eq_ignore_ascii_case(name) {
            return None;
        }
        let after = rest[name.len()..].trim_start();
        after.starts_with('(').then(|| rest.len() - after.len() + 1)
    })
}

/// The index just past the `)` closing the call whose arguments start at `from`, nesting and
/// quotes respected; the end of the text when it never closes.
fn close_of(bytes: &[u8], from: usize) -> usize {
    let mut depth = 1usize;
    let mut quote = None::<u8>;
    let mut i = from;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    bytes.len()
}

/// Replace the value of each property that runs code with `none`.
fn drop_running(css: &str) -> String {
    let mut out = css.to_owned();
    for name in RUNNING {
        let mut from = 0;
        while let Some(at) = find_ci(&out[from..], name).map(|at| at + from) {
            let value_start = at + name.len();
            let end = out[value_start..]
                .find([';', '}', '"'])
                .map_or(out.len(), |end| value_start + end);
            match out[value_start..end].find(':') {
                Some(colon) => {
                    out.replace_range(value_start + colon + 1..end, "none");
                    from = value_start + colon + 1 + "none".len();
                }
                None => from = value_start,
            }
        }
    }
    out
}

/// Where `needle` (ASCII) first occurs in `hay`, ignoring ASCII case.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let n = needle.len();
    hay.char_indices().map(|(i, _)| i).find(|&i| {
        hay.get(i..i + n)
            .is_some_and(|s| s.eq_ignore_ascii_case(needle))
    })
}

/// Every `<style>` element's text in markup the sanitizer serialized, scrubbed. The serializer
/// writes a style element's text raw, and the parser ended it at the first `</style`, so the text
/// between an opening tag and the next `</style>` is the whole sheet.
pub(super) fn scrub_style_elements(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = find_ci(rest, "<style") {
        let Some(tag_end) = rest[open..].find('>').map(|end| open + end + 1) else {
            break;
        };
        out.push_str(&rest[..tag_end]);
        let body = &rest[tag_end..];
        let close = find_ci(body, "</style").unwrap_or(body.len());
        out.push_str(&scrub(&body[..close]));
        rest = &body[close..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::{scrub, scrub_style_elements};

    #[test]
    fn what_fetches_or_runs_is_removed_and_the_rest_kept() {
        const CASES: &[(&str, &str)] = &[
            ("p{color:red}", "p{color:red}"),
            (
                "body{background:url(https://t.test/x)}",
                "body{background:none}",
            ),
            ("a{background:URL( 'x' )}", "a{background:none}"),
            (
                "a{background:u\\72l(https://t.test/x)}",
                "a{background:u72l(https://t.test/x)}",
            ),
            (
                "@import url(https://t.test/s.css);p{margin:0}",
                "p{margin:0}",
            ),
            ("@IMPORT 'https://t.test/s.css';p{margin:0}", "p{margin:0}"),
            ("a{background:image-set('x.png' 1x)}", "a{background:none}"),
            (
                "a{background:-webkit-image-set(url(x) 1x)}",
                "a{background:none}",
            ),
            ("a{width:expression(alert(1))}", "a{width:none}"),
            ("a{-moz-binding:url(x.xml#b)}", "a{-moz-binding:none}"),
            ("a{background:curl(x)}", "a{background:curl(x)}"),
            ("a{background:url(\"a)b\")}", "a{background:none}"),
            ("a{background:url(x", "a{background:none"),
            (".x{font-family:'Ünï'}", ".x{font-family:'Ünï'}"),
            (
                "<img src=x onerror=alert(1)>",
                "img src=x onerror=alert(1)>",
            ),
            ("ul>li{margin:0}", "ul>li{margin:0}"),
        ];
        for (css, want) in CASES {
            assert_eq!(scrub(css), *want, "{css}");
        }
    }

    #[test]
    fn only_the_text_inside_style_elements_is_scrubbed() {
        let html = "<style>p{background:url(x)}</style><p>url(y)</p><STYLE media=\"all\">@import 'z';</STYLE>";
        assert_eq!(
            scrub_style_elements(html),
            "<style>p{background:none}</style><p>url(y)</p><STYLE media=\"all\"></STYLE>"
        );
    }
}
