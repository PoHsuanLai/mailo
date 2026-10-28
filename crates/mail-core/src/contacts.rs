//! The address book beyond what the window asks of the store with `Store::contacts_matching`:
//! importing and exporting `.vcf` files, and syncing a CardDAV address book.

use crate::environment::Environment;
use crate::error::CoreError;
use chrono::{DateTime, Utc};
use mail_pim::vcard::{self, Card, Email};
use mail_runtime::carddav::{self, Dav, DavAuth};
use mail_runtime::{AccountSecrets, ClientRegistry};
use mail_store::{SqliteStore, Store};
use porter_capability::CapabilityKind;
use porter_core::{Credential, SecretKey, SecretPurpose, SecretText};
use std::collections::BTreeSet;

pub mod groups;
pub mod people;

pub use mail_runtime::carddav::How;
/// The address book's own types, which a front end names to read and edit it.
pub use mail_store::{
    AddressBook, BookCard, CardId, Contact, Edit, Group, GroupHome, GroupId, Kind, Origin,
    PersonCard,
};

impl crate::mail::ContactOps<'_> {
    /// Sync a CardDAV address book (see [`sync`]).
    pub async fn sync(
        &self,
        url: Option<&str>,
        account: Option<&str>,
        user: Option<&str>,
    ) -> Result<Synced, CoreError> {
        let mail = self.0;
        sync(
            mail.store(),
            url,
            account,
            user,
            &SignIn {
                secrets: mail.secrets().as_ref(),
                env: mail.environment(),
                saved: &mail.saved_clients(),
                now: mail.now(),
            },
        )
        .await
    }
}

/// The contacts that best match what was typed, best first, or the top of the book.
pub fn find(
    store: &dyn Store,
    typed: &str,
    limit: usize,
) -> Result<Vec<mail_store::Contact>, CoreError> {
    Ok(store.contacts_matching(typed, limit)?)
}

/// Add a contact by hand, or give an address already in the book this name.
pub fn add(
    store: &dyn Store,
    address: &str,
    name: Option<&str>,
) -> Result<mail_store::Contact, CoreError> {
    Ok(store.put_contact(address, name, &Origin::Manual)?)
}

/// Take `address` out of the book.
pub fn remove(store: &dyn Store, address: &str) -> Result<(), CoreError> {
    if store.delete_contact(address)? {
        Ok(())
    } else {
        Err(CoreError::NoContact(address.to_owned()))
    }
}

/// What reading a `.vcf` file did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Imported {
    /// Cards in the file.
    pub cards: usize,
    /// Addresses added to the book.
    pub addresses: usize,
    /// Cards of kind group, each now a group of this book.
    pub groups: usize,
    /// Cards of people, each now a card of this book.
    pub people: usize,
    /// Cards that are not groups and carried no email address: kept, but nothing to write to.
    pub empty: usize,
}

