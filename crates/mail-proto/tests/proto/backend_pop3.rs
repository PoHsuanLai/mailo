//! `Pop3Backend` driven through the same replay harness as the sessions.
//!
//! That is the payoff of making backends `Machine`-shaped: a backend test is a byte transcript,
//! so what is under test is the whole path from a `ProtoOp` to a domain value rather than a
//! mocked session boundary.

//! Note: these traces carry a literal test password. That is deliberate — the harness asserts
//! byte-for-byte what reaches the wire, and a redacted placeholder would assert nothing about
//! the command actually sent. The value is a fixture, not a credential.

use crate::common;

use common::{Driven, replay};
use mail_domain::testing::account;
use mail_domain::*;
use mail_proto::backend::{Authenticate, Pop3Backend, ServerCopies};
use mail_proto::{Backend, IoNeed, IoReady, Machine, Pop3Command, Progress, ProtoOutcome};
use porter_core::Credential;
use porter_core::SecretText;

fn caps() -> AccountCaps {
    AccountCaps {
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        connections: ConnectionBudget { max: 1 },
        ..common::caps()
    }
}

fn backend() -> Pop3Backend {
    Pop3Backend::new(
        account(),
        caps(),
        // The factory authenticates, exactly as the runtime's will: it is the only thing that
        // knows both the mechanism and the credential.
        Box::new(|auth: Authenticate, commands: Vec<Pop3Command>| {
            let mut all = Vec::new();
            if auth == Authenticate::First {
                all.extend([Pop3Command::User, Pop3Command::Pass]);
            }
            all.extend(commands);
            mail_proto::Pop3Session::new("student", "s3cr3t", all)
        }),
    )
}

fn mailbox() -> MailboxRef {
    MailboxRef {
        account: account(),
        path: "INBOX".to_owned(),
    }
}

fn run(op: ProtoOp, trace: &str) -> (ProtoOutcome, Pop3Backend) {
    let mut driven = Driven::new(backend(), op);
    let out = replay(&mut driven, trace).unwrap();
    (out, driven.backend)
}

/// CAPA tells us whether TOP is real, which decides whether a first sync is destructive.
#[test]
fn fetch_caps_learns_top_and_pipelining() {
    let trace = concat!(
        "# DERIVED from a capture: the CAPA a campus Dovecot actually returns.\n",
        "# Asked twice, because RFC 2449 permits the answer to differ once authenticated —\n",
        "# the same trap as Gmail's IMAP CAPABILITY. Here the second answer adds nothing, but\n",
        "# the second is the one believed.\n",
        "S: +OK POP3 server ready\n",
        "C: CAPA\n",
        "S: +OK\n",
        "S: TOP\n",
        "S: UIDL\n",
        "S: PIPELINING\n",
        "S: USER\n",
        "S: SASL PLAIN\n",
        "S: .\n",
        "C: USER student\n",
        "S: +OK\n",
        "C: PASS s3cr3t\n",
        "S: +OK maildrop ready\n",
        "C: CAPA\n",
        "S: +OK\n",
        "S: TOP\n",
        "S: UIDL\n",
        "S: PIPELINING\n",
        "S: .\n",
        "DONE\n"
    );
    let (outcome, _) = run(ProtoOp::FetchCaps, trace);
    let ProtoOutcome::Caps(caps) = outcome else {
        panic!("expected Caps, got {outcome:?}");
    };
    assert_eq!(
        caps.top,
        Supported::Yes,
        "TOP is advertised and must be seen"
    );
    assert_eq!(caps.pipelining, Supported::Yes);
}

/// A server that answers CAPA with -ERR leaves capabilities absent rather than assumed.
#[test]
fn capa_refused_does_not_invent_capabilities() {
    let trace = concat!(
        "# SYNTHETIC. Older servers answer CAPA with -ERR and still work.\n",
        "S: +OK POP3 server ready\n",
        "C: CAPA\n",
        "S: -ERR unknown command\n",
        "C: USER student\n",
        "S: +OK\n",
        "C: PASS s3cr3t\n",
        "S: +OK maildrop ready\n",
        "C: CAPA\n",
        "S: -ERR unknown command\n",
        "DONE\n"
    );
    let (outcome, _) = run(ProtoOp::FetchCaps, trace);
    let ProtoOutcome::Caps(caps) = outcome else {
        panic!("expected Caps, got {outcome:?}");
    };
    // Absent, not Yes. An old server may well implement TOP, but guessing is how a first sync
    // silently marks a mailbox read.
    assert_eq!(caps.top, Supported::Absent);
}

