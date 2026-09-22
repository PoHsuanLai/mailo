//! `ImapBackend`: the product decisions, checked as bytes on the wire.
//!
//! What matters here is not that a command was sent but *which* — `BODY.PEEK` rather than
//! `BODY`, a label change rather than a delete, no `EXPUNGE` at all.

mod common;

use common::replay;
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{
    Backend, ImapAuth, ImapCommand, ImapSession, IoReady, Machine, Progress, ProtoError,
    ProtoOutcome,
};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn caps(labels: ServerLabels, archive: ArchiveMeans) -> AccountCaps {
    AccountCaps {
        labels,
        threads: ServerThreads::ProviderId,
        watch: WatchMode::Idle,
        archive,
        folders: FolderRoles::default(),
        condstore: Condstore::Supported,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 5 },
        observed_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
    }
}

fn backend(caps: AccountCaps) -> ImapBackend {
    ImapBackend::new(
        ACCOUNT,
        caps,
        // The factory owns authentication, which is why it and not the backend decides that
        // this account uses XOAUTH2.
        Box::new(|auth: Authenticate, commands: Vec<ImapCommand>| {
            let mut all = Vec::new();
            if auth == Authenticate::First {
                all.push(ImapCommand::AuthenticateXoauth2);
            }
            all.extend(commands);
            ImapSession::new(
                ImapAuth {
                    username: "ada@example.test".to_owned(),
                    credential: Credential::OAuth {
                        access: "ya29.token".to_owned(),
                        refresh: "1//refresh".to_owned(),
                        expires_at: chrono::DateTime::from_timestamp(2_000_000_000, 0).unwrap(),
                    },
                    sasl: vec![SaslMech::XOauth2],
                },
                all,
            )
        }),
    )
}

struct Driven {
    backend: ImapBackend,
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

fn inbox() -> MailboxRef {
    MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    }
}

/// The same backend, authenticating with a password: what every server but Gmail gets.
fn password_backend(caps: AccountCaps) -> ImapBackend {
    ImapBackend::new(
        ACCOUNT,
        caps,
        Box::new(|auth: Authenticate, commands: Vec<ImapCommand>| {
            let mut all = Vec::new();
            if auth == Authenticate::First {
                all.push(ImapCommand::Login);
            }
            all.extend(commands);
            ImapSession::new(
                ImapAuth {
                    username: "ada@example.test".to_owned(),
                    credential: Credential::Password("hunter2".to_owned()),
                    sasl: vec![SaslMech::Plain],
                },
                all,
            )
        }),
    )
}

fn imap_ref(mailbox: &str, uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: mailbox.to_owned(),
        uidvalidity: 1,
        uid,
    }
}

/// Fetching headers must never mark the message read.
///
/// `BODY.PEEK[HEADER]` and not `BODY[HEADER]`: the peeking form is IMAP's equivalent of POP3's
/// TOP, and the non-peeking one sets \Seen as a side effect of populating a list view.
#[test]
fn fetching_headers_uses_body_peek() {
    let trace = concat!(
        "# SYNTHETIC. The assertion that matters is the C: line: BODY.PEEK, never BODY.\n",
        "S: * OK Gimap ready\n",
        "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n",
        "S: a001 OK authenticated\n",
        "C: a002 EXAMINE \"INBOX\"\n",
        "S: * OK [UIDVALIDITY 1] UIDs valid.\n",
        "S: a002 OK [READ-ONLY] EXAMINE completed\n",
        "C: a003 UID FETCH 42 (UID FLAGS BODY.PEEK[HEADER])\n",
        "S: * 1 FETCH (UID 42 FLAGS (\\Seen))\n",
        "S: a003 OK Success\n",
        "DONE\n"
    );
    let mut driven = Driven {
        backend: backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox)),
        op: Some(ProtoOp::FetchHeaders {
            remotes: vec![imap_ref("INBOX", 42)],
        }),
    };
    let outcome = replay(&mut driven, trace).unwrap();
    // And the flags it asked for are kept.
    //
    // This trace has said `FLAGS (\Seen)` since the day it was written and the assertion was
    // `matches!(outcome, Fetched { .. })` — true whatever happened to them, and they were
    // dropped. Every message was therefore built unread, and only a later flag sweep could
    // correct it; on a CONDSTORE server that sweep asks `CHANGEDSINCE` and never revisits old
    // mail, so anything found by backfill stayed unread for ever. Against a real account, 177
    // of every 200 messages. See `CONVENTIONS.md`, "An assertion that was already true".
    match outcome {
        ProtoOutcome::Fetched { flags, .. } => assert_eq!(
            flags,
            vec![(
                imap_ref("INBOX", 42),
                mail_domain::ReadState::Read,
                mail_domain::Star::Unstarred
            )],
            "the server said \\Seen and the fetch discarded it"
        ),
        other => panic!("{other:?}"),
    }
}

