//! Query parameters that exist only to tell a sender who clicked.
//!
//! A link in a newsletter carries its destination and, beside it, a campaign name, an ad
//! network's click id or the recipient's own mailing-list id. The page opens the same without
//! them. They are taken off before the reader shows the link or opens it, so the pill shows where
//! the link goes and the browser does not hand the sender a receipt.
//!
//! Only parameters named in [`TRACKING`] come off, compared whole and case-sensitively, so an
//! unknown parameter is never removed. A redirect wrapper — a click tracker's own URL with the real
//! destination in its query — is left exactly as it is: unwrapping it would change where the link
//! goes. Only the query of an `http`/`https` URL is touched; a `mailto:` query is the message's
//! subject and body. The fragment is never touched.
//!
//! Pieces are split on `&` only, as the WHATWG URL standard's `application/x-www-form-urlencoded`
//! parser does. Some servers also split on `;` (HTML 4.01 §B.2.2 suggested it), so a tracking
//! piece whose value holds a `;` is kept whole: what follows the `;` may be a parameter this
//! module does not know. Each kept piece stays byte for byte as the sender wrote it:
//! re-serializing the query would change `%20` into `+` and move a link somewhere else.

use ammonia::Url;

/// The parameters removed, each a whole key. A comment names who defines it and what it carries.
pub(crate) const TRACKING: &[&str] = &[
    // Google Analytics campaign parameters: which campaign, source and medium sent the click.
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    // Google Analytics 4 campaign parameters: the campaign's id, platform, format and tactic.
    "utm_id",
    "utm_source_platform",
    "utm_creative_format",
    "utm_marketing_tactic",
    // Google Ads click identifier, set by auto-tagging.
    "gclid",
    // Google Ads: which kind of Google Ads source set the `gclid`.
    "gclsrc",
    // Google Ads click identifiers for app and web conversions on iOS.
    "gbraid",
    "wbraid",
    // Google Display & Video 360 / Campaign Manager click identifier.
    "dclid",
    // Meta (Facebook) click identifier, added to outbound links.
    "fbclid",
    // Microsoft Advertising click identifier, set by auto-tagging.
    "msclkid",
    // Yandex Direct click identifier.
    "yclid",
    // TikTok Ads click identifier.
    "ttclid",
    // X (Twitter) Ads click identifier.
    "twclid",
    // LinkedIn Insight Tag's first-party click identifier.
    "li_fat_id",
    // Mailchimp: the recipient's email id, which names who clicked.
    "mc_eid",
    // Mailchimp: the campaign id.
    "mc_cid",
    // HubSpot email tracking: the encrypted recipient and the message.
    "_hsenc",
    "_hsmi",
    // HubSpot cross-domain tracking: the visitor, session and browser-fingerprint cookies' values.
    "__hstc",
    "__hssc",
    "__hsfp",
    // Marketo (Adobe) email tracking token, which names the recipient.
    "mkt_tok",
];

/// `url` with every [`TRACKING`] parameter taken out of its query. A query that had nothing else
/// is removed, `?` and all. A query with no tracking parameter is not touched at all. Anything but
/// `http` and `https` is left alone.
pub(crate) fn strip(url: &mut Url) {
    if !matches!(url.scheme(), "http" | "https") {
        return;
    }
    let Some(query) = url.query() else {
        return;
    };
    let pieces: Vec<&str> = query.split('&').collect();
    let kept: Vec<&str> = pieces
        .iter()
        .copied()
        .filter(|piece| !is_tracking(piece))
        .collect();
    if kept.len() == pieces.len() {
        return;
    }
    // An empty piece carries nothing; once the query is rewritten anyway, a gap left by a removed
    // parameter is not kept as `&&` or a trailing `&`.
    let rest = kept
        .into_iter()
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>()
        .join("&");
    url.set_query((!rest.is_empty()).then_some(rest.as_str()));
}

/// Whether the key of `piece` (everything before its first `=`), percent-decoded, is in
/// [`TRACKING`], and nothing after it could be another parameter.
fn is_tracking(piece: &str) -> bool {
    if piece.contains(';') {
        return false;
    }
    let key = piece.split_once('=').map_or(piece, |(key, _)| key);
    let key = percent_decode(key);
    TRACKING
        .iter()
        .any(|name| name.as_bytes() == key.as_slice())
}

/// `%XX` escapes decoded to their bytes; a `%` not followed by two hex digits is kept as it is.
fn percent_decode(raw: &str) -> Vec<u8> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(at + 1).and_then(|b| hex(*b)),
                bytes.get(at + 2).and_then(|b| hex(*b)),
            )
        {
            out.push(high << 4 | low);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    out
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
