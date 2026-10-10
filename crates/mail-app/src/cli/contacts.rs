//! `mailo contacts`: the address book from the command line.
//!
//! The window asks the store the same question with `Store::contacts_matching`; this is the
//! CLI's side of it, plus the things only a command line does: importing and exporting `.vcf`
//! files and syncing a CardDAV address book. What the book does is [`mail_core::contacts`]'s;
//! this reads the words and says the result.

use super::SqliteStore;
use chrono::{DateTime, Utc};
use mail_core::Environment;
use mail_core::contacts::{self, BookSync, How, Imported, Synced};
use mail_core::error::CoreError;
use mail_runtime::{AccountSecrets, ClientRegistry};
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
                    "--user" => {
                        user = Some(words.next().ok_or("--user needs a login")?.clone());
                    }
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

/// Run a contacts command over `store`, signing in through `secrets`, returning what to print.
pub async fn run(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    env: &Environment,
    command: &Contacts,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<String, CoreError> {
    match command {
        Contacts::Find { typed, limit } => {
            let found = contacts::find(store, typed, *limit)?;
            if found.is_empty() {
                return Ok("no contacts match\n".to_owned());
            }
            Ok(found
                .iter()
                .map(|c| format!("{}\n", shown(c.name.as_deref(), &c.address)))
                .collect())
        }
        Contacts::Add { address, name } => {
            let contact = contacts::add(store, address, name.as_deref())?;
            Ok(format!(
                "added {}\n",
                shown(contact.name.as_deref(), &contact.address)
            ))
        }
        Contacts::Remove { address } => {
            contacts::remove(store, address)?;
            Ok(format!("removed {address}\n"))
        }
        Contacts::Import { path } => {
            let bytes = std::fs::read(path)
                .map_err(|e| CoreError::cannot(format!("read {}", path.display()), e))?;
            Ok(imported(&contacts::import(store, &bytes)?))
        }
        Contacts::Export { path } => {
            let text = contacts::export(store)?;
            match path {
                None => Ok(text),
                Some(path) => {
                    std::fs::write(path, &text)
                        .map_err(|e| CoreError::cannot(format!("write {}", path.display()), e))?;
                    Ok(format!("wrote {}\n", path.display()))
                }
            }
        }
        Contacts::Sync { url, account, user } => {
            let done = contacts::sync(
                store,
                url.as_deref(),
                account.as_deref(),
                user.as_deref(),
                secrets,
                env,
                saved,
                now,
            )
            .await?;
            Ok(synced(&done))
        }
    }
}

/// `Name <address>`, or the address alone.
fn shown(name: Option<&str>, address: &str) -> String {
    mail_domain::Address {
        name: name.map(str::to_owned),
        email: address.to_owned(),
    }
    .to_string()
}

/// What reading a `.vcf` file says.
pub fn imported(done: &Imported) -> String {
    let mut out = format!(
        "imported {} addresses from {} cards\n",
        done.addresses, done.cards
    );
    if done.groups > 0 {
        let _ = writeln!(
            out,
            "imported {} {}",
            done.groups,
            if done.groups == 1 { "group" } else { "groups" }
        );
    }
    if done.empty > 0 {
        let _ = writeln!(
            out,
            "{} cards had no email address and were skipped",
            done.empty
        );
    }
    out
}

/// What a sync of address books says: a line for each, and what it wrote back.
pub fn synced(done: &Synced) -> String {
    match done {
        Synced::NothingYet => {
            "no address book has been synced yet: mailo contacts sync <url>\n".to_owned()
        }
        Synced::Books(books) => books.iter().map(book_synced).collect(),
    }
}

fn book_synced(book: &BookSync) -> String {
    let how = match book.done.how {
        How::Incremental => "changes since last time",
        How::Full => "everything",
        How::Etags => "everything, compared by etag",
    };
    let mut out = format!(
        "{} ({}): {} changed, {} removed — {how}\n",
        book.name.as_deref().unwrap_or("address book"),
        book.url,
        book.done.changed,
        book.done.removed
    );
    if book.done.written > 0 {
        let _ = writeln!(out, "  {} edited groups written back", book.done.written);
    }
    for unwritten in &book.done.unwritten {
        let _ = writeln!(
            out,
            "  {} not written back: {}",
            unwritten.name, unwritten.why
        );
    }
    out
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
            assert_eq!(parse(&args(line)).unwrap(), expected, "{line:?}");
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
        assert_eq!(
            parse(&args("sync --user")).unwrap_err(),
            "--user needs a login"
        );
    }

    #[test]
    fn an_import_says_how_many_addresses_groups_and_empty_cards() {
        let said = imported(&Imported {
            cards: 2,
            addresses: 1,
            groups: 0,
            empty: 1,
        });
        assert!(
            said.starts_with("imported 1 addresses from 2 cards"),
            "{said}"
        );
        assert!(said.contains("1 cards had no email address"), "{said}");
        let said = imported(&Imported {
            cards: 2,
            addresses: 1,
            groups: 1,
            empty: 0,
        });
        assert!(said.contains("imported 1 group"), "{said}");
    }

    #[test]
    fn a_sync_with_nothing_to_go_on_says_how_to_start() {
        assert!(synced(&Synced::NothingYet).starts_with("no address book has been synced yet"),);
    }
}