/// Archiving on Gmail is a label change. Nothing is deleted, and no EXPUNGE is issued.
#[test]
fn archiving_on_gmail_removes_the_inbox_label() {
    let trace = concat!(
        "# SYNTHETIC. Gmail files by label: adding \\All and removing \\Inbox archives.\n",
        "# There is deliberately no \\Deleted and no EXPUNGE anywhere in this transcript.\n",
        "S: * OK Gimap ready\n",
        "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n",
        "S: a001 OK authenticated\n",
        "C: a002 SELECT \"INBOX\"\n",
        "S: * OK [UIDVALIDITY 1] UIDs valid.\n",
        "S: a002 OK [READ-WRITE] SELECT completed\n",
        "C: a003 UID STORE 42 +X-GM-LABELS (\\All)\n",
        "S: a003 OK Success\n",
        "C: a004 UID STORE 42 -X-GM-LABELS (\\Inbox)\n",
        "S: a004 OK Success\n",
        "DONE\n"
    );
    let mut driven = Driven {
        backend: backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox)),
        op: Some(ProtoOp::SetMailbox {
            remotes: vec![imap_ref("INBOX", 42)],
            role: MailboxRole::Archive,
        }),
    };
    assert!(matches!(
        replay(&mut driven, trace).unwrap(),
        ProtoOutcome::Applied
    ));
}

/// Expunging is refused outright, not gated on a capability.
#[test]
fn expunge_is_refused_because_the_setting_cannot_be_read() {
    let mut backend = backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox));
    let outcome = backend.begin(ProtoOp::Expunge {
        remotes: vec![imap_ref("INBOX", 42)],
    });
    // Gmail may be configured to delete permanently and IMAP cannot report that, so there is
    // nothing to gate on — the command does not exist.
    assert!(matches!(outcome, Progress::Failed(_)), "{outcome:?}");
}

/// Without CONDSTORE, or without a trusted modseq, the sweep fetches every flag.
#[test]
fn a_flag_sweep_without_a_modseq_fetches_everything() {
    let trace = concat!(
        "# SYNTHETIC. No modseq means a full flag fetch: slow and correct, which is the right\n",
        "# way round to be wrong. Dovecot 2.0.18 froze HIGHESTMODSEQ while EXISTS climbed.\n",
        "S: * OK Gimap ready\n",
        "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n",
        "S: a001 OK authenticated\n",
        "C: a002 EXAMINE \"INBOX\"\n",
        "S: a002 OK [READ-ONLY] EXAMINE completed\n",
        "C: a003 UID FETCH 1:* (UID FLAGS)\n",
        "S: * 1 FETCH (UID 42 FLAGS (\\Seen))\n",
        "S: a003 OK Success\n",
        "DONE\n"
    );
    let mut driven = Driven {
        backend: backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox)),
        op: Some(ProtoOp::FetchFlags {
            mailbox: MailboxRef {
                account: ACCOUNT,
                path: "INBOX".to_owned(),
            },
            since_modseq: None,
        }),
    };
    assert!(matches!(
        replay(&mut driven, trace).unwrap(),
        ProtoOutcome::Ingested(_)
    ));
}

