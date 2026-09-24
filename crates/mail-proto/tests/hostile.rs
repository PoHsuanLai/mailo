//! What the sessions do when the server is not the one we wrote.
//!
//! Every other test in this crate replays a transcript someone chose, which answers "does this
//! work against a server behaving as expected". It cannot answer the question a real deployment
//! asks immediately: what happens when the bytes are wrong. A server can be malicious, or merely
//! old, or a middlebox, or a TLS error page delivered on port 143 — and a sans-I/O machine is
//! exactly where that arrives first.
//!
//! The properties asserted here are the two that make a parser safe to point at the internet:
//!
//! 1. **No input panics.** A panic in a mail client is a crash on receiving mail, which is a
//!    denial of service that the sender chooses the timing of.
//! 2. **Every input terminates.** A machine that neither finishes nor asks for more bytes has
//!    hung the connection, and a hang is harder to diagnose than a crash because nothing is
//!    reported at all.
//!
//! Neither says the parse is *correct* — correctness is what the transcript tests are for. These
//! say the failure mode is a clean error rather than a crashed or wedged client.

use mail_domain::{Credential, SaslMech};
use mail_proto::machine::{IoReady, Machine, Progress};
use mail_proto::{ImapAuth, ImapCommand, ImapSession, Pop3Command, Pop3Session};
use proptest::prelude::*;

/// Feed `chunks` to `machine` and report whether it settled.
///
/// Bounded: a machine that keeps asking for bytes after every chunk has been delivered is given
/// a fixed number of empty reads to finish, and anything still undecided after that is a hang.
fn drive_to_rest<M: Machine>(machine: &mut M, chunks: Vec<Vec<u8>>) -> bool {
    match machine.start() {
        Progress::Done(_) | Progress::Failed(_) => return true,
        Progress::Need(_) => {}
    }
    for chunk in chunks {
        match machine.feed(IoReady::Bytes(chunk)) {
            Progress::Done(_) | Progress::Failed(_) => return true,
            Progress::Need(_) => {}
        }
    }
    // The peer hangs up. Everything must resolve now: there are no more bytes coming, ever.
    matches!(
        machine.feed(IoReady::Eof),
        Progress::Done(_) | Progress::Failed(_)
    )
}

fn imap_session(commands: Vec<ImapCommand>) -> ImapSession {
    ImapSession::new(
        ImapAuth {
            username: "ada@example.test".to_owned(),
            credential: Credential::Password("s3cr3t".to_owned()),
            sasl: vec![SaslMech::Plain],
        },
        commands,
    )
    .expect("a clean credential")
}

/// Byte sequences that look enough like protocol to reach the interesting paths.
///
/// Uniformly random bytes are rejected by the first byte of almost every parser, which tests the
/// rejection and nothing after it. Mixing in real protocol fragments gets past the front door,
/// which is where the parsers actually have state to corrupt.
fn wire_chunk() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..64),
        proptest::sample::select(vec![
            b"* OK ready\r\n".to_vec(),
            b"a001 OK done\r\n".to_vec(),
            b"* 1 FETCH (UID 1 BODY[] {5}\r\nabcde)\r\n".to_vec(),
            b"* 1 FETCH (UID 1 BODY[] {999999}\r\n".to_vec(),
            b"+ continue\r\n".to_vec(),
            b"* SEARCH 1 2 3\r\n".to_vec(),
            b"* CAPABILITY IMAP4rev1\r\n".to_vec(),
            b"a001 NO no\r\n".to_vec(),
            b"a001 BAD bad\r\n".to_vec(),
            b"+OK pop ready\r\n".to_vec(),
            b"-ERR nope\r\n".to_vec(),
            b"220 smtp ready\r\n".to_vec(),
            b"250-one\r\n250 two\r\n".to_vec(),
            b"354 go ahead\r\n".to_vec(),
            b"\r\n".to_vec(),
            b".\r\n".to_vec(),
            b"{".to_vec(),
            b"{-1}\r\n".to_vec(),
            vec![0u8; 8],
        ]),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    /// The IMAP session, which has the largest parse surface and the only literals.
    #[test]
    fn imap_never_panics_and_always_settles(
        chunks in proptest::collection::vec(wire_chunk(), 0..12)
    ) {
        let mut session = imap_session(vec![
            ImapCommand::Capability,
            ImapCommand::Select { mailbox: "INBOX".to_owned(), read_only: true, qresync: None },
            ImapCommand::UidFetch { set: "1:*".to_owned(), items: "(UID)".to_owned() },
        ]);
        prop_assert!(drive_to_rest(&mut session, chunks), "the session neither finished nor failed");
    }

    /// The same, for a session parked in IDLE — where a stray continuation used to be the only
    /// way in, and where the news check now also runs.
    #[test]
    fn imap_idle_never_panics_and_always_settles(
        chunks in proptest::collection::vec(wire_chunk(), 0..12)
    ) {
        let mut session = imap_session(vec![ImapCommand::Idle]);
        prop_assert!(drive_to_rest(&mut session, chunks));
    }

    /// An APPEND is the one command that sends a literal, so its phase can receive a `+`, a
    /// tagged refusal, or garbage in place of either.
    #[test]
    fn imap_append_never_panics_and_always_settles(
        chunks in proptest::collection::vec(wire_chunk(), 0..12),
        body in proptest::collection::vec(any::<u8>(), 0..128),
    ) {
        let mut session = imap_session(vec![ImapCommand::Append {
            mailbox: "Drafts".to_owned(),
            flags: vec!["\\Draft".to_owned()],
            date: None,
            raw: body,
        }]);
        prop_assert!(drive_to_rest(&mut session, chunks));
    }

    #[test]
    fn pop3_never_panics_and_always_settles(
        chunks in proptest::collection::vec(wire_chunk(), 0..12)
    ) {
        let mut session = Pop3Session::new(
            "ada",
            "s3cr3t",
            vec![Pop3Command::Capa, Pop3Command::User, Pop3Command::Pass, Pop3Command::Uidl],
        )
        .expect("a clean credential");
        prop_assert!(drive_to_rest(&mut session, chunks));
    }

    /// Modified UTF-7 decoding is total by contract: every mailbox name a server can send has
    /// *some* reading, because refusing one means a folder the user cannot open.
    #[test]
    fn mutf7_decoding_is_total(raw in ".*") {
        let _ = mail_proto::mutf7::decode(&raw);
    }

    /// And encoding survives its own output.
    #[test]
    fn mutf7_round_trips_any_name(name in ".*") {
        let encoded = mail_proto::mutf7::encode(&name);
        prop_assert_eq!(mail_proto::mutf7::decode(&encoded), name);
    }
}

/// A literal announcing more bytes than will ever arrive must not wedge the session.
///
/// `{999999}` followed by a hang-up is what a truncated response looks like, and a parser that
/// waits for the promised bytes waits for ever.
#[test]
fn an_overlong_literal_that_never_arrives_ends_rather_than_hangs() {
    let mut session = imap_session(vec![ImapCommand::UidFetch {
        set: "1".to_owned(),
        items: "(UID BODY.PEEK[])".to_owned(),
    }]);
    let settled = drive_to_rest(
        &mut session,
        vec![
            b"* OK ready\r\n".to_vec(),
            b"* 1 FETCH (UID 1 BODY[] {999999}\r\nonly this much".to_vec(),
        ],
    );
    assert!(settled, "a truncated literal left the session waiting");
}
