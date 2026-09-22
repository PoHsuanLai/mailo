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
#[allow(dead_code)]
#[path = "../src/query.rs"]
mod query;
#[allow(dead_code)]
#[path = "../src/snooze.rs"]
mod snooze;
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
    configured_with(
        port,
        caps,
        AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
    )
}

/// The same, with the account's authentication spelled out.
fn configured_with(
    port: u16,
    caps: AccountCaps,
    auth: AuthPlan,
) -> (Arc<SqliteStore>, tempfile::TempDir) {
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
        auth,
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
    .map(|ran| ran.text)
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
        .map(|ran| ran.text)
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
    .unwrap()
    .text;
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
    .map(|ran| ran.text)
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

    // And the server's flags reached the store.
    //
    // The fixture serves both messages as `\Seen`. They arrived as unread, because a header
    // fetch does not carry flags — the flag sweep does, and `AccountEngine::sweep` was called
    // from nowhere but its own tests. Every message in the application therefore stayed unread
    // for ever: mail read on a phone stayed bold here, the unread counts were the mailbox size,
    // and on Gmail the labels never appeared either, because they ride the same survey. Found
    // against a real account, where 138 messages the user had *sent* were all marked unread.
    let unread: i64 = store
        .connection()
        .query_row(
            "SELECT count(*) FROM messages WHERE read = '\"unread\"'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        unread, 0,
        "the server reports both messages \\Seen; the pass never swept for flags"
    );
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

        let out = sync::run_with(store, secrets, &OAuthRegistry::default(), now())
            .unwrap()
            .text;

        assert!(out.contains("no OAuth client id is configured"), "{out}");
        // Read by a person, and `cargo fmt` collapses a `\`-continuation in a literal into a
        // run of spaces in the middle of the sentence.
        assert!(!out.contains("  "), "a run of spaces in a message: {out:?}");
        assert!(out.contains("MAILO_OAUTH_CLIENT_ID"), "{out}");
        assert!(out.contains("ada@example.test"), "{out}");
    }
}

/// Which mailboxes a pass fetches.
mod folders {
    use super::*;

    fn caps_with(folders: Vec<(String, MailboxRole)>) -> AccountCaps {
        let mut caps = caps();
        caps.folders = FolderRoles(folders);
        caps
    }

    fn paths(store: &Arc<SqliteStore>, caps: AccountCaps) -> Vec<String> {
        store
            .connection()
            .execute(
                "UPDATE account_caps SET caps = ?1 WHERE account = ?2",
                rusqlite::params![serde_json::to_string(&caps).unwrap(), ACCOUNT.to_string()],
            )
            .unwrap();
        // Through the same function the pass uses, rather than a copy of its rules — the
        // mistake F116 was about.
        sync::mailboxes_by_account(store)
            .unwrap()
            .into_iter()
            .next()
            .expect("one account")
            .1
    }

    #[test]
    fn a_server_with_no_folder_roles_gets_the_inbox_alone() {
        // POP3, and any IMAP server that answered no `LIST (SPECIAL-USE)` flags. This is what
        // happened before more than one mailbox was ever fetched, and it must keep happening.
        let (store, _dir) = configured(1, caps());
        assert_eq!(paths(&store, caps_with(vec![])), vec!["INBOX".to_owned()]);
    }

    #[test]
    fn sent_is_fetched_where_the_server_names_one() {
        let (store, _dir) = configured(1, caps());
        let found = paths(
            &store,
            caps_with(vec![
                ("[Gmail]/Sent Mail".to_owned(), MailboxRole::Sent),
                ("[Gmail]/All Mail".to_owned(), MailboxRole::Archive),
                ("[Gmail]/Spam".to_owned(), MailboxRole::Spam),
                ("[Gmail]/Drafts".to_owned(), MailboxRole::Drafts),
            ]),
        );
        assert_eq!(
            found,
            vec!["INBOX".to_owned(), "[Gmail]/Sent Mail".to_owned()],
            "Archive, Spam and Drafts are deliberately not fetched — see `to_sync`"
        );
        assert_eq!(found[0], "INBOX", "the inbox is fetched first and always");
    }

