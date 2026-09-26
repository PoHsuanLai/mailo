//! Bringing one address book's cards into the contacts table.
//!
//! Three ways to learn what changed, best first:
//!
//! 1. `sync-collection` from the stored token (RFC 6578): only what changed, and a 404 for what
//!    went. A server that no longer honours the token (`403`/`409` with `valid-sync-token`, or
//!    any refusal) is asked again from scratch.
//! 2. `sync-collection` with an empty token: every member and a first token.
//! 3. For a server without RFC 6578, a `PROPFIND` of every member's etag, compared with the ones
//!    stored — what changed is what differs, what went is what is no longer listed.
//!
//! The cards that changed are then fetched by `addressbook-multiget`, a batch at a time.
//!
//! A card's addresses become contacts under [`Origin::Book`] with the card's name. An address
//! the user added by hand keeps their entry; an address a card no longer lists is handed back to
//! the mail history that knew it, or forgotten if that history is empty.
//!
//! A `KIND:group` card also becomes a [`Group`], known by the card's URL, and goes when the card
//! does. Last, every group of the book edited here is written back (`group::write_back`).

use super::group::{Unwritten, put_group, write_back};
use super::{CardDavFailure, Dav, resolve, same};
use crate::RuntimeError;
use mail_pim::dav::{self, Prop};
use mail_store::{AddressBook, BookCard, GroupId, Origin, Store};
use url::Url;

/// How many cards one `addressbook-multiget` asks for.
const BATCH: usize = 50;

/// What one sync did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synced {
    /// Cards added or changed.
    pub changed: usize,
    /// Cards that went.
    pub removed: usize,
    pub how: How,
    /// Groups edited here that the server now has.
    pub written: usize,
    /// Groups edited here that were not written, and why. Each stays edited, and the next sync
    /// tries again.
    pub unwritten: Vec<Unwritten>,
}

/// Which way the changes were learned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// `sync-collection` from the stored token.
    Incremental,
    /// `sync-collection` from nothing.
    Full,
    /// Every etag, compared.
    Etags,
}

/// What the server says the collection holds or changed.
struct Listing {
    /// Members present, with their etags where given.
    members: Vec<(Url, Option<String>)>,
    /// Members the server says went.
    removed: Vec<Url>,
    /// Whether `members` is everything, so anything stored and not listed went too.
    complete: bool,
    token: Option<String>,
    how: How,
}

/// Sync `book` — its `url` the collection's — and store where it got to.
pub async fn sync<S: Store + ?Sized>(
    dav: &Dav,
    store: &S,
    mut book: AddressBook,
) -> Result<Synced, RuntimeError> {
    let collection = Url::parse(&book.url)
        .map_err(|e| CardDavFailure::Malformed(format!("{}: {e}", book.url)))?;
    let source = format!("carddav:{}", book.url);
    let listing = listing(dav, &collection, book.token.as_deref()).await?;

    let listed: Vec<String> = listing
        .members
        .iter()
        .map(|(u, _)| u.as_str().to_owned())
        .collect();
    let mut removed: Vec<String> = listing
        .removed
        .iter()
        .map(|u| u.as_str().to_owned())
        .collect();
    if listing.complete {
        removed.extend(
            book.cards
                .keys()
                .filter(|href| !listed.contains(href))
                .cloned(),
        );
    }
    let changed: Vec<Url> = listing
        .members
        .into_iter()
        .filter(|(url, etag)| match (book.cards.get(url.as_str()), etag) {
            (Some(held), Some(etag)) => held.etag != *etag,
            _ => true,
        })
        .map(|(url, _)| url)
        .collect();

    let mut gone = 0;
    for href in &removed {
        if let Some(card) = book.cards.remove(href) {
            for address in &card.addresses {
                release(store, &book, &source, address)?;
            }
            store.delete_group(&GroupId(href.clone()))?;
            gone += 1;
        }
    }

    let mut fetched = 0;
    for batch in changed.chunks(BATCH) {
        for (href, etag, data) in multiget(dav, &collection, batch).await? {
            match data {
                Some(data) => {
                    put_card(store, &mut book, &source, href.as_str(), etag, data)?;
                    fetched += 1;
                }
                // Gone between the listing and the fetch.
                None => {
                    if let Some(card) = book.cards.remove(href.as_str()) {
                        for address in &card.addresses {
                            release(store, &book, &source, address)?;
                        }
                        store.delete_group(&GroupId(href.as_str().to_owned()))?;
                        gone += 1;
                    }
                }
            }
        }
    }

    book.token = listing.token;
    store.put_address_book(&book)?;
    let (written, unwritten) = write_back(dav, store, &mut book).await?;
    store.put_address_book(&book)?;
    Ok(Synced {
        changed: fetched,
        removed: gone,
        how: listing.how,
        written,
        unwritten,
    })
}

async fn listing(
    dav: &Dav,
    collection: &Url,
    token: Option<&str>,
) -> Result<Listing, RuntimeError> {
    if let Some(token) = token {
        match report_changes(dav, collection, Some(token)).await {
            Ok(listing) => return Ok(listing),
            // The token is no longer good, or the server stopped offering sync: start over.
            Err(CardDavFailure::Refused { .. }) => {}
            Err(other) => return Err(other.into()),
        }
    }
    match report_changes(dav, collection, None).await {
        Ok(listing) => return Ok(listing),
        Err(CardDavFailure::Refused { .. }) => {}
        Err(other) => return Err(other.into()),
    }
    etags(dav, collection).await.map_err(Into::into)
}