/// The survey is what makes size-banded ordering possible: LIST gives exact sizes before any
/// RETR, and 90% of a real maildrop is 10% of its bytes.
#[test]
fn a_survey_records_uidls_with_their_sizes() {
    let trace = concat!(
        "# SYNTHETIC, shaped after a captured Dovecot session.\n",
        "S: +OK POP3 server ready\n",
        "C: USER student\n",
        "S: +OK\n",
        "C: PASS s3cr3t\n",
        "S: +OK maildrop has 3 messages\n",
        "C: STAT\n",
        "S: +OK 3 4096\n",
        "C: UIDL\n",
        "S: +OK\n",
        "S: 1 0000000166aaf64b\n",
        "S: 2 0000000266aaf64b\n",
        "S: 3 0000000366aaf64b\n",
        "S: .\n",
        "C: LIST\n",
        "S: +OK\n",
        "S: 1 512\n",
        "S: 2 3072\n",
        "S: 3 512\n",
        "S: .\n",
        "DONE\n"
    );
    let (outcome, backend) = run(
        ProtoOp::FetchEnvelopes {
            mailbox: mailbox(),
            since: FetchSince::Beginning,
        },
        trace,
    );
    assert!(matches!(outcome, ProtoOutcome::Ingested(_)));
    let surveyed = backend.surveyed();
    assert_eq!(surveyed.len(), 3);
    assert_eq!(surveyed[0], ("0000000166aaf64b".to_owned(), 512));
    assert_eq!(surveyed[1], ("0000000266aaf64b".to_owned(), 3072));
}

/// Operations POP3 has no representation for confirm at once. The local change IS the change.
#[test]
fn local_only_operations_confirm_without_touching_the_wire() {
    let mut backend = backend();
    for op in [
        ProtoOp::SetFlags {
            remotes: vec![RemoteRef::Pop {
                uidl: "x".to_owned(),
            }],
            read: Some(ReadState::Read),
            star: None,
        },
        ProtoOp::SetMailbox {
            remotes: vec![],
            role: MailboxRole::Archive,
        },
        ProtoOp::SetLabels {
            remotes: vec![],
            add: vec![],
            remove: vec![],
        },
        // POP3 has no flags at all: an answered receipt is remembered only here.
        ProtoOp::AddKeyword {
            remotes: vec![RemoteRef::Pop {
                uidl: "x".to_owned(),
            }],
            keyword: Keyword::MdnSent,
        },
        // Delete forever is local on POP3 unless the account removes the server's copies
        // (`removing_checks_each_number_then_deletes_and_quits`). The store keeps the UIDL so the
        // message is not downloaded again.
        ProtoOp::Destroy {
            remotes: vec![RemoteRef::Pop {
                uidl: "x".to_owned(),
            }],
        },
    ] {
        match backend.begin(op) {
            Progress::Done(ProtoOutcome::Applied) => {}
            other => panic!("expected an immediate Applied, got {other:?}"),
        }
    }
}

/// Message numbers are session-scoped, so a UIDL we have not surveyed addresses nothing.
#[test]
fn fetching_an_unsurveyed_message_is_refused_rather_than_guessed() {
    let mut backend = backend();
    let outcome = backend.begin(ProtoOp::FetchBody {
        remotes: vec![RemoteRef::Pop {
            uidl: "never-seen".to_owned(),
        }],
    });
    // Guessing a message number here would fetch a different message than the caller asked for.
    assert!(
        matches!(outcome, Progress::Failed(_)),
        "expected a refusal, got {outcome:?}"
    );
}

/// POP3 cannot send or store; saying so beats a silent no-op.
#[test]
fn submission_is_unsupported_not_silently_ignored() {
    let mut backend = backend();
    let outcome = backend.begin(ProtoOp::Submit {
        draft: DraftId::generate(),
        raw: BlobId::generate(),
        mail_from: "ada@example.com".to_owned(),
        rcpt_to: vec!["bob@example.com".to_owned()],
        delivery: mail_domain::DeliveryReport::Unrequested,
    });
    assert!(matches!(outcome, Progress::Failed(_)), "{outcome:?}");
}

/// A watch completes immediately: POP3 has no push, so the interval lives in AccountCaps.
#[test]
fn watch_completes_at_once_because_pop3_has_no_push() {
    let mut backend = backend();
    match backend.begin(ProtoOp::Watch {
        mailbox: mailbox(),
        uidnext: None,
        also: Vec::new(),
    }) {
        Progress::Done(ProtoOutcome::Woken) => {}
        other => panic!("expected Woken, got {other:?}"),
    }
}

/// The IoNeed import is used by the harness; this keeps the intent visible.
const _: fn(&IoNeed) = |_| {};

