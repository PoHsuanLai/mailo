//! `sync::run`, which is the function the binary calls and the one nothing could run.
//!
//! Every layer below this has tests — the session, the backend, the engine, the store — and the
//! assembly had none, because it reached for `KeyringSecrets` directly and a test cannot have a
//! keyring. What is worth checking here *is* the assembly: that a stored plan becomes a working
//! connection, that an account without a credential is skipped with a reason rather than
//! stopping the run, and that what the user is told matches what happened.
//!
//! The IMAP half is `#[ignore]`d because it needs `scripts/live-imapd.py`; everything that does
//! not need a server runs normally.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_runtime::{MapSecrets, OAuthRegistry, Secrets};
use mail_store::SqliteStore;
use std::sync::Arc;

#[allow(dead_code)]
#[path = "../src/account.rs"]
mod account;
#[allow(dead_code)]
#[path = "../src/attach.rs"]
mod attach;
#[allow(dead_code)]
#[path = "../src/cli.rs"]
mod cli;
#[allow(dead_code)]
#[path = "../src/compose.rs"]
mod compose;
#[path = "../src/sync.rs"]
mod sync;
#[allow(dead_code)]
#[path = "../src/view.rs"]
mod view;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// A store with one IMAP account pointed at `port`, in plaintext.
///
/// The plan is written directly rather than through `account add`, which only produces
/// `Tls::Implicit` — deliberately, since there is no configuration that sends a password in
/// clear. A loopback test server has no certificate a public root would sign, so this is the one
/// place that shape is constructed by hand.
fn configured(port: u16, caps: AccountCaps) -> (Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    let plan = AccountPlan {
        address: "ada@example.test".to_owned(),
        incoming: Incoming::Imap {
            host: "127.0.0.1".to_owned(),
            port,
            tls: Tls::Plaintext,
        },
        outgoing: Outgoing::Smtp {
            host: "127.0.0.1".to_owned(),
            port: 1,
            tls: Tls::Plaintext,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    };
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'ada@example.test', ?2, datetime('now'))",
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                ACCOUNT.to_string(),
                serde_json::to_string(&caps).unwrap(),
                now().to_rfc3339()
            ],
        )
        .unwrap();
    }
    (store, dir)
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles(Vec::new()),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        // Fresh, so the pass does not try to re-read capabilities from a server that may not be
        // there. Staleness is covered in `imap_end_to_end.rs`.
        observed_at: now(),
    }
}

fn with_password(store_secrets: &MapSecrets, password: &str) {
    store_secrets
        .put(
            &SecretKey {
                account: ACCOUNT,
                purpose: SecretPurpose::IncomingPassword,
            },
            &Credential::Password(password.to_owned()),
        )
        .unwrap();
}

#[test]
fn an_account_with_no_credential_is_skipped_with_a_reason() {
    // One account needing attention must not stop the others fetching mail, and the reason has
    // to name the command that fixes it — this is the first thing a new user sees.
    let (store, _dir) = configured(1, caps());
    let out = sync::run_with(
        store,
        Arc::new(MapSecrets::default()),
        &OAuthRegistry::default(),
        now(),
    )
    .expect("a missing credential is not a failure of the run");

    assert!(out.contains("ada@example.test"), "{out}");
    assert!(out.contains("no credential stored"), "{out}");
    assert!(
        out.contains("mailo account add"),
        "the way out is named: {out}"
    );
}

#[test]
fn an_unreachable_server_is_reported_per_account_not_thrown() {
    // Port 1 refuses. The pass should say so against that account and return.
    let (store, _dir) = configured(1, caps());
    let secrets = MapSecrets::default();
    with_password(&secrets, "s3cr3t-pass");

    let out = sync::run_with(store, Arc::new(secrets), &OAuthRegistry::default(), now())
        .expect("an unreachable server is reported, not returned as an error");
    assert!(out.contains("ada@example.test"), "{out}");
    assert!(
        out.to_lowercase().contains("connect") || out.to_lowercase().contains("refused"),
        "the reason should reach the user: {out}"
    );
}

#[test]
fn no_accounts_explains_how_to_add_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    let out = sync::run_with(
        store,
        Arc::new(MapSecrets::default()),
        &OAuthRegistry::default(),
        now(),
    )
    .unwrap();
    assert!(out.contains("mailo account add"), "{out}");
}

