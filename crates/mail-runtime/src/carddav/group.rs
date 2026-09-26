//! Contact groups over CardDAV: a synced `KIND:group` card kept as a [`Group`], and a group
//! edited here written back as one.

use super::{CardDavFailure, Dav, Put};
use crate::RuntimeError;
use mail_pim::vcard::{self, CardKind};
use mail_store::{AddressBook, BookCard, Edit, Group, GroupHome, GroupId, Store};
use url::Url;

/// A group edited here that one sync did not write back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unwritten {
    pub name: String,
    pub why: String,
}

/// `group` as a vCard 4.0 `KIND:group` card (RFC 6350 §6.1.4, §6.6.5).
///
/// `held` is the card the group was last synced from, when it was: everything of it the group
/// does not decide (its `UID`, a photo, a note, properties nothing here reads) is written back
/// as it came. The name and the members are the group's, every member URI as it is stored,
/// whether or not anything here knows who it names. `REV` is dropped rather than left saying the
/// card is older than it is.
pub fn group_card(group: &Group, held: Option<&str>) -> String {
    let mut card = held
        .and_then(|text| vcard::parse(text).into_iter().next())
        .unwrap_or_default();
    card.kind = Some(CardKind::Group);
    card.formatted_name = Some(group.name.clone());
    card.members = group.members.clone();
    card.revision = None;
    if card.uid.is_none() {
        card.uid = group.uid.clone();
    }
    // A `MEMBER` kept among the unread properties, from a card that was not a group when it was
    // read, would now be written twice.
    card.other.retain(|line| line.name != "MEMBER");
    vcard::write(&card)
}

/// Send every group of `book` edited here to the server, each only if its card is still the one
/// last synced (`If-Match`), and mark each one sent as synced.
///
/// A card changed on the server since (`412`) is not overwritten blind: the group stays edited,
/// and the next sync fetches the newer card — keeping the edit's name and members, and whatever
/// else the newer card says — and sends that. A group whose card is not in the book any more
/// went with it, so it is not here to send. A server that refuses a write (a read-only book)
/// is reported and asked again next time; the edit is never dropped here.
pub(super) async fn write_back<S: Store + ?Sized>(
    dav: &Dav,
    store: &S,
    book: &mut AddressBook,
) -> Result<(usize, Vec<Unwritten>), RuntimeError> {
    let mut written = 0;
    let mut unwritten = Vec::new();
    for group in store.groups()? {
        let GroupHome::Book {
            url,
            href,
            edit: Edit::Edited,
        } = &group.home
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
        let text = super::group_card(&group, Some(&held.vcard));
        match dav.put_card(&at, &held.etag, text.clone()).await {
            Ok(Put::Stored(etag)) => {
                // No etag back means the server may have changed what it stored: an empty one
                // differs from whatever the next listing says, so that sync fetches it again.
                let card = BookCard {
                    etag: etag.unwrap_or_default(),
                    vcard: text,
                    ..held.clone()
                };
                book.cards.insert(href.clone(), card);
                let mut sent = group.clone();
                sent.home = GroupHome::Book {
                    url: url.clone(),
                    href: href.clone(),
                    edit: Edit::Synced,
                };
                store.put_group(&sent)?;
                written += 1;
            }
            Ok(Put::Changed) => unwritten.push(Unwritten {
                name: group.name.clone(),
                why: "it changed on the server; the next sync brings that in and sends this again"
                    .to_owned(),
            }),
            Err(CardDavFailure::Refused { status, .. }) => unwritten.push(Unwritten {
                name: group.name.clone(),
                why: format!("the server refused it ({status})"),
            }),
            Err(other) => return Err(other.into()),
        }
    }
    Ok((written, unwritten))
}

/// The group a card at `href` of the book at `url` is, or is no longer.
///
/// A group edited here keeps its edit — the next [`write_back`] sends it onto this newer card —
/// and takes only the card's `UID`.
pub(super) fn put_group<S: Store + ?Sized>(
    store: &S,
    url: &str,
    href: &str,
    card: Option<&mail_pim::Card>,
    name: Option<&str>,
) -> Result<(), RuntimeError> {
    let id = GroupId(href.to_owned());
    let Some(card) = card.filter(|c| c.is_group()) else {
        store.delete_group(&id)?;
        return Ok(());
    };
    let held = store.group(&id)?;
    let edited = held.as_ref().filter(|g| {
        matches!(
            g.home,
            GroupHome::Book {
                edit: Edit::Edited,
                ..
            }
        )
    });
    let group = match edited {
        Some(edited) => Group {
            uid: card.uid.clone(),
            ..edited.clone()
        },
        None => Group {
            id,
            uid: card.uid.clone(),
            name: name.unwrap_or_default().to_owned(),
            members: card.members.clone(),
            home: GroupHome::Book {
                url: url.to_owned(),
                href: href.to_owned(),
                edit: Edit::Synced,
            },
        },
    };
    store.put_group(&group)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(members: &[&str]) -> Group {
        Group {
            id: GroupId("https://dav.example.test/book/g.vcf".to_owned()),
            uid: Some("urn:uuid:03a0e51f-d1aa-4385-8a53-e29025acd8af".to_owned()),
            name: "Renamed".to_owned(),
            members: members.iter().map(|m| (*m).to_owned()).collect(),
            home: GroupHome::Book {
                url: "https://dav.example.test/book/".to_owned(),
                href: "https://dav.example.test/book/g.vcf".to_owned(),
                edit: Edit::Edited,
            },
        }
    }

    #[test]
    fn an_edited_group_keeps_what_it_does_not_decide() {
        let held = "BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:group\r\nUID:held-uid\r\nFN:Old\r\n\
                    MEMBER:mailto:old@example.test\r\nREV:20200101T000000Z\r\n\
                    NOTE:kept\r\nX-COLOUR:green\r\nEND:VCARD\r\n";
        let text = group_card(
            &group(&["mailto:new@example.test", "urn:uuid:unknown"]),
            Some(held),
        );
        let card = vcard::parse(&text).remove(0);
        assert!(card.is_group());
        assert_eq!(card.formatted_name.as_deref(), Some("Renamed"));
        assert_eq!(
            card.uid.as_deref(),
            Some("held-uid"),
            "the server's UID stays"
        );
        assert_eq!(
            card.members,
            ["mailto:new@example.test", "urn:uuid:unknown"]
        );
        assert_eq!(card.note.as_deref(), Some("kept"));
        assert_eq!(card.revision, None);
        assert_eq!(card.other.len(), 1);
        assert_eq!(card.other[0].name, "X-COLOUR");
    }

    #[test]
    fn a_group_with_no_card_behind_it_is_written_whole() {
        let text = group_card(&group(&[]), None);
        assert!(text.contains("\r\nKIND:group\r\n"), "{text}");
        assert!(text.contains("\r\nUID:urn:uuid:03a0e51f-d1aa-4385-8a53-e29025acd8af\r\n"));
        assert!(!text.contains("MEMBER"));
    }
}