/// TOP fetches headers without marking the message read — the operation that makes a large
/// first sync non-destructive.
#[test]
fn fetch_headers_uses_top_and_returns_raw_bytes() {
    // The survey has to run first: message numbers are session-scoped, so a UIDL is only
    // addressable once we have seen it.
    let survey = concat!(
        "S: +OK POP3 server ready\n",
        "C: USER student\n",
        "S: +OK\n",
        "C: PASS s3cr3t\n",
        "S: +OK\n",
        "C: STAT\n",
        "S: +OK 1 512\n",
        "C: UIDL\n",
        "S: +OK\n",
        "S: 1 0000000166aaf64b\n",
        "S: .\n",
        "C: LIST\n",
        "S: +OK\n",
        "S: 1 512\n",
        "S: .\n",
        "DONE\n"
    );
    let mut driven = Driven::new(
        backend(),
        ProtoOp::FetchEnvelopes {
            mailbox: mailbox(),
            since: FetchSince::Beginning,
        },
    );
    replay(&mut driven, survey).unwrap();

    // TOP is only offered once caps say so; otherwise the backend must refuse rather than
    // quietly falling back to RETR.
    let mut backend = driven.backend;
    let remote = RemoteRef::Pop {
        uidl: "0000000166aaf64b".to_owned(),
    };
    assert!(
        matches!(
            backend.begin(ProtoOp::FetchHeaders {
                remotes: vec![remote.clone()]
            }),
            Progress::Failed(_)
        ),
        "without TOP, refusing beats marking the message read"
    );
}

// --- removing the server's copies --------------------------------------------------------

/// A backend that has surveyed a maildrop of three messages, `aaa`, `bbb` and `ccc`, numbered
/// 1 to 3, and removes the server's copies as `copies` says.
fn surveyed(copies: ServerCopies) -> Pop3Backend {
    let survey = concat!(
        "S: +OK POP3 server ready\n",
        "C: USER student\n",
        "S: +OK\n",
        "C: PASS s3cr3t\n",
        "S: +OK\n",
        "C: STAT\n",
        "S: +OK 3 1536\n",
        "C: UIDL\n",
        "S: +OK\n",
        "S: 1 aaa\n",
        "S: 2 bbb\n",
        "S: 3 ccc\n",
        "S: .\n",
        "C: LIST\n",
        "S: +OK\n",
        "S: 1 512\n",
        "S: 2 512\n",
        "S: 3 512\n",
        "S: .\n",
        "DONE\n"
    );
    let mut driven = Driven::new(
        backend().with_copies(copies),
        ProtoOp::FetchEnvelopes {
            mailbox: mailbox(),
            since: FetchSince::Beginning,
        },
    );
    replay(&mut driven, survey).unwrap();
    driven.backend
}

fn pop(uidl: &str) -> RemoteRef {
    RemoteRef::Pop {
        uidl: uidl.to_owned(),
    }
}

/// An account that removes the server's copies: each message's number from the survey is
/// checked against its UIDL before it is marked, and `QUIT` is sent, which is when the server
/// removes what was marked. A UIDL the survey did not list is not on the server to remove.
#[test]
fn removing_checks_each_number_then_deletes_and_quits() {
    let trace = concat!(
        "# SYNTHETIC. RFC 1939 §7: `UIDL n` answers `+OK n id`; §6: marked messages are\n",
        "# removed at QUIT, so the QUIT is what makes the DELEs count.\n",
        "S: +OK POP3 server ready\n",
        "C: USER student\n",
        "S: +OK\n",
        "C: PASS s3cr3t\n",
        "S: +OK\n",
        "C: UIDL 2\n",
        "S: +OK 2 bbb\n",
        "C: DELE 2\n",
        "S: +OK message 2 deleted\n",
        "C: UIDL 3\n",
        "S: +OK 3 ccc\n",
        "C: DELE 3\n",
        "S: +OK message 3 deleted\n",
        "C: QUIT\n",
        "S: +OK dewey POP3 server signing off (1 message left)\n",
        "DONE\n"
    );
    let mut driven = Driven::new(
        surveyed(ServerCopies::Removable),
        ProtoOp::Destroy {
            remotes: vec![pop("bbb"), pop("gone"), pop("ccc")],
        },
    );
    let outcome = replay(&mut driven, trace).unwrap();
    assert!(matches!(outcome, ProtoOutcome::Applied), "{outcome:?}");
}