#[tokio::test]
#[ignore = "needs a local Twisted IMAP4 server; run deliberately with --ignored"]
async fn a_whole_pass_against_a_real_server_lands_mail_and_says_what_it_did() {
    // The assembly, end to end: stored plan, stored capabilities, a credential, a real server,
    // and a line of output a person reads.
    let (store, _dir) = configured(11143, caps());
    let secrets = MapSecrets::default();
    with_password(&secrets, "s3cr3t-pass");

    let store_for_pass = store.clone();
    let out = tokio::task::spawn_blocking(move || {
        sync::run_with(
            store_for_pass,
            Arc::new(secrets),
            &OAuthRegistry::default(),
            now(),
        )
    })
    .await
    .expect("the pass did not panic")
    .expect("a reachable server with a good credential syncs");

    eprintln!("{out}");
    assert!(out.contains("ada@example.test"), "{out}");
    assert!(
        !out.contains("needs attention") && !out.contains("protocol:"),
        "a clean pass should report neither trouble nor a protocol error: {out}"
    );
    assert!(
        out.contains("headers") && out.contains("bodies"),
        "the line a user reads should say what was fetched: {out}"
    );
    // The fixture serves two messages; they must be in the store afterwards.
    let count: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2, "the pass reported success and stored nothing");
}

/// Renewing a sign-in that has expired, which nothing did.
///
/// `mail_runtime::oauth` could refresh a token from the day it was written and no caller ever
/// asked it to: the sync path read the credential out of the keyring and handed it to the
/// backend verbatim. An OAuth account therefore fetched mail for about an hour and then failed
/// on every pass afterwards, permanently, with an authentication error — while the refresh token
/// that would have fixed it sat in the same keyring entry, unused. The client id needed to spend
/// it was not persisted anywhere either, so even a caller would have had nothing to call with.
///
/// No network: the token endpoint is a socket in this process, which is what substitutable
/// `Endpoints` are for.
mod renewing_an_expired_sign_in {
    use super::*;
    use mail_runtime::Registration;
    use mail_runtime::oauth::Endpoints;
    use std::io::{Read as _, Write as _};
    use std::sync::Mutex;

    /// What the token endpoint was asked, so the request shape can be asserted.
    type Seen = Arc<Mutex<Vec<String>>>;

    const RENEWED: &str =
        r#"{"access_token":"ya29.renewed","token_type":"Bearer","expires_in":3599}"#;

