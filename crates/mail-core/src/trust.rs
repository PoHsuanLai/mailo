//! What a name or a link claims, against where it really goes.
//!
//! Two checks, both on the **registered domain**: the part of a host someone registered, one
//! label under its public suffix. `mail.example.co.uk` is registered as `example.co.uk`, not
//! `co.uk`, which is why this reads the Public Suffix List (through `psl`) instead of taking
//! the last two labels — the naive rule would call every British sender the same one.
//!
//! Domains are tokens, compared whole (CONVENTIONS, "Substrings are not tokens"): a brand's
//! domain matches only when the registered domains are equal, so `evil-paypal.com` and
//! `paypal.com.evil.example` are both somebody else.

/// The registered domain of `host`, lower-cased. `None` for an IP literal, a bare suffix, or a
/// name that is not ASCII (a Unicode host arrives here already in its `xn--` form or not at all).
pub fn registered(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || !host.is_ascii() || host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    if host.split('.').any(|label| label.is_empty()) {
        return None;
    }
    psl::domain_str(&host).map(str::to_owned)
}

/// Whether `host` ends in a suffix the list actually knows. `note.txt` is not a domain.
fn known_suffix(host: &str) -> bool {
    psl::suffix(host.as_bytes()).is_some_and(|suffix| suffix.is_known())
}

/// A name phishing borrows, and the registered domains that really are theirs.
///
/// Small, fixed and public, on purpose: the check is "this display name names a company whose
/// mail comes from a short list of domains", and a list that grew to guess at every company
/// would turn the flag into noise.
pub const BRANDS: &[(&str, &str, &[&str])] = &[
    (
        "google",
        "Google",
        &["google.com", "gmail.com", "googlemail.com", "youtube.com"],
    ),
    (
        "gmail",
        "Google",
        &["google.com", "gmail.com", "googlemail.com"],
    ),
    (
        "microsoft",
        "Microsoft",
        &[
            "microsoft.com",
            "outlook.com",
            "office.com",
            "office365.com",
            "live.com",
            "hotmail.com",
            "microsoftonline.com",
        ],
    ),
    (
        "outlook",
        "Microsoft",
        &["microsoft.com", "outlook.com", "office.com", "live.com"],
    ),
    ("apple", "Apple", &["apple.com", "icloud.com", "me.com"]),
    ("icloud", "Apple", &["apple.com", "icloud.com", "me.com"]),
    (
        "paypal",
        "PayPal",
        &["paypal.com", "paypal.co.uk", "paypal.de", "paypal.fr"],
    ),
    (
        "amazon",
        "Amazon",
        &[
            "amazon.com",
            "amazon.co.uk",
            "amazon.de",
            "amazon.fr",
            "amazon.co.jp",
            "amazon.ca",
            "amazonses.com",
        ],
    ),
    ("netflix", "Netflix", &["netflix.com"]),
    (
        "facebook",
        "Meta",
        &["facebook.com", "facebookmail.com", "meta.com"],
    ),
    (
        "instagram",
        "Meta",
        &["instagram.com", "facebookmail.com", "meta.com"],
    ),
    ("dhl", "DHL", &["dhl.com", "dhl.de"]),
];

/// The display name names a brand, and the address is not one of theirs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spoof {
    /// The brand as it is written: "Google".
    pub brand: &'static str,
    /// Where the mail really came from: the address's registered domain, or its whole domain
    /// when it has none.
    pub domain: String,
}

/// Check a sender. `None` when the name names no brand, or when it does and the address is
/// really theirs, subdomains included.
pub fn spoof(display: Option<&str>, email: &str) -> Option<Spoof> {
    let name = display?;
    let words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let (_, brand, domains) = BRANDS
        .iter()
        .find(|(word, _, _)| words.iter().any(|w| w == word))?;
    let host = email.rsplit_once('@').map_or("", |(_, host)| host);
    let theirs = registered(host);
    let genuine = theirs
        .as_deref()
        .is_some_and(|domain| domains.contains(&domain));
    (!genuine).then(|| Spoof {
        brand,
        domain: theirs.unwrap_or_else(|| host.to_ascii_lowercase()),
    })
}

/// Where a link goes, as the pill shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// `https://` + `sub.` + **registered** + path.
    Web {
        scheme: String,
        sub: String,
        registered: String,
        path: String,
    },
    /// The link's text names one registered domain and the link goes to another.
    Lies { goes_to: String, claims: String },
    /// Not a web address with a registered domain: `mailto:`, an IP, a bare suffix.
    Other(String),
}

/// Read a link: its target, and whether its visible text names somewhere else.
pub fn destination(text: &str, href: &str) -> Destination {
    let Some((scheme, host, path)) = split_url(href) else {
        return Destination::Other(href.to_owned());
    };
    let Some(reg) = registered(&host) else {
        return Destination::Other(href.to_owned());
    };
    if let Some(claims) = named_domain(text)
        && claims != reg
    {
        return Destination::Lies {
            goes_to: reg,
            claims,
        };
    }
    let sub = host[..host.len() - reg.len()].to_owned();
    Destination::Web {
        scheme: format!("{scheme}://"),
        sub,
        registered: reg,
        path,
    }
}

/// `(scheme, host, path)` of an `http` or `https` URL. The host is lower-cased, and the
/// userinfo is dropped: in `https://paypal.com@evil.example/` the host is `evil.example`.
fn split_url(href: &str) -> Option<(String, String, String)> {
    let (scheme, rest) = href.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = match host.strip_prefix('[') {
        // An IPv6 literal has no registered domain; say so by returning its text.
        Some(_) => host,
        None => host.split(':').next().unwrap_or(host),
    };
    let path = path.split(['?', '#']).next().unwrap_or("").to_owned();
    Some((scheme, host.to_ascii_lowercase(), path))
}

/// The registered domain the text of a link names, if it names one.
///
/// A word that looks like a host — labels and dots, ending in a suffix the list knows — after
/// any scheme and before any path. "Click here" names none; "g00gle.com/verify" names
/// `g00gle.com`.
fn named_domain(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '(' | ')' | ','))
        .filter_map(|word| {
            let word = word.trim_matches(|c: char| matches!(c, '.' | ':' | ';' | '!' | '?'));
            let word = word
                .split_once("://")
                .map_or(word, |(_, rest)| rest)
                .split(['/', '?', '#'])
                .next()
                .unwrap_or("");
            let word = word.rsplit_once('@').map_or(word, |(_, host)| host);
            let looks = word.contains('.')
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
            (looks && known_suffix(&word.to_ascii_lowercase()))
                .then(|| registered(word))
                .flatten()
        })
        .next()
}

#[cfg(test)]
mod tests;