/// Every address on every card in `bytes`, added by hand under the card's name, every
/// `KIND:group` card as a group of this book ([`import_group`]), and every other card kept whole
/// as a person's card of this book ([`people::import_card`]).
pub fn import(store: &dyn Store, bytes: &[u8]) -> Result<Imported, CoreError> {
    let cards = vcard::parse_bytes(bytes);
    let (mut added, mut empty, mut groups, mut people) = (0, 0, 0, 0);
    for card in &cards {
        if card.is_group() {
            import_group(store, card, &cards)?;
            groups += 1;
        } else if people::import_card(store, card)? {
            people += 1;
        }
        let addresses = card.addresses_by_preference();
        if card.is_group() && addresses.is_empty() {
            continue;
        }
        if addresses.is_empty() {
            empty += 1;
        }
        let name = card.display_name();
        for address in addresses {
            // One malformed address on a card does not stop the rest of the file. A store that
            // will not write does: counting it as a bad address would report an import that
            // saved nothing as one that worked.
            match store.put_contact(address, name.as_deref(), &Origin::Manual) {
                Ok(_) => added += 1,
                Err(mail_store::StoreError::BadAddress(_)) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(Imported {
        cards: cards.len(),
        addresses: added,
        groups,
        people,
        empty,
    })
}

/// `card`, a `KIND:group`, as a group made here, replacing one imported before with its `UID`.
///
/// Every member is kept as written, with one change: a `urn:uuid:` naming another card of the
/// same file becomes that card's `mailto:`. Its addresses are imported by hand, and a hand-added
/// contact has no `UID` for the member to find it by afterwards. A member naming nothing in the
/// file stays as it was, to be found in a synced book or kept unresolved.
fn import_group(
    store: &dyn Store,
    card: &Card,
    file: &[Card],
) -> Result<(), mail_store::StoreError> {
    let uid = card
        .uid
        .clone()
        .unwrap_or_else(|| format!("urn:uuid:{}", uuid::Uuid::new_v4()));
    let members = card
        .members
        .iter()
        .map(|uri| match vcard::Member::of(uri) {
            vcard::Member::Card(key) => file
                .iter()
                .filter(|other| !other.is_group())
                .find(|other| other.uid.as_deref().map(vcard::uid_key) == Some(key.clone()))
                .and_then(|other| other.addresses_by_preference().first().copied())
                .map(|address| format!("mailto:{address}"))
                .unwrap_or_else(|| uri.clone()),
            vcard::Member::Mailto(_) => uri.clone(),
        })
        .collect();
    store.put_group(&Group {
        id: GroupId::local(&uid),
        uid: Some(uid),
        name: card.display_name().unwrap_or_default(),
        members,
        home: GroupHome::Local,
    })
}

/// The book as vCard 4.0: every synced card as the address book holds it — a group or a
/// person's card edited here as edited — then every group made here, then every person's card
/// made here, then one card per address the user added or has written to that no card covers.
/// Addresses only ever heard from are not the user's contacts and are left out, as are the
/// user's own.
pub fn export(store: &dyn Store) -> Result<String, CoreError> {
    let mut cards: Vec<Card> = Vec::new();
    let mut covered: BTreeSet<String> = BTreeSet::new();
    let groups = store.groups()?;
    let people = store.cards()?;
    for book in store.address_books()? {
        for (href, held) in &book.cards {
            let edited = groups
                .iter()
                .find(|g| g.id.0 == *href && g.home.is_edited());
            let person = people
                .iter()
                .find(|p| p.id.0 == *href && p.home.is_edited());
            let text = match (edited, person) {
                (Some(group), _) => carddav::group_card(group, Some(&held.vcard)),
                (None, Some(person)) => person.vcard.clone(),
                (None, None) => held.vcard.clone(),
            };
            cards.extend(vcard::parse(&text).into_iter().take(1));
            covered.extend(held.addresses.iter().cloned());
        }
    }
    for group in groups.iter().filter(|g| g.home == GroupHome::Local) {
        cards.extend(
            vcard::parse(&carddav::group_card(group, None))
                .into_iter()
                .take(1),
        );
    }
    for person in people.iter().filter(|p| p.home == GroupHome::Local) {
        cards.extend(vcard::parse(&person.vcard).into_iter().take(1));
        covered.extend(person.addresses.iter().cloned());
    }
    for contact in store.contacts()? {
        let theirs = contact.origin != Origin::History || contact.written.count > 0;
        if !theirs || contact.kind == Kind::Own || covered.contains(&contact.address) {
            continue;
        }
        cards.push(Card {
            formatted_name: contact.name.clone(),
            emails: vec![Email {
                address: contact.address.clone(),
                kinds: Vec::new(),
                pref: None,
            }],
            ..Card::new()
        });
    }
    Ok(vcard::write_all(&cards))
}

/// One address book after a sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookSync {
    /// The collection's own name, when the server gave one.
    pub name: Option<String>,
    pub url: String,
    pub done: carddav::Synced,
}

/// What a sync of address books came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Synced {
    /// No URL was given and none was synced before, so there is nothing to go on.
    NothingYet,
    /// Each address book synced, in order.
    Books(Vec<BookSync>),
}

/// What signing in to an address book server draws on: where the secrets are kept, what the
/// program was started with, the OAuth clients earlier sign-ins recorded, and the time.
struct SignIn<'a> {
    secrets: &'a dyn AccountSecrets,
    env: &'a Environment,
    saved: &'a ClientRegistry,
    now: DateTime<Utc>,
}

