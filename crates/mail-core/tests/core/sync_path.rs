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
use mail_core::fetch::{self, Effect, Event, First, Link, Step};
use mail_core::sync::report::PassEnd;
use mail_core::{account, sync};
use mail_domain::id::{account_id_from_uuid, new_account_id};
use mail_domain::*;
use mail_runtime::{AccountSecrets, ClientRegistry};
use mail_store::{SqliteStore, Store};
use porter_core::SecretText;
use porter_core::UnixSeconds;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
use porter_provider::Issuer;
use porter_secrets::MemorySecrets;
use std::sync::Arc;

/// What a run says, as one string to search: each address with its reason or its trouble.
fn words(ends: &[PassEnd]) -> String {
    ends.iter()
        .map(|end| match end {
            PassEnd::Finished(report) => {
                let said: Vec<&str> = report
                    .trouble
                    .iter()
                    .filter_map(|t| t.why.as_deref())
                    .collect();
                format!("{}: {}\n", report.address, said.join("\n"))
            }
            PassEnd::Failed { address, why, .. } => format!("{address}: {why}\n"),
            PassEnd::Cancelled { address, .. } => format!("{address}: cancelled\n"),
        })
        .collect()
}

/// What the link does with the way a pass ended: the pass's own classification handed to the
/// machine that decides when to try again, as the window's runner does it.
///
/// `from` is where the link was when the pass began, and so how many passes had failed before.
fn linked(end: PassEnd, from: Link) -> (Link, Vec<Effect>) {
    let syncing = Link::Syncing {
        first: First::No,
        step: Step::Connecting,
        count: None,
        after: Box::new(from),
    };
    fetch::step(
        &syncing,
        end.event(),
        now(),
        std::time::Duration::from_secs(300),
    )
}

/// A link that has failed `failures` times in a row and is due.
fn waiting(failures: u32) -> Link {
    Link::Waiting {
        until: now(),
        why: fetch::Pause::Unreachable,
        failures,
        first: First::No,
    }
}

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

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
        mail_store::testing::seed_account_plan(
            &store,
            acct_account(),
            "ada@example.test",
            &plan,
            None,
        );
        mail_store::testing::seed_caps(&store, acct_account(), &caps, now()).unwrap();
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

fn with_password(store_secrets: &MemorySecrets, password: &str) {
    mail_runtime::block_on(store_secrets.put(
        &SecretKey {
            account: acct_account(),
            purpose: SecretPurpose::IncomingPassword,
        },
        &Credential::Password(SecretText::new(password.to_owned())),
    ))
    .unwrap();
}

#[test]
fn an_account_with_no_credential_is_skipped_with_a_reason() {
    // One account needing attention must not stop the others fetching mail, and the reason has
    // to say what is missing — this is the first thing a new user sees. The command that fixes it
    // is the front end's to word (`Remedy::SignIn`).
    let (store, _dir) = configured(1, caps());
    let out = crate::blocking::run_with(
        store,
        Arc::new(MemorySecrets::default()),
        &ClientRegistry::default(),
        now(),
        sync::report::Hooks::default(),
    )
    .map(|ends| words(&ends))
    .expect("a missing credential is not a failure of the run");

    assert!(out.contains("ada@example.test"), "{out}");
    assert!(out.contains("no credential stored"), "{out}");
    assert!(!out.contains("mailo "), "core names no command: {out}");
}

#[test]
fn an_unreachable_server_is_reported_per_account_not_thrown() {
    // Port 1 refuses. The pass should say so against that account and return.
    let (store, _dir) = configured(1, caps());
    let secrets = MemorySecrets::default();
    with_password(&secrets, "s3cr3t-pass");

    let out = crate::blocking::run_with(
        store,
        Arc::new(secrets),
        &ClientRegistry::default(),
        now(),
        sync::report::Hooks::default(),
    )
    .map(|ends| words(&ends))
    .expect("an unreachable server is reported, not returned as an error");
    assert!(out.contains("ada@example.test"), "{out}");
    assert!(
        out.to_lowercase().contains("connect") || out.to_lowercase().contains("refused"),
        "the reason should reach the user: {out}"
    );
}

