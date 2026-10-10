use super::*;
use crate::sync::run_with;
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{AccountSecrets, ClientRegistry};
use mail_store::SqliteStore;
use porter_core::SecretText;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
use porter_secrets::MemorySecrets;
use std::sync::Arc;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000f1"))
}

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
    mail_store::testing::seed_account_plan(&store, acct_account(), "ada@example.test", &plan, None);
    mail_store::testing::seed_caps(&store, acct_account(), &caps, now()).unwrap();
    (store, dir)
}

fn with_password(secrets: &MemorySecrets) {
    mail_runtime::block_on(secrets.put(
        &SecretKey {
            account: acct_account(),
            purpose: SecretPurpose::IncomingPassword,
        },
        &Credential::Password(SecretText::new("s3cr3t-pass".to_owned())),
    ))
    .unwrap();
}

#[tokio::test]
async fn a_missing_credential_fails_the_account_as_needing_a_sign_in() {
    let (store, _dir) = store_at(1);
    let ends = run_with(
        store,
        Arc::new(MemorySecrets::default()),
        &ClientRegistry::default(),
        now(),
        Hooks::default(),
    )
    .await
    .unwrap();
    let [PassEnd::Failed { retry, why, .. }] = ends.as_slice() else {
        panic!("one failed account expected: {ends:?}");
    };
    assert_eq!(*retry, Retry::NeedsReauth);
    assert!(why.contains("no credential stored"), "{why}");
}

#[tokio::test]
async fn an_unreachable_server_fails_the_account_with_a_wait() {
    let (store, _dir) = store_at(1);
    let secrets = MemorySecrets::default();
    with_password(&secrets);
    let ends = run_with(
        store,
        Arc::new(secrets),
        &ClientRegistry::default(),
        now(),
        Hooks::default(),
    )
    .await
    .unwrap();
    let [PassEnd::Failed { retry, pause, .. }] = ends.as_slice() else {
        panic!("one failed account expected: {ends:?}");
    };
    assert!(matches!(retry, Retry::After(_)), "{retry:?}");
    assert_eq!(
        *pause,
        Pause::Unreachable,
        "a refused connection is a server that cannot be reached"
    );
}

#[tokio::test]
async fn a_mailbox_that_fails_lands_in_trouble_with_its_decision() {
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
    let secrets = MemorySecrets::default();
    with_password(&secrets);
    let ends = run_with(
        store,
        Arc::new(secrets),
        &ClientRegistry::default(),
        now(),
        Hooks::default(),
    )
    .await
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
    assert!(needs_person(&done.trouble));
    assert_eq!(hold(&done.trouble), Some(std::time::Duration::from_secs(9)));
}

#[test]
fn each_failure_says_what_kind_of_wait_it_asks_for() {
    use mail_proto::Refusal;
    use std::time::Duration;
    let proto = |e: ProtoError| RuntimeError::Proto(e);
    let cases: Vec<(&str, RuntimeError, Pause)> = vec![
        (
            "refused connection",
            RuntimeError::Connect("refused".into()),
            Pause::Unreachable,
        ),
        (
            "dropped socket",
            RuntimeError::Io("reset".into()),
            Pause::Unreachable,
        ),
        (
            "hang-up mid-command",
            proto(ProtoError::UnexpectedEof),
            Pause::Unreachable,
        ),
        (
            "rate limit, with a named wait",
            proto(ProtoError::Throttled {
                reason: "slow down".into(),
                retry_after: Some(Duration::from_secs(90)),
            }),
            Pause::Throttled,
        ),
        (
            "rate limit, with none",
            proto(ProtoError::Throttled {
                reason: "slow down".into(),
                retry_after: None,
            }),
            Pause::Throttled,
        ),
        (
            "greylisting",
            proto(ProtoError::Refused {
                kind: Refusal::Transient,
                text: "try later".into(),
            }),
            Pause::ServerBusy,
        ),
        (
            "unreadable answer",
            proto(ProtoError::Malformed("x".into())),
            Pause::ServerBusy,
        ),
    ];
    for (name, error, expected) in cases {
        assert_eq!(error.pause(), expected, "{name}");
        // And it travels with the decision, through the constructor a pass uses.
        let failure = Failure::of("", &error);
        assert_eq!(failure.pause, expected, "{name}: Failure::of");
        assert_eq!(
            failure.retry,
            error.retry(),
            "{name}: the decision is unchanged"
        );
    }
}

#[test]
fn only_a_pass_that_ran_may_have_stored_something() {
    let finished = PassEnd::Finished(AccountReport {
        account: acct_account(),
        address: "ada@example.test".to_owned(),
        counts: Counts::default(),
        trouble: vec![],
    });
    let failed = PassEnd::Failed {
        account: acct_account(),
        address: "ada@example.test".to_owned(),
        retry: Retry::After(std::time::Duration::from_secs(5)),
        why: "cannot connect".to_owned(),
        pause: crate::fetch::Pause::Unreachable,
    };
    let cancelled = PassEnd::Cancelled {
        account: acct_account(),
        address: "ada@example.test".to_owned(),
    };
    // A flag sweep stores without counting, so even an all-zero pass that ran says yes.
    assert!(finished.may_have_stored());
    assert!(cancelled.may_have_stored());
    assert!(!failed.may_have_stored());
}
