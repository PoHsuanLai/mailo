//! Request bodies. Built as text: they are small, fixed in shape, and the only variable parts —
//! a sync token and some hrefs — are escaped on the way in.

use super::{CARDDAV, DAV};

/// A property a `PROPFIND` can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prop {
    CurrentUserPrincipal,
    AddressbookHomeSet,
    ResourceType,
    DisplayName,
    GetEtag,
    SyncToken,
}

impl Prop {
    fn element(self) -> &'static str {
        match self {
            Prop::CurrentUserPrincipal => "<d:current-user-principal/>",
            Prop::AddressbookHomeSet => "<card:addressbook-home-set/>",
            Prop::ResourceType => "<d:resourcetype/>",
            Prop::DisplayName => "<d:displayname/>",
            Prop::GetEtag => "<d:getetag/>",
            Prop::SyncToken => "<d:sync-token/>",
        }
    }
}

const HEAD: &str = r#"<?xml version="1.0" encoding="utf-8"?>"#;

/// A `PROPFIND` body asking for `props`.
pub fn propfind(props: &[Prop]) -> String {
    let asked: String = props.iter().map(|p| p.element()).collect();
    format!(
        r#"{HEAD}<d:propfind xmlns:d="{DAV}" xmlns:card="{CARDDAV}"><d:prop>{asked}</d:prop></d:propfind>"#
    )
}

/// A `sync-collection` `REPORT` body (RFC 6578 §3.2) asking for every change since `token`, or
/// for everything when there is no token yet. Only etags are asked for: the cards come by
/// [`multiget`], because not every server returns `address-data` from a sync report.
pub fn sync_collection(token: Option<&str>) -> String {
    let token = token.map(escape).unwrap_or_default();
    format!(
        r#"{HEAD}<d:sync-collection xmlns:d="{DAV}"><d:sync-token>{token}</d:sync-token><d:sync-level>1</d:sync-level><d:prop><d:getetag/></d:prop></d:sync-collection>"#
    )
}

/// An `addressbook-multiget` `REPORT` body for `hrefs`, asking for each card and its etag.
pub fn multiget(hrefs: &[&str]) -> String {
    let hrefs: String = hrefs
        .iter()
        .map(|h| format!("<d:href>{}</d:href>", escape(h)))
        .collect();
    format!(
        r#"{HEAD}<card:addressbook-multiget xmlns:d="{DAV}" xmlns:card="{CARDDAV}"><d:prop><d:getetag/><card:address-data/></d:prop>{hrefs}</card:addressbook-multiget>"#
    )
}

/// XML character escaping for text content.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn well_formed(xml: &str) -> roxmltree::Document<'_> {
        roxmltree::Document::parse(xml).unwrap_or_else(|e| panic!("{e}: {xml}"))
    }

    #[test]
    fn a_propfind_names_each_property_in_its_namespace() {
        let body = propfind(&[Prop::CurrentUserPrincipal, Prop::AddressbookHomeSet]);
        let doc = well_formed(&body);
        let asked: Vec<(Option<&str>, &str)> = doc
            .descendants()
            .filter(|n| n.parent().is_some_and(|p| p.has_tag_name((DAV, "prop"))))
            .map(|n| (n.tag_name().namespace(), n.tag_name().name()))
            .collect();
        assert_eq!(
            asked,
            [
                (Some(DAV), "current-user-principal"),
                (Some(CARDDAV), "addressbook-home-set")
            ]
        );
    }

    #[test]
    fn a_sync_token_and_hrefs_are_escaped_into_the_body() {
        let body = sync_collection(Some("http://x.test/sync?a=1&b=<2>"));
        let doc = well_formed(&body);
        let token = doc
            .descendants()
            .find(|n| n.has_tag_name((DAV, "sync-token")))
            .and_then(|n| n.text());
        assert_eq!(token, Some("http://x.test/sync?a=1&b=<2>"));

        let body = multiget(&["/book/a&b.vcf", "/book/c.vcf"]);
        let doc = well_formed(&body);
        let hrefs: Vec<&str> = doc
            .descendants()
            .filter(|n| n.has_tag_name((DAV, "href")))
            .filter_map(|n| n.text())
            .collect();
        assert_eq!(hrefs, ["/book/a&b.vcf", "/book/c.vcf"]);
    }

    #[test]
    fn a_first_sync_sends_an_empty_token() {
        let body = sync_collection(None);
        assert!(body.contains("<d:sync-token></d:sync-token>"), "{body}");
    }
}