    #[test]
    fn a_server_that_calls_its_inbox_sent_does_not_get_it_twice() {
        // Defensive: a server is free to flag INBOX with a special use, and fetching the same
        // mailbox twice in one pass would double every count in the report.
        let (store, _dir) = configured(1, caps());
        assert_eq!(
            paths(
                &store,
                caps_with(vec![("INBOX".to_owned(), MailboxRole::Sent)])
            ),
            vec!["INBOX".to_owned()]
        );
    }
}

/// How often the background loop runs.
mod polling {
    use super::*;

    fn with_watch(store: &Arc<SqliteStore>, watch: WatchMode) {
        let mut caps = caps();
        caps.watch = watch;
        store
            .connection()
            .execute(
                "UPDATE account_caps SET caps = ?1 WHERE account = ?2",
                rusqlite::params![serde_json::to_string(&caps).unwrap(), ACCOUNT.to_string()],
            )
            .unwrap();
    }

    #[test]
    fn the_interval_comes_from_the_account_rather_than_a_constant() {
        let (store, _dir) = configured(1, caps());
        with_watch(
            &store,
            WatchMode::Poll {
                every: std::time::Duration::from_secs(90),
            },
        );
        assert_eq!(
            sync::poll_interval(&store),
            std::time::Duration::from_secs(90)
        );
    }

    #[test]
    fn an_idle_capable_account_is_polled_like_any_other_until_idle_is_held() {
        // `WatchMode::Idle` means the server offers a long-lived connection this loop does not
        // hold. Treating it as "no polling needed" would mean an IMAP account never syncing.
        let (store, _dir) = configured(1, caps());
        with_watch(&store, WatchMode::Idle);
        assert_eq!(
            sync::poll_interval(&store),
            std::time::Duration::from_secs(300),
            "an IDLE account fell through to no polling at all"
        );
    }

    #[test]
    fn a_database_with_no_accounts_still_answers() {
        // The loop starts before anything is configured, and asking an empty store must not be
        // an error it has to handle.
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
        assert_eq!(
            sync::poll_interval(&store),
            std::time::Duration::from_secs(300)
        );
    }
}

/// What a pass reports when the server refuses the credential.
///
/// The poll loop stops on this rather than backing off, because five minutes is 288 attempts a
/// day and 288 failed logins a day against the user's own mail server is how an account gets
/// locked. That rule is only worth anything if the classification actually fires — so this drives
/// a real pass against a server that says no, rather than trusting the chain by reading it.
mod a_refused_sign_in {
    use super::*;
    use std::io::{Read, Write};