/// With CONDSTORE and a modseq, the sweep is one CHANGEDSINCE fetch.
#[test]
fn a_flag_sweep_with_a_modseq_uses_changedsince() {
    let trace = concat!(
        "S: * OK Gimap ready\n",
        "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n",
        "S: a001 OK authenticated\n",
        "C: a002 EXAMINE \"INBOX\"\n",
        "S: a002 OK [READ-ONLY] EXAMINE completed\n",
        "C: a003 UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 3737642)\n",
        "S: a003 OK Success\n",
        "DONE\n"
    );
    let mut driven = Driven {
        backend: backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox)),
        op: Some(ProtoOp::FetchFlags {
            mailbox: MailboxRef {
                account: ACCOUNT,
                path: "INBOX".to_owned(),
            },
            since_modseq: Some(3_737_642),
        }),
    };
    assert!(matches!(
        replay(&mut driven, trace).unwrap(),
        ProtoOutcome::Ingested(_)
    ));
}

/// A label change on a server without labels is local, and touches no socket.
#[test]
fn labels_on_a_server_without_them_stay_local() {
    let mut backend = backend(caps(ServerLabels::LocalOnly, ArchiveMeans::LocalOnly));
    match backend.begin(ProtoOp::SetLabels {
        remotes: vec![imap_ref("INBOX", 42)],
        add: vec!["work".to_owned()],
        remove: vec![],
    }) {
        Progress::Done(ProtoOutcome::Applied) => {}
        other => panic!("expected an immediate Applied, got {other:?}"),
    }
}

/// Submission belongs to another backend and another connection.
#[test]
fn submission_is_refused_here() {
    let mut backend = backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox));
    assert!(matches!(
        backend.begin(ProtoOp::Submit {
            draft: DraftId::generate(),
            raw: BlobId::generate(),
            mail_from: "ada@example.com".to_owned(),
            rcpt_to: vec!["bob@example.com".to_owned()],
        }),
        Progress::Failed(_)
    ));
}

/// A fetched body is the literal's bytes, and nothing else.
mod literal_bodies {
    use mail_proto::{ImapTranscript, Untagged};

    fn untagged(raw: &[u8]) -> Untagged {
        Untagged {
            during: 0,
            text: String::from_utf8_lossy(raw).trim_end().to_owned(),
            raw: raw.to_vec(),
        }
    }

    #[test]
    fn the_closing_paren_of_the_response_is_not_part_of_the_message() {
        // Found by reading what the CLI printed: every message fetched over IMAP carried a
        // trailing `)`. The end-to-end tests asserted `contains`, so none of them saw it.
        let body = b"Subject: s\r\n\r\nand a closing paren )\r\n";
        let mut raw = format!("* 2 FETCH (UID 102 BODY[] {{{}}}\r\n", body.len()).into_bytes();
        raw.extend_from_slice(body);
        raw.extend_from_slice(b")\r\n");

        let got = untagged(&raw)
            .literal()
            .expect("a literal is present")
            .to_vec();
        assert_eq!(got, body, "the body must be exactly the literal's bytes");
    }

    #[test]
    fn the_fetch_header_is_not_part_of_the_message_either() {
        let body = b"Subject: s\r\n\r\nbody\r\n";
        let mut raw = format!("* 1 FETCH (UID 1 BODY[] {{{}}}\r\n", body.len()).into_bytes();
        raw.extend_from_slice(body);
        raw.extend_from_slice(b")\r\n");

        let got = untagged(&raw).literal().unwrap().to_vec();
        assert!(
            !String::from_utf8_lossy(&got).contains("FETCH"),
            "the response header leaked into the message: {:?}",
            String::from_utf8_lossy(&got)
        );
    }

