//! People's cards over CardDAV: a synced card kept whole as a [`PersonCard`], and one edited here
//! written back.

use super::group::Unwritten;
use super::{CardDavFailure, Dav, Put};
use crate::RuntimeError;
use mail_pim::Card;
use mail_store::{AddressBook, BookCard, CardId, Edit, GroupHome, PersonCard, Store};
use url::Url;

/// `card`'s addresses as the book keys them: normalised, most preferred first, each once.
pub fn addresses_of(card: &Card) -> Vec<String> {
    let mut addresses: Vec<String> = Vec::new();
    for address in card
        .addresses_by_preference()
        .into_iter()
        .filter_map(mail_store::contact::normalise)
    {
        if !addresses.contains(&address) {
            addresses.push(address);
        }
    }
    addresses
}

/// `card`, whose text is `vcard`, as the store keeps a person's card: listed by its display
/// name, else its first address.
pub fn person_card(card: &Card, vcard: String, id: CardId, home: GroupHome) -> PersonCard {
    let addresses = addresses_of(card);
    let name = card
        .display_name()
        .or_else(|| addresses.first().cloned())
        // A card with no name or address is listed by an empty name.
        .unwrap_or_default();
    PersonCard {
        id,
        name,
        addresses,
        vcard,
        home,
    }
}

/// The person's card at `href` of the book at `url`, as the server now has it, or gone.
///
/// A card edited here keeps its edit: the next [`write_back`] sends it over this newer one, the
/// edit winning, as the person made it last.
pub(super) fn put_person<S: Store + ?Sized>(
    store: &S,
    url: &str,
    href: &str,
    card: Option<&Card>,
    vcard: &str,
) -> Result<(), RuntimeError> {
    let id = CardId(href.to_owned());
    let Some(card) = card.filter(|c| !c.is_group()) else {
        store.delete_card(&id)?;
        return Ok(());
    };
    if store.card(&id)?.is_some_and(|held| held.home.is_edited()) {
        return Ok(());
    }
    let home = GroupHome::Book {
        url: url.to_owned(),
        href: href.to_owned(),
        edit: Edit::Synced,
    };
    store.put_card(&person_card(card, vcard.to_owned(), id, home))?;
    Ok(())
}

/// Send every card of `book` edited here to the server, each only if the server's is still the
/// one last synced (`If-Match`), and mark each one sent as synced. What is not sent stays
/// edited, and is said, as [`super::group::write_back`] says a group's.
pub(super) async fn write_back<S: Store + ?Sized>(
    dav: &Dav,
    store: &S,
    book: &mut AddressBook,
) -> Result<(usize, Vec<Unwritten>), RuntimeError> {
    let mut written = 0;
    let mut unwritten = Vec::new();
    for person in store.cards()? {
        let GroupHome::Book {
            url,
            href,
            edit: Edit::Edited,
        } = &person.home
        else {
            continue;
        };
        if *url != book.url {
            continue;
        }
        let Some(held) = book.cards.get(href) else {
            continue;
        };
        let at = Url::parse(href).map_err(|e| CardDavFailure::Malformed(format!("{href}: {e}")))?;
        match dav.put_card(&at, &held.etag, person.vcard.clone()).await {
            Ok(Put::Stored(etag)) => {
                book.cards.insert(
                    href.clone(),
                    BookCard {
                        // No etag back means the server may have changed what it stored: an empty
                        // one differs from whatever the next listing says, so that sync fetches
                        // it again.
                        etag: etag.unwrap_or_default(),
                        addresses: person.addresses.clone(),
                        vcard: person.vcard.clone(),
                        ..held.clone()
                    },
                );
                store.put_card(&PersonCard {
                    home: GroupHome::Book {
                        url: url.clone(),
                        href: href.clone(),
                        edit: Edit::Synced,
                    },
                    ..person.clone()
                })?;
                written += 1;
            }
            Ok(Put::Changed) => unwritten.push(Unwritten {
                name: person.name.clone(),
                why: "it changed on the server; the next sync brings that in and sends this again"
                    .to_owned(),
            }),
            Err(CardDavFailure::Refused { status, .. }) => unwritten.push(Unwritten {
                name: person.name.clone(),
                why: format!("the server refused it ({status})"),
            }),
            Err(other) => return Err(other.into()),
        }
    }
    Ok((written, unwritten))
}
