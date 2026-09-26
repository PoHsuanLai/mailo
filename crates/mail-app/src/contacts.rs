//! `mailo contacts`: the address book from the command line.
//!
//! The window asks the store the same question with `Store::contacts_matching`; this module is
//! the CLI's side of it, plus the things only a command line does — importing and exporting
//! `.vcf` files and syncing a CardDAV address book.

use chrono::{DateTime, Utc};
use mail_domain::{Credential, SecretKey, SecretPurpose};
use mail_pim::vcard::{self, Card, Email};
use mail_runtime::carddav::{self, Dav, DavAuth, How};
use mail_runtime::{KeyringSecrets, OAuthRegistry, Secrets};
use mail_store::{AddressBook, Edit, Group, GroupHome, GroupId, Kind, Origin, SqliteStore, Store};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;

/// What `mailo contacts …` asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contacts {
    /// Autocomplete: the best matches for what was typed, or the top of the book.
    Find {
        typed: String,
        limit: usize,
    },
    /// Add or rename a contact by hand.
    Add {
        address: String,
        name: Option<String>,
    },
    Remove {
        address: String,
    },
    /// Read a `.vcf` file of any version into the book.
    Import {
        path: PathBuf,
    },
    /// Write the book as vCard 4.0, to a file or to standard output.
    Export {
        path: Option<PathBuf>,
    },
    /// Sync a CardDAV address book, or every one synced before when no URL is given.
    Sync {
        url: Option<String>,
        /// The account it belongs to, by address. The only one, when there is only one.
        account: Option<String>,
        /// The address book's own login; its password comes from `MAILO_PASSWORD` and is kept
        /// in the keyring. Without one, the account's own sign-in is presented.
        user: Option<String>,
    },
}

/// Parse what follows `contacts`.
pub fn parse(args: &[String]) -> Result<Contacts, String> {
    let rest = |from: usize| args.get(from..).unwrap_or_default().join(" ");
    match args.first().map(String::as_str) {
        Some("add") => {
            let address = args
                .get(1)
                .ok_or("contacts add needs an address: mailo contacts add ada@example.com Ada")?;
            let name = rest(2);
            Ok(Contacts::Add {
                address: address.clone(),
                name: (!name.trim().is_empty()).then(|| name.trim().to_owned()),
            })
        }
        Some("remove") => Ok(Contacts::Remove {
            address: args
                .get(1)
                .ok_or("contacts remove needs an address")?
                .clone(),
        }),
        Some("import") => Ok(Contacts::Import {
            path: args
                .get(1)
                .map(PathBuf::from)
                .ok_or("contacts import needs a .vcf file")?,
        }),
        Some("export") => Ok(Contacts::Export {
            path: args.get(1).map(PathBuf::from),
        }),
        Some("sync") => {
            let (mut url, mut account, mut user) = (None, None, None);
            let mut words = args[1..].iter();
            while let Some(word) = words.next() {
                match word.as_str() {
                    "--account" => {
                        account = Some(words.next().ok_or("--account needs an address")?.clone());
                    }
                    "--user" => user = Some(words.next().ok_or("--user needs a login")?.clone()),
                    flag if flag.starts_with("--") => {
                        return Err(format!("unknown option {flag:?}"));
                    }
                    _ if url.is_none() => url = Some(word.clone()),
                    other => return Err(format!("unexpected {other:?}")),
                }
            }
            Ok(Contacts::Sync { url, account, user })
        }
        _ => Ok(Contacts::Find {
            typed: rest(0),
            limit: 20,
        }),
    }
}

/// Run a contacts command, returning what to print.
pub fn run(
    store: &SqliteStore,
    command: &Contacts,
    saved: &OAuthRegistry,
    now: DateTime<Utc>,
) -> Result<String, String> {
    match command {
        Contacts::Find { typed, limit } => find(store, typed, *limit),
        Contacts::Add { address, name } => {
            let contact = store
                .put_contact(address, name.as_deref(), &Origin::Manual)
                .map_err(|e| e.to_string())?;
            Ok(format!("added {}\n", shown(&contact)))
        }
        Contacts::Remove { address } => match store.delete_contact(address) {
            Ok(true) => Ok(format!("removed {address}\n")),
            Ok(false) => Err(format!("no contact {address:?}")),
            Err(e) => Err(e.to_string()),
        },
        Contacts::Import { path } => {
            let bytes =
                std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            import(store, &bytes)
        }
        Contacts::Export { path } => {
            let text = export(store)?;
            match path {
                None => Ok(text),
                Some(path) => {
                    std::fs::write(path, &text)
                        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
                    Ok(format!("wrote {}\n", path.display()))
                }
            }
        }
        Contacts::Sync { url, account, user } => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("cannot sync: {e}"))?;
            runtime.block_on(sync(
                store,
                url.as_deref(),
                account.as_deref(),
                user.as_deref(),
                saved,
                now,
            ))
        }
    }
}

