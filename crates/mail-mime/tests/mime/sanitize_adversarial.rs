//! An independent adversarial pass over `sanitize`, written against the delegated
//! implementation rather than alongside it.
//!
//! The suite that ships with a sanitizer tends to test the vectors its author thought of. These
//! are deliberately different payloads: obfuscation that defeats string scanning, mutation-XSS
//! shapes that exploit the parser rather than the filter, and every attribute that makes a
//! browser fetch something without the reader clicking.

use mail_mime::{SanitizePolicy, sanitize};

fn blocked() -> SanitizePolicy {
    SanitizePolicy::CURRENT
}

/// Payloads that try to leave behind something that executes.
const PAYLOADS: &[(&str, &str)] = &[
    ("plain script", "<script>alert(1)</script>"),
    ("img onerror", "<img src=x onerror=alert(1)>"),
    ("svg onload", "<svg onload=alert(1)>"),
    ("body onload", "<body onload=alert(1)>x</body>"),
    (
        "details ontoggle",
        "<details ontoggle=alert(1) open>x</details>",
    ),
    ("javascript href", r#"<a href="javascript:alert(1)">x</a>"#),
    ("uppercase scheme", r#"<a href="jAvAsCrIpT:alert(1)">x</a>"#),
    ("leading space", r#"<a href=" javascript:alert(1)">x</a>"#),
    ("embedded tab", "<a href=\"java\tscript:alert(1)\">x</a>"),
    (
        "embedded newline",
        "<a href=\"java\nscript:alert(1)\">x</a>",
    ),
    (
        "entity encoded",
        r#"<a href="&#106;avascript:alert(1)">x</a>"#,
    ),
    ("hex entity", r#"<a href="&#x6a;avascript:alert(1)">x</a>"#),
    (
        "colon entity",
        r#"<a href="javascript&colon;alert(1)">x</a>"#,
    ),
    ("vbscript", r#"<a href="vbscript:msgbox(1)">x</a>"#),
    (
        "data html",
        r#"<a href="data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==">x</a>"#,
    ),
    (
        "iframe srcdoc",
        r#"<iframe srcdoc="<script>alert(1)</script>"></iframe>"#,
    ),
    (
        "object data",
        r#"<object data="javascript:alert(1)"></object>"#,
    ),
    ("embed src", r#"<embed src="javascript:alert(1)">"#),
    (
        "form formaction",
        r#"<form><button formaction="javascript:alert(1)">x</button></form>"#,
    ),
    (
        "meta refresh",
        r#"<meta http-equiv="refresh" content="0;url=javascript:alert(1)">"#,
    ),
    (
        "base tag",
        r#"<base href="javascript:"><a href="alert(1)">x</a>"#,
    ),
    // Mutation XSS: the payload is inert until a browser re-parses the serialized output.
    (
        "noscript mXSS",
        r#"<noscript><p title="</noscript><img src=x onerror=alert(1)>">"#,
    ),
    (
        "style mXSS",
        r#"<style><img src=x onerror=alert(1)></style>"#,
    ),
    (
        "textarea mXSS",
        r#"<textarea><img src=x onerror=alert(1)></textarea>"#,
    ),
    (
        "svg foreignObject",
        r#"<svg><foreignObject><script>alert(1)</script></foreignObject></svg>"#,
    ),
    (
        "xlink href",
        r#"<svg><use xlink:href="javascript:alert(1)"/></svg>"#,
    ),
    (
        "css expression",
        r#"<div style="width:expression(alert(1))">x</div>"#,
    ),
    (
        "import in style",
        r#"<style>@import url("javascript:alert(1)");</style>"#,
    ),
];

/// No payload may leave behind anything that executes.
#[test]
fn nothing_executable_survives() {
    for (name, payload) in PAYLOADS {
        let out = sanitize(payload, blocked()).as_str().to_ascii_lowercase();

        // Only MARKUP can execute. Escaped text such as `&lt;img onerror=...&gt;` renders as
        // literal characters and is inert, so scanning the raw output for "alert(" reports a
        // vulnerability where there is none. Scan the tags instead.
        let markup = tags_of(&out);
        for forbidden in [
            "javascript",
            "vbscript",
            "onerror",
            "onload",
            "ontoggle",
            "expression(",
        ] {
            assert!(
                !markup.contains(forbidden),
                "{name}: {forbidden:?} survived inside live markup\n  in:  {payload}\n  out: {out}"
            );
        }
        for tag in [
            "<script", "<iframe", "<object", "<embed", "<form", "<meta", "<base", "<style",
        ] {
            assert!(!out.contains(tag), "{name}: {tag} survived\n  out: {out}");
        }
    }
}

/// The parts of `html` that a browser parses as tags, joined.
///
/// Everything outside a tag is text: it may legitimately contain `alert(` or `javascript:` as
/// characters on the page, which is exactly what a sanitizer neutralising a payload produces.
fn tags_of(html: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for ch in html.chars() {
        match ch {
            '<' => {
                inside = true;
                out.push(ch);
            }
            '>' if inside => {
                inside = false;
                out.push(ch);
            }
            _ if inside => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Every attribute a browser fetches without the reader doing anything. A remote image is a
/// read receipt the sender never asked permission for.
const FETCHERS: &[(&str, &str)] = &[
    ("img src", r#"<img src="https://tracker.test/pixel.gif">"#),
    (
        "img srcset",
        r#"<img srcset="https://tracker.test/1x.png 1x, https://tracker.test/2x.png 2x">"#,
    ),
    (
        "video poster",
        r#"<video poster="https://tracker.test/p.jpg"></video>"#,
    ),
    (
        "body background",
        r#"<body background="https://tracker.test/b.gif">x</body>"#,
    ),
    (
        "table background",
        r#"<table background="https://tracker.test/t.gif"><tr><td>x</td></tr></table>"#,
    ),
    (
        "input src",
        r#"<input type="image" src="https://tracker.test/i.gif">"#,
    ),
    (
        "link stylesheet",
        r#"<link rel="stylesheet" href="https://tracker.test/s.css">"#,
    ),
    (
        "style background",
        r#"<div style="background:url(https://tracker.test/x)">y</div>"#,
    ),
    (
        "svg image href",
        r#"<svg><image href="https://tracker.test/s.png"/></svg>"#,
    ),
    (
        "http not just https",
        r#"<img src="http://tracker.test/pixel.gif">"#,
    ),
];

#[test]
fn nothing_fetches_the_network_when_remote_images_are_blocked() {
    for (name, payload) in FETCHERS {
        let out = sanitize(payload, blocked()).as_str().to_ascii_lowercase();
        assert!(
            !out.contains("tracker.test"),
            "{name}: a remote fetch survived blocking\n  in:  {payload}\n  out: {out}"
        );
    }
}

/// The markup returned reads back as the tree it was written from. Dropping `<object>` once left
/// its `<table>` inside the `<p>`, which a browser reads as three siblings: the sanitizer had
/// checked one tree and the reader drew another.
#[test]
fn what_a_removed_element_leaves_reads_back_as_itself() {
    let once = sanitize("<p><object><table>", blocked());
    assert_eq!(once.as_str(), "<p></p><table></table><p></p>");
    assert_eq!(sanitize(once.as_str(), blocked()).as_str(), once.as_str());
}

/// A mail body is attacker-controlled bytes; none of these may panic or hang.
#[test]
fn hostile_shapes_do_not_panic() {
    let deep = "<div>".repeat(500) + "x" + &"</div>".repeat(500);
    let unclosed = "<div><span><b>".repeat(200);
    for payload in [
        "",
        "<",
        "<<<<>>>>",
        "<a href=",
        "\u{0}<script>alert(1)</script>",
        "<img src=\"\u{0}javascript:alert(1)\">",
        &deep,
        &unclosed,
        &"a".repeat(100_000),
    ] {
        let out = sanitize(payload, blocked());
        assert!(!out.as_str().to_ascii_lowercase().contains("alert("));
    }
}

/// The same checks over markup made at random from the pieces above: tags, the attributes that
/// run or fetch, their obfuscated values, and stray bytes between them.
mod any_markup {
    use super::{FETCHERS, PAYLOADS, tags_of};
    use cssparser::{ParseError, Parser, Token as Css};
    use html5ever::buffer_queue::BufferQueue;
    use html5ever::tendril::StrTendril;
    use html5ever::tokenizer::{
        Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
    };
    use mail_mime::{RemoteImages, SanitizePolicy, Styles, sanitize};
    use proptest::prelude::*;
    use std::cell::RefCell;

    const POLICIES: [SanitizePolicy; 4] = [
        SanitizePolicy::CURRENT,
        SanitizePolicy {
            styles: Styles::Kept,
            ..SanitizePolicy::CURRENT
        },
        SanitizePolicy {
            remote_images: RemoteImages::Allowed,
            ..SanitizePolicy::CURRENT
        },
        SanitizePolicy {
            remote_images: RemoteImages::Allowed,
            styles: Styles::Kept,
            ..SanitizePolicy::CURRENT
        },
    ];

    /// Attributes whose value is a URL a browser follows or fetches.
    const URL_ATTRS: &[&str] = &[
        "href",
        "src",
        "srcset",
        "action",
        "formaction",
        "background",
        "poster",
        "data",
        "cite",
        "xlink:href",
        "lowsrc",
        "dynsrc",
    ];

    /// Attributes a browser fetches from without the reader doing anything.
    const FETCH_ATTRS: &[&str] = &[
        "src",
        "srcset",
        "background",
        "poster",
        "data",
        "lowsrc",
        "dynsrc",
    ];

    fn piece() -> impl Strategy<Value = String> {
        let tag = prop::sample::select(vec![
            "a", "img", "div", "p", "svg", "image", "style", "script", "iframe", "table", "td",
            "video", "body", "math", "noscript", "textarea", "title", "form", "base", "object",
        ]);
        let attr = prop::sample::select(vec![
            "href",
            "src",
            "srcset",
            "style",
            "onload",
            "onerror",
            "ONCLICK",
            "background",
            "poster",
            "xlink:href",
            "class",
            "title",
            "alt",
        ]);
        let value = prop::sample::select(vec![
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "&#106;avascript&#58;alert(1)",
            " java\tscript:alert(1)",
            "https://tracker.test/x.png",
            "http://tracker.test/x.png",
            "//tracker.test/x.png",
            "cid:logo@here",
            "background:url(https://tracker.test/x)",
            "u\\72l(https://tracker.test/x)",
            "data:text/html,<script>alert(1)</script>",
            "\"><script>alert(1)</script>",
            "x onerror=alert(1)",
        ]);
        let element = (
            tag,
            prop::collection::vec((attr, value), 0..3),
            any::<bool>(),
        )
            .prop_map(|(tag, attrs, close)| {
                let mut out = format!("<{tag}");
                for (name, value) in attrs {
                    out.push_str(&format!(" {name}=\"{value}\""));
                }
                out.push('>');
                if close {
                    out.push_str(&format!("x</{tag}>"));
                }
                out
            });
        prop_oneof![
            4 => element,
            1 => prop::sample::select(PAYLOADS.iter().map(|(_, html)| *html).collect::<Vec<_>>())
                .prop_map(str::to_owned),
            1 => prop::sample::select(FETCHERS.iter().map(|(_, html)| *html).collect::<Vec<_>>())
                .prop_map(str::to_owned),
            1 => prop::sample::select(vec!["<", ">", "\"", "'", "</", "<!--", "-->", "&", "\0"])
                .prop_map(str::to_owned),
            1 => "[ -~]{0,12}",
        ]
    }

    fn markup() -> impl Strategy<Value = String> {
        prop::collection::vec(piece(), 0..10).prop_map(|pieces| pieces.concat())
    }

    /// A start tag's name and its attributes, as `(name, value)`.
    type StartTag = (String, Vec<(String, String)>);

    /// The start tags of `html` as a tokenizer reads them, and the text inside every `<style>`.
    fn read(html: &str) -> (Vec<StartTag>, String) {
        struct Sink {
            tags: RefCell<Vec<StartTag>>,
            sheet: RefCell<String>,
            in_style: RefCell<bool>,
        }
        impl TokenSink for Sink {
            type Handle = ();
            fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
                match token {
                    Token::TagToken(Tag {
                        kind, name, attrs, ..
                    }) => {
                        let name = name.to_string();
                        if name == "style" {
                            *self.in_style.borrow_mut() = kind == TagKind::StartTag;
                        }
                        if kind == TagKind::StartTag {
                            let attrs = attrs
                                .iter()
                                .map(|a| {
                                    let key = match &a.name.prefix {
                                        Some(prefix) => format!("{prefix}:{}", a.name.local),
                                        None => a.name.local.to_string(),
                                    };
                                    (key, a.value.to_string())
                                })
                                .collect();
                            self.tags.borrow_mut().push((name, attrs));
                        }
                    }
                    Token::CharacterTokens(text) if *self.in_style.borrow() => {
                        self.sheet.borrow_mut().push_str(&text);
                    }
                    _ => {}
                }
                TokenSinkResult::Continue
            }
        }
        let mut tendril = StrTendril::new();
        tendril.push_slice(html);
        let queue = BufferQueue::default();
        queue.push_back(tendril);
        let tokenizer = Tokenizer::new(
            Sink {
                tags: RefCell::new(Vec::new()),
                sheet: RefCell::new(String::new()),
                in_style: RefCell::new(false),
            },
            TokenizerOpts::default(),
        );
        // The whole input is queued, so the tokenizer never stops to ask for more.
        let _ = tokenizer.feed(&queue);
        tokenizer.end();
        let sink = tokenizer.sink;
        (sink.tags.into_inner(), sink.sheet.into_inner())
    }

    /// A URL as a browser reads its scheme: lowercased, with the whitespace and control
    /// characters it skips taken out.
    fn scheme_text(value: &str) -> String {
        value
            .chars()
            .filter(|c| !c.is_whitespace() && !c.is_control())
            .collect::<String>()
            .to_ascii_lowercase()
    }

    /// Whether `css`, read as CSS is, names something to fetch: a `url()` or another fetching
    /// function, an unquoted url, or an `@import`. A string that merely spells `url(` is text.
    fn css_fetches(css: &str) -> bool {
        fn walk(parser: &mut Parser<'_>) -> bool {
            while let Ok(token) = parser.next_including_whitespace_and_comments().cloned() {
                match token {
                    Css::UnquotedUrl(_) | Css::BadUrl(_) => return true,
                    Css::AtKeyword(name) if name.eq_ignore_ascii_case("import") => return true,
                    Css::Function(name)
                        if ["url", "src", "image", "image-set", "-webkit-image-set"]
                            .iter()
                            .any(|f| name.eq_ignore_ascii_case(f)) =>
                    {
                        return true;
                    }
                    Css::Function(_)
                    | Css::ParenthesisBlock
                    | Css::SquareBracketBlock
                    | Css::CurlyBracketBlock => {
                        let inner = parser
                            .parse_nested_block(|inner| Ok::<bool, ParseError<()>>(walk(inner)));
                        if inner.unwrap_or(false) {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            false
        }
        walk(&mut Parser::new(css))
    }

    fn is_remote(value: &str) -> bool {
        let value = scheme_text(value);
        value.contains("http:") || value.contains("https:") || value.contains("//")
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn sanitizing_twice_is_sanitizing_once(html in markup()) {
            for policy in POLICIES {
                let once = sanitize(&html, policy);
                let twice = sanitize(once.as_str(), policy);
                prop_assert_eq!(twice.as_str(), once.as_str(), "{:?} of {:?}", policy, html);
            }
        }

        #[test]
        fn nothing_that_runs_or_fetches_unasked_survives(html in markup()) {
            for policy in POLICIES {
                let out = sanitize(&html, policy);
                let out = out.as_str();
                let blocked = policy.remote_images == RemoteImages::Blocked;
                prop_assert!(!tags_of(&out.to_ascii_lowercase()).contains("<script"), "{}", out);
                let (tags, sheet) = read(out);
                for (tag, attrs) in &tags {
                    prop_assert_ne!(tag.as_str(), "script", "{}", out);
                    for (key, value) in attrs {
                        let key = key.to_ascii_lowercase();
                        prop_assert!(!key.starts_with("on"), "{} on <{}>: {}", key, tag, out);
                        if URL_ATTRS.contains(&key.as_str()) {
                            let url = scheme_text(value);
                            for scheme in ["javascript:", "vbscript:", "data:text/html"] {
                                prop_assert!(!url.starts_with(scheme), "{}={:?}: {}", key, value, out);
                            }
                        }
                        if blocked && FETCH_ATTRS.contains(&key.as_str()) {
                            prop_assert!(!is_remote(value), "{}={:?}: {}", key, value, out);
                        }
                        if blocked && key == "style" {
                            prop_assert!(!css_fetches(value), "{:?}: {}", value, out);
                        }
                    }
                }
                if blocked {
                    prop_assert!(!css_fetches(&sheet), "{}", out);
                }
            }
        }
    }
}