    /// An IMAP server that greets, then refuses whatever it is asked to log in with.
    ///
    /// It words the refusal the way Exchange, Courier and UW-imapd do — a bare `NO LOGIN
    /// failed.`, with none of the response codes Dovecot and Gmail send. Picking the wording
    /// that already worked would have made this test agree with itself.
    fn serve_refusing() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for sock in listener.incoming() {
                let Ok(mut sock) = sock else { continue };
                let _ = sock.write_all(b"* OK [CAPABILITY IMAP4rev1] ready\r\n");
                let mut buf = [0u8; 4096];
                while let Ok(n) = sock.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    for line in text.lines() {
                        let Some(tag) = line.split_whitespace().next() else {
                            continue;
                        };
                        let upper = line.to_uppercase();
                        let reply = if upper.contains("CAPABILITY") {
                            format!("* CAPABILITY IMAP4rev1\r\n{tag} OK done\r\n")
                        } else if upper.contains("LOGOUT") {
                            format!("* BYE\r\n{tag} OK done\r\n")
                        } else {
                            format!("{tag} NO LOGIN failed.\r\n")
                        };
                        let _ = sock.write_all(reply.as_bytes());
                    }
                }
            }
        });
        port
    }

    #[test]
    fn the_pass_says_the_credential_was_rejected() {
        let (store, _dir) = configured(serve_refusing(), caps());
        let secrets = MapSecrets::default();
        with_password(&secrets, "definitely-not-the-password");

        let ran =
            sync::run_with(store, Arc::new(secrets), &OAuthRegistry::default(), now()).unwrap();

        assert!(
            ran.rejected,
            "a refused sign-in was not reported as one: {}",
            ran.text
        );
        // And the user is told, in the text as well as in the flag.
        assert!(
            ran.text.to_lowercase().contains("login failed"),
            "the server's own words should reach the user: {}",
            ran.text
        );
    }

    /// The control, and the half that matters more: a server that is merely *down* must not be
    /// classified as a refusal. If it were, one flaky minute of network would stop the loop until
    /// the user next noticed, and the stored credential was never the problem.
    #[test]
    fn a_server_that_is_simply_down_is_not_a_refusal() {
        let (store, _dir) = configured(1, caps()); // port 1 refuses the connection
        let secrets = MapSecrets::default();
        with_password(&secrets, "the-right-password");

        let ran =
            sync::run_with(store, Arc::new(secrets), &OAuthRegistry::default(), now()).unwrap();

        assert!(
            !ran.rejected,
            "an unreachable server was blamed on the credential: {}",
            ran.text
        );
        match view::next_sync(
            view::Passed::Transient,
            3,
            std::time::Duration::from_secs(300),
        ) {
            view::NextSync::After(_) => {}
            other => panic!("it would have given up on a server being down: {other:?}"),
        }
        // And it must not look like a rate limit either. `Throttled` resets the consecutive
        // failure count, so a down server that set `hold` would be polled at the flat interval
        // for ever instead of backing off — the loop would never reach the ceiling.
        assert!(
            ran.hold.is_none(),
            "a refused connection asked us to wait {:?}",
            ran.hold
        );
    }

    #[test]
    fn and_the_loop_stops_rather_than_backing_off() {
        // The two halves joined: the classification the pass produces, fed to the decision the
        // loop makes. Either alone proves nothing about what the client does to a mail server.
        let (store, _dir) = configured(serve_refusing(), caps());
        let secrets = MapSecrets::default();
        with_password(&secrets, "wrong");

        let ran =
            sync::run_with(store, Arc::new(secrets), &OAuthRegistry::default(), now()).unwrap();
        let passed = if ran.rejected {
            view::Passed::Rejected
        } else {
            view::Passed::Fine
        };

        match view::next_sync(passed, 1, std::time::Duration::from_secs(300)) {
            view::NextSync::Wait(why) => assert!(why.contains("rejected"), "{why}"),
            other => panic!("it would have tried again: {other:?}"),
        }
    }
}

/// A server that asks to be left alone, and whether anyone listens.
///
/// `ProtoError::Throttled` carries a wait — the server's own `Retry-After` where it gives one,
/// and an hour where it does not, because Gmail's lockouts are measured in hours and hammering
/// lengthens them. The domain layer has computed that number since the beginning. The question
/// here is whether it reaches the loop that decides when to knock again.
mod a_server_asking_to_be_left_alone {
    use super::*;
    use std::io::{Read, Write};

    /// Refuses the sign-in for rate limiting, which is what Gmail does to a client that
    /// reconnects too often — not a wrong password, and not something a new password fixes.
    fn serve_throttling() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for sock in listener.incoming() {
                let Ok(mut sock) = sock else { continue };
                let _ = sock.write_all(b"* OK [CAPABILITY IMAP4rev1] ready\r\n");
                let mut buf = [0u8; 4096];
                while let Ok(n) = sock.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    for line in text.lines() {
                        let Some(tag) = line.split_whitespace().next() else {
                            continue;
                        };
                        let _ = sock.write_all(
                            format!("{tag} NO [LIMIT] Too many simultaneous connections\r\n")
                                .as_bytes(),
                        );
                    }
                }
            }
        });
        port
    }

    #[test]
    fn the_wait_the_server_asked_for_survives_as_far_as_the_loop() {
        let (store, _dir) = configured(serve_throttling(), caps());
        let secrets = MapSecrets::default();
        with_password(&secrets, "the-right-password");

        let ran =
            sync::run_with(store, Arc::new(secrets), &OAuthRegistry::default(), now()).unwrap();

        assert!(
            !ran.rejected,
            "being asked to slow down is not a bad password: {}",
            ran.text
        );
        let hold = ran.hold.expect("the server named a wait; nothing kept it");
        assert!(
            hold >= std::time::Duration::from_secs(3600),
            "an hour is the floor when the server gives no hint, got {hold:?}"
        );
    }

    #[test]
    fn and_the_loop_waits_that_long_rather_than_the_usual_five_minutes() {
        let interval = std::time::Duration::from_secs(300);
        let hold = std::time::Duration::from_secs(3600);
        match view::next_sync(view::Passed::Throttled { wait: hold }, 1, interval) {
            view::NextSync::After(next) => assert!(
                next >= hold,
                "the client would knock again in {next:?}, after being asked for {hold:?}"
            ),
            other => panic!("a rate limit is not something to give up over: {other:?}"),
        }
    }
}