    #[test]
    fn a_body_that_is_not_utf8_survives_intact() {
        // `Untagged::text` goes through `from_utf8_lossy`, which turns every 8-bit byte into
        // U+FFFD. A Latin-1 message fetched that way is silently mangled — the reader shows
        // replacement characters where the sender wrote accents, and the stored blob is wrong
        // for ever after.
        let mut body = b"Subject: caf\xe9\r\n\r\n".to_vec();
        body.extend_from_slice(&[0xe9, 0xfc, 0xff, 0x00, 0x41]);
        let mut raw = format!("* 1 FETCH (UID 1 BODY[] {{{}}}\r\n", body.len()).into_bytes();
        raw.extend_from_slice(&body);
        raw.extend_from_slice(b")\r\n");

        let got = untagged(&raw).literal().unwrap().to_vec();
        assert_eq!(got, body, "8-bit bytes must reach the store unchanged");
        assert!(
            !got.contains(&0xef),
            "a replacement character appeared: {got:?}"
        );
    }

    #[test]
    fn trailing_whitespace_in_a_message_is_not_trimmed() {
        // `text` is trimmed, which is right for protocol vocabulary and wrong for mail: a
        // message legitimately ends with blank lines.
        let body = b"Subject: s\r\n\r\nbody\r\n\r\n   \r\n";
        let mut raw = format!("* 1 FETCH (UID 1 BODY[] {{{}}}\r\n", body.len()).into_bytes();
        raw.extend_from_slice(body);
        raw.extend_from_slice(b")\r\n");

        assert_eq!(untagged(&raw).literal().unwrap(), body);
    }

    #[test]
    fn a_response_with_no_literal_offers_no_body() {
        // A FLAGS-only FETCH has nothing to hand up, and inventing something from its text is
        // how the response header became a message in the first place.
        assert!(
            untagged(b"* 1 FETCH (UID 1 FLAGS (\\Seen))\r\n")
                .literal()
                .is_none()
        );
        assert!(untagged(b"* SEARCH 1 2 3\r\n").literal().is_none());
        assert!(untagged(b"").literal().is_none());
        // A truncated literal — the count promises more than arrived — is not a body.
        assert!(
            untagged(b"* 1 FETCH (UID 1 BODY[] {99}\r\nshort")
                .literal()
                .is_none()
        );
    }

    #[test]
    fn the_transcript_still_exposes_text_for_protocol_parsing() {
        // `text` stays, because SEARCH results and FETCH attribute names are ASCII vocabulary
        // and reading them as text is what the rest of this backend does.
        let t = ImapTranscript {
            untagged: vec![untagged(b"* SEARCH 101 102\r\n")],
            capabilities: Vec::new(),
        };
        assert!(t.untagged[0].text.contains("SEARCH"));
    }
}

/// Gmail's labels, asked for and discarded for the life of the project.
///
/// The response bytes below are the shape `traces/imap/gmail_fetch.trace` records from a real
/// capture, which is why this can be checked at all without an account on a server that has
/// labels.
mod gmail_labels {
    use super::*;