#[test]
fn no_accounts_is_a_run_with_no_account_in_it() {
    // What to say about that is the command line's: see `mail_app::cli::sync`.
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    let ends = crate::blocking::run_with(
        store,
        Arc::new(MemorySecrets::default()),
        &ClientRegistry::default(),
        now(),
        sync::report::Hooks::default(),
    )
    .unwrap();
    assert_eq!(ends, Vec::new());
}

#[tokio::test]
#[ignore = "needs a local Twisted IMAP4 server; run deliberately with --ignored"]
async fn a_whole_pass_against_a_real_server_lands_mail_and_reports_what_it_fetched() {
    // The assembly, end to end: stored plan, stored capabilities, a credential, a real server,
    // and a report of what the pass fetched. How it is worded is the command line's.
    let (store, _dir) = configured(11143, caps());
    let secrets = MemorySecrets::default();
    with_password(&secrets, "s3cr3t-pass");

    let store_for_pass = store.clone();
    let ends = tokio::task::spawn_blocking(move || {
        crate::blocking::run_with(
            store_for_pass,
            Arc::new(secrets),
            &ClientRegistry::default(),
            now(),
            sync::report::Hooks::default(),
        )
    })
    .await
    .expect("the pass did not panic")
    .expect("a reachable server with a good credential syncs");

    let [PassEnd::Finished(report)] = ends.as_slice() else {
        panic!("one account, one finished pass: {ends:?}");
    };
    assert_eq!(report.address, "ada@example.test");
    assert!(
        report.trouble.is_empty(),
        "a clean pass should report no trouble: {:?}",
        report.trouble
    );
    assert!(
        report.counts.headers_fetched > 0 && report.counts.bodies_fetched > 0,
        "the pass should report what it fetched: {:?}",
        report.counts
    );
    // The fixture serves two messages; they must be in the store afterwards.
    let count: i64 = mail_store::testing::count(&store, "messages");
    assert_eq!(count, 2, "the pass reported success and stored nothing");

    // And the server's flags reached the store.
    //
    // The fixture serves both messages as `\Seen`. They arrived as unread, because a header
    // fetch does not carry flags — the flag sweep does, and `AccountEngine::sweep` was called
    // from nowhere but its own tests. Every message in the application therefore stayed unread
    // for ever: mail read on a phone stayed bold here, the unread counts were the mailbox size,
    // and on Gmail the labels never appeared either, because they ride the same survey. Found
    // against a real account, where 138 messages the user had *sent* were all marked unread.
    let unread = mail_store::testing::message_ids(&store)
        .into_iter()
        .filter(|id| store.message(*id).unwrap().read == ReadState::Unread)
        .count();
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
/// a client's own endpoints are for.
mod renewing_an_expired_sign_in {
    use super::*;
    use mail_runtime::clients;
    use porter_core::EndpointUrl;
    use porter_provider::{ClientEntry, IssuerEndpoints};
    use std::io::{Read as _, Write as _};
    use std::sync::Mutex;

    /// Where a test's own listener serves the issuer's two pages.
    #[derive(Debug, Clone)]
    struct Endpoints {
        auth: String,
        token: String,
    }

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
                issuer: Issuer::Google,
                scopes: vec!["https://mail.google.com/".to_owned()],
            },
            identities: Vec::new(),
        };
        store.set_account_plan(acct_account(), &plan).unwrap();
        (store, dir)
    }

    fn key() -> SecretKey {
        SecretKey {
            account: acct_account(),
            purpose: SecretPurpose::IncomingPassword,
        }
    }

    /// An OAuth credential expiring `minutes` from `now()`.
    fn token(minutes: i64) -> Credential {
        Credential::OAuth {
            access: SecretText::new("ya29.stale".to_owned()),
            refresh: SecretText::new("the-refresh-token".to_owned()),
            expires_at: UnixSeconds(
                (now() + chrono::TimeDelta::try_minutes(minutes).unwrap()).timestamp(),
            ),
        }
    }

    /// A registry whose one client signs in at the endpoints a test's own listener serves.
    fn registry_at(issuer: Issuer, ends: Endpoints) -> ClientRegistry {
        let client = ClientEntry {
            endpoints: Some(IssuerEndpoints {
                authorize: EndpointUrl::parse(&ends.auth).unwrap(),
                token: EndpointUrl::parse(&ends.token).unwrap(),
                revoke: None,
                device: None,
            }),
            ..clients::entry(issuer, "client-id", None)
        };
        clients::registry_of(vec![client])
    }

    fn registry(ends: Endpoints) -> ClientRegistry {
        registry_at(Issuer::Google, ends)
    }

    #[test]
    fn an_expired_access_token_is_renewed_and_the_new_one_stored() {
        let (store, _dir) = oauth_account();
        let secrets: Arc<dyn AccountSecrets> = Arc::new(MemorySecrets::default());
        // Two hours past expiry, which is where every OAuth account ended up an hour after it
        // was added and stayed forever.
        mail_runtime::block_on(secrets.put(&key(), &token(-120))).unwrap();
        let (ends, seen) = token_endpoint(RENEWED);

        let _ = crate::blocking::run_with(
            store,
            secrets.clone(),
            &registry(ends),
            now(),
            sync::report::Hooks::default(),
        );

        let asked = seen.lock().unwrap().join("\n");
        assert!(
            asked.contains("grant_type=refresh_token"),
            "the issuer was never asked to renew: {asked:?}"
        );
        assert!(
            asked.contains("refresh_token=the-refresh-token"),
            "the stored refresh token was not the one spent: {asked:?}"
        );

        match mail_runtime::block_on(secrets.get(&key())).unwrap() {
            Credential::OAuth {
                access,
                refresh,
                expires_at,
            } => {
                assert_eq!(
                    access.expose(),
                    "ya29.renewed",
                    "the new token was not written back"
                );
                // This issuer returned no new refresh token, which is the usual case. Dropping
                // the old one logs the user out at the next expiry with nothing to recover from.
                assert_eq!(refresh.expose(), "the-refresh-token");
                assert!(
                    expires_at.0 > now().timestamp(),
                    "{} is not in the future",
                    expires_at.0
                );
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
        let secrets: Arc<dyn AccountSecrets> = Arc::new(MemorySecrets::default());
        mail_runtime::block_on(secrets.put(&key(), &token(-120))).unwrap();
        let (ends, _seen) = token_endpoint(RENEWED);

        let _ = crate::blocking::run_with(
            store,
            secrets.clone(),
            &registry(ends),
            now(),
            sync::report::Hooks::default(),
        );

        let stored = mail_runtime::block_on(secrets.get(&SecretKey {
            account: acct_account(),
            purpose: SecretPurpose::OAuthRefresh,
        }))
        .expect("the refresh entry should have been rewritten");
        match stored {
            Credential::OAuth { access, .. } => assert_eq!(access.expose(), "ya29.renewed"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_token_that_is_still_good_is_not_sent_to_the_issuer() {
        // Renewing on every pass would turn a five-minute poll into a five-minute round trip to
        // the issuer, and issuers rate-limit that.
        let (store, _dir) = oauth_account();
        let secrets: Arc<dyn AccountSecrets> = Arc::new(MemorySecrets::default());
        mail_runtime::block_on(secrets.put(&key(), &token(45))).unwrap();
        let (ends, seen) = token_endpoint(RENEWED);

        let _ = crate::blocking::run_with(
            store,
            secrets.clone(),
            &registry(ends),
            now(),
            sync::report::Hooks::default(),
        );

        assert!(
            seen.lock().unwrap().is_empty(),
            "a valid token was sent to the issuer anyway: {:?}",
            seen.lock().unwrap()
        );
        assert_eq!(
            mail_runtime::block_on(secrets.get(&key())).unwrap(),
            token(45),
            "and left alone"
        );
    }

    /// A Microsoft sign-in that also consented to Graph renews its IMAP token by name.
    ///
    /// Microsoft issues each access token for one resource and refuses a request that spans two
    /// (`AADSTS28003`). The renewal named nothing, which asks for what the sign-in was for — and
    /// this sign-in was for two.
    #[test]
    fn a_sign_in_for_two_resources_renews_each_by_its_own_scopes() {
        let (store, _dir) = oauth_account();
        let plan = mail_domain::presets::send_through_graph(
            mail_domain::presets::microsoft_preset("ada@example.test", now()),
        )
        .plan;
        store.set_account_plan(acct_account(), &plan).unwrap();
        let secrets: Arc<dyn AccountSecrets> = Arc::new(MemorySecrets::default());
        mail_runtime::block_on(secrets.put(&key(), &token(-120))).unwrap();
        mail_runtime::block_on(secrets.put(
            &SecretKey {
                account: acct_account(),
                purpose: SecretPurpose::OAuthRefresh,
            },
            &token(-120),
        ))
        .unwrap();
        let (ends, seen) = token_endpoint(RENEWED);
        let registry = registry_at(Issuer::Microsoft, ends);

        let _ = crate::blocking::run_with(
            store,
            secrets,
            &registry,
            now(),
            sync::report::Hooks::default(),
        );

        let asked = seen.lock().unwrap().clone();
        let imap = "IMAP.AccessAsUser.All";
        let graph = "Mail.Send";
        assert!(
            asked.iter().any(|r| r.contains(imap) && !r.contains(graph)),
            "the IMAP token was not renewed by its own scope: {asked:?}"
        );
        assert!(
            asked.iter().any(|r| r.contains(graph) && !r.contains(imap)),
            "the Graph token was not minted by its own scope: {asked:?}"
        );
        assert!(
            !asked.iter().any(|r| r.contains(imap) && r.contains(graph)),
            "one request named two resources: {asked:?}"
        );
    }

    #[test]
    fn a_password_account_never_reaches_the_issuer() {
        // The plan is what decides, not the credential: a password has no expiry and there is
        // nothing to renew it with.
        let (store, _dir) = configured(1, caps());
        let secrets: Arc<dyn AccountSecrets> = Arc::new(MemorySecrets::default());
        mail_runtime::block_on(secrets.put(
            &key(),
            &Credential::Password(SecretText::new("hunter2".to_owned())),
        ))
        .unwrap();
        let (ends, seen) = token_endpoint(RENEWED);

        let _ = crate::blocking::run_with(
            store,
            secrets,
            &registry(ends),
            now(),
            sync::report::Hooks::default(),
        );

        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn an_expired_sign_in_with_no_client_id_says_so_instead_of_failing_to_authenticate() {
        // "authentication failed" points at the password the user does not have. The actual
        // problem is that this installation cannot renew, and the message has to say that.
        let (store, _dir) = oauth_account();
        let secrets: Arc<dyn AccountSecrets> = Arc::new(MemorySecrets::default());
        mail_runtime::block_on(secrets.put(&key(), &token(-120))).unwrap();

        let out = crate::blocking::run_with(
            store,
            secrets,
            &ClientRegistry::default(),
            now(),
            sync::report::Hooks::default(),
        )
        .map(|ends| words(&ends))
        .unwrap();

        assert!(out.contains("no OAuth client id is configured"), "{out}");
        // Read by a person, and `cargo fmt` collapses a `\`-continuation in a literal into a
        // run of spaces in the middle of the sentence.
        assert!(!out.contains("  "), "a run of spaces in a message: {out:?}");
        assert!(!out.contains("mailo "), "core names no command: {out}");
        assert!(out.contains("ada@example.test"), "{out}");
    }
}

/// How often the background loop runs.
mod polling {
    use super::*;

    fn with_watch(store: &Arc<SqliteStore>, watch: WatchMode) {
        let mut caps = caps();
        caps.watch = watch;
        mail_store::testing::seed_caps(store, acct_account(), &caps, chrono::Utc::now()).unwrap();
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
        let secrets = MemorySecrets::default();
        with_password(&secrets, "definitely-not-the-password");

        let ends = crate::blocking::run_with(
            store,
            Arc::new(secrets),
            &ClientRegistry::default(),
            now(),
            sync::report::Hooks::default(),
        )
        .unwrap();
        let said = words(&ends);
        let [end] = <[PassEnd; 1]>::try_from(ends).expect("one account");

        assert!(
            matches!(
                end.event(),
                Event::Failed {
                    retry: Retry::NeedsReauth,
                    ..
                }
            ),
            "a refused sign-in was not reported as one: {said}"
        );
        // And the user is told, in the reason as well as in the decision.
        assert!(
            said.to_lowercase().contains("login failed"),
            "the server's own words should reach the user: {said}"
        );
    }

    /// The control, and the half that matters more: a server that is merely *down* must not be
    /// classified as a refusal. If it were, one flaky minute of network would stop the loop until
    /// the user next noticed, and the stored credential was never the problem.
    #[test]
    fn a_server_that_is_simply_down_is_not_a_refusal() {
        let (store, _dir) = configured(1, caps()); // port 1 refuses the connection
        let secrets = MemorySecrets::default();
        with_password(&secrets, "the-right-password");

        let ends = crate::blocking::run_with(
            store,
            Arc::new(secrets),
            &ClientRegistry::default(),
            now(),
            sync::report::Hooks::default(),
        )
        .unwrap();
        let said = words(&ends);
        let [end] = <[PassEnd; 1]>::try_from(ends).expect("one account");
        assert!(
            !matches!(
                end.clone().event(),
                Event::Failed {
                    retry: Retry::NeedsReauth,
                    ..
                }
            ),
            "an unreachable server was blamed on the credential: {said}"
        );
        match linked(end, waiting(2)) {
            (Link::Waiting { why, failures, .. }, effects) => {
                // Not a rate limit either: `Throttled` resets the consecutive failure count, so a
                // down server read as one would be polled at the flat interval for ever instead of
                // backing off, and the loop would never reach the ceiling.
                assert_eq!(
                    why,
                    fetch::Pause::Unreachable,
                    "it is a server that is not there"
                );
                assert_eq!(failures, 3, "and the failures are counted, so it backs off");
                assert!(
                    matches!(effects.as_slice(), [Effect::WakeAt(_)]),
                    "{effects:?}"
                );
            }
            other => panic!("it would have given up on a server being down: {other:?}"),
        }
    }

    #[test]
    fn and_the_loop_stops_rather_than_backing_off() {
        // The two halves joined: the classification the pass produces, fed to the decision the
        // loop makes. Either alone proves nothing about what the client does to a mail server.
        let (store, _dir) = configured(serve_refusing(), caps());
        let secrets = MemorySecrets::default();
        with_password(&secrets, "wrong");

        let ends = crate::blocking::run_with(
            store,
            Arc::new(secrets),
            &ClientRegistry::default(),
            now(),
            sync::report::Hooks::default(),
        )
        .unwrap();
        let [end] = <[PassEnd; 1]>::try_from(ends).expect("one account");

        let (link, effects) = linked(end, waiting(0));
        assert!(
            matches!(link, Link::NeedsSignIn { .. }),
            "it would have tried again: {link:?}"
        );
        assert!(
            effects.is_empty(),
            "a refused credential sets no timer: {effects:?}"
        );
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
        let secrets = MemorySecrets::default();
        with_password(&secrets, "the-right-password");

        let ends = crate::blocking::run_with(
            store,
            Arc::new(secrets),
            &ClientRegistry::default(),
            now(),
            sync::report::Hooks::default(),
        )
        .unwrap();
        let [end] = <[PassEnd; 1]>::try_from(ends).expect("one account");

        // A wait, and not a refusal: being asked to slow down is not a bad password.
        let Event::Failed { retry, .. } = end.event() else {
            panic!("the server named a wait; nothing kept it");
        };
        let Retry::After(hold) = retry else {
            panic!("being asked to slow down was read as {retry:?}");
        };
        assert!(
            hold >= std::time::Duration::from_secs(3600),
            "an hour is the floor when the server gives no hint, got {hold:?}"
        );
    }

    #[test]
    fn and_the_loop_waits_that_long_rather_than_the_usual_five_minutes() {
        let (store, _dir) = configured(serve_throttling(), caps());
        let secrets = MemorySecrets::default();
        with_password(&secrets, "the-right-password");

        let ends = crate::blocking::run_with(
            store,
            Arc::new(secrets),
            &ClientRegistry::default(),
            now(),
            sync::report::Hooks::default(),
        )
        .unwrap();
        let [end] = <[PassEnd; 1]>::try_from(ends).expect("one account");

        let hold = std::time::Duration::from_secs(3600);
        match linked(end, waiting(0)) {
            (Link::Waiting { until, why, .. }, _) => {
                assert_eq!(
                    why,
                    fetch::Pause::Throttled,
                    "a rate limit is its own kind of wait"
                );
                assert!(
                    until >= now() + chrono::TimeDelta::from_std(hold).unwrap(),
                    "the client would knock again at {until}, after being asked for {hold:?}"
                );
            }
            other => panic!("a rate limit is not something to give up over: {other:?}"),
        }
    }
}

/// What an account with no credential is told, and which step is its remedy.
///
/// This is the first thing a new user reads, and it was one sentence for every account: "Run:
/// MAILO_PASSWORD=… mailo account add <address>". For a password account that is exactly right. For
/// a Gmail account it is advice that cannot work — Google turned off password authentication for
/// IMAP in May 2022 — and following it means a failed sign-in against Google with a password
/// that was never going to be accepted. So core says what is wrong in words that name no command,
/// and `sign_in_remedy` says how that account signs in; the front end words the command
/// (`mail-app`'s `cli::remedy`).
mod an_account_with_nothing_stored {
    use super::*;

    fn told(auth: AuthPlan) -> String {
        let (store, _dir) = configured_with(1, caps(), auth);
        crate::blocking::run_with(
            store,
            Arc::new(MemorySecrets::default()),
            &ClientRegistry::default(),
            now(),
            sync::report::Hooks::default(),
        )
        .map(|ends| words(&ends))
        .unwrap()
    }

    #[test]
    fn a_password_account_is_told_about_the_password() {
        let out = told(AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        });
        assert!(out.contains("no credential stored"), "{out}");
        assert!(!out.contains("mailo "), "core names no command: {out}");
        assert!(!out.contains("<address>"), "{out}");
    }

    #[test]
    fn an_oauth_account_is_not_sent_to_find_a_password() {
        let out = told(AuthPlan::OAuth {
            issuer: Issuer::Google,
            scopes: vec!["https://mail.google.com/".to_owned()],
        });
        assert!(
            !out.contains("password will work") && !out.contains("MAILO_PASSWORD"),
            "a Google account was told to set a password, which Google has not accepted since \
             2022: {out}"
        );
        assert!(out.contains("OAuth"), "{out}");
        assert!(out.contains("client id"), "{out}");
        assert!(!out.contains("mailo "), "core names no command: {out}");
    }

    /// How the account signs in is what the front end needs to word the command, and it is read
    /// from the stored plan, not guessed from the address.
    #[test]
    fn the_remedy_for_a_refused_sign_in_follows_the_stored_plan() {
        use mail_core::{Remedy, SignInWith};
        let remedy = |auth: AuthPlan| {
            let (store, _dir) = configured_with(1, caps(), auth);
            sync::sign_in_remedy(&store, "ada@example.test")
        };
        let signs_in = |with| Remedy::SignIn {
            address: "ada@example.test".to_owned(),
            with,
        };
        assert_eq!(
            remedy(AuthPlan::Password {
                username: Username::SameAsAddress,
                sasl: vec![SaslMech::Plain],
            }),
            signs_in(SignInWith::Password)
        );
        for issuer in [Issuer::Google, Issuer::Microsoft] {
            assert_eq!(
                remedy(AuthPlan::OAuth {
                    issuer,
                    scopes: Vec::new(),
                }),
                signs_in(SignInWith::OAuth { issuer })
            );
        }
        let (store, _dir) = configured(1, caps());
        assert_eq!(
            sync::sign_in_remedy(&store, "nobody@example.test"),
            Remedy::SignIn {
                address: "nobody@example.test".to_owned(),
                with: SignInWith::Unknown
            }
        );
    }

    /// `mailo account list` is the third surface, and it has to agree with the other two.
    ///
    /// It reads the stored credential from the store it is handed, here an empty one in memory:
    /// a test never asks the person's keyring anything, not even for a key that is not there.
    #[test]
    fn the_account_listing_says_the_same_thing_in_fewer_words() {
        let secrets = MemorySecrets::default();
        let (oauth, _a) = configured_with(
            1,
            caps(),
            AuthPlan::OAuth {
                issuer: Issuer::Google,
                scopes: vec!["https://mail.google.com/".to_owned()],
            },
        );
        let listed = crate::blocking::block_on(account::list(&oauth, &secrets)).unwrap();
        assert_eq!(
            listed.iter().map(|l| l.state).collect::<Vec<_>>(),
            [account::Readiness::NotSignedIn],
            "an OAuth account was told a credential was missing, which reads as \"find a \
             password\": {listed:?}"
        );

        let (password, _b) = configured_with(
            1,
            caps(),
            AuthPlan::Password {
                username: Username::SameAsAddress,
                sasl: vec![SaslMech::Plain],
            },
        );
        let listed = crate::blocking::block_on(account::list(&password, &secrets)).unwrap();
        assert_eq!(
            listed.iter().map(|l| l.state).collect::<Vec<_>>(),
            [account::Readiness::NoCredential],
            "{listed:?}"
        );
    }
}

/// What the window is told about the server — the loader behind F139.
///
/// `apply_op` built an `AccountCaps` out of safe defaults rather than reading the account's own,
/// and `Op::remote_intent` is the only consumer of `caps`: under `ArchiveMeans::LocalOnly` it
/// returns `None` for Archive and Trash. So every conversation archived in the window was
/// archived on this machine and nowhere else.
mod capabilities_the_window_reads {
    use super::*;

    #[test]
    fn what_was_observed_is_what_comes_back() {
        let (store, _dir) = configured_with(
            1,
            AccountCaps {
                archive: ArchiveMeans::DropInbox,
                labels: ServerLabels::Supported,
                ..caps()
            },
            AuthPlan::Password {
                username: Username::SameAsAddress,
                sasl: vec![SaslMech::Plain],
            },
        );

        let read = sync::caps_of(&store, acct_account()).expect("the account has capabilities");
        assert_eq!(read.archive, ArchiveMeans::DropInbox);
        assert_eq!(read.labels, ServerLabels::Supported);
    }

    #[test]
    fn an_account_nothing_has_connected_to_yet_has_none() {
        // Not an error: the window can be opened before the first sync, and the caller's answer
        // is to assume nothing rather than to refuse to act.
        let (store, _dir) = configured(1, caps());
        assert!(sync::caps_of(&store, new_account_id()).is_none());
    }
}

/// Two accounts, one pass, at the same time — `plan.md` phase 8f.
///
/// Accounts are independent by construction: `AccountId` partitions every table, and two accounts
/// are two conversations with two servers that have never heard of each other. Run one after
/// another, a pass spends the *sum* of their waiting, and almost all of a pass is waiting — so a
/// slow Gmail backfill held up another account's poll that had nothing to do with it.
mod both_accounts_at_once {
    use super::*;
    use std::sync::{Arc as StdArc, Mutex as StdMutex};
    use std::time::Instant;

    /// When a server was being talked to: the moment it accepted, and the moment it gave up.
    type Window = StdArc<StdMutex<Vec<(Instant, Instant)>>>;

    /// A server that accepts, says nothing for `holds`, and closes.
    ///
    /// Saying nothing is the point. The client waits for a greeting, so the connection stays open
    /// for the whole of `holds` and the account's pass fails afterwards — which is fine, because
    /// what is being measured is *when* each conversation happened, not whether it succeeded.
    fn slow_server(holds: std::time::Duration) -> (u16, Window) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let window: Window = StdArc::new(StdMutex::new(Vec::new()));
        let recording = window.clone();
        std::thread::spawn(move || {
            for sock in listener.incoming() {
                let Ok(sock) = sock else { continue };
                let opened = Instant::now();
                std::thread::sleep(holds);
                drop(sock);
                recording.lock().unwrap().push((opened, Instant::now()));
            }
        });
        (port, window)
    }

    /// A store with two password accounts, each pointed at its own port.
    fn other_account() -> AccountId {
        account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"))
    }

    fn two_accounts(first: u16, second: u16) -> (Arc<SqliteStore>, tempfile::TempDir) {
        let (store, dir) = configured(first, caps());
        let other = other_account();
        let plan = AccountPlan {
            address: "bee@example.test".to_owned(),
            incoming: Incoming::Imap {
                host: "127.0.0.1".to_owned(),
                port: second,
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
        mail_store::testing::seed_account_plan(
            &store,
            other.clone(),
            "bee@example.test",
            &plan,
            Some(chrono::Utc::now() + chrono::Duration::seconds(1)),
        );
        mail_store::testing::seed_caps(&store, other.clone(), &caps(), now()).unwrap();
        (store, dir)
    }

    /// Two intervals that share any instant at all.
    fn overlap(a: (Instant, Instant), b: (Instant, Instant)) -> bool {
        a.0 < b.1 && b.0 < a.1
    }

    #[test]
    fn the_two_conversations_happen_at_the_same_time() {
        // Asserted as an overlap rather than as a duration. A threshold in milliseconds is a
        // test that fails on a loaded machine and then gets deleted; two connections being open
        // at the same instant is the property itself, and it is either true or it is not.
        let hold = std::time::Duration::from_millis(400);
        let (first, one) = slow_server(hold);
        let (second, two) = slow_server(hold);
        let (store, _dir) = two_accounts(first, second);

        // Both accounts need a credential, or the one without it is skipped before it ever
        // opens a socket — and a test of concurrency with one participant proves nothing.
        let secrets = MemorySecrets::default();
        with_password(&secrets, "s3cr3t-pass");
        mail_runtime::block_on(secrets.put(
            &SecretKey {
                account: other_account(),
                purpose: SecretPurpose::IncomingPassword,
            },
            &Credential::Password(SecretText::new("s3cr3t-pass".to_owned())),
        ))
        .unwrap();
        let _ = crate::blocking::run_with(
            store,
            Arc::new(secrets),
            &ClientRegistry::default(),
            now(),
            sync::report::Hooks::default(),
        );

        let one = one.lock().unwrap().clone();
        let two = two.lock().unwrap().clone();
        // At least one each: a pass may open more than one connection to the same server, and
        // how many is not what this is about. The first of each is the one that matters.
        assert!(!one.is_empty(), "the first account was never contacted");
        assert!(!two.is_empty(), "the second account was never contacted");
        assert!(
            overlap(one[0], two[0]),
            "the accounts were synced one after the other: \
             the first was open for {:?} and the second started {:?} after it finished",
            one[0].1.duration_since(one[0].0),
            two[0].0.duration_since(one[0].1),
        );
    }
}

/// Downloading a part is IMAP's, and a message with no such account says so rather than trying.
#[test]
fn fetching_a_part_of_a_pop3_message_is_refused_before_anything_is_sent() {
    let (store, _dir) = configured(1, caps());
    // Repoint the account at POP3: the one fact under test.
    let mut plan = store.list_accounts().unwrap().remove(0).plan.unwrap();
    plan.incoming = Incoming::Pop3 {
        host: "127.0.0.1".to_owned(),
        port: 1,
        tls: Tls::Plaintext,
        leave: LeaveOnServer::Keep,
    };
    store.set_account_plan(acct_account(), &plan).unwrap();
    mail_runtime::absorb(
        &store,
        acct_account(),
        MailboxRef {
            account: acct_account(),
            path: "INBOX".to_owned(),
        },
        None,
        vec![mail_runtime::Arrival {
            remote: RemoteRef::Pop {
                uidl: "u1".to_owned(),
            },
            raw: b"From: a@example.test\r\nSubject: s\r\n\r\nbody\r\n".to_vec(),
        }],
        false,
        now(),
    )
    .unwrap();
    let id = mail_store::testing::message_ids(&store).remove(0);

    let err = crate::blocking::fetch_part_with(
        &store,
        Arc::new(MemorySecrets::default()),
        &ClientRegistry::default(),
        id,
        &"2".parse().unwrap(),
        now(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("only IMAP"), "{err}");
}
