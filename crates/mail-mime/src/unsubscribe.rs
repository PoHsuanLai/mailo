//! What a list message says about leaving the list: `List-Unsubscribe` (RFC 2369),
//! `List-Unsubscribe-Post` (RFC 8058) and `List-Id` (RFC 2919).
//!
//! Pure: this reads headers and says what could be done. Doing it — an HTTPS `POST`, or a
//! message queued for sending — belongs to the runtime and the app. A link that is only a web
//! page is reported here and never fetched anywhere: a `GET` on an unsubscribe link is exactly
//! what a mail scanner does by accident, and exactly what a sender can use to confirm that an
//! address is read.

use crate::mailto::MailtoUri;
use mail_domain::Address;
use mail_parser::{HeaderName, MessageParser};

/// Everything a message's list headers say.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListHeaders {
    /// `List-Id`, when the message names the list it came through.
    pub id: Option<ListId>,
    /// The ways out this message offers, in the order its `List-Unsubscribe` gave them.
    /// Empty when it offers none that this client can use.
    pub unsubscribe: Vec<Unsubscribe>,
}

/// `List-Id: Description <list.example.test>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListId {
    /// The phrase before the id, decoded. `None` when the sender gave only the id.
    pub description: Option<String>,
    /// The id itself, without its angle brackets.
    pub id: String,
}

/// One way to leave a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsubscribe {
    /// RFC 8058: an HTTPS `POST` of `List-Unsubscribe=One-Click` to `url` does it, with no page
    /// to visit and nothing to confirm. Only offered when the message carries
    /// `List-Unsubscribe-Post: List-Unsubscribe=One-Click` as well as an `https:` URI.
    OneClick { url: HttpsUrl },
    /// A message to send. The list's software reads it and takes the sender off.
    Mailto(Mailto),
    /// A page the person may open themselves. Shown, never fetched: see the module docs.
    Web { url: String },
}

/// An absolute `https:` URL with a host and no credentials in it.
///
/// The only thing the one-click `POST` accepts, so a plain `http:` target is a type error
/// rather than a check someone can forget. Credentials are refused because RFC 8058 forbids
/// the request from carrying any, and a URL with `user:pass@` would make the HTTP client send
/// them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HttpsUrl(String);

