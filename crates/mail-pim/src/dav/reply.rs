//! Reading a `207 Multi-Status` reply (RFC 4918 §13).
//!
//! Elements are matched by namespace and local name, never by prefix: one server writes
//! `<d:href>`, another `<D:href>`, a third declares `DAV:` as the default namespace and writes
//! `<href>`, and all three mean the same element.

use super::{CARDDAV, DAV, Multistatus, Props, Resource, Response};
use crate::PimError;
use roxmltree::{Document, Node};

/// Read a multistatus reply.
///
/// A document type declaration is refused rather than expanded (roxmltree's default): a server
/// has no reason to send one, and entity expansion is how an XML reply is made to eat memory.
pub fn multistatus(xml: &str) -> Result<Multistatus, PimError> {
    let doc = Document::parse(xml).map_err(|e| PimError::Xml(e.to_string()))?;
    let root = doc.root_element();
    if !root.has_tag_name((DAV, "multistatus")) {
        return Err(PimError::Unexpected {
            expected: "multistatus",
        });
    }
    Ok(Multistatus {
        responses: children(root, DAV, "response").map(response).collect(),
        sync_token: children(root, DAV, "sync-token").next().map(text),
    })
}

fn response(node: Node<'_, '_>) -> Response {
    let mut props = Props::default();
    for propstat in children(node, DAV, "propstat") {
        let ok = children(propstat, DAV, "status")
            .next()
            .and_then(|s| status(&text(s)))
            .is_none_or(|code| (200..300).contains(&code));
        if !ok {
            continue;
        }
        for prop in children(propstat, DAV, "prop") {
            read_props(prop, &mut props);
        }
    }
    Response {
        href: children(node, DAV, "href")
            .next()
            .map(text)
            .unwrap_or_default(),
        status: children(node, DAV, "status")
            .next()
            .and_then(|s| status(&text(s))),
        props,
    }
}

fn read_props(prop: Node<'_, '_>, props: &mut Props) {
    for p in prop.children().filter(Node::is_element) {
        let name = p.tag_name();
        match (name.namespace(), name.name()) {
            (Some(DAV), "resourcetype") => {
                props.resource = p
                    .children()
                    .filter(Node::is_element)
                    .map(resource)
                    .collect();
            }
            (Some(DAV), "current-user-principal") => {
                props.current_user_principal = children(p, DAV, "href").next().map(text);
            }
            (Some(CARDDAV), "addressbook-home-set") => {
                props.addressbook_home_set = children(p, DAV, "href").map(text).collect();
            }
            (Some(DAV), "displayname") => props.display_name = Some(text(p)),
            (Some(DAV), "getetag") => props.etag = Some(text(p)),
            (Some(DAV), "sync-token") => props.sync_token = Some(text(p)),
            // Not trimmed beyond what `text` does: the card's own line structure is its own.
            (Some(CARDDAV), "address-data") => props.address_data = Some(text(p)),
            _ => {}
        }
    }
}

fn resource(node: Node<'_, '_>) -> Resource {
    let name = node.tag_name();
    match (name.namespace(), name.name()) {
        (Some(DAV), "collection") => Resource::Collection,
        (Some(DAV), "principal") => Resource::Principal,
        (Some(CARDDAV), "addressbook") => Resource::AddressBook,
        (_, other) => Resource::Other(other.to_owned()),
    }
}

/// `HTTP/1.1 404 Not Found` is 404.
fn status(line: &str) -> Option<u16> {
    line.split_whitespace().nth(1)?.parse().ok()
}

fn children<'a, 'input: 'a>(
    node: Node<'a, 'input>,
    namespace: &'a str,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    node.children()
        .filter(move |c| c.is_element() && c.has_tag_name((namespace, name)))
}