/// What an account with no credential is told to do about it.
///
/// This is the first thing a new user reads, and it was one sentence for every account: "Run:
/// MAILO_PASSWORD=… mailo account add <address>". For the NTU account that is exactly right. For
/// a Gmail account it is advice that cannot work — Google turned off password authentication for
/// IMAP in May 2022 — and following it means a failed sign-in against Google with a password
/// that was never going to be accepted. `mailo account add` prints the right thing for that
/// account; `mailo sync` contradicted it, and sync is the command someone runs second.
mod an_account_with_nothing_stored {
    use super::*;

    fn told(auth: AuthPlan) -> String {
        let (store, _dir) = configured_with(1, caps(), auth);
        sync::run_with(
            store,
            Arc::new(MapSecrets::default()),
            &OAuthRegistry::default(),
            now(),
        )
        .unwrap()
        .text
    }

    #[test]
    fn a_password_account_is_told_about_the_password() {
        let out = told(AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        });
        assert!(out.contains("MAILO_PASSWORD"), "{out}");
        // The address, not the word "<address>": advice that has to be edited before it can be
        // run is advice someone gets wrong at the point they are least able to tell.
        assert!(out.contains("mailo account add ada@example.test"), "{out}");
        assert!(!out.contains("<address>"), "{out}");
    }

    #[test]
    fn an_oauth_account_is_not_sent_to_find_a_password() {
        let out = told(AuthPlan::OAuth {
            issuer: OAuthIssuer::Google,
            scopes: vec!["https://mail.google.com/".to_owned()],
        });
        assert!(
            !out.contains("MAILO_PASSWORD"),
            "a Google account was told to set a password, which Google has not accepted since \
             2022: {out}"
        );
        assert!(out.contains("MAILO_OAUTH_CLIENT_ID"), "{out}");
        assert!(out.contains("mailo account add ada@example.test"), "{out}");
    }

    /// `mailo account list` is the third surface, and it has to agree with the other two.
    ///
    /// It reads the real keyring for the stored credential, which is safe here and only here:
    /// the account ids are generated per test and nothing of theirs exists, so this is a read of
    /// a key that is not present. It writes nothing, so there is nothing to clear afterwards.
    #[test]
    fn the_account_listing_says_the_same_thing_in_fewer_words() {
        let (oauth, _a) = configured_with(
            1,
            caps(),
            AuthPlan::OAuth {
                issuer: OAuthIssuer::Google,
                scopes: vec!["https://mail.google.com/".to_owned()],
            },
        );
        let listed = account::list(&oauth).unwrap();
        assert!(
            listed.contains("not signed in"),
            "an OAuth account was told a credential was missing, which reads as \"find a \
             password\": {listed}"
        );

        let (password, _b) = configured_with(
            1,
            caps(),
            AuthPlan::Password {
                username: Username::SameAsAddress,
                sasl: vec![SaslMech::Plain],
            },
        );
        let listed = account::list(&password).unwrap();
        assert!(listed.contains("no credential stored"), "{listed}");
    }

    /// Microsoft needs `--microsoft` to reproduce the account, and an instruction that does not
    /// work when followed is worse than none.
    #[test]
    fn a_microsoft_account_keeps_the_flag_that_makes_the_command_work() {
        let out = told(AuthPlan::OAuth {
            issuer: OAuthIssuer::Microsoft,
            scopes: vec!["https://outlook.office.com/IMAP.AccessAsUser.All".to_owned()],
        });
        assert!(
            out.contains("mailo account add ada@example.test --microsoft"),
            "{out}"
        );
    }
}