impl HttpsUrl {
    /// `raw`, if it is an `https:` URL this client will `POST` to.
    pub fn parse(raw: &str) -> Option<HttpsUrl> {
        let url = url::Url::parse(raw).ok()?;
        let usable = url.scheme() == "https"
            && url.host_str().is_some_and(|host| !host.is_empty())
            && url.username().is_empty()
            && url.password().is_none();
        usable.then(|| HttpsUrl(url.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HttpsUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A `mailto:` URI (RFC 6068), reduced to the message it asks for.
///
/// `cc` and `bcc` in the URI are dropped on purpose: leaving a list is a message to the list's
/// software, and a URI that asks for a copy to go to someone else is asking this client to
/// write to a stranger on the user's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailto {
    /// Never empty: a `mailto:` with nobody to send to is not offered at all.
    pub to: Vec<Address>,
    pub subject: String,
    /// Line breaks as `\n`, whatever the URI encoded them as.
    pub body: String,
}

impl ListHeaders {
    /// The way out to take, when there is a choice: one-click, then a message, then a page.
    ///
    /// One-click first because it is the only one that is finished when it returns. A message
    /// is next because it is still done by this client, where a page needs the person.
    pub fn preferred(&self) -> Option<&Unsubscribe> {
        let rank = |method: &Unsubscribe| match method {
            Unsubscribe::OneClick { .. } => 0,
            Unsubscribe::Mailto(_) => 1,
            Unsubscribe::Web { .. } => 2,
        };
        // `min_by_key` keeps the first of equals, so header order breaks ties.
        self.unsubscribe.iter().min_by_key(|method| rank(method))
    }
}

/// The value RFC 8058 requires of `List-Unsubscribe-Post`, and the body the `POST` carries.
pub const ONE_CLICK: &str = "List-Unsubscribe=One-Click";

/// Read the list headers of a whole message.
///
/// Never fails. A message with no list headers, or with ones that are malformed, has no way
/// out to offer, which is an empty [`ListHeaders`] rather than an error.
pub fn list_headers(raw: &[u8]) -> ListHeaders {
    // Only these three headers, as unstructured text: that unfolds them and decodes RFC 2047
    // encoded words, which some senders apply to the whole field. The body is not read.
    let parser = MessageParser::new()
        .header_text(HeaderName::ListUnsubscribe)
        .header_text(HeaderName::ListUnsubscribePost)
        .header_text(HeaderName::ListId)
        .default_header_ignore();
    let Some(message) = parser.parse_headers(raw) else {
        return ListHeaders::default();
    };
    let values = |name: HeaderName<'static>| -> Vec<String> {
        message
            .header_values(name)
            .filter_map(|value| value.as_text().map(str::to_owned))
            .collect()
    };
    let one_click = values(HeaderName::ListUnsubscribePost)
        .iter()
        .any(|value| value.trim() == ONE_CLICK);
    let uris: Vec<String> = values(HeaderName::ListUnsubscribe)
        .iter()
        .flat_map(|value| uris_of(value))
        .collect();
    ListHeaders {
        id: values(HeaderName::ListId)
            .first()
            .and_then(|value| list_id(value)),
        unsubscribe: methods(&uris, one_click),
    }
}

/// The methods `uris` offer, in order, duplicates removed.
///
/// With one-click on, the first `https:` URI is the one-click target (RFC 8058 §3.1 requires
/// exactly one); any other web link is still offered as a page.
fn methods(uris: &[String], one_click: bool) -> Vec<Unsubscribe> {
    let mut out: Vec<Unsubscribe> = Vec::new();
    let mut claimed = !one_click;
    for uri in uris {
        let Some(method) = method_of(uri, &mut claimed) else {
            continue;
        };
        if !out.contains(&method) {
            out.push(method);
        }
    }
    out
}

/// What one URI offers. `claimed` is whether the one-click slot is already taken.
fn method_of(uri: &str, claimed: &mut bool) -> Option<Unsubscribe> {
    let (scheme, rest) = uri.split_once(':')?;
    match scheme.to_ascii_lowercase().as_str() {
        "mailto" => mailto(rest).map(Unsubscribe::Mailto),
        "https" => {
            if !*claimed && let Some(url) = HttpsUrl::parse(uri) {
                *claimed = true;
                return Some(Unsubscribe::OneClick { url });
            }
            web(uri)
        }
        "http" => web(uri),
        // `ftp:`, `javascript:`, `data:`, and anything else nobody should act on.
        _ => None,
    }
}

fn web(uri: &str) -> Option<Unsubscribe> {
    let url = url::Url::parse(uri).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return None;
    }
    Some(Unsubscribe::Web { url: url.into() })
}

/// The URIs in one `List-Unsubscribe` value, in order.
///
/// RFC 2369 puts each in angle brackets and says whitespace inside them is to be ignored, which
/// is what lets a long URL be folded. Text outside the brackets is comment. A value with no
/// brackets at all is read as a bare comma-separated list, which some senders write.
fn uris_of(value: &str) -> Vec<String> {
    let strip = |text: &str| -> String { text.chars().filter(|c| !c.is_whitespace()).collect() };
    if !value.contains('<') {
        return value
            .split(',')
            .map(strip)
            .filter(|uri| !uri.is_empty())
            .collect();
    }
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            break;
        };
        let uri = strip(&after[..close]);
        if !uri.is_empty() {
            out.push(uri);
        }
        rest = &after[close + 1..];
    }
    out
}

/// The part of a `mailto:` URI after the scheme, as a message (RFC 6068 §2), when it names
/// somebody to send it to. Its `cc` and `bcc` are dropped (see [`Mailto`]).
fn mailto(rest: &str) -> Option<Mailto> {
    let read = MailtoUri::after_scheme(rest);
    if read.to.is_empty() {
        return None;
    }
    Some(Mailto {
        to: read.to,
        subject: read.subject,
        body: read.body,
    })
}

/// `Description <id>` (RFC 2919 §2). The id is required; the description is not.
fn list_id(value: &str) -> Option<ListId> {
    let open = value.rfind('<')?;
    let close = open + value[open..].find('>')?;
    let id: String = value[open + 1..close]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if id.is_empty() {
        return None;
    }
    let description = value[..open].trim().trim_matches('"').trim().to_owned();
    Some(ListId {
        description: (!description.is_empty()).then_some(description),
        id,
    })
}
