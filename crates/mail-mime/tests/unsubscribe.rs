//! `List-Unsubscribe` and `List-Unsubscribe-Post`, read from whole messages.
//!
//! Each case is the header block of a message and the methods it should offer. The message is
//! otherwise ordinary, because the parser is handed whole messages in the app too.

use mail_domain::Address;
use mail_mime::{HttpsUrl, ListHeaders, ListId, Mailto, Unsubscribe, list_headers};

fn message(headers: &str) -> Vec<u8> {
    format!(
        "From: news@example.test\r\nTo: me@example.test\r\nSubject: news\r\n{headers}\r\nbody\r\n"
    )
    .into_bytes()
}

fn one_click(url: &str) -> Unsubscribe {
    Unsubscribe::OneClick {
        url: HttpsUrl::parse(url).expect("a usable https url"),
    }
}

fn web(url: &str) -> Unsubscribe {
    Unsubscribe::Web {
        url: url.to_owned(),
    }
}

fn mailto(to: &[&str], subject: &str, body: &str) -> Unsubscribe {
    Unsubscribe::Mailto(Mailto {
        to: to
            .iter()
            .map(|email| Address {
                name: None,
                email: (*email).to_owned(),
            })
            .collect(),
        subject: subject.to_owned(),
        body: body.to_owned(),
    })
}

#[test]
fn each_header_offers_the_methods_it_names() {
    let cases: Vec<(&str, &str, Vec<Unsubscribe>)> = vec![
        (
            "a mailto and an https link, with one-click",
            "List-Unsubscribe: <mailto:leave@example.test?subject=unsubscribe>, \
             <https://example.test/u/abc>\r\n\
             List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n",
            vec![
                mailto(&["leave@example.test"], "unsubscribe", ""),
                one_click("https://example.test/u/abc"),
            ],
        ),
        (
            "the same without the Post header is a page, not one-click",
            "List-Unsubscribe: <mailto:leave@example.test>, <https://example.test/u/abc>\r\n",
            vec![
                mailto(&["leave@example.test"], "", ""),
                web("https://example.test/u/abc"),
            ],
        ),
        (
            "http only, even with the Post header, is never one-click",
            "List-Unsubscribe: <http://example.test/u/abc>\r\n\
             List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n",
            vec![web("http://example.test/u/abc")],
        ),
        (
            "a Post header with another value is not RFC 8058's",
            "List-Unsubscribe: <https://example.test/u>\r\n\
             List-Unsubscribe-Post: List-Unsubscribe=Yes-Please\r\n",
            vec![web("https://example.test/u")],
        ),
        (
            "folded, with whitespace inside the brackets",
            "List-Unsubscribe:\r\n <https://example.test/u/\r\n  a-long-token>,\r\n\t<mailto:leave@example.test>\r\n\
             List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n",
            vec![
                one_click("https://example.test/u/a-long-token"),
                mailto(&["leave@example.test"], "", ""),
            ],
        ),
        (
            // Base64, so no bracket survives in the raw bytes: only a decoder finds this URI.
            "encoded as an RFC 2047 word",
            "List-Unsubscribe: =?us-ascii?B?PG1haWx0bzpsZWF2ZUBleGFtcGxlLnRlc3Q+?=\r\n",
            vec![mailto(&["leave@example.test"], "", "")],
        ),
        (
            "several URIs keep their order; the second https is a page",
            "List-Unsubscribe: <https://a.example.test/1>, <mailto:x@example.test>, \
             <https://b.example.test/2>, <mailto:y@example.test>\r\n\
             List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n",
            vec![
                one_click("https://a.example.test/1"),
                mailto(&["x@example.test"], "", ""),
                web("https://b.example.test/2"),
                mailto(&["y@example.test"], "", ""),
            ],
        ),
        (
            "schemes nobody should act on are dropped",
            "List-Unsubscribe: <javascript:alert(1)>, <ftp://example.test/x>, <data:text/html,hi>\r\n",
            vec![],
        ),
        (
            "garbage offers nothing",
            "List-Unsubscribe: <<<>>>, <, >, <mailto:>, <https://>, <mailto:no-at-sign>\r\n",
            vec![],
        ),
        (
            "an unclosed bracket stops the list rather than swallowing the rest",
            "List-Unsubscribe: <mailto:leave@example.test>, <https://example.test/u\r\n",
            vec![mailto(&["leave@example.test"], "", "")],
        ),
        (
            "bare URIs without brackets are read as a list",
            "List-Unsubscribe: mailto:leave@example.test, https://example.test/u\r\n",
            vec![
                mailto(&["leave@example.test"], "", ""),
                web("https://example.test/u"),
            ],
        ),
        (
            "an https link carrying credentials is never posted to",
            "List-Unsubscribe: <https://user:pw@example.test/u>\r\n\
             List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n",
            vec![web("https://user:pw@example.test/u")],
        ),
        (
            "a repeated URI is offered once",
            "List-Unsubscribe: <mailto:leave@example.test>, <mailto:leave@example.test>\r\n",
            vec![mailto(&["leave@example.test"], "", "")],
        ),
        ("no header at all", "", vec![]),
    ];
    for (name, headers, expected) in cases {
        let found = list_headers(&message(headers));
        assert_eq!(found.unsubscribe, expected, "case: {name}");
    }
}

