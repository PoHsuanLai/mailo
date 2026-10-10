//! The address book beyond what the window asks of the store with `Store::contacts_matching`:
//! importing and exporting `.vcf` files, and syncing a CardDAV address book.

use crate::environment::Environment;
use crate::error::CoreError;
use chrono::{DateTime, Utc};
use mail_domain::AuthPlan;
use mail_pim::vcard::{self, Card, Email};
use mail_runtime::carddav::{self, Dav, DavAuth};
use mail_runtime::link::LinkError;
use mail_runtime::{AccountSecrets, ClientRegistry};
use mail_store::{AddressBook, Edit, Group, GroupHome, GroupId, Kind, Origin, SqliteStore, Store};
use porter_core::{CapabilityKind, Credential, Family, SecretKey, SecretPurpose, SecretText};
use std::collections::BTreeSet;

pub use mail_runtime::carddav::How;

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
            mail.secrets().as_ref(),
            mail.environment(),
            &mail.saved_clients(),
            mail.now(),
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
    /// Cards that are not groups and carried no email address.
    pub empty: usize,
}

/// Every address on every card in `bytes`, added by hand under the card's name, and every
/// `KIND:group` card as a group of this book ([`import_group`]).
pub fn import(store: &dyn Store, bytes: &[u8]) -> Result<Imported, CoreError> {
    let cards = vcard::parse_bytes(bytes);
    let (mut added, mut empty, mut groups) = (0, 0, 0);
    for card in &cards {
        if card.is_group() {
            import_group(store, card, &cards)?;
            groups += 1;
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
            // One malformed address on a card does not stop the rest of the file.
            if store
                .put_contact(address, name.as_deref(), &Origin::Manual)
                .is_ok()
            {
                added += 1;
            }
        }
    }
    Ok(Imported {
        cards: cards.len(),
        addresses: added,
        groups,
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

/// The book as vCard 4.0: every synced card as the address book holds it — a group edited here
/// as edited — then every group made here, then one card per address the user added or has
/// written to that no synced card covers. Addresses only ever heard from are not the user's
/// contacts and are left out, as are the user's own.
pub fn export(store: &dyn Store) -> Result<String, CoreError> {
    let mut cards: Vec<Card> = Vec::new();
    let mut covered: BTreeSet<String> = BTreeSet::new();
    let groups = store.groups()?;
    for book in store.address_books()? {
        for (href, held) in &book.cards {
            let edited = groups.iter().find(|g| {
                g.id.0 == *href
                    && matches!(
                        g.home,
                        GroupHome::Book {
                            edit: Edit::Edited,
                            ..
                        }
                    )
            });
            let text = match edited {
                Some(group) => carddav::group_card(group, Some(&held.vcard)),
                None => held.vcard.clone(),
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

/// Sync the address book at `url`, or every one synced before when there is none, over the
/// secrets (and so the link) it is given: the handle's, or a test's.
#[allow(clippy::too_many_arguments)]
pub async fn sync(
    store: &SqliteStore,
    url: Option<&str>,
    account: Option<&str>,
    user: Option<&str>,
    secrets: &dyn AccountSecrets,
    env: &Environment,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
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
                let (dav, _) = open_dav(
                    owner,
                    book.login.as_deref(),
                    Some(&base),
                    &http,
                    secrets,
                    env,
                    saved,
                    now,
                )
                .await?;
                synced.push(sync_one(&dav, store, book, None).await?);
            }
            return Ok(Synced::Books(synced));
        }
        // Nothing synced yet and no address given: an account of the desktop's accountd names its
        // own address book server (its grant lists it), so the first sync needs no URL. Any other
        // account has none to name.
        let named = owner_of(&accounts, account)
            .ok()
            .filter(|owner| user.is_none() && owner.plan.grant().is_some());
        if named.is_none() {
            return Ok(Synced::NothingYet);
        }
    }

    let owner = owner_of(&accounts, account)?;
    let start = url.map(parse_url).transpose()?;
    let (dav, start) =
        open_dav(owner, user, start.as_ref(), &http, secrets, env, saved, now).await?;
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

/// The session with the address book server of `owner`, and the URL to start from.
///
/// An account of the desktop's accountd with no login of its own goes through accountd's relay
/// ([`relayed`]): Mail holds no password or token for it. Every other account, and a login given
/// with `--user`, is a client of ours with the credential [`auth_for`] finds, as it always was.
#[allow(clippy::too_many_arguments)]
async fn open_dav(
    owner: &crate::sync::Configured,
    login: Option<&str>,
    start: Option<&url::Url>,
    http: &reqwest::Client,
    secrets: &dyn AccountSecrets,
    env: &Environment,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<(Dav, url::Url), CoreError> {
    if login.is_none() && owner.plan.grant().is_some() {
        return relayed(owner, start, secrets).await;
    }
    let start = start.ok_or(CoreError::BookNeedsServer)?;
    let auth = auth_for(owner, login, secrets, env, saved, now).await?;
    let dav = Dav::new(http.clone(), start, auth)?;
    Ok((dav, start.clone()))
}

/// The desktop's accountd's relay to the CardDAV server of `owner`, on the grant Mail holds on the
/// account's contacts. That is a grant of its own: when Mail has none, accountd is asked for it
/// the way it is for mail (its chooser and consent sheet; Mail opens no dialog), and a refusal is
/// said in the account's own words.
///
/// An account whose contacts are not CardDAV (Google serves them through People) syncs none, and
/// says so: Mail has no People client.
async fn relayed(
    owner: &crate::sync::Configured,
    start: Option<&url::Url>,
    secrets: &dyn AccountSecrets,
) -> Result<(Dav, url::Url), CoreError> {
    let address = &owner.address;
    let AuthPlan::Granted { account, .. } = &owner.plan.auth else {
        return Err(CoreError::NotAServiceAccount {
            address: address.clone(),
        });
    };
    let ungranted = |source: LinkError| CoreError::ContactsNotAllowed {
        address: address.clone(),
        source,
    };
    let Some(link) = secrets.link() else {
        return Err(CoreError::AccountServiceUnreachableFromHere {
            address: address.clone(),
        });
    };
    let held = link
        .contacts()
        .await
        .map_err(|e| CoreError::context(address.clone(), e))?;
    let candidate = match held.into_iter().find(|c| c.account == *account) {
        Some(candidate) => candidate,
        None => {
            let asked = link.request_contacts().await.map_err(ungranted)?;
            if asked.account != *account {
                return Err(CoreError::ContactsGrantMisdirected {
                    address: address.clone(),
                });
            }
            asked
        }
    };
    let servers = || {
        candidate
            .endpoints
            .iter()
            .filter(|e| e.family == Family::CardDav)
    };
    let origin_of = |endpoint: &porter_core::ServiceEndpoint| {
        url::Url::parse(endpoint.url.as_str())
            .ok()
            .map(|u| u.origin())
    };
    let Some(first) = servers().next() else {
        return Err(CoreError::ContactsNotCardDav {
            address: address.clone(),
        });
    };
    let endpoint = match start {
        Some(start) => servers()
            .find(|e| origin_of(e) == Some(start.origin()))
            .ok_or_else(|| CoreError::NotTheBookServer {
                start: start.to_string(),
                address: address.clone(),
                relays: first.url.as_str().to_owned(),
            })?,
        None => first,
    };
    let dav = Dav::relayed(link, candidate.grant.clone(), endpoint.clone())?;
    let start = match start {
        Some(start) => start.clone(),
        None => dav
            .endpoint_url()
            .ok_or_else(|| CoreError::BookServerNoAddress {
                address: address.clone(),
            })?,
    };
    Ok((dav, start))
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
    secrets: &dyn AccountSecrets,
    env: &Environment,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<DavAuth, CoreError> {
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
                _ => {
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
    // An account of the desktop's accountd holds nothing here to sign in to a CardDAV server with:
    // its contacts go through accountd's relay ([`relayed`]) and never reach this.
    let stored = secrets
        .get(&SecretKey {
            account: account.id.clone(),
            purpose: SecretPurpose::IncomingPassword,
        })
        .await
        .map_err(|_| CoreError::NoCredential {
            address: account.address.clone(),
            auth: account.plan.auth.clone(),
        })?;
    match crate::sync::signed_in(account, stored, secrets, saved, now).await? {
        Credential::OAuth { access, .. } => Ok(DavAuth::Bearer(access.expose().to_owned())),
        // A pasted token is presented as it was given: a bearer, with no username.
        Credential::Bearer(token) => Ok(DavAuth::Bearer(token.expose().to_owned())),
        Credential::Password(password) => Ok(DavAuth::Basic {
            user: account.plan.username(),
            password: password.expose().to_owned(),
        }),
        Credential::ApiKey(_) | Credential::KeyPair { .. } => Err(CoreError::NotASignIn {
            address: account.address.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                empty: 1,
            }
        );
        let found = find(&store, "ren", 5).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name.as_deref(), Some("Renée"));
        assert_eq!(found[0].address, "renee@example.test");
        let exported = export(&store).unwrap();
        let back = vcard::parse(&exported);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].formatted_name.as_deref(), Some("Renée"));
        assert_eq!(back[0].emails[0].address, "renee@example.test");
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

    // -----------------------------------------------------------------------------------------
    // An account of the desktop's accountd (step E7)
    // -----------------------------------------------------------------------------------------

    use mail_runtime::Transport;
    use mail_runtime::link::{Accountd, Answer, Changes, LinkError, LinkedSecrets};
    use porter_core::capability::{
        Access, Capability, Delta, LabelModel, MailCap, MailTransport, Offered, PimCap,
        PimTransport,
    };
    use porter_core::wire::Refusal;
    use porter_core::{
        AccountId, AccountLabel, Audience, Candidate, EndpointUrl, GrantId, IssuedToken, LoginName,
        ProviderId, Restriction, ServiceEndpoint, Subject, Tls,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const ME: &str = "me@example.test";

    fn endpoint(family: Family, url: &str, tls: Tls) -> ServiceEndpoint {
        ServiceEndpoint {
            family,
            url: EndpointUrl::parse(url).unwrap(),
            tls,
            login: LoginName(ME.to_owned()),
        }
    }

    fn base(
        account: &str,
        grant: &str,
        capability: Capability,
        endpoints: Vec<ServiceEndpoint>,
    ) -> Candidate {
        Candidate {
            account: AccountId::parse(account).unwrap(),
            label: AccountLabel(ME.to_owned()),
            provider: ProviderId::parse("fastmail").unwrap(),
            subject: Subject::Account,
            capability,
            restriction: Restriction::none(),
            grant: GrantId::parse(grant).unwrap(),
            endpoints,
        }
    }

    /// What accountd lists for Mail's mail grant on the account.
    fn mail() -> Candidate {
        base(
            "fastmail-me",
            "grant-mail",
            Capability::Mail(MailCap {
                access: Access::ReadWrite,
                send: Offered::Present,
                delta: Delta::Push,
                transport: MailTransport::Imap,
                labels: LabelModel::Folders,
            }),
            vec![
                endpoint(Family::Imap, "imaps://imap.example.test:993", Tls::Implicit),
                endpoint(Family::Smtp, "smtp://smtp.example.test:587", Tls::StartTls),
            ],
        )
    }

    fn pim(transport: PimTransport) -> Capability {
        Capability::Contacts(PimCap {
            access: Access::ReadWrite,
            delta: Delta::Poll,
            transport,
            collections: Offered::Present,
        })
    }

    /// What accountd lists for Mail's contacts grant: a CardDAV server.
    fn carddav_grant(account: &str) -> Candidate {
        base(
            account,
            "grant-contacts",
            pim(PimTransport::CardDav),
            vec![endpoint(
                Family::CardDav,
                "https://carddav.example.test/",
                Tls::Implicit,
            )],
        )
    }

    /// Google: contacts through People.
    fn people_grant() -> Candidate {
        base(
            "fastmail-me",
            "grant-contacts",
            pim(PimTransport::GoogleApi),
            vec![endpoint(
                Family::GooglePeople,
                "https://people.googleapis.com/",
                Tls::Implicit,
            )],
        )
    }

    /// accountd, as far as contacts go: the grants held, and what asking for one comes to.
    #[derive(Debug)]
    struct Daemon {
        held: Vec<Candidate>,
        asked: Result<Candidate, LinkError>,
        requests: AtomicUsize,
    }

    impl Daemon {
        fn new(held: Vec<Candidate>, asked: Result<Candidate, LinkError>) -> Arc<Daemon> {
            Arc::new(Daemon {
                held,
                asked,
                requests: AtomicUsize::new(0),
            })
        }

        fn requests(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }
    }

    fn nothing<T>() -> Answer<'static, T> {
        Box::pin(async { Err(LinkError::Other("not part of this test".to_owned())) })
    }

    impl Accountd for Daemon {
        fn candidates(&self) -> Answer<'_, Vec<Candidate>> {
            nothing()
        }
        fn token<'a>(&'a self, _: &'a GrantId, _: &'a Audience) -> Answer<'a, IssuedToken> {
            nothing()
        }
        fn open<'a>(&'a self, _: &'a GrantId, _: &'a ServiceEndpoint) -> Answer<'a, Transport> {
            nothing()
        }
        fn contacts(&self) -> Answer<'_, Vec<Candidate>> {
            let held = self.held.clone();
            Box::pin(async move { Ok(held) })
        }
        fn request_contacts(&self) -> Answer<'_, Candidate> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            let asked = self.asked.clone();
            Box::pin(async move { asked })
        }
        fn add_account(&self) -> Answer<'_, AccountId> {
            nothing()
        }
        fn reauthenticate<'a>(&'a self, _: &'a AccountId) -> Answer<'a, ()> {
            nothing()
        }
        fn request_grant(&self) -> Answer<'_, Candidate> {
            nothing()
        }
        fn revoke<'a>(&'a self, _: &'a GrantId) -> Answer<'a, ()> {
            nothing()
        }
        fn changes(&self) -> Answer<'_, Option<Box<dyn Changes>>> {
            Box::pin(async { Ok(None) })
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    /// The account as the store has it once accountd's mail grant is read.
    fn owner() -> crate::sync::Configured {
        let preset = crate::account::preset_of(&mail(), now()).unwrap();
        crate::sync::Configured {
            id: mail_domain::id::new_account_id(),
            address: preset.plan.address.clone(),
            plan: preset.plan,
            caps: preset.expected_caps,
            keep: Default::default(),
        }
    }

    fn linked(daemon: &Arc<Daemon>) -> LinkedSecrets {
        LinkedSecrets::new(daemon.clone())
    }

    #[tokio::test]
    async fn a_linked_account_is_asked_for_its_contacts_once_and_reads_its_carddav_server() {
        // No grant on its contacts yet: accountd's own sheet is asked, once.
        let daemon = Daemon::new(Vec::new(), Ok(carddav_grant("fastmail-me")));
        let (dav, start) = relayed(&owner(), None, &linked(&daemon)).await.unwrap();
        assert_eq!(start.as_str(), "https://carddav.example.test/");
        assert_eq!(dav.endpoint_url(), Some(start));
        assert_eq!(daemon.requests(), 1);

        // A grant already held is used as it is: nothing is asked.
        let daemon = Daemon::new(vec![carddav_grant("fastmail-me")], nothing_asked());
        relayed(&owner(), None, &linked(&daemon)).await.unwrap();
        assert_eq!(daemon.requests(), 0);

        // A grant the person gave to some other account is not this account's.
        let daemon = Daemon::new(Vec::new(), Ok(carddav_grant("fastmail-other")));
        let why = relayed(&owner(), None, &linked(&daemon))
            .await
            .unwrap_err()
            .to_string();
        assert!(why.contains("another account"), "{why}");
    }

    fn nothing_asked() -> Result<Candidate, LinkError> {
        Err(LinkError::Other("nothing should be asked".to_owned()))
    }

    #[tokio::test]
    async fn a_linked_account_whose_contacts_are_not_allowed_says_so_in_its_own_words() {
        let daemon = Daemon::new(Vec::new(), Err(LinkError::Refused(Refusal::Denied)));
        let why = relayed(&owner(), None, &linked(&daemon))
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(
            why,
            "me@example.test is an account of the desktop's account service, and Mail has not \
             been allowed to read its contacts (the desktop's account service refused: the \
             person said no)"
        );
        assert_eq!(daemon.requests(), 1);
    }

    #[tokio::test]
    async fn a_linked_account_whose_contacts_are_not_carddav_syncs_none_and_says_so() {
        let daemon = Daemon::new(vec![people_grant()], nothing_asked());
        let why = relayed(&owner(), None, &linked(&daemon))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            why.contains("not CardDAV")
                && why.contains("People")
                && why.contains("no contacts were synced"),
            "{why}"
        );
        assert_eq!(daemon.requests(), 0, "a grant it holds is not asked again");
    }

    #[tokio::test]
    async fn a_linked_account_is_relayed_to_its_own_server_and_no_other() {
        let daemon = Daemon::new(vec![carddav_grant("fastmail-me")], nothing_asked());
        let elsewhere = url::Url::parse("https://evil.example.test/dav/").unwrap();
        let why = relayed(&owner(), Some(&elsewhere), &linked(&daemon))
            .await
            .unwrap_err()
            .to_string();
        assert!(why.contains("is not the address book server of"), "{why}");

        let own = url::Url::parse("https://carddav.example.test/dav/").unwrap();
        let (_, start) = relayed(&owner(), Some(&own), &linked(&daemon))
            .await
            .unwrap();
        assert_eq!(
            start, own,
            "a path on the account's server is the start as given"
        );
    }

    #[tokio::test]
    async fn an_account_of_accountd_with_no_link_is_not_reachable_rather_than_asked_for_a_password()
    {
        let secrets = porter_secrets::MemorySecrets::default();
        let why = relayed(&owner(), None, &secrets)
            .await
            .unwrap_err()
            .to_string();
        assert!(why.contains("not reachable"), "{why}");
    }

    #[tokio::test]
    async fn the_first_sync_of_a_linked_account_needs_no_address_and_another_account_still_does() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        let saved = ClientRegistry::default();

        // No account at all: the old answer.
        let none = Daemon::new(Vec::new(), nothing_asked());
        let said = sync(
            &store,
            None,
            None,
            None,
            &linked(&none),
            &Environment::default(),
            &saved,
            now(),
        )
        .await
        .unwrap();
        assert_eq!(said, Synced::NothingYet);

        // accountd's account: the first sync asks accountd for its contacts and goes on to its
        // server (here Google's, which is not CardDAV) with no address given.
        crate::account::reconcile(&store, &[mail()], now()).unwrap();
        let daemon = Daemon::new(vec![people_grant()], nothing_asked());
        let why = sync(
            &store,
            None,
            None,
            None,
            &linked(&daemon),
            &Environment::default(),
            &saved,
            now(),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(why.contains("People"), "{why}");
        // A login of its own is the address book's own sign-in: not the relay's, and not asked.
        let said = sync(
            &store,
            None,
            None,
            Some("ada"),
            &linked(&daemon),
            &Environment::default(),
            &saved,
            now(),
        )
        .await
        .unwrap();
        assert_eq!(said, Synced::NothingYet);
    }
}
