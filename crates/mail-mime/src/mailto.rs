//! A `mailto:` URI (RFC 6068), read as the message it asks to have written.
//!
//! Pure: this turns the text of a link into recipients, a subject and a body. Opening a composer
//! on them is the app's; nothing here sends, and nothing a link says can make anything send.
//!
//! Only `to`, `cc`, `bcc`, `subject` and `body` are read. Every other header field a URI may
//! carry is dropped, as RFC 6068 §3 and §7 advise: a link is written by a stranger, and headers
//! such as `In-Reply-To`, `From` or anything naming a file are not the stranger's to set.

use mail_domain::Address;

/// What a `mailto:` URI asks for.
///
/// Any field may be empty: `mailto:?subject=hello` is a well-formed link that names nobody, and
/// the person fills in the rest.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MailtoUri {
    /// The URI's path, then every `to` field, in order.
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    /// One line: a line break in the URI becomes a space, since a subject is one header.
    pub subject: String,
    /// Line breaks as `\n`, whatever the URI encoded them as.
    pub body: String,
}

impl MailtoUri {
    /// Read `uri`, which must start with the `mailto:` scheme (in any case). `None` when it is
    /// some other scheme or no URI at all.
    pub fn parse(uri: &str) -> Option<Self> {
        let (scheme, rest) = uri.trim().split_once(':')?;
        scheme
            .eq_ignore_ascii_case("mailto")
            .then(|| Self::after_scheme(rest))
    }

    /// Read the part of a `mailto:` URI after the scheme (RFC 6068 §2).
    ///
    /// A field that appears twice: the address lists gather every occurrence, the subject and
    /// body keep the first, so a second one cannot quietly replace what the first said.
    pub(crate) fn after_scheme(rest: &str) -> Self {
        // A fragment is not part of a `mailto:` (RFC 6068 §2 has none); drop it rather than
        // write it into the last field.
        let rest = rest.split_once('#').map_or(rest, |(before, _)| before);
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let mut out = Self {
            to: recipients(path),
            ..Self::default()
        };
        let (mut subject, mut body) = (None, None);
        for field in query.split('&') {
            let Some((name, value)) = field.split_once('=') else {
                continue;
            };
            match percent_decode(name).to_ascii_lowercase().as_str() {
                "to" => out.to.extend(recipients(value)),
                "cc" => out.cc.extend(recipients(value)),
                "bcc" => out.bcc.extend(recipients(value)),
                "subject" if subject.is_none() => subject = Some(percent_decode(value)),
                "body" if body.is_none() => body = Some(percent_decode(value)),
                _ => {}
            }
        }
        out.subject = one_line(&subject.unwrap_or_default());
        out.body = body
            .unwrap_or_default()
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        out
    }
}

/// A subject's line breaks, and any other control character, as spaces; a CRLF is one space.
fn one_line(text: &str) -> String {
    text.replace("\r\n", " ")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// A comma-separated, percent-encoded list of addr-specs, keeping only the plausible ones.
fn recipients(encoded: &str) -> Vec<Address> {
    encoded
        .split(',')
        .map(percent_decode)
        .filter_map(|email| {
            let email = email.trim();
            plausible(email).then(|| Address {
                name: None,
                email: email.to_owned(),
            })
        })
        .collect()
}

/// `local@domain`, both halves non-empty, nothing that would let it become a second header or
/// a second recipient once it is written into a message.
fn plausible(email: &str) -> bool {
    let Some((local, domain)) = email.rsplit_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !email
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';'))
}