#[test]
fn a_mailto_is_read_the_way_rfc_6068_writes_it() {
    let cases: Vec<(&str, Option<Unsubscribe>)> = vec![
        (
            "mailto:leave@example.test?subject=Unsubscribe%20me&body=please%0D%0Anow",
            Some(mailto(
                &["leave@example.test"],
                "Unsubscribe me",
                "please\nnow",
            )),
        ),
        (
            // `+` is literal in a mailto, not a space: it is part of many list addresses.
            "mailto:list+unsubscribe@example.test?subject=a+b",
            Some(mailto(&["list+unsubscribe@example.test"], "a+b", "")),
        ),
        (
            "mailto:?to=leave@example.test&SUBJECT=x",
            Some(mailto(&["leave@example.test"], "x", "")),
        ),
        (
            "mailto:a@example.test,b%40example.test?to=c@example.test",
            Some(mailto(
                &["a@example.test", "b@example.test", "c@example.test"],
                "",
                "",
            )),
        ),
        (
            // A copy to someone else is not part of leaving a list.
            "mailto:leave@example.test?cc=boss@example.test&bcc=eve@example.test",
            Some(mailto(&["leave@example.test"], "", "")),
        ),
        (
            // An address that would become a second header is not an address.
            "mailto:leave@example.test%0D%0ABcc:eve@example.test",
            None,
        ),
        ("mailto:?subject=nobody", None),
    ];
    for (uri, expected) in cases {
        let found = list_headers(&message(&format!("List-Unsubscribe: <{uri}>\r\n")));
        assert_eq!(
            found.unsubscribe.into_iter().next(),
            expected,
            "case: {uri}"
        );
    }
}

#[test]
fn one_click_is_preferred_then_mail_then_a_page() {
    let found = list_headers(&message(
        "List-Unsubscribe: <http://example.test/page>, <mailto:leave@example.test>, \
         <https://example.test/post>\r\n\
         List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n",
    ));
    assert_eq!(
        found.preferred(),
        Some(&one_click("https://example.test/post"))
    );

    let found = list_headers(&message(
        "List-Unsubscribe: <http://example.test/page>, <mailto:leave@example.test>\r\n",
    ));
    assert_eq!(
        found.preferred(),
        Some(&mailto(&["leave@example.test"], "", ""))
    );

    let found = list_headers(&message("List-Unsubscribe: <http://example.test/page>\r\n"));
    assert_eq!(found.preferred(), Some(&web("http://example.test/page")));

    assert_eq!(list_headers(&message("")).preferred(), None);
}

#[test]
fn list_id_is_read_for_display() {
    // Headers, then the expected (description, id).
    type Case<'a> = (&'a str, Option<(Option<&'a str>, &'a str)>);
    let cases: &[Case] = &[
        (
            "List-Id: Weekly News <news.example.test>\r\n",
            Some((Some("Weekly News"), "news.example.test")),
        ),
        (
            "List-Id: \"Quoted Name\" <q.example.test>\r\n",
            Some((Some("Quoted Name"), "q.example.test")),
        ),
        (
            "List-Id: <bare.example.test>\r\n",
            Some((None, "bare.example.test")),
        ),
        (
            "List-Id: =?utf-8?q?Caf=C3=A9_list?= <cafe.example.test>\r\n",
            Some((Some("Café list"), "cafe.example.test")),
        ),
        ("List-Id: no brackets here\r\n", None),
        ("", None),
    ];
    for (headers, expected) in cases {
        let expected = expected.map(|(description, id)| ListId {
            description: description.map(str::to_owned),
            id: id.to_owned(),
        });
        assert_eq!(
            list_headers(&message(headers)).id,
            expected,
            "case: {headers:?}"
        );
    }
}

#[test]
fn bytes_that_are_not_a_message_offer_nothing_and_do_not_panic() {
    for raw in [
        &b""[..],
        b"\0\0\0\0",
        b"List-Unsubscribe",
        b"\xff\xfe List-Unsubscribe: <mailto:\xff@example.test>\r\n\r\n",
    ] {
        assert_eq!(
            list_headers(raw).unsubscribe,
            ListHeaders::default().unsubscribe
        );
    }
}

#[test]
fn only_an_https_url_without_credentials_is_one_to_post_to() {
    assert!(HttpsUrl::parse("https://example.test/u?x=1").is_some());
    for refused in [
        "http://example.test/u",
        "HTTP://example.test/u",
        "https://user@example.test/u",
        "https://:pw@example.test/u",
        "mailto:x@example.test",
        "https://",
        "not a url",
    ] {
        assert_eq!(HttpsUrl::parse(refused), None, "case: {refused}");
    }
}