    fn surveyed(fetch_lines: &str) -> Ingest {
        let trace = format!(
            concat!(
                "S: * OK Gimap ready\n",
                "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n",
                "S: a001 OK authenticated\n",
                "C: a002 EXAMINE \"INBOX\"\n",
                "S: * OK [UIDVALIDITY 1] UIDs valid.\n",
                "S: * OK [UIDNEXT 43] Predicted next UID.\n",
                "S: a002 OK [READ-ONLY] EXAMINE completed\n",
                "C: a003 UID FETCH 1:* (UID FLAGS RFC822.SIZE X-GM-LABELS)\n",
                "{}",
                "S: a003 OK Success\n",
                "DONE\n"
            ),
            fetch_lines
        );
        let mut driven = Driven {
            backend: backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox)),
            op: Some(ProtoOp::FetchEnvelopes {
                mailbox: inbox(),
                since: FetchSince::Beginning,
            }),
        };
        match replay(&mut driven, &trace).unwrap() {
            ProtoOutcome::Ingested(ingest) => *ingest,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_survey_now_carries_what_the_server_says_each_message_is_labelled() {
        // The capture's own shape: one message with a system label and a user label, one with
        // an empty list.
        let ingest = surveyed(concat!(
            "S: * 1 FETCH (UID 42 FLAGS (\\Seen) RFC822.SIZE 100 X-GM-LABELS (\\Inbox \"travel\"))\n",
            "S: * 2 FETCH (UID 43 FLAGS () RFC822.SIZE 200 X-GM-LABELS ())\n",
        ));

        assert_eq!(
            ingest.label_names,
            vec![(imap_ref("INBOX", 42), vec!["travel".to_owned()])],
            "the user label, and only messages that have one"
        );
    }

    #[test]
    fn gmails_names_for_mailboxes_and_flags_are_not_labels() {
        // `\Inbox` is `mailbox`, `\Starred` is `star`, `\Unread` is `read`. Carried through as
        // labels they would appear on every row, and the user could not remove them.
        let ingest = surveyed(
            "S: * 1 FETCH (UID 42 FLAGS (\\Seen) RFC822.SIZE 100 X-GM-LABELS (\\Inbox \\Sent \\Draft \\Spam \\Trash \\Important \\Starred \\Muted))\n",
        );
        assert!(
            ingest.label_names.is_empty(),
            "a system name became a label: {:?}",
            ingest.label_names
        );
    }

    #[test]
    fn a_quoted_label_may_contain_spaces_quotes_and_backslashes() {
        let ingest = surveyed(
            "S: * 1 FETCH (UID 42 FLAGS () RFC822.SIZE 100 X-GM-LABELS (\"two words\" \"with \\\"quotes\\\"\" plain))\n",
        );
        assert_eq!(
            ingest.label_names[0].1,
            vec![
                "two words".to_owned(),
                "with \"quotes\"".to_owned(),
                "plain".to_owned()
            ]
        );
    }

    #[test]
    fn a_non_ascii_label_is_decoded_rather_than_shown_as_wire_bytes() {
        // Gmail sends modified UTF-7. A Chinese label arriving as `&Ux1Tgg-` and being shown
        // that way is the same class of bug as F44's mailbox names.
        let ingest =
            surveyed("S: * 1 FETCH (UID 42 FLAGS () RFC822.SIZE 100 X-GM-LABELS (\"&Ux1Tgg-\"))\n");
        let name = &ingest.label_names[0].1[0];
        assert!(!name.contains('&'), "still modified UTF-7: {name:?}");
        assert!(!name.is_ascii(), "{name:?}");
    }

    #[test]
    fn a_server_without_labels_is_not_asked_for_them() {
        // F113's condition: ask for what is read, and only that. A Dovecot account has
        // no X-GM-LABELS and must not be sent an attribute it will reject.
        let trace = concat!(
            "S: * OK ready\n",
            "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
            "S: a001 OK authenticated\n",
            "C: a002 EXAMINE \"INBOX\"\n",
            "S: * OK [UIDVALIDITY 1] UIDs valid.\n",
            "S: a002 OK [READ-ONLY] EXAMINE completed\n",
            "C: a003 UID FETCH 1:* (UID FLAGS RFC822.SIZE)\n",
            "S: * 1 FETCH (UID 42 FLAGS () RFC822.SIZE 100)\n",
            "S: a003 OK Success\n",
            "DONE\n"
        );
        let mut driven = Driven {
            backend: password_backend(caps(ServerLabels::LocalOnly, ArchiveMeans::LocalOnly)),
            op: Some(ProtoOp::FetchEnvelopes {
                mailbox: inbox(),
                since: FetchSince::Beginning,
            }),
        };
        // `replay` asserts the client's side of the transcript, so the absence of X-GM-LABELS in
        // the C: line above is the assertion.
        let outcome = replay(&mut driven, trace).unwrap();
        match outcome {
            ProtoOutcome::Ingested(ingest) => assert!(ingest.label_names.is_empty()),
            other => panic!("{other:?}"),
        }
    }
}