/// One line per match, best first: `Name <address>`, or the address alone.
pub fn find(store: &dyn Store, typed: &str, limit: usize) -> Result<String, String> {
    let found = store
        .contacts_matching(typed, limit)
        .map_err(|e| e.to_string())?;
    if found.is_empty() {
        return Ok("no contacts match\n".to_owned());
    }
    Ok(found.iter().map(|c| format!("{}\n", shown(c))).collect())
}

fn shown(contact: &mail_store::Contact) -> String {
    match &contact.name {
        Some(name) => format!("{name} <{}>", contact.address),
        None => contact.address.clone(),
    }
}

/// Every address on every card in `bytes`, added by hand under the card's name, and every
/// `KIND:group` card as a group of this book ([`import_group`]).
pub fn import(store: &dyn Store, bytes: &[u8]) -> Result<String, String> {
    let cards = vcard::parse_bytes(bytes);
    let (mut added, mut empty, mut groups) = (0, 0, 0);
    for card in &cards {
        if card.is_group() {
            import_group(store, card, &cards).map_err(|e| e.to_string())?;
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
    let mut out = format!("imported {added} addresses from {} cards\n", cards.len());
    if groups > 0 {
        let _ = writeln!(
            out,
            "imported {groups} {}",
            if groups == 1 { "group" } else { "groups" }
        );
    }
    if empty > 0 {
        let _ = writeln!(out, "{empty} cards had no email address and were skipped");
    }
    Ok(out)
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
pub fn export(store: &dyn Store) -> Result<String, String> {
    let failed = |e: mail_store::StoreError| e.to_string();
    let mut cards: Vec<Card> = Vec::new();
    let mut covered: BTreeSet<String> = BTreeSet::new();
    let groups = store.groups().map_err(failed)?;
    for book in store.address_books().map_err(failed)? {
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
    for contact in store.contacts().map_err(failed)? {
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

async fn sync(
    store: &SqliteStore,
    url: Option<&str>,
    account: Option<&str>,
    user: Option<&str>,
    saved: &OAuthRegistry,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let accounts = crate::sync::configured(store)?;
    let secrets = KeyringSecrets;
    let http = carddav::client().map_err(|e| e.to_string())?;
    let mut out = String::new();

    let Some(url) = url else {
        let books = store.address_books().map_err(|e| e.to_string())?;
        if books.is_empty() {
            return Ok("no address book has been synced yet: mailo contacts sync <url>\n".into());
        }
        for book in books {
            let owner = accounts
                .iter()
                .find(|a| Some(a.id) == book.account)
                .ok_or_else(|| format!("{}: its account is no longer configured", book.url))?;
            let auth = auth_for(owner, book.login.as_deref(), &secrets, saved, now).await?;
            let base = parse_url(&book.url)?;
            let dav = Dav::new(http.clone(), &base, auth).map_err(|e| e.to_string())?;
            out.push_str(&sync_one(&dav, store, book, None).await?);
        }
        return Ok(out);
    };

    let owner = match account {
        Some(address) => accounts
            .iter()
            .find(|a| a.address.eq_ignore_ascii_case(address))
            .ok_or_else(|| format!("no account {address:?}"))?,
        None => match accounts.as_slice() {
            [only] => only,
            [] => return Err("add an account first: an address book is kept under one".into()),
            _ => return Err("name the account it belongs to: --account you@example.com".into()),
        },
    };
    let auth = auth_for(owner, user, &secrets, saved, now).await?;
    let start = parse_url(url)?;
    let dav = Dav::new(http, &start, auth).map_err(|e| e.to_string())?;
    let found = carddav::discover(&dav, &start)
        .await
        .map_err(|e| e.to_string())?;
    for collection in found {
        let key = collection.url.to_string();
        let mut book = store
            .address_book(&key)
            .map_err(|e| e.to_string())?
            .unwrap_or(AddressBook {
                url: key,
                ..AddressBook::default()
            });
        book.account = Some(owner.id);
        book.login = user.map(str::to_owned);
        out.push_str(&sync_one(&dav, store, book, collection.name).await?);
    }
    Ok(out)
}

async fn sync_one(
    dav: &Dav,
    store: &SqliteStore,
    book: AddressBook,
    name: Option<String>,
) -> Result<String, String> {
    let url = book.url.clone();
    let done = carddav::sync(dav, store, book)
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    let how = match done.how {
        How::Incremental => "changes since last time",
        How::Full => "everything",
        How::Etags => "everything, compared by etag",
    };
    let mut out = format!(
        "{} ({url}): {} changed, {} removed — {how}\n",
        name.as_deref().unwrap_or("address book"),
        done.changed,
        done.removed
    );
    if done.written > 0 {
        let _ = writeln!(out, "  {} edited groups written back", done.written);
    }
    for unwritten in &done.unwritten {
        let _ = writeln!(
            out,
            "  {} not written back: {}",
            unwritten.name, unwritten.why
        );
    }
    Ok(out)
}

fn parse_url(text: &str) -> Result<url::Url, String> {
    let with_scheme = if text.contains("://") {
        text.to_owned()
    } else {
        format!("https://{text}")
    };
    url::Url::parse(&with_scheme).map_err(|e| format!("{text:?} is not a URL: {e}"))
}

/// How to sign in to an address book kept under `account`.
///
/// With a login of its own, a password: `MAILO_PASSWORD` when set, which is then kept in the
/// keyring, else the one kept there before. Without one, the account's own credential — its
/// OAuth token renewed if need be, or its password under its login.
async fn auth_for(
    account: &crate::sync::Configured,
    login: Option<&str>,
    secrets: &dyn Secrets,
    saved: &OAuthRegistry,
    now: DateTime<Utc>,
) -> Result<DavAuth, String> {
    if let Some(login) = login {
        let key = SecretKey {
            account: account.id,
            purpose: SecretPurpose::AddressBook,
        };
        let password = match std::env::var("MAILO_PASSWORD") {
            Ok(password) if !password.is_empty() => {
                secrets
                    .put(&key, &Credential::Password(password.clone()))
                    .map_err(|e| format!("cannot save the password: {e}"))?;
                password
            }
            _ => match secrets.get(&key) {
                Ok(Credential::Password(password)) => password,
                _ => {
                    return Err(format!(
                        "no password stored for {login}. Re-run with MAILO_PASSWORD set"
                    ));
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
            account: account.id,
            purpose: SecretPurpose::IncomingPassword,
        })
        .map_err(|_| crate::view::no_credential(&account.address, &account.plan.auth))?;
    match crate::sync::signed_in(account, stored, secrets, saved, now).await? {
        Credential::OAuth { access, .. } => Ok(DavAuth::Bearer(access)),
        Credential::Password(password) => Ok(DavAuth::Basic {
            user: account.plan.username(),
            password,
        }),
        Credential::OpenPgp(_) | Credential::SmimeKey(_) => Err(format!(
            "the credential stored for {} is a private key, not a sign-in",
            account.address
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn every_form_parses_to_what_it_says() {
        let cases: Vec<(&str, Contacts)> = vec![
            (
                "",
                Contacts::Find {
                    typed: String::new(),
                    limit: 20,
                },
            ),
            (
                "ada love",
                Contacts::Find {
                    typed: "ada love".into(),
                    limit: 20,
                },
            ),
            (
                "add ada@example.test Ada Lovelace",
                Contacts::Add {
                    address: "ada@example.test".into(),
                    name: Some("Ada Lovelace".into()),
                },
            ),
            (
                "add ada@example.test",
                Contacts::Add {
                    address: "ada@example.test".into(),
                    name: None,
                },
            ),
            (
                "remove ada@example.test",
                Contacts::Remove {
                    address: "ada@example.test".into(),
                },
            ),
            (
                "import cards.vcf",
                Contacts::Import {
                    path: "cards.vcf".into(),
                },
            ),
            ("export", Contacts::Export { path: None }),
            (
                "export out.vcf",
                Contacts::Export {
                    path: Some("out.vcf".into()),
                },
            ),
            (
                "sync",
                Contacts::Sync {
                    url: None,
                    account: None,
                    user: None,
                },
            ),
            (
                "sync https://dav.example.test/ --user ada --account me@example.test",
                Contacts::Sync {
                    url: Some("https://dav.example.test/".into()),
                    account: Some("me@example.test".into()),
                    user: Some("ada".into()),
                },
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(parse(&args(line)), Ok(expected), "{line:?}");
        }
    }

    #[test]
    fn a_malformed_form_says_what_is_missing() {
        for line in [
            "add",
            "remove",
            "import",
            "sync --user",
            "sync a b",
            "sync --bogus",
        ] {
            assert!(parse(&args(line)).is_err(), "{line:?}");
        }
    }

    #[test]
    fn an_imported_file_is_offered_and_exported_back_as_4_0() {
        let store = mail_store::MemoryStore::new();
        let file = "BEGIN:VCARD\r\nVERSION:2.1\r\nFN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Ren=C3=A9e\r\n\
                    EMAIL;INTERNET:Renee@example.test\r\nEND:VCARD\r\n\
                    BEGIN:VCARD\r\nVERSION:3.0\r\nFN:No Mail\r\nTEL:1\r\nEND:VCARD\r\n";
        let said = import(&store, file.as_bytes()).unwrap();
        assert!(
            said.starts_with("imported 1 addresses from 2 cards"),
            "{said}"
        );
        assert!(said.contains("1 cards had no email address"), "{said}");
        assert_eq!(
            find(&store, "ren", 5).unwrap(),
            "Renée <renee@example.test>\n"
        );
        let exported = export(&store).unwrap();
        let back = vcard::parse(&exported);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].formatted_name.as_deref(), Some("Renée"));
        assert_eq!(back[0].emails[0].address, "renee@example.test");
        assert!(exported.contains("VERSION:4.0"));
        assert_eq!(find(&store, "zzz", 5).unwrap(), "no contacts match\n");
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
        let said = import(&store, file.as_bytes()).unwrap();
        assert!(said.contains("imported 1 group"), "{said}");
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