/// A `sync-collection` report from `token`, or from nothing.
async fn report_changes(
    dav: &Dav,
    collection: &Url,
    token: Option<&str>,
) -> Result<Listing, CardDavFailure> {
    let (url, reply) = dav
        .multistatus(
            "REPORT",
            collection,
            "0",
            dav::sync_collection(token),
            "sync-collection",
        )
        .await?;
    let mut members = Vec::new();
    let mut removed = Vec::new();
    for response in &reply.responses {
        let href = resolve(&url, &response.href)?;
        if same(&href, collection) {
            continue;
        }
        if response.gone() {
            removed.push(href);
        } else {
            members.push((href, response.props.etag.clone()));
        }
    }
    Ok(Listing {
        members,
        removed,
        complete: token.is_none(),
        token: reply.sync_token,
        how: if token.is_some() {
            How::Incremental
        } else {
            How::Full
        },
    })
}

/// Every member and its etag, for a server without `sync-collection`.
async fn etags(dav: &Dav, collection: &Url) -> Result<Listing, CardDavFailure> {
    let (url, reply) = dav
        .multistatus(
            "PROPFIND",
            collection,
            "1",
            dav::propfind(&[Prop::GetEtag, Prop::ResourceType]),
            "the etag PROPFIND",
        )
        .await?;
    let mut members = Vec::new();
    for response in &reply.responses {
        let href = resolve(&url, &response.href)?;
        if same(&href, collection)
            || response
                .props
                .resource
                .contains(&mail_pim::Resource::Collection)
        {
            continue;
        }
        members.push((href, response.props.etag.clone()));
    }
    Ok(Listing {
        members,
        removed: Vec::new(),
        complete: true,
        token: None,
        how: How::Etags,
    })
}

/// The cards at `hrefs`: each with its etag and text, or with no text where the server says it
/// is not there.
async fn multiget(
    dav: &Dav,
    collection: &Url,
    hrefs: &[Url],
) -> Result<Vec<(Url, String, Option<String>)>, CardDavFailure> {
    let paths: Vec<&str> = hrefs.iter().map(Url::path).collect();
    let (url, reply) = dav
        .multistatus(
            "REPORT",
            collection,
            "1",
            dav::multiget(&paths),
            "addressbook-multiget",
        )
        .await?;
    let mut out = Vec::new();
    for response in reply.responses {
        let href = resolve(&url, &response.href)?;
        if response.gone() {
            out.push((href, String::new(), None));
            continue;
        }
        if let Some(data) = response.props.address_data {
            out.push((href, response.props.etag.unwrap_or_default(), Some(data)));
        }
    }
    Ok(out)
}

/// Store one card: its addresses become contacts with its name, and addresses it used to list
/// and no longer does are let go of.
fn put_card<S: Store + ?Sized>(
    store: &S,
    book: &mut AddressBook,
    source: &str,
    href: &str,
    etag: String,
    data: String,
) -> Result<(), RuntimeError> {
    let card = mail_pim::vcard::parse(&data).into_iter().next();
    let name = card.as_ref().and_then(|c| c.display_name());
    put_group(store, &book.url, href, card.as_ref(), name.as_deref())?;
    let mut addresses: Vec<String> = Vec::new();
    for address in card
        .iter()
        .flat_map(|c| c.addresses_by_preference())
        .filter_map(mail_store::contact::normalise)
    {
        if !addresses.contains(&address) {
            addresses.push(address);
        }
    }
    let before = book.cards.remove(href);
    for address in before.iter().flat_map(|c| &c.addresses) {
        if !addresses.contains(address) {
            release(store, book, source, address)?;
        }
    }
    for address in &addresses {
        let theirs = store.contact(address)?;
        // Typed by the user: theirs to keep, whatever an address book says.
        if theirs.is_some_and(|c| c.origin == Origin::Manual) {
            continue;
        }
        store.put_contact(address, name.as_deref(), &Origin::Book(source.to_owned()))?;
    }
    book.cards.insert(
        href.to_owned(),
        BookCard {
            uid: card.and_then(|c| c.uid),
            etag,
            addresses,
            vcard: data,
        },
    );
    Ok(())
}

/// Let go of `address`, which a card of this book no longer lists: unless another card still
/// does, an entry this book owns goes back to its mail history, or away if it has none.
fn release<S: Store + ?Sized>(
    store: &S,
    book: &AddressBook,
    source: &str,
    address: &str,
) -> Result<(), RuntimeError> {
    if book
        .cards
        .values()
        .any(|c| c.addresses.iter().any(|a| a == address))
    {
        return Ok(());
    }
    let Some(contact) = store.contact(address)? else {
        return Ok(());
    };
    if contact.origin != Origin::Book(source.to_owned()) {
        return Ok(());
    }
    if contact.written.count == 0 && contact.received.count == 0 {
        store.delete_contact(address)?;
    } else {
        store.put_contact(address, None, &Origin::History)?;
    }
    Ok(())
}
