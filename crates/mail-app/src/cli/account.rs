//! `account remove <address> [--yes]`: what it would take with it, and then, asked again with
//! `--yes`, taking it.
//!
//! Without `--yes` nothing is removed and the run fails, so a script that forgot it does not read
//! a removal into a success. The words are the window's sheet's, said for a terminal.

use super::{Consent, account_named};
use mail_domain::{AccountPlan, Incoming, LeaveOnServer};
use mail_runtime::Secrets;
use mail_store::{SqliteStore, Store};
use std::path::Path;

/// Remove the account at `address`, or say what removing it would take. `config` is where the
/// offline setting is kept, when there is a config directory.
pub(super) fn remove(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    config: Option<&Path>,
    address: &str,
    consent: Consent,
) -> Result<String, String> {
    let id = account_named(store, address)?;
    match consent {
        Consent::Ask => {
            let held = store.offline(id).map_or(0, |offline| offline.messages);
            let incoming = plan_of(store, address).map(|plan| plan.incoming);
            Err(asking(address, held, incoming.as_ref()))
        }
        Consent::Given => {
            let removed =
                mail_core::account::remove(store, secrets, id).map_err(|e| e.to_string())?;
            if let Some(config) = config {
                let _ = mail_core::offline::save(config, id, mail_core::offline::Keep::Bodies);
            }
            Ok(format!(
                "removed {} and {} of stored mail\n",
                removed.address,
                mail_core::attach::human_size(removed.freed.bytes)
            ))
        }
    }
}

fn plan_of(store: &SqliteStore, address: &str) -> Option<AccountPlan> {
    let plan: String = store
        .connection()
        .query_row(
            "SELECT plan FROM accounts WHERE address = ?1",
            [address.to_lowercase()],
            |r| r.get(0),
        )
        .ok()?;
    serde_json::from_str(&plan).ok()
}

/// What removing `address`, which holds `held` messages here, would take, and how to go on.
fn asking(address: &str, held: u64, incoming: Option<&Incoming>) -> String {
    let mail = match held {
        1 => "1 message".to_owned(),
        n => format!("{n} messages"),
    };
    let server = match incoming {
        Some(Incoming::Pop3 {
            leave: LeaveOnServer::DeleteAfterFetch,
            ..
        }) => format!(
            "This server deletes mail once it is downloaded, so these {mail} are the only copy \
             and will be gone for good."
        ),
        _ => "Mail on the server is not touched.".to_owned(),
    };
    format!(
        "removing {address} takes its {mail} on this computer, its folders and rules and its \
         saved sign-in. {server} Keys and certificates stay.\nNothing was removed. Run it again \
         with --yes to remove it."
    )
}

#[cfg(test)]
mod tests {
    use super::{asking, remove};
    use crate::cli::Consent;
    use mail_domain::{Incoming, LeaveOnServer, Tls, presets};
    use mail_runtime::MapSecrets;
    use mail_store::SqliteStore;

    const ADDRESS: &str = "me@nowhere.example";

    fn accounts(store: &SqliteStore) -> i64 {
        store
            .connection()
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn without_yes_nothing_is_removed_and_with_it_the_account_goes() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        let manual = presets::Manual {
            imap_host: "imap.nowhere.example".to_owned(),
            imap_port: 993,
            smtp_host: "smtp.nowhere.example".to_owned(),
            smtp_port: 465,
            login: None,
        };
        let preset = presets::manual(ADDRESS, &manual, chrono::Utc::now());
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, ?2, ?3, datetime('now'))",
                [
                    mail_domain::AccountId::generate().to_string(),
                    ADDRESS.to_owned(),
                    serde_json::to_string(&preset.plan).unwrap(),
                ],
            )
            .unwrap();
        let secrets = MapSecrets::default();

        let asked = remove(&store, &secrets, None, ADDRESS, Consent::Ask).unwrap_err();
        assert!(asked.contains("Nothing was removed"), "{asked}");
        assert!(asked.contains("its 0 messages"), "{asked}");
        assert_eq!(accounts(&store), 1);

        let said = remove(&store, &secrets, None, ADDRESS, Consent::Given).unwrap();
        assert!(said.starts_with("removed me@nowhere.example"), "{said}");
        assert_eq!(accounts(&store), 0);

        let again = remove(&store, &secrets, None, ADDRESS, Consent::Given).unwrap_err();
        assert!(again.contains("no account for"), "{again}");
    }

    #[test]
    fn asking_names_the_mail_and_whether_the_server_keeps_a_copy() {
        let pop3 = |leave| Incoming::Pop3 {
            host: "pop.example.test".to_owned(),
            port: 995,
            tls: Tls::Implicit,
            leave,
        };
        // (held, incoming, words it has, words it lacks)
        let cases = [
            (1, None, "its 1 message on this computer", "only copy"),
            (
                12,
                Some(pop3(LeaveOnServer::Keep)),
                "not touched",
                "only copy",
            ),
            (
                12,
                Some(pop3(LeaveOnServer::DeleteAfterFetch)),
                "these 12 messages are the only copy",
                "not touched",
            ),
        ];
        for (held, incoming, has, lacks) in cases {
            let said = asking("me@example.test", held, incoming.as_ref());
            assert!(said.contains(has), "{said}");
            assert!(!said.contains(lacks), "{said}");
            assert!(said.contains("Nothing was removed"), "{said}");
            assert!(said.contains("--yes"), "{said}");
        }
    }
}
