//! The spoof table, the link table, and the registered domain they both stand on.

use super::{Destination, Spoof, destination, registered, spoof};

#[test]
fn the_registered_domain_reads_the_public_suffix_list() {
    const CASES: &[(&str, Option<&str>)] = &[
        ("example.com", Some("example.com")),
        ("mail.example.com", Some("example.com")),
        ("MAIL.Example.COM.", Some("example.com")),
        // The naive last-two-labels rule would say `co.uk` for both of these.
        ("mail.example.co.uk", Some("example.co.uk")),
        ("a.b.example.co.uk", Some("example.co.uk")),
        ("co.uk", None),
        ("com", None),
        ("", None),
        ("192.0.2.1", None),
        ("::1", None),
        ("bad..example.com", None),
    ];
    for (host, want) in CASES {
        assert_eq!(registered(host).as_deref(), *want, "host {host:?}");
    }
}

#[test]
fn a_brand_in_the_name_is_checked_against_the_brands_domains() {
    /// (display name, address, the flag as brand and domain)
    type Case = (
        Option<&'static str>,
        &'static str,
        Option<(&'static str, &'static str)>,
    );
    const CASES: &[Case] = &[
        // The brand's own domains, and their subdomains, are genuine.
        (Some("Google"), "no-reply@accounts.google.com", None),
        (Some("Google"), "someone@gmail.com", None),
        (Some("PayPal"), "service@paypal.co.uk", None),
        (Some("Amazon.com"), "ship-confirm@amazon.com", None),
        (Some("Amazon"), "orders@mail.amazon.co.uk", None),
        (
            Some("Microsoft account team"),
            "account-security@microsoft.com",
            None,
        ),
        // Off the list: the flag names the brand and where the mail really came from.
        (
            Some("Google"),
            "security@g00gle-security.xyz",
            Some(("Google", "g00gle-security.xyz")),
        ),
        (
            Some("Google Security"),
            "alert@mail.google.com.evil.example",
            Some(("Google", "evil.example")),
        ),
        (
            Some("PayPal Service"),
            "service@evil-paypal.com",
            Some(("PayPal", "evil-paypal.com")),
        ),
        (
            Some("Apple Support"),
            "id@apple.co.uk.example.com",
            Some(("Apple", "example.com")),
        ),
        (
            Some("Amazon"),
            "orders@amazon.shop.co.uk",
            Some(("Amazon", "shop.co.uk")),
        ),
        // No brand named, as a whole word: nothing to check.
        (Some("Googleplex Cafe"), "hello@cafe.example", None),
        (Some("Dana Okafor"), "dana@example.org", None),
        (None, "security@g00gle-security.xyz", None),
    ];
    for (name, email, want) in CASES {
        let got = spoof(*name, email);
        let want = want.map(|(brand, domain)| Spoof {
            brand,
            domain: domain.to_owned(),
        });
        assert_eq!(got, want, "{name:?} <{email}>");
    }
}

#[test]
fn a_link_is_checked_against_the_domain_its_text_names() {
    fn web(sub: &str, reg: &str, path: &str) -> Destination {
        Destination::Web {
            scheme: "https://".to_owned(),
            sub: sub.to_owned(),
            registered: reg.to_owned(),
            path: path.to_owned(),
        }
    }
    fn lies(goes: &str, claims: &str) -> Destination {
        Destination::Lies {
            goes_to: goes.to_owned(),
            claims: claims.to_owned(),
        }
    }
    let cases: Vec<(&str, &str, Destination)> = vec![
        // The text names the same registered domain, a subdomain of it included.
        (
            "www.rfc-editor.org",
            "https://www.rfc-editor.org/rfc/rfc1939",
            web("www.", "rfc-editor.org", "/rfc/rfc1939"),
        ),
        (
            "rfc-editor.org",
            "https://datatracker.rfc-editor.org/doc?x=1",
            web("datatracker.", "rfc-editor.org", "/doc"),
        ),
        (
            "Your order at amazon.co.uk",
            "https://www.amazon.co.uk/orders",
            web("www.", "amazon.co.uk", "/orders"),
        ),
        // It names a different one.
        (
            "google.com",
            "https://g00gle-security.xyz/verify",
            lies("g00gle-security.xyz", "google.com"),
        ),
        (
            "https://accounts.google.com/signin",
            "https://accounts.google.com.evil.example/signin",
            lies("evil.example", "google.com"),
        ),
        (
            "paypal.com",
            "https://paypal.com@evil.example/login",
            lies("evil.example", "paypal.com"),
        ),
        // The text is not a domain at all.
        (
            "RFC 1939, section 7",
            "https://www.rfc-editor.org/rfc/rfc1939#section-7",
            web("www.", "rfc-editor.org", "/rfc/rfc1939"),
        ),
        (
            "click here",
            "https://example.com/",
            web("", "example.com", "/"),
        ),
        (
            "notes.txt",
            "https://example.com/n",
            web("", "example.com", "/n"),
        ),
        (
            "mail me",
            "mailto:dana@example.org",
            Destination::Other("mailto:dana@example.org".to_owned()),
        ),
        (
            "router",
            "http://192.0.2.1/admin",
            Destination::Other("http://192.0.2.1/admin".to_owned()),
        ),
    ];
    for (text, href, want) in cases {
        assert_eq!(destination(text, href), want, "{text:?} → {href}");
    }
}