fn qresync_caps() -> AccountCaps {
    AccountCaps {
        condstore: Condstore::Qresync,
        ..caps(ServerLabels::LocalOnly, ArchiveMeans::LocalOnly)
    }
}

fn resync(uidvalidity: u32, modseq: u64) -> ProtoOp {
    ProtoOp::ListRemote {
        mailbox: inbox(),
        since: Some(Resync {
            uidvalidity,
            modseq,
        }),
    }
}

/// With QRESYNC, one `SELECT` says what was expunged, and nothing is listed.
///
/// The server half is RFC 7162 §3.2.5.2's own example, plus a range written backwards, which is
/// legal and means the same range.
#[test]
fn a_qresync_sweep_asks_the_server_what_vanished() {
    let trace = concat!(
        "S: * OK Dovecot ready\n",
        "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n",
        "S: a001 OK authenticated\n",
        "C: a002 ENABLE QRESYNC\n",
        "S: * ENABLED QRESYNC\n",
        "S: a002 OK Enabled\n",
        "C: a003 EXAMINE \"INBOX\" (QRESYNC (67890007 20050715194045000))\n",
        "S: * 314 EXISTS\n",
        "S: * OK [UIDVALIDITY 67890007] UIDVALIDITY\n",
        "S: * OK [UIDNEXT 567] Predicted next UID\n",
        "S: * OK [HIGHESTMODSEQ 20050715194045319] Highest\n",
        "S: * VANISHED (EARLIER) 41,43:116,118,120:211,540:214\n",
        "S: * 49 FETCH (UID 117 FLAGS (\\Seen \\Answered) MODSEQ (20050715194045301))\n",
        "S: a003 OK [READ-ONLY] Examine completed\n",
        "DONE\n"
    );
    let mut driven = Driven {
        backend: backend(qresync_caps()),
        op: Some(resync(67_890_007, 20_050_715_194_045_000)),
    };
    let ProtoOutcome::Resynced { ingest, vanished } = replay(&mut driven, trace).unwrap() else {
        panic!("a QRESYNC select is not a listing, and must not come back as one");
    };
    assert_eq!(
        vanished,
        [(41, 41), (43, 116), (118, 118), (120, 211), (214, 540)]
    );
    assert!(
        ingest.gone.is_empty(),
        "the caller fills this, from what it holds"
    );
    assert_eq!(
        ingest.cursor,
        Some(SyncCursor::Imap {
            uidvalidity: 67_890_007,
            uidnext: 567,
            modseq: Some(20_050_715_194_045_319),
        })
    );
    assert_eq!(
        ingest.flags,
        [(
            RemoteRef::Imap {
                mailbox: "INBOX".to_owned(),
                uidvalidity: 67_890_007,
                uid: 117,
            },
            ReadState::Read,
            Star::Unstarred,
        )]
    );
}

/// A renumbered mailbox: the server ignores QRESYNC, so nothing it says is about our UIDs.
#[test]
fn a_qresync_sweep_after_a_renumbering_reports_nothing() {
    let trace = concat!(
        "S: * OK Dovecot ready\n",
        "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n",
        "S: a001 OK authenticated\n",
        "C: a002 ENABLE QRESYNC\n",
        "S: * ENABLED QRESYNC\n",
        "S: a002 OK Enabled\n",
        "C: a003 EXAMINE \"INBOX\" (QRESYNC (67890007 20050715194045000))\n",
        "S: * OK [UIDVALIDITY 99] UIDVALIDITY\n",
        "S: * OK [HIGHESTMODSEQ 5] Highest\n",
        "S: * VANISHED (EARLIER) 1:4294967295\n",
        "S: a003 OK [READ-ONLY] Examine completed\n",
        "DONE\n"
    );
    let mut driven = Driven {
        backend: backend(qresync_caps()),
        op: Some(resync(67_890_007, 20_050_715_194_045_000)),
    };
    let ProtoOutcome::Resynced { ingest, vanished } = replay(&mut driven, trace).unwrap() else {
        panic!("expected a resync");
    };
    assert!(vanished.is_empty());
    assert_eq!(
        ingest.cursor, None,
        "the cursor stays on the mailbox we know"
    );
}