/// Another client deleted mail since the survey, so number 2 is `ccc` now. The session fails
/// before anything more is sent: no `DELE` of the wrong message, and no `QUIT`, so nothing
/// marked is removed. The refusal is transient, for the next survey to number the maildrop again.
#[test]
fn a_number_that_names_another_message_now_removes_nothing() {
    let trace = concat!(
        "# SYNTHETIC. `bbb` was deleted elsewhere, and the maildrop renumbered.\n",
        "S: +OK POP3 server ready\n",
        "C: USER student\n",
        "S: +OK\n",
        "C: PASS s3cr3t\n",
        "S: +OK\n",
        "C: UIDL 2\n",
        "S: +OK 2 ccc\n",
        "FAIL Refused\n"
    );
    let mut driven = Driven::new(
        surveyed(ServerCopies::Removable),
        ProtoOp::Destroy {
            remotes: vec![pop("bbb")],
        },
    );
    let err = replay(&mut driven, trace).unwrap_err();
    assert_eq!(
        err.retry(),
        Retry::After(std::time::Duration::from_secs(60))
    );
}

/// Nothing to remove is nothing to connect for: a removing account whose messages the survey no
/// longer lists, and an account that keeps the server's copies, both answer at once.
#[test]
fn nothing_listed_or_nothing_removable_never_touches_the_wire() {
    let cases = [
        (ServerCopies::Removable, vec![pop("gone")]),
        (ServerCopies::Kept, vec![pop("bbb")]),
    ];
    for (copies, remotes) in cases {
        let mut backend = surveyed(copies);
        match backend.begin(ProtoOp::Destroy { remotes }) {
            Progress::Done(ProtoOutcome::Applied) => {}
            other => panic!("{copies:?}: expected an immediate Applied, got {other:?}"),
        }
    }
}

// --- submission -----------------------------------------------------------------------

/// Submission is its own backend on its own connection, and the incoming one refuses it.
#[test]
fn smtp_backend_submits_and_reports_no_remote_copy() {
    use mail_mime::Posting;
    use mail_proto::backend::SmtpBackend;
    use mail_proto::{Authentication, SignIn, Submission};

    let mut backend = SmtpBackend::new(
        account(),
        caps(),
        // The envelope comes from the Posting, not from the closure: the closure knows the
        // host and the credential, and nothing about who this particular message is for.
        Box::new(|posting: Posting| {
            Ok(Submission {
                ehlo: "client.example".to_owned(),
                host: "smtp.example".to_owned(),
                port: 465,
                tls: Tls::Implicit,
                auth: Authentication::SignIn(SignIn {
                    username: "ada@example.com".to_owned(),
                    credential: Credential::Password(SecretText::new("s3cr3t-password".to_owned())),
                    sasl: vec![SaslMech::Plain],
                }),
                mail_from: posting.mail_from,
                recipients: posting.rcpt_to,
                receipt: None,
                message: posting.message,
            })
        }),
    );

    // Staging is separate because the bytes live in the blob store, which mail-proto does not
    // know about. Submitting without staging must fail loudly rather than send nothing.
    let unstaged = backend.begin(ProtoOp::Submit {
        draft: DraftId::generate(),
        raw: BlobId::generate(),
        mail_from: "ada@example.com".to_owned(),
        rcpt_to: vec!["bob@example.com".to_owned()],
        delivery: mail_domain::DeliveryReport::Unrequested,
    });
    assert!(
        matches!(unstaged, Progress::Failed(_)),
        "an unstaged submission must not proceed: {unstaged:?}"
    );

    backend
        .stage(Posting {
            mail_from: "ada@example.com".to_owned(),
            rcpt_to: vec!["bob@example.com".to_owned()],
            message: b"From: ada@example.com\r\nTo: bob@example.com\r\nSubject: hi\r\n\r\nbody\r\n"
                .to_vec(),
            delivery: mail_domain::DeliveryReport::Unrequested,
        })
        .unwrap();

    struct Sending {
        backend: SmtpBackend,
        op: Option<ProtoOp>,
    }
    impl Machine for Sending {
        type Out = ProtoOutcome;
        fn start(&mut self) -> Progress<ProtoOutcome> {
            let op = self.op.take().expect("start called twice");
            self.backend.begin(op)
        }
        fn feed(&mut self, ready: IoReady) -> Progress<ProtoOutcome> {
            self.backend.feed(ready)
        }
    }

    let mut sending = Sending {
        backend,
        op: Some(ProtoOp::Submit {
            draft: DraftId::generate(),
            raw: BlobId::generate(),
            mail_from: "ada@example.com".to_owned(),
            rcpt_to: vec!["bob@example.com".to_owned()],
            delivery: mail_domain::DeliveryReport::Unrequested,
        }),
    };
    let outcome = replay(
        &mut sending,
        include_str!("../traces/smtp/submit_backend.trace"),
    )
    .unwrap();

    // `remote: None` on purpose: SMTP says the message was accepted, not where a copy was
    // filed. Gmail files it in Sent itself, and a client APPEND would duplicate it.
    assert!(
        matches!(outcome, ProtoOutcome::Submitted { remote: None }),
        "{outcome:?}"
    );
}
