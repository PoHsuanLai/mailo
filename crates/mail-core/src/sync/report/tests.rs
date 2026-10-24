use super::*;
use crate::support_tests::{account, caps, now, with_password};
use crate::sync::run_with;
use mail_domain::*;
use mail_runtime::ClientRegistry;
use mail_store::SqliteStore;
use porter_secrets::MemorySecrets;
use std::sync::Arc;

/// A store with one IMAP account at `port` over plaintext, with capabilities fresh enough that a
/// pass does not ask the server for them.
fn store_at(port: u16) -> Arc<SqliteStore> {
    let store = Arc::new(mail_store::testing::in_memory());
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
    mail_store::testing::seed_account_plan(&store, account(), "ada@example.test", &plan, None);
    mail_store::testing::seed_caps(&store, account(), &caps(), now()).unwrap();
    store
}

#[tokio::test]
async fn a_missing_credential_fails_the_account_as_needing_a_sign_in() {
    let store = store_at(1);
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
    let store = store_at(1);
    let secrets = MemorySecrets::default();
    with_password(&secrets, "s3cr3t-pass");
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

/// How each pass ended is kept, so a window opened later can say since when an account has not
/// been fetched: a failure is recorded without forgetting the last finish, and a finish clears it.
#[tokio::test]
async fn how_a_pass_ended_is_kept_in_the_store() {
    let store = store_at(1);
    let secrets = Arc::new(MemorySecrets::default());
    with_password(&secrets, "s3cr3t-pass");
    store.pass_finished(account(), now()).unwrap();
    let later = now() + chrono::TimeDelta::minutes(40);
    let ends = run_with(
        store.clone(),
        secrets,
        &ClientRegistry::default(),
        later,
        Hooks::default(),
    )
    .await
    .unwrap();
    assert!(
        matches!(ends.as_slice(), [PassEnd::Failed { .. }]),
        "{ends:?}"
    );
    let passes = store.passes(account()).unwrap();
    assert_eq!(passes.finished, Some(now()), "the last finish is kept");
    assert_eq!(passes.failed.map(|failed| failed.at), Some(later));
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
    let store = store_at(port);
    let secrets = MemorySecrets::default();
    with_password(&secrets, "s3cr3t-pass");
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
            "a connection gone silent",
            RuntimeError::Stalled {
                doing: "waiting for its answer",
                after: Duration::from_secs(240),
            },
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

/// A pass stopped by a silent connection leaves its account waiting to try again, on a timer,
/// never syncing for ever: the 40 hours an IDLE account sat on a socket a NAT had forgotten.
#[test]
fn a_stalled_pass_leaves_the_account_waiting_with_a_wake() {
    use crate::fetch::{Effect, Event, First, Link, Step, step};
    let error = RuntimeError::Stalled {
        doing: "waiting for its answer",
        after: std::time::Duration::from_secs(240),
    };
    let failure = Failure::of("", &error);
    let syncing = Link::Syncing {
        first: First::No,
        step: Step::Headers {
            mailbox: "INBOX".to_owned(),
        },
        count: None,
        after: Box::new(Link::Fresh),
    };
    let event = Event::Failed {
        retry: failure.retry,
        why: failure.why,
        pause: failure.pause,
    };
    let every = std::time::Duration::from_secs(300);
    let (next, effects) = step(&syncing, event, now(), every);
    assert!(
        matches!(
            next,
            Link::Waiting {
                why: Pause::Unreachable,
                ..
            }
        ),
        "{next:?}"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::WakeAt(at) if *at > now())),
        "nothing will wake it: {effects:?}"
    );
}

#[test]
fn only_a_pass_that_ran_may_have_stored_something() {
    let finished = PassEnd::Finished(AccountReport {
        account: account(),
        address: "ada@example.test".to_owned(),
        counts: Counts::default(),
        trouble: vec![],
    });
    let failed = PassEnd::Failed {
        account: account(),
        address: "ada@example.test".to_owned(),
        retry: Retry::After(std::time::Duration::from_secs(5)),
        why: "cannot connect".to_owned(),
        pause: crate::fetch::Pause::Unreachable,
        offer: None,
    };
    let cancelled = PassEnd::Cancelled {
        account: account(),
        address: "ada@example.test".to_owned(),
    };
    // A flag sweep stores without counting, so even an all-zero pass that ran says yes.
    assert!(finished.may_have_stored());
    assert!(cancelled.may_have_stored());
    assert!(!failed.may_have_stored());
}