/// `%XX` decoded as UTF-8, with anything undecodable replaced rather than refused.
///
/// Not `application/x-www-form-urlencoded`: RFC 6068 gives `+` no meaning, so it stays a `+`,
/// which matters in the local part of an address.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to(emails: &[&str]) -> Vec<Address> {
        emails
            .iter()
            .map(|email| Address {
                name: None,
                email: (*email).to_owned(),
            })
            .collect()
    }

    struct Case {
        uri: &'static str,
        to: &'static [&'static str],
        cc: &'static [&'static str],
        bcc: &'static [&'static str],
        subject: &'static str,
        body: &'static str,
    }

    const NONE: &[&str] = &[];

    /// RFC 6068 §6.1 and §6.2's examples, with the example addresses they use, and the edges
    /// around them.
    const CASES: &[Case] = &[
        Case {
            uri: "mailto:chris@example.com",
            to: &["chris@example.com"],
            cc: NONE,
            bcc: NONE,
            subject: "",
            body: "",
        },
        Case {
            uri: "mailto:infobot@example.com?subject=current-issue",
            to: &["infobot@example.com"],
            cc: NONE,
            bcc: NONE,
            subject: "current-issue",
            body: "",
        },
        Case {
            uri: "mailto:infobot@example.com?body=send%20current-issue%0D%0Asend%20index",
            to: &["infobot@example.com"],
            cc: NONE,
            bcc: NONE,
            subject: "",
            body: "send current-issue\nsend index",
        },
        Case {
            uri: "mailto:joe@example.com?cc=bob@example.com&body=hello",
            to: &["joe@example.com"],
            cc: &["bob@example.com"],
            bcc: NONE,
            subject: "",
            body: "hello",
        },
        Case {
            uri: "mailto:?to=joe@example.com&cc=bob@example.com&body=hello",
            to: &["joe@example.com"],
            cc: &["bob@example.com"],
            bcc: NONE,
            subject: "",
            body: "hello",
        },
        Case {
            uri: "mailto:addr1@an.example,addr2@an.example",
            to: &["addr1@an.example", "addr2@an.example"],
            cc: NONE,
            bcc: NONE,
            subject: "",
            body: "",
        },
        Case {
            uri: "mailto:?to=addr1@an.example,addr2@an.example",
            to: &["addr1@an.example", "addr2@an.example"],
            cc: NONE,
            bcc: NONE,
            subject: "",
            body: "",
        },
        Case {
            uri: "mailto:addr1@an.example?to=addr2@an.example&bcc=hidden@an.example",
            to: &["addr1@an.example", "addr2@an.example"],
            cc: NONE,
            bcc: &["hidden@an.example"],
            subject: "",
            body: "",
        },
        // `%25` is a literal percent, and `+` stays a `+` (RFC 6068 §2, §6.1).
        Case {
            uri: "mailto:%22not%40me%22@example.org",
            to: &["\"not@me\"@example.org"],
            cc: NONE,
            bcc: NONE,
            subject: "",
            body: "",
        },
        Case {
            uri: "mailto:user+mailto@example.org?subject=100%25+sure",
            to: &["user+mailto@example.org"],
            cc: NONE,
            bcc: NONE,
            subject: "100%+sure",
            body: "",
        },
        // Scheme and field names in any case; UTF-8 in percent escapes.
        Case {
            uri: "MAILTO:user@example.org?SUBJECT=caf%C3%A9&Body=%E2%98%95",
            to: &["user@example.org"],
            cc: NONE,
            bcc: NONE,
            subject: "café",
            body: "☕",
        },
        // A link that names nobody is still a composer to open.
        Case {
            uri: "mailto:?subject=hello",
            to: NONE,
            cc: NONE,
            bcc: NONE,
            subject: "hello",
            body: "",
        },
        Case {
            uri: "mailto:",
            to: NONE,
            cc: NONE,
            bcc: NONE,
            subject: "",
            body: "",
        },
        // A second subject does not replace the first; a line break in a subject cannot start
        // a header of its own.
        Case {
            uri: "mailto:a@example.org?subject=one%0D%0ABcc:%20x@example.org&subject=two",
            to: &["a@example.org"],
            cc: NONE,
            bcc: NONE,
            subject: "one Bcc: x@example.org",
            body: "",
        },
        // Fields this client does not set from a link are dropped, and so is a fragment.
        Case {
            uri: "mailto:a@example.org?in-reply-to=%3C1@x%3E&from=boss@example.org&attach=/etc/passwd#frag",
            to: &["a@example.org"],
            cc: NONE,
            bcc: NONE,
            subject: "",
            body: "",
        },
        // An address that could smuggle a second one, or is not an address, is dropped.
        Case {
            uri: "mailto:a@example.org%0D%0ABcc:b@example.org,not-an-address,%3Cc@example.org%3E",
            to: NONE,
            cc: NONE,
            bcc: NONE,
            subject: "",
            body: "",
        },
    ];

    #[test]
    fn a_mailto_uri_reads_as_the_message_it_asks_for() {
        for case in CASES {
            let got = MailtoUri::parse(case.uri).unwrap_or_else(|| panic!("{}: refused", case.uri));
            let want = MailtoUri {
                to: to(case.to),
                cc: to(case.cc),
                bcc: to(case.bcc),
                subject: case.subject.to_owned(),
                body: case.body.to_owned(),
            };
            assert_eq!(got, want, "case {}", case.uri);
        }
    }

    #[test]
    fn only_the_mailto_scheme_is_read() {
        for uri in [
            "https://example.org/",
            "mail:to@example.org",
            "a@example.org",
            "",
        ] {
            assert_eq!(MailtoUri::parse(uri), None, "case {uri:?}");
        }
    }

    #[test]
    fn percent_decoding_leaves_plus_and_broken_escapes_alone() {
        const CASES: &[(&str, &str)] = &[
            ("a%20b", "a b"),
            ("a+b", "a+b"),
            ("100%", "100%"),
            ("%zz", "%zz"),
            ("caf%C3%A9", "café"),
            ("%0D%0A", "\r\n"),
        ];
        for (input, expected) in CASES {
            assert_eq!(percent_decode(input), *expected, "case {input:?}");
        }
    }
}