/// Gmail offers CONDSTORE without QRESYNC: a `since` changes nothing, and the sweep lists.
#[test]
fn without_qresync_the_sweep_still_lists_every_uid() {
    let trace = concat!(
        "S: * OK Gimap ready\n",
        "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n",
        "S: a001 OK authenticated\n",
        "C: a002 EXAMINE \"INBOX\"\n",
        "S: * OK [UIDVALIDITY 1] UIDs valid\n",
        "S: a002 OK [READ-ONLY] EXAMINE completed\n",
        "C: a003 UID SEARCH ALL\n",
        "S: * SEARCH 7 9\n",
        "S: a003 OK Success\n",
        "DONE\n"
    );
    let mut driven = Driven {
        backend: backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox)),
        op: Some(resync(1, 3_737_642)),
    };
    assert!(matches!(
        replay(&mut driven, trace).unwrap(),
        ProtoOutcome::Ingested(_)
    ));
}

/// Believed only together: QRESYNC without CONDSTORE is a misconfigured server.
#[test]
fn qresync_is_believed_only_alongside_condstore() {
    for (advertised, expected) in [
        ("IMAP4rev1 CONDSTORE QRESYNC", Condstore::Qresync),
        ("IMAP4rev1 CONDSTORE", Condstore::Supported),
        ("IMAP4rev1 QRESYNC", Condstore::Absent),
    ] {
        let trace = format!(
            "S: * OK ready\n\
             C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n\
             S: a001 OK authenticated\n\
             C: a002 CAPABILITY\n\
             S: * CAPABILITY {advertised}\n\
             S: a002 OK done\n\
             C: a003 CAPABILITY\n\
             S: * CAPABILITY {advertised}\n\
             S: a003 OK done\n\
             DONE\n"
        );
        let mut driven = Driven {
            backend: backend(caps(ServerLabels::LocalOnly, ArchiveMeans::LocalOnly)),
            op: Some(ProtoOp::FetchCaps),
        };
        let ProtoOutcome::Caps(found) = replay(&mut driven, &trace).unwrap() else {
            panic!("expected caps");
        };
        assert_eq!(found.condstore, expected, "{advertised}");
    }
}

/// Large-message fetching: the structure first, then only the sections worth downloading.
mod parts {
    use super::*;

    const AUTH: &str = "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n";

