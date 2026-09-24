//! Finding a user's address books from whatever URL they were given (RFC 6352 §7, RFC 6764).
//!
//! People are handed one of three things: an address book's own URL, their principal's, or just
//! the server's. So the URL is asked what it is first — an address book is the answer itself, and
//! a `current-user-principal` or `addressbook-home-set` says where to look next. A bare server,
//! or a URL that says neither, is asked again at `/.well-known/carddav`. From the principal, the
//! home set; from each home, its children that are address books.

use super::{CardDavFailure, Dav, resolve, same};
use crate::RuntimeError;
use mail_pim::dav::{self, Prop};
use url::Url;

/// One address book found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    pub url: Url,
    /// `displayname`, where the server gives one.
    pub name: Option<String>,
}

/// Every address book reachable from `start`.
pub async fn discover(dav: &Dav, start: &Url) -> Result<Vec<Collection>, RuntimeError> {
    let mut candidates = Vec::new();
    if !matches!(start.path(), "" | "/") {
        candidates.push(start.clone());
    }
    candidates.push(well_known(start)?);

    let mut homes: Vec<Url> = Vec::new();
    let mut principal: Option<Url> = None;
    for candidate in &candidates {
        let asked = dav::propfind(&[
            Prop::ResourceType,
            Prop::DisplayName,
            Prop::CurrentUserPrincipal,
            Prop::AddressbookHomeSet,
        ]);
        let (url, reply) = match dav
            .multistatus("PROPFIND", candidate, "0", asked, "PROPFIND")
            .await
        {
            Ok(found) => found,
            // A URL that is not a DAV resource is a reason to try the next, not to stop: the
            // server's root answering 404 or 405 is what sends a client to `.well-known`.
            Err(CardDavFailure::Refused { .. }) => continue,
            Err(other) => return Err(other.into()),
        };
        let Some(first) = reply.responses.first() else {
            continue;
        };
        if first.props.is_address_book() {
            return Ok(vec![Collection {
                url,
                name: first.props.display_name.clone(),
            }]);
        }
        for home in &first.props.addressbook_home_set {
            homes.push(resolve(&url, home)?);
        }
        if let Some(href) = &first.props.current_user_principal {
            principal = Some(resolve(&url, href)?);
        }
        if !homes.is_empty() || principal.is_some() {
            break;
        }
    }

    if homes.is_empty() {
        let Some(principal) = principal else {
            return Err(CardDavFailure::NotFound(start.to_string()).into());
        };
        let asked = dav::propfind(&[Prop::AddressbookHomeSet]);
        let (url, reply) = dav
            .multistatus("PROPFIND", &principal, "0", asked, "the principal PROPFIND")
            .await?;
        for response in &reply.responses {
            for home in &response.props.addressbook_home_set {
                homes.push(resolve(&url, home)?);
            }
        }
    }

    let mut found = Vec::new();
    for home in &homes {
        let asked = dav::propfind(&[Prop::ResourceType, Prop::DisplayName]);
        let (url, reply) = dav
            .multistatus(
                "PROPFIND",
                home,
                "1",
                asked,
                "the address book home PROPFIND",
            )
            .await?;
        for response in &reply.responses {
            if !response.props.is_address_book() {
                continue;
            }
            let collection = resolve(&url, &response.href)?;
            if !found.iter().any(|c: &Collection| same(&c.url, &collection)) {
                found.push(Collection {
                    url: collection,
                    name: response.props.display_name.clone(),
                });
            }
        }
    }
    if found.is_empty() {
        return Err(CardDavFailure::NotFound(start.to_string()).into());
    }
    Ok(found)
}

/// `/.well-known/carddav` on `start`'s origin.
fn well_known(start: &Url) -> Result<Url, CardDavFailure> {
    start
        .join("/.well-known/carddav")
        .map_err(|e| CardDavFailure::Malformed(e.to_string()))
}