/// Sync the address book at `url`, or every one synced before when there is none, over the
/// secrets it is given: the handle's, or a test's.
async fn sync(
    store: &SqliteStore,
    url: Option<&str>,
    account: Option<&str>,
    user: Option<&str>,
    sign_in: &SignIn<'_>,
) -> Result<Synced, CoreError> {
    let accounts = crate::sync::configured(store)?;
    let http = carddav::client()?;
    let mut synced = Vec::new();

    if url.is_none() {
        let books = store.address_books()?;
        if !books.is_empty() {
            for book in books {
                let owner = accounts
                    .iter()
                    .find(|a| Some(a.id.clone()) == book.account)
                    .ok_or_else(|| CoreError::BookAccountGone {
                        url: book.url.clone(),
                    })?;
                let base = parse_url(&book.url)?;
                let (dav, _) =
                    open_dav(owner, book.login.as_deref(), Some(&base), &http, sign_in).await?;
                synced.push(sync_one(&dav, store, book, None).await?);
            }
            return Ok(Synced::Books(synced));
        }
        // Nothing synced yet and no address given: there is no server to go to.
        return Ok(Synced::NothingYet);
    }

    let owner = owner_of(&accounts, account)?;
    let start = url.map(parse_url).transpose()?;
    let (dav, start) = open_dav(owner, user, start.as_ref(), &http, sign_in).await?;
    let found = carddav::discover(&dav, &start).await?;
    for collection in found {
        let key = collection.url.to_string();
        let mut book = store.address_book(&key)?.unwrap_or(AddressBook {
            url: key,
            ..AddressBook::default()
        });
        book.account = Some(owner.id.clone());
        book.login = user.map(str::to_owned);
        synced.push(sync_one(&dav, store, book, collection.name).await?);
    }
    Ok(Synced::Books(synced))
}

/// The account an address book is kept under: the one named, or the only one.
fn owner_of<'a>(
    accounts: &'a [crate::sync::Configured],
    account: Option<&str>,
) -> Result<&'a crate::sync::Configured, CoreError> {
    match account {
        Some(address) => accounts
            .iter()
            .find(|a| a.address.eq_ignore_ascii_case(address))
            .ok_or_else(|| CoreError::UnknownAccount(address.to_owned())),
        None => match accounts {
            [only] => Ok(only),
            [] => Err(CoreError::AddAnAccountForBook),
            _ => Err(CoreError::NameTheBookAccount),
        },
    }
}

/// The session with the address book server of `owner`, and the URL to start from: a client of
/// ours with the credential [`auth_for`] finds.
async fn open_dav(
    owner: &crate::sync::Configured,
    login: Option<&str>,
    start: Option<&url::Url>,
    http: &reqwest::Client,
    sign_in: &SignIn<'_>,
) -> Result<(Dav, url::Url), CoreError> {
    let start = start.ok_or(CoreError::BookNeedsServer)?;
    let auth = auth_for(owner, login, sign_in).await?;
    let dav = Dav::new(http.clone(), start, auth)?;
    Ok((dav, start.clone()))
}

async fn sync_one(
    dav: &Dav,
    store: &SqliteStore,
    book: AddressBook,
    name: Option<String>,
) -> Result<BookSync, CoreError> {
    let url = book.url.clone();
    let done = carddav::sync(dav, store, book)
        .await
        .map_err(|e| CoreError::context(url.clone(), e))?;
    Ok(BookSync { name, url, done })
}