/// Every piece of text under `node`, joined — CDATA sections included — and trimmed.
fn text(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_are_matched_by_namespace_whatever_the_prefix() {
        const CASES: &[&str] = &[
            r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/a</d:href></d:response></d:multistatus>"#,
            r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/a</D:href></D:response></D:multistatus>"#,
            r#"<multistatus xmlns="DAV:"><response><href>/a</href></response></multistatus>"#,
        ];
        for xml in CASES {
            let reply = multistatus(xml).unwrap();
            assert_eq!(reply.responses.len(), 1, "{xml}");
            assert_eq!(reply.responses[0].href, "/a", "{xml}");
        }
    }

    #[test]
    fn an_element_in_another_namespace_is_not_the_dav_one() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:x="urn:x"><x:response><d:href>/a</d:href></x:response></d:multistatus>"#;
        assert!(multistatus(xml).unwrap().responses.is_empty());
    }

    #[test]
    fn properties_from_a_failed_propstat_are_left_out() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav">
          <d:response><d:href>/p/</d:href>
            <d:propstat><d:prop><d:displayname>Me</d:displayname>
              <d:resourcetype><d:collection/><c:addressbook/><x:other xmlns:x="urn:x"/></d:resourcetype>
            </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
            <d:propstat><d:prop><d:getetag>"lie"</d:getetag></d:prop>
              <d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>
          </d:response></d:multistatus>"#;
        let props = &multistatus(xml).unwrap().responses[0].props;
        assert_eq!(props.display_name.as_deref(), Some("Me"));
        assert_eq!(props.etag, None);
        assert_eq!(
            props.resource,
            [
                Resource::Collection,
                Resource::AddressBook,
                Resource::Other("other".into())
            ]
        );
        assert!(props.is_address_book());
    }

    #[test]
    fn a_sync_report_carries_its_new_token_and_its_removals() {
        let xml = r#"<d:multistatus xmlns:d="DAV:">
          <d:response><d:href>/book/a.vcf</d:href>
            <d:propstat><d:prop><d:getetag>"1"</d:getetag></d:prop>
            <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
          <d:response><d:href>/book/b.vcf</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>
          <d:sync-token>http://x.test/sync/2</d:sync-token>
        </d:multistatus>"#;
        let reply = multistatus(xml).unwrap();
        assert_eq!(reply.sync_token.as_deref(), Some("http://x.test/sync/2"));
        assert_eq!(reply.responses[0].props.etag.as_deref(), Some("\"1\""));
        assert!(!reply.responses[0].gone());
        assert!(reply.responses[1].gone());
    }

    #[test]
    fn principal_and_home_hrefs_are_read_from_inside_their_properties() {
        let xml = r#"<multistatus xmlns="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav"><response><href>/</href>
          <propstat><prop>
            <current-user-principal><href>/principals/me/</href></current-user-principal>
            <c:addressbook-home-set><href>/home/one/</href><href>/home/two/</href></c:addressbook-home-set>
          </prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#;
        let props = &multistatus(xml).unwrap().responses[0].props;
        assert_eq!(
            props.current_user_principal.as_deref(),
            Some("/principals/me/")
        );
        assert_eq!(props.addressbook_home_set, ["/home/one/", "/home/two/"]);
    }

    #[test]
    fn address_data_in_cdata_is_read_as_text() {
        let xml = "<d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:carddav\"><d:response><d:href>/a</d:href><d:propstat><d:prop><c:address-data><![CDATA[BEGIN:VCARD\r\nFN:A & B\r\nEND:VCARD]]></c:address-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>";
        let data = multistatus(xml).unwrap().responses[0]
            .props
            .address_data
            .clone()
            .unwrap();
        assert!(data.contains("FN:A & B"), "{data}");
    }

    #[test]
    fn a_reply_that_is_not_a_multistatus_is_an_error_not_an_empty_one() {
        const CASES: &[&str] = &[
            "<html><body>Sign in</body></html>",
            "not xml at all",
            r#"<!DOCTYPE x [<!ENTITY a "aaaa">]><d:multistatus xmlns:d="DAV:">&a;</d:multistatus>"#,
        ];
        for xml in CASES {
            assert!(multistatus(xml).is_err(), "{xml}");
        }
    }
}