    /// A report with a plain and an HTML body, and a nine-megabyte PDF beside them.
    #[test]
    fn a_structure_becomes_a_tree_with_sections_and_sizes() {
        let trace = format!(
            "S: * OK ready\n\
             {AUTH}\
             S: a001 OK authenticated\n\
             C: a002 EXAMINE \"INBOX\"\n\
             S: a002 OK [READ-ONLY] done\n\
             C: a003 UID FETCH 7 (UID BODYSTRUCTURE)\n\
             S: * 1 FETCH (UID 7 BODYSTRUCTURE (((\"TEXT\" \"PLAIN\" (\"CHARSET\" \"utf-8\") NIL NIL \"7BIT\" 120 4 NIL NIL NIL)(\"TEXT\" \"HTML\" (\"CHARSET\" \"utf-8\") NIL NIL \"QUOTED-PRINTABLE\" 900 20 NIL NIL NIL) \"ALTERNATIVE\" (\"BOUNDARY\" \"alt-b\") NIL NIL)(\"APPLICATION\" \"PDF\" (\"NAME\" \"report.pdf\") NIL NIL \"BASE64\" 9437184 NIL (\"ATTACHMENT\" (\"FILENAME\" \"report.pdf\")) NIL) \"MIXED\" (\"BOUNDARY\" \"mix-b\") NIL NIL))\n\
             S: a003 OK done\n\
             DONE\n"
        );
        let mut driven = Driven {
            backend: backend(caps(ServerLabels::LocalOnly, ArchiveMeans::LocalOnly)),
            op: Some(ProtoOp::FetchStructure {
                remotes: vec![imap_ref("INBOX", 7)],
            }),
        };
        let ProtoOutcome::Structures(found) = replay(&mut driven, &trace).unwrap() else {
            panic!("expected structures");
        };
        let leaf = |section: &str, mime: &str, octets: u64, attachment: bool| PartTree::Leaf {
            section: section.to_owned(),
            mime: mime.to_owned(),
            octets,
            attachment,
        };
        assert_eq!(
            found,
            [(
                imap_ref("INBOX", 7),
                PartTree::Multipart {
                    section: String::new(),
                    subtype: "mixed".to_owned(),
                    boundary: "mix-b".to_owned(),
                    parts: vec![
                        PartTree::Multipart {
                            section: "1".to_owned(),
                            subtype: "alternative".to_owned(),
                            boundary: "alt-b".to_owned(),
                            parts: vec![
                                leaf("1.1", "text/plain", 120, false),
                                leaf("1.2", "text/html", 900, false),
                            ],
                        },
                        leaf("2", "application/pdf", 9_437_184, true),
                    ],
                }
            )]
        );
    }

    /// Several literals in one response, each paired with the section it answers.
    #[test]
    fn sections_come_back_by_name() {
        let trace = format!(
            "S: * OK ready\n\
             {AUTH}\
             S: a001 OK authenticated\n\
             C: a002 EXAMINE \"INBOX\"\n\
             S: a002 OK [READ-ONLY] done\n\
             C: a003 UID FETCH 7 (UID BODY.PEEK[HEADER] BODY.PEEK[2.MIME] BODY.PEEK[1.1])\n\
             S: * 1 FETCH (UID 7 BODY[HEADER] {{12}}\n\
             S: Subject: x\n\
             S:  BODY[2.MIME] {{7}}\n\
             S: X: yy\n\
             S:  BODY[1.1] NIL)\n\
             S: a003 OK done\n\
             DONE\n"
        );
        let mut driven = Driven {
            backend: backend(caps(ServerLabels::LocalOnly, ArchiveMeans::LocalOnly)),
            op: Some(ProtoOp::FetchSections {
                remote: imap_ref("INBOX", 7),
                sections: vec!["HEADER".into(), "2.MIME".into(), "1.1".into()],
            }),
        };
        let ProtoOutcome::Sections { parts, .. } = replay(&mut driven, &trace).unwrap() else {
            panic!("expected sections");
        };
        assert_eq!(
            parts,
            [
                ("HEADER".to_owned(), b"Subject: x\r\n".to_vec()),
                ("2.MIME".to_owned(), b"X: yy\r\n".to_vec()),
                ("1.1".to_owned(), Vec::new()),
            ]
        );
    }

    /// A section name is checked before it reaches the command line, not escaped after.
    #[test]
    fn a_section_that_is_not_one_is_refused_before_anything_is_sent() {
        for bad in [
            "1] BODY[",
            "TEXT",
            "0",
            "1..2",
            "",
            "1.MIME.MIME",
            "2 FLAGS",
        ] {
            let mut b = backend(caps(ServerLabels::LocalOnly, ArchiveMeans::LocalOnly));
            let progress = b.begin(ProtoOp::FetchSections {
                remote: imap_ref("INBOX", 7),
                sections: vec![bad.to_owned()],
            });
            assert!(
                matches!(progress, Progress::Failed(ProtoError::Malformed(_))),
                "{bad:?} was not refused: {progress:?}"
            );
        }
    }
}