fn parse_url(text: &str) -> Result<url::Url, CoreError> {
    let with_scheme = if text.contains("://") {
        text.to_owned()
    } else {
        format!("https://{text}")
    };
    url::Url::parse(&with_scheme).map_err(|source| CoreError::NotAUrl {
        text: text.to_owned(),
        source,
    })
}

/// How to sign in to an address book kept under `account`.
///
/// With a login of its own, a password: `MAILO_PASSWORD` when set, which is then kept in the
/// keyring, else the one kept there before. Without one, the account's own credential — its
/// OAuth token renewed if need be, or its password under its login.
async fn auth_for(
    account: &crate::sync::Configured,
    login: Option<&str>,
    sign_in: &SignIn<'_>,
) -> Result<DavAuth, CoreError> {
    let SignIn {
        secrets,
        env,
        saved,
        now,
    } = *sign_in;
    if let Some(login) = login {
        let key = SecretKey {
            account: account.id.clone(),
            purpose: SecretPurpose::ServicePassword(CapabilityKind::Contacts),
        };
        let password = match env.password.as_deref() {
            Some(password) if !password.is_empty() => {
                secrets
                    .put(
                        &key,
                        &Credential::Password(SecretText::new(password.to_owned())),
                    )
                    .await
                    .map_err(|e| CoreError::cannot("save the password", e))?;
                password.to_owned()
            }
            _ => match secrets.get(&key).await {
                Ok(Credential::Password(password)) => password.expose().to_owned(),
                // A keyring that did not answer may hold the password: it is waited for, not
                // asked for again.
                Err(error) => match mail_runtime::SecretTrouble::of(&error) {
                    mail_runtime::SecretTrouble::Unreachable(why) => {
                        return Err(CoreError::KeyringUnreachable {
                            address: login.to_owned(),
                            why,
                        });
                    }
                    mail_runtime::SecretTrouble::Absent => {
                        return Err(CoreError::NoServicePassword {
                            login: login.to_owned(),
                        });
                    }
                },
                Ok(_) => {
                    return Err(CoreError::NoServicePassword {
                        login: login.to_owned(),
                    });
                }
            },
        };
        return Ok(DavAuth::Basic {
            user: login.to_owned(),
            password,
        });
    }
    let stored = secrets
        .get(&SecretKey {
            account: account.id.clone(),
            purpose: SecretPurpose::IncomingPassword,
        })
        .await
        .map_err(|e| CoreError::unread_credential(&account.address, &account.plan.auth, &e))?;
    match crate::sync::signed_in(account, stored, secrets, saved, now).await? {
        Credential::OAuth { access, .. } => Ok(DavAuth::Bearer(access.expose().to_owned())),
        // A pasted token is presented as it was given: a bearer, with no username.
        Credential::Bearer(token) => Ok(DavAuth::Bearer(token.expose().to_owned())),
        Credential::Password(password) => Ok(DavAuth::Basic {
            user: account.plan.username(),
            password: password.expose().to_owned(),
        }),
        // An API key, a key pair, or a kind this build does not know is no sign-in.
        _ => Err(CoreError::NotASignIn {
            address: account.address.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store that will not write used to be counted as a malformed address, and the import
    /// reported success with nothing saved.
    #[test]
    fn an_import_the_store_refuses_fails_and_a_malformed_address_does_not() {
        let card =
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ren\r\nEMAIL:ren@example.test\r\nEND:VCARD\r\n";
        let refusing = mail_store::testing::in_memory();
        mail_store::testing::refuse_contact_writes(&refusing);
        let failed = import(&refusing, card.as_bytes());
        assert!(matches!(failed, Err(CoreError::Store(_))), "{failed:?}");

        let malformed =
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Nobody\r\nEMAIL:not an address\r\nEND:VCARD\r\n";
        let store = mail_store::testing::in_memory();
        let imported = import(&store, malformed.as_bytes()).unwrap();
        assert_eq!(imported.addresses, 0);
        assert_eq!(imported.cards, 1);
    }

    #[test]
    fn an_imported_file_is_offered_and_exported_back_as_4_0() {
        let store = mail_store::MemoryStore::new();
        let file = "BEGIN:VCARD\r\nVERSION:2.1\r\nFN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Ren=C3=A9e\r\n\
                    EMAIL;INTERNET:Renee@example.test\r\nEND:VCARD\r\n\
                    BEGIN:VCARD\r\nVERSION:3.0\r\nFN:No Mail\r\nTEL:1\r\nEND:VCARD\r\n";
        let imported = import(&store, file.as_bytes()).unwrap();
        assert_eq!(
            imported,
            Imported {
                cards: 2,
                addresses: 1,
                groups: 0,
                people: 2,
                empty: 1,
            }
        );
        let found = find(&store, "ren", 5).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name.as_deref(), Some("Renée"));
        assert_eq!(found[0].address, "renee@example.test");
        let exported = export(&store).unwrap();
        let back = vcard::parse(&exported);
        assert_eq!(
            back.len(),
            2,
            "a card with no address is still a person's card"
        );
        assert_eq!(back[0].formatted_name.as_deref(), Some("No Mail"));
        assert_eq!(back[0].phones[0].number, "1");
        assert_eq!(back[1].formatted_name.as_deref(), Some("Renée"));
        assert_eq!(
            back[1].emails[0].address, "Renee@example.test",
            "as the card wrote it"
        );
        assert!(exported.contains("VERSION:4.0"));
        assert!(find(&store, "zzz", 5).unwrap().is_empty());
    }

    #[test]
    fn a_bare_host_is_taken_as_https() {
        assert_eq!(
            parse_url("dav.example.test/book").unwrap().as_str(),
            "https://dav.example.test/book"
        );
    }

    #[test]
    fn a_group_in_a_file_is_imported_as_a_group_and_exported_as_one() {
        let file = "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:4fbe8971-0bc3-424c-9c26-36c3e1eff6b1\r\n\
                    FN:Grace\r\nEMAIL:grace@example.test\r\nEND:VCARD\r\n\
                    BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:group\r\nUID:team-1\r\nFN:Team\r\n\
                    MEMBER:URN:UUID:4FBE8971-0BC3-424C-9C26-36C3E1EFF6B1\r\n\
                    MEMBER:mailto:ada@example.test\r\n\
                    MEMBER:urn:uuid:ffffffff-0000-4000-8000-000000000000\r\nEND:VCARD\r\n";
        let store = mail_store::MemoryStore::new();
        let before = store.groups().unwrap().len();
        let imported = import(&store, file.as_bytes()).unwrap();
        assert_eq!((imported.groups, imported.cards), (1, 2));
        let groups = store.groups().unwrap();
        assert_eq!(groups.len(), before + 1);
        assert_eq!(groups[0].id, GroupId::local("team-1"));
        assert_eq!(
            groups[0].members,
            [
                "mailto:grace@example.test",
                "mailto:ada@example.test",
                "urn:uuid:ffffffff-0000-4000-8000-000000000000",
            ],
            "a card of the same file becomes its address; a card of none is kept as written"
        );
        // Importing the same file again replaces the group rather than adding a second.
        import(&store, file.as_bytes()).unwrap();
        assert_eq!(store.groups().unwrap().len(), before + 1);

        let out = export(&store).unwrap();
        let team = vcard::parse(&out)
            .into_iter()
            .find(|card| card.is_group())
            .unwrap_or_else(|| panic!("no group exported:\n{out}"));
        assert_eq!(team.uid.as_deref(), Some("team-1"));
        assert_eq!(team.members, groups[0].members);
    }
}