    /// A token endpoint on loopback that answers `body` to anything.
    fn token_endpoint(body: &'static str) -> (Endpoints, Seen) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let recording = seen.clone();
        std::thread::spawn(move || {
            for sock in listener.incoming() {
                let Ok(mut sock) = sock else { continue };
                let mut buf = vec![0u8; 8192];
                let read = match sock.read(&mut buf) {
                    Ok(0) | Err(_) => continue,
                    Ok(n) => n,
                };
                recording
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..read]).to_string());
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(response.as_bytes());
            }
        });
        (
            Endpoints {
                // Never fetched — the browser goes there — but it has to parse.
                auth: format!("http://127.0.0.1:{port}/authorize"),
                token: format!("http://127.0.0.1:{port}/token"),
            },
            seen,
        )
    }

    /// The same store as above, but an account that signs in with Google rather than a password.
    fn oauth_account() -> (Arc<SqliteStore>, tempfile::TempDir) {
        // Port 1: the connection after the renewal is not the subject here and is expected to
        // fail. What is being tested is what happens before it.
        let (store, dir) = configured(1, caps());
        let plan = AccountPlan {
            address: "ada@example.test".to_owned(),
            incoming: Incoming::Imap {
                host: "127.0.0.1".to_owned(),
                port: 1,
                tls: Tls::Plaintext,
            },
            outgoing: Outgoing::Smtp {
                host: "127.0.0.1".to_owned(),
                port: 1,
                tls: Tls::Plaintext,
            },
            auth: AuthPlan::OAuth {
                issuer: OAuthIssuer::Google,
                scopes: vec!["https://mail.google.com/".to_owned()],
            },
            identities: Vec::new(),
        };
        store
            .connection()
            .execute(
                "UPDATE accounts SET plan = ?1 WHERE id = ?2",
                rusqlite::params![serde_json::to_string(&plan).unwrap(), ACCOUNT.to_string()],
            )
            .unwrap();
        (store, dir)
    }

    fn key() -> SecretKey {
        SecretKey {
            account: ACCOUNT,
            purpose: SecretPurpose::IncomingPassword,
        }
    }

    /// An OAuth credential expiring `minutes` from `now()`.
    fn token(minutes: i64) -> Credential {
        Credential::OAuth {
            access: "ya29.stale".to_owned(),
            refresh: "the-refresh-token".to_owned(),
            expires_at: now() + chrono::TimeDelta::try_minutes(minutes).unwrap(),
        }
    }

    fn registry(ends: Endpoints) -> OAuthRegistry {
        let mut registry = OAuthRegistry::default();
        registry.set(Registration::new(OAuthIssuer::Google, "client-id").at(ends));
        registry
    }

    #[test]
    fn an_expired_access_token_is_renewed_and_the_new_one_stored() {
        let (store, _dir) = oauth_account();
        let secrets: Arc<dyn Secrets> = Arc::new(MapSecrets::default());
        // Two hours past expiry, which is where every OAuth account ended up an hour after it
        // was added and stayed forever.
        secrets.put(&key(), &token(-120)).unwrap();
        let (ends, seen) = token_endpoint(RENEWED);

        let _ = sync::run_with(store, secrets.clone(), &registry(ends), now());

        let asked = seen.lock().unwrap().join("\n");
        assert!(
            asked.contains("grant_type=refresh_token"),
            "the issuer was never asked to renew: {asked:?}"
        );
        assert!(
            asked.contains("refresh_token=the-refresh-token"),
            "the stored refresh token was not the one spent: {asked:?}"
        );

        match secrets.get(&key()).unwrap() {
            Credential::OAuth {
                access,
                refresh,
                expires_at,
            } => {
                assert_eq!(access, "ya29.renewed", "the new token was not written back");
                // This issuer returned no new refresh token, which is the usual case. Dropping
                // the old one logs the user out at the next expiry with nothing to recover from.
                assert_eq!(refresh, "the-refresh-token");
                assert!(expires_at > now(), "{expires_at} is not in the future");
            }
            other => panic!("the stored credential became {other:?}"),
        }
    }

    #[test]
    fn the_renewed_token_is_written_under_both_keys() {
        // `account add` writes the credential twice, under `IncomingPassword` and under
        // `OAuthRefresh`. Renewing only one leaves the other stale, which is the same account
        // failing an hour later by a different route.
        let (store, _dir) = oauth_account();
        let secrets: Arc<dyn Secrets> = Arc::new(MapSecrets::default());
        secrets.put(&key(), &token(-120)).unwrap();
        let (ends, _seen) = token_endpoint(RENEWED);

        let _ = sync::run_with(store, secrets.clone(), &registry(ends), now());

        let stored = secrets
            .get(&SecretKey {
                account: ACCOUNT,
                purpose: SecretPurpose::OAuthRefresh,
            })
            .expect("the refresh entry should have been rewritten");
        match stored {
            Credential::OAuth { access, .. } => assert_eq!(access, "ya29.renewed"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_token_that_is_still_good_is_not_sent_to_the_issuer() {
        // Renewing on every pass would turn a five-minute poll into a five-minute round trip to
        // the issuer, and issuers rate-limit that.
        let (store, _dir) = oauth_account();
        let secrets: Arc<dyn Secrets> = Arc::new(MapSecrets::default());
        secrets.put(&key(), &token(45)).unwrap();
        let (ends, seen) = token_endpoint(RENEWED);

        let _ = sync::run_with(store, secrets.clone(), &registry(ends), now());

        assert!(
            seen.lock().unwrap().is_empty(),
            "a valid token was sent to the issuer anyway: {:?}",
            seen.lock().unwrap()
        );
        assert_eq!(secrets.get(&key()).unwrap(), token(45), "and left alone");
    }

    #[test]
    fn a_password_account_never_reaches_the_issuer() {
        // The plan is what decides, not the credential: a password has no expiry and there is
        // nothing to renew it with.
        let (store, _dir) = configured(1, caps());
        let secrets: Arc<dyn Secrets> = Arc::new(MapSecrets::default());
        secrets
            .put(&key(), &Credential::Password("hunter2".to_owned()))
            .unwrap();
        let (ends, seen) = token_endpoint(RENEWED);

        let _ = sync::run_with(store, secrets, &registry(ends), now());

        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn an_expired_sign_in_with_no_client_id_says_so_instead_of_failing_to_authenticate() {
        // "authentication failed" points at the password the user does not have. The actual
        // problem is that this installation cannot renew, and the message has to say that.
        let (store, _dir) = oauth_account();
        let secrets: Arc<dyn Secrets> = Arc::new(MapSecrets::default());
        secrets.put(&key(), &token(-120)).unwrap();

        let out = sync::run_with(store, secrets, &OAuthRegistry::default(), now()).unwrap();

        assert!(out.contains("no OAuth client id is configured"), "{out}");
        // Read by a person, and `cargo fmt` collapses a `\`-continuation in a literal into a
        // run of spaces in the middle of the sentence.
        assert!(!out.contains("  "), "a run of spaces in a message: {out:?}");
        assert!(out.contains("MAILO_OAUTH_CLIENT_ID"), "{out}");
        assert!(out.contains("ada@example.test"), "{out}");
    }
}
