//! The WebDAV half of CardDAV (RFC 6352), as text: request bodies built, `207 Multi-Status`
//! replies read. Which request goes where, and what to do with the answer, is `mail-runtime`'s.
//!
//! Three conversations need these:
//!
//! - discovery — `PROPFIND` for `current-user-principal` (RFC 5397), then
//!   `addressbook-home-set`, then the home's children and their `resourcetype`;
//! - incremental sync — the `sync-collection` `REPORT` (RFC 6578), which answers "what changed
//!   since this token" with the changed hrefs and their etags, and a 404 for each one removed;
//! - fetching — the `addressbook-multiget` `REPORT` (RFC 6352 §8.7), for the cards themselves,
//!   and the `PROPFIND` of every etag that stands in for sync on a server without RFC 6578.

mod reply;
mod request;

pub use reply::multistatus;
pub use request::{Prop, multiget, propfind, sync_collection};

/// `DAV:`, the WebDAV namespace.
pub const DAV: &str = "DAV:";
/// The CardDAV namespace (RFC 6352 §10.1).
pub const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";

/// A `207 Multi-Status` reply.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Multistatus {
    pub responses: Vec<Response>,
    /// The token a `sync-collection` report ends with: where the next sync starts from.
    pub sync_token: Option<String>,
}

/// One `response`: a resource, and what the server said about it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Response {
    /// As the server wrote it: usually an absolute path, sometimes a full URL, possibly
    /// percent-encoded. Resolved against the request's URL by whoever sent it.
    pub href: String,
    /// The status of the response as a whole, where it has one instead of per-property ones.
    /// In a `sync-collection` reply, 404 is how a removed card is reported.
    pub status: Option<u16>,
    /// Properties from every `propstat` whose status was 2xx. Properties the server could not
    /// return are absent rather than empty.
    pub props: Props,
}

impl Response {
    /// Whether this response reports the resource as no longer there.
    pub fn gone(&self) -> bool {
        self.status == Some(404)
    }
}

/// The properties discovery and sync ask for, as far as the server gave them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Props {
    pub resource: Vec<Resource>,
    pub current_user_principal: Option<String>,
    /// `addressbook-home-set`: collections whose children are this principal's address books.
    pub addressbook_home_set: Vec<String>,
    pub display_name: Option<String>,
    /// `getetag`, quotes included: it is compared and sent back in `If-Match`, never read.
    pub etag: Option<String>,
    pub sync_token: Option<String>,
    /// `address-data`: the card itself, as vCard text.
    pub address_data: Option<String>,
}

/// One kind named in a `resourcetype`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    Collection,
    /// A CardDAV address book (RFC 6352 §5.2).
    AddressBook,
    Principal,
    /// Anything else, by its local name: a calendar, a server's own extension.
    Other(String),
}

impl Props {
    /// Whether the resource is an address book.
    pub fn is_address_book(&self) -> bool {
        self.resource.contains(&Resource::AddressBook)
    }
}
