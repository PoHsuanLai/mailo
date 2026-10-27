//! People's cards over CardDAV: a synced card kept whole as a [`PersonCard`], and one edited here
//! written back.

use super::group::{Unwritten, refusal, refused_for_good};
use super::{CardDavFailure, Dav, Put};
use crate::RuntimeError;
use mail_pim::Card;
use mail_pim::vcard;
use mail_store::{AddressBook, BookCard, CardId, Edit, GroupHome, PersonCard, Store};
use url::Url;

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
    store.put_card(&PersonCard::new(
        id,
        card.list_name(),
        card.addresses(),
        vcard.to_owned(),
        home,
    ))?;
    Ok(())
}

/// What to send for a card edited here: the card as the server has it now (`server`, from the
/// book's last listing), with the fields the editor shows replaced by the edit's (`edited`). An
/// edit sent whole would put back, over the server's etag, whatever the server changed in what
/// the editor does not show since the edit began. A server's card that cannot be read leaves the
/// edit as it is.
fn merged(edited: &str, server: &str) -> String {
    let (Some(mine), Some(theirs)) = (
        vcard::parse(edited).into_iter().next(),
        vcard::parse(server).into_iter().next(),
    ) else {
        return edited.to_owned();
    };
    vcard::write(&vcard::apply(theirs, &vcard::details_of(&mine)))
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
        let body = merged(&person.vcard, &held.vcard);
        match dav.put_card(&at, &held.etag, body.clone()).await {
            Ok(Put::Stored(etag)) => {
                book.cards.insert(
                    href.clone(),
                    BookCard {
                        // No etag back means the server may have changed what it stored: an empty
                        // one differs from whatever the next listing says, so that sync fetches
                        // it again.
                        etag: etag.unwrap_or_default(),
                        addresses: person.addresses.clone(),
                        vcard: body.clone(),
                        ..held.clone()
                    },
                );
                store.put_card(&PersonCard {
                    vcard: body,
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
            Err(CardDavFailure::Refused { status, .. }) => {
                unwritten.push(Unwritten {
                    name: person.name.clone(),
                    why: refusal(status),
                });
                if refused_for_good(status) {
                    store.put_card(&PersonCard {
                        home: person.home.clone().refused(),
                        ..person.clone()
                    })?;
                }
            }
            Err(other) => return Err(other.into()),
        }
    }
    Ok((written, unwritten))
}
