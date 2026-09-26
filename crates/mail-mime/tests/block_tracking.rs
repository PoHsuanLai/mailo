//! Links lose the query parameters that only tell a sender who clicked, and nothing else.

#[path = "block/mod.rs"]
mod support;

use mail_mime::{ImgSrc, RemoteImages, SafeUrl};
use support::{html, html_images, sketch};

/// `(case, href as sent, href the reader keeps)`.
const CASES: &[(&str, &str, &str)] = &[
    (
        "no query",
        "https://shop.example.test/autumn",
        "https://shop.example.test/autumn",
    ),
    (
        "an unknown parameter stays",
        "https://shop.example.test/item?id=7",
        "https://shop.example.test/item?id=7",
    ),
    (
        "only tracking: the query goes, with its ?",
        "https://shop.example.test/item?utm_source=letter&utm_medium=email&utm_campaign=autumn",
        "https://shop.example.test/item",
    ),
    (
        "one tracking parameter alone",
        "http://shop.example.test/item?fbclid=IwAR0abc",
        "http://shop.example.test/item",
    ),
    (
        "mixed: the others stay, in their order",
        "https://shop.example.test/list?utm_term=wool&page=2&gclid=Cj0K&sort=asc&mc_eid=f00d",
        "https://shop.example.test/list?page=2&sort=asc",
    ),
    (
        "every named family",
        "https://shop.example.test/?utm_id=1&utm_content=a&utm_source_platform=b\
         &utm_creative_format=c&utm_marketing_tactic=d&gclsrc=aw.ds&gbraid=1&wbraid=2&dclid=3\
         &msclkid=4&yclid=5&ttclid=6&twclid=7&li_fat_id=8&mc_cid=9&_hsenc=p2&_hsmi=10\
         &__hstc=11&__hssc=12&__hsfp=13&mkt_tok=14&keep=yes",
        "https://shop.example.test/?keep=yes",
    ),
    (
        "a fragment stays after a removed query",
        "https://shop.example.test/item?utm_source=letter#reviews",
        "https://shop.example.test/item#reviews",
    ),
    (
        "a fragment stays after a kept query",
        "https://shop.example.test/item?id=7&utm_source=letter#reviews",
        "https://shop.example.test/item?id=7#reviews",
    ),
    (
        "a fragment is not a query and is never touched",
        "https://shop.example.test/app#/item?utm_source=letter",
        "https://shop.example.test/app#/item?utm_source=letter",
    ),
    (
        "a percent-encoded key is compared decoded",
        "https://shop.example.test/item?utm%5Fsource=letter&id=7",
        "https://shop.example.test/item?id=7",
    ),
    (
        "a key encoded letter by letter",
        "https://shop.example.test/item?%66%62%63%6C%69%64=x&id=7",
        "https://shop.example.test/item?id=7",
    ),
    (
        "a percent-encoded key that decodes to something else stays",
        "https://shop.example.test/item?utm%5Fsources=letter",
        "https://shop.example.test/item?utm%5Fsources=letter",
    ),
    (
        "a key with no value",
        "https://shop.example.test/item?fbclid&id=7",
        "https://shop.example.test/item?id=7",
    ),
    (
        "a key with an empty value",
        "https://shop.example.test/item?id=7&gclid=",
        "https://shop.example.test/item?id=7",
    ),
    (
        "keys compare case-sensitively: an unknown spelling stays",
        "https://shop.example.test/item?UTM_SOURCE=letter&Fbclid=x",
        "https://shop.example.test/item?UTM_SOURCE=letter&Fbclid=x",
    ),
    (
        "a longer key that starts like one stays",
        "https://shop.example.test/item?utm_sourcery=1&fbclids=2&gclid_x=3",
        "https://shop.example.test/item?utm_sourcery=1&fbclids=2&gclid_x=3",
    ),
    (
        "an unlisted utm_ key stays",
        "https://shop.example.test/item?utm_foo=1&utm_=2",
        "https://shop.example.test/item?utm_foo=1&utm_=2",
    ),
    (
        "a tracking name as a value stays",
        "https://shop.example.test/search?q=utm_source&field=fbclid",
        "https://shop.example.test/search?q=utm_source&field=fbclid",
    ),
    (
        "a piece holding ; is kept: what follows may be a parameter",
        "https://shop.example.test/item?utm_source=letter;id=7",
        "https://shop.example.test/item?utm_source=letter;id=7",
    ),
    (
        "kept values keep their encoding",
        "https://shop.example.test/search?q=wool+socks&c=a%20b&utm_medium=email&x=%2F",
        "https://shop.example.test/search?q=wool+socks&c=a%20b&x=%2F",
    ),
    (
        "a gap a removed parameter leaves is closed",
        "https://shop.example.test/item?id=7&&utm_source=letter&",
        "https://shop.example.test/item?id=7",
    ),
    (
        "a query with no tracking is not rewritten, gaps and all",
        "https://shop.example.test/item?id=7&&size=m&",
        "https://shop.example.test/item?id=7&&size=m&",
    ),
    (
        "an empty query is not a tracking one",
        "https://shop.example.test/item?",
        "https://shop.example.test/item?",
    ),
    (
        "a redirect wrapper is not unwrapped: its target stays exactly as sent",
        "https://click.example.test/r?u=https%3A%2F%2Fshop.example.test%2F%3Futm_source%3Dletter&sig=ab",
        "https://click.example.test/r?u=https%3A%2F%2Fshop.example.test%2F%3Futm_source%3Dletter&sig=ab",
    ),
    (
        "a redirect wrapper's own tracking parameter comes off, its target does not",
        "https://click.example.test/r?u=https%3A%2F%2Fshop.example.test%2F&utm_source=letter",
        "https://click.example.test/r?u=https%3A%2F%2Fshop.example.test%2F",
    ),
    (
        "a mailto query is the message and stays",
        "mailto:orders@shop.example.test?subject=Order&utm_source=letter",
        "mailto:orders@shop.example.test?subject=Order&utm_source=letter",
    ),
];

