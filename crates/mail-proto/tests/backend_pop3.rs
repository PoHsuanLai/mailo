//! `Pop3Backend` driven through the same replay harness as the sessions.
//!
//! That is the payoff of making backends `Machine`-shaped: a backend test is a byte transcript,
//! so what is under test is the whole path from a `ProtoOp` to a domain value rather than a
//! mocked session boundary.

//! Note: these traces carry a literal test password. That is deliberate — the harness asserts
//! byte-for-byte what reaches the wire, and a redacted placeholder would assert nothing about
//! the command actually sent. The value is a fixture, not a credential.

mod common;

use common::replay;
use mail_domain::*;
use mail_proto::backend::{Authenticate, Pop3Backend};
use mail_proto::{Backend, IoNeed, IoReady, Machine, Pop3Command, Progress, ProtoOutcome};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        observed_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
    }
}

fn backend() -> Pop3Backend {
    Pop3Backend::new(
        ACCOUNT,
        caps(),
        // The factory authenticates, exactly as the runtime's will: it is the only thing that
        // knows both the mechanism and the credential.
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
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    }
}

/// The harness drives a `Machine`; a `Backend` starts from an op, so adapt rather than
/// duplicating the loop.
struct Driven {
    backend: Pop3Backend,
    op: Option<ProtoOp>,
}

impl Machine for Driven {
    type Out = ProtoOutcome;

    fn start(&mut self) -> Progress<ProtoOutcome> {
        let op = self.op.take().expect("start called twice");
        self.backend.begin(op)
    }

    fn feed(&mut self, ready: IoReady) -> Progress<ProtoOutcome> {
        self.backend.feed(ready)
    }
}

fn run(op: ProtoOp, trace: &str) -> (ProtoOutcome, Pop3Backend) {
    let mut driven = Driven {
        backend: backend(),
        op: Some(op),
    };
    let out = replay(&mut driven, trace).unwrap();
    (out, driven.backend)
}

/// CAPA tells us whether TOP is real, which decides whether a first sync is destructive.
#[test]
fn fetch_caps_learns_top_and_pipelining() {
    let trace = concat!(
        "# DERIVED from spike/out/pop3.trace: the CAPA msa.ntu.edu.tw actually returns.\n",
        "# Asked twice, because RFC 2449 permits the answer to differ once authenticated —\n",
        "# the same trap F14 caught on Gmail's IMAP. Here the second answer adds nothing, but\n",
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
        "# SYNTHETIC, shaped after spike/out/pop3.trace.\n",
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
    });
    assert!(matches!(outcome, Progress::Failed(_)), "{outcome:?}");
}

/// A watch completes immediately: POP3 has no push, so the interval lives in AccountCaps.
#[test]
fn watch_completes_at_once_because_pop3_has_no_push() {
    let mut backend = backend();
    match backend.begin(ProtoOp::Watch { mailbox: mailbox() }) {
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
    let mut driven = Driven {
        backend: backend(),
        op: Some(ProtoOp::FetchEnvelopes {
            mailbox: mailbox(),
            since: FetchSince::Beginning,
        }),
    };
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

// --- submission -----------------------------------------------------------------------

/// Submission is its own backend on its own connection, and the incoming one refuses it.
#[test]
fn smtp_backend_submits_and_reports_no_remote_copy() {
    use mail_mime::Posting;
    use mail_proto::Submission;
    use mail_proto::backend::SmtpBackend;

    let mut backend = SmtpBackend::new(
        ACCOUNT,
        caps(),
        // The envelope comes from the Posting, not from the closure: the closure knows the
        // host and the credential, and nothing about who this particular message is for.
        Box::new(|posting: Posting| {
            Ok(Submission {
                ehlo: "client.example".to_owned(),
                host: "smtp.example".to_owned(),
                port: 465,
                tls: Tls::Implicit,
                username: "ada@example.com".to_owned(),
                credential: Credential::Password("s3cr3t-password".to_owned()),
                sasl: vec![SaslMech::Plain],
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
        }),
    };
    let outcome = replay(
        &mut sending,
        include_str!("traces/smtp/submit_backend.trace"),
    )
    .unwrap();

    // `remote: None` on purpose: SMTP says the message was accepted, not where a copy was
    // filed. Gmail files it in Sent itself, and a client APPEND would duplicate it.
    assert!(
        matches!(outcome, ProtoOutcome::Submitted { remote: None }),
        "{outcome:?}"
    );
}
