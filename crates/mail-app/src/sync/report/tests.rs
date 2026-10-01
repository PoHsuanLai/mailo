use super::*;
use crate::sync::{one_line, run_typed_with};
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_runtime::{MapSecrets, OAuthRegistry, Secrets};
use mail_store::SqliteStore;
use std::sync::Arc;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000f1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// A store with one IMAP account at `port` over plaintext, with capabilities fresh enough that a
/// pass does not ask the server for them.
fn store_at(port: u16) -> (Arc<SqliteStore>, tempfile::TempDir) {
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
    let caps = AccountCaps {
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
        observed_at: now(),
    };
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
    drop(db);
    (store, dir)
}

fn with_password(secrets: &MapSecrets) {
    secrets
        .put(
            &SecretKey {
                account: ACCOUNT,
                purpose: SecretPurpose::IncomingPassword,
            },
            &Credential::Password("s3cr3t-pass".to_owned()),
        )
        .unwrap();
}

#[test]
fn a_finished_account_reads_as_the_line_it_always_did() {
    let counts = Counts {
        headers_fetched: 3,
        bodies_fetched: 2,
        outbox_settled: 1,
        submitted: 1,
        parts_fetched: 4,
        still_queued: 2,
        ..Counts::default()
    };
    let trouble = vec![
        Trouble {
            mailbox: Some("INBOX".to_owned()),
            retry: Retry::Now,
            why: Some("INBOX: cannot select".to_owned()),
        },
        // Classified and silent: the line above it already said what happened.
        Trouble {
            mailbox: None,
            retry: Retry::NeedsReauth,
            why: None,
        },
    ];
    let end = PassEnd::Finished(AccountReport {
        account: ACCOUNT,
        address: "ada@example.test".to_owned(),
        counts: counts.clone(),
        trouble: trouble.clone(),
    });
    let old = SyncReport {
        headers_fetched: 3,
        bodies_fetched: 2,
        outbox_settled: 1,
        submitted: 1,
        parts_fetched: 4,
        still_queued: 2,
        needs_attention: vec!["INBOX: cannot select".to_owned()],
        needs_reauth: true,
        ..SyncReport::default()
    };
    assert_eq!(
        prose(&end, now()),
        one_line("ada@example.test", &old, now())
    );
    assert_eq!(
        prose(&end, now()),
        "ada@example.test: 3 headers, 2 bodies, 1 queued operations settled, 1 sent\n  \
         4 attachment(s) kept offline\n  \
         2 still queued; run sync again to retry, or `mailo drafts` to see why\n  \
         needs attention: INBOX: cannot select\n"
    );
}

#[test]
fn a_failed_account_reads_as_address_and_reason() {
    let end = PassEnd::Failed {
        account: ACCOUNT,
        address: "ada@example.test".to_owned(),
        retry: Retry::NeedsReauth,
        why: "not signed in".to_owned(),
    };
    assert_eq!(prose(&end, now()), "ada@example.test: not signed in\n");
}

#[test]
fn what_a_loop_acts_on_comes_from_the_typed_results() {
    let finished = |trouble| {
        PassEnd::Finished(AccountReport {
            account: ACCOUNT,
            address: "a".to_owned(),
            counts: Counts::default(),
            trouble,
        })
    };
    let wait = |secs| Trouble {
        mailbox: None,
        retry: Retry::After(std::time::Duration::from_secs(secs)),
        why: None,
    };
    let ends = [
        finished(vec![wait(60)]),
        finished(vec![wait(3600)]),
        PassEnd::Failed {
            account: ACCOUNT,
            address: "b".to_owned(),
            retry: Retry::NeedsReauth,
            why: "x".to_owned(),
        },
    ];
    let ran = summarise(&ends, now());
    assert!(ran.rejected);
    assert_eq!(ran.hold, Some(std::time::Duration::from_secs(3600)));
}

#[test]
fn a_missing_credential_fails_the_account_as_needing_a_sign_in() {
    let (store, _dir) = store_at(1);
    let ends = run_typed_with(
        store,
        Arc::new(MapSecrets::default()),
        &OAuthRegistry::default(),
        now(),
        Hooks::default(),
    )
    .unwrap();
    let [PassEnd::Failed { retry, why, .. }] = ends.as_slice() else {
        panic!("one failed account expected: {ends:?}");
    };
    assert_eq!(*retry, Retry::NeedsReauth);
    assert!(why.contains("no credential stored"), "{why}");
}

#[test]
fn an_unreachable_server_fails_the_account_with_a_wait() {
    let (store, _dir) = store_at(1);
    let secrets = MapSecrets::default();
    with_password(&secrets);
    let ends = run_typed_with(
        store,
        Arc::new(secrets),
        &OAuthRegistry::default(),
        now(),
        Hooks::default(),
    )
    .unwrap();
    let [PassEnd::Failed { retry, .. }] = ends.as_slice() else {
        panic!("one failed account expected: {ends:?}");
    };
    assert!(matches!(retry, Retry::After(_)), "{retry:?}");
}

#[test]
fn a_mailbox_that_fails_lands_in_trouble_with_its_decision() {
    // A server that accepts the connection and hangs up on everything after it: the account is
    // reachable, and every mailbox in it fails.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for sock in listener.incoming() {
            drop(sock);
        }
    });
    let (store, _dir) = store_at(port);
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO folders (account, path, role, followed) VALUES (?1, 'INBOX', 'inbox', 1)",
            [ACCOUNT.to_string()],
        )
        .ok();
    }
    let secrets = MapSecrets::default();
    with_password(&secrets);
    let ends = run_typed_with(
        store,
        Arc::new(secrets),
        &OAuthRegistry::default(),
        now(),
        Hooks::default(),
    )
    .unwrap();
    // Reachable, so the pass ran; the inbox failed in it, and said which mailbox and what to do.
    let [PassEnd::Finished(report)] = ends.as_slice() else {
        panic!("a finished pass with trouble expected: {ends:?}");
    };
    let inbox = report
        .trouble
        .iter()
        .find(|t| t.mailbox.as_deref() == Some("INBOX"))
        .unwrap_or_else(|| panic!("no trouble named the inbox: {report:?}"));
    assert_eq!(inbox.retry, Retry::Now);
    assert!(
        inbox
            .why
            .as_deref()
            .is_some_and(|w| w.starts_with("INBOX: "))
    );
    // The folder list is the account's own step, so it names no mailbox.
    assert!(report.trouble.iter().any(|t| t.mailbox.is_none()));
}

#[test]
fn the_trouble_a_pass_gathers_keeps_its_mailbox_and_decision() {
    let mut done = Done::default();
    done.trouble(
        Some("Archive"),
        Retry::After(std::time::Duration::from_secs(9)),
        "Archive: slow down".to_owned(),
    );
    done.trouble(None, Retry::NeedsReauth, "rules: refused".to_owned());
    assert_eq!(
        done.trouble,
        vec![
            Trouble {
                mailbox: Some("Archive".to_owned()),
                retry: Retry::After(std::time::Duration::from_secs(9)),
                why: Some("Archive: slow down".to_owned()),
            },
            Trouble {
                mailbox: None,
                retry: Retry::NeedsReauth,
                why: Some("rules: refused".to_owned()),
            },
        ]
    );
    let old = done.to_report();
    assert_eq!(
        old.needs_attention,
        ["Archive: slow down", "rules: refused"]
    );
    assert!(old.needs_reauth);
    assert_eq!(old.hold, Some(std::time::Duration::from_secs(9)));
}