#[test]
fn links_lose_tracking_parameters_and_keep_everything_else() {
    let mut failures = Vec::new();
    for (case, sent, kept) in CASES {
        let got = SafeUrl::link(sent).map(|url| url.as_str().to_owned());
        if got.as_deref() != Some(*kept) {
            failures.push(format!(
                "{case}:\n  sent {sent}\n  got  {got:?}\n  want {kept}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A cleaned link is a stable value: cleaning it again, or parsing it again, changes nothing.
#[test]
fn a_cleaned_link_is_already_canonical() {
    for (case, sent, _) in CASES {
        let once = SafeUrl::link(sent).expect("every case is a link");
        assert_eq!(SafeUrl::link(once.as_str()), Some(once.clone()), "{case}");
        assert_eq!(SafeUrl::parse(once.as_str()), Some(once.clone()), "{case}");
    }
}

/// A link refused by `parse` is refused by `link`: cleaning never makes a URL acceptable.
#[test]
fn link_refuses_what_parse_refuses() {
    for raw in [
        "javascript:alert(1)?utm_source=x",
        "data:text/html,hi?utm_source=x",
        "file:///etc/passwd?utm_source=x",
        "https://shop.example.test/\u{202E}?utm_source=x",
        "https://?utm_source=x",
        "",
    ] {
        assert_eq!(SafeUrl::parse(raw), None, "{raw:?}");
        assert_eq!(SafeUrl::link(raw), None, "{raw:?}");
    }
}

/// The reader's links come from the HTML part through `link`, so the document holds the cleaned
/// address. The same address as an image's source is not a link and is left as sent.
#[test]
fn an_html_part_holds_its_links_cleaned_and_its_images_as_sent() {
    let doc = html(
        r#"<p>See <a href="https://shop.example.test/item?id=7&amp;utm_source=letter&amp;fbclid=x">the item</a>.</p>"#,
    );
    let urls: Vec<String> = support::urls(&doc)
        .iter()
        .map(|url| url.as_str().to_owned())
        .collect();
    assert_eq!(
        urls,
        vec!["https://shop.example.test/item?id=7".to_owned()],
        "{}",
        sketch(&doc)
    );

    let image = "https://images.example.test/hero.png?utm_source=letter";
    let doc = html_images(
        &format!(r#"<p><img src="{image}" alt="hero"></p>"#),
        &[],
        RemoteImages::Allowed,
    );
    let sources: Vec<String> = support::images(&doc)
        .into_iter()
        .filter_map(|src| match src {
            ImgSrc::Remote(url) => Some(url.as_str().to_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(sources, vec![image.to_owned()], "{}", sketch(&doc));
}
