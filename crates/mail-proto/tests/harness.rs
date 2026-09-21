//! Tests for the replay harness itself.
//!
//! Four session implementations will trust this to tell them whether they are correct. An
//! unverified harness that silently passes everything is worse than none, because it produces
//! four green suites that prove nothing.

mod common;

use common::replay;
use mail_proto::{IoNeed, IoReady, Machine, Progress, ProtoError, Refusal};

/// A deliberately simple protocol: greet, send one command, read one reply, finish.
#[derive(Default)]
struct Echo {
    step: u8,
    seen: Vec<u8>,
}

impl Machine for Echo {
    type Out = String;

    fn start(&mut self) -> Progress<String> {
        Progress::Need(vec![IoNeed::Read])
    }

    fn feed(&mut self, ready: IoReady) -> Progress<String> {
        match ready {
            IoReady::Interrupt => Progress::Need(vec![IoNeed::Write(b"DONE\r\n".to_vec())]),
            IoReady::Eof => Progress::Failed(ProtoError::UnexpectedEof),
            IoReady::TlsOpen | IoReady::Woke => Progress::Need(vec![IoNeed::Read]),
            IoReady::Bytes(bytes) => {
                self.seen.extend_from_slice(&bytes);
                // A complete line is a complete response; anything less needs another read.
                if !self.seen.ends_with(b"\r\n") {
                    return Progress::Need(vec![IoNeed::Read]);
                }
                let line = String::from_utf8_lossy(&self.seen).trim_end().to_owned();
                self.seen.clear();
                self.step += 1;
                match self.step {
                    1 => Progress::Need(vec![IoNeed::Write(b"HELLO\r\n".to_vec()), IoNeed::Read]),
                    _ if line.starts_with("ERR") => Progress::Failed(ProtoError::Refused {
                        kind: Refusal::Permanent,
                        text: line,
                    }),
                    _ => Progress::Done(line),
                }
            }
        }
    }
}

const HAPPY: &str = concat!(
    "# greeting, one command, one reply\n",
    "S: +OK server ready\n",
    "C: HELLO\n",
    "S: +OK done here\n",
    "DONE\n"
);

#[test]
fn a_passing_trace_returns_the_machines_output() {
    assert_eq!(
        replay(&mut Echo::default(), HAPPY).unwrap(),
        "+OK done here"
    );
}

#[test]
fn split_delivery_is_accepted_when_the_machine_reassembles() {
    let trace = concat!(
        "S: +OK server ready\n",
        "C: HELLO\n",
        "SPLIT\n",
        "S: +OK arriving in pieces\n",
        "DONE\n"
    );
    assert_eq!(
        replay(&mut Echo::default(), trace).unwrap(),
        "+OK arriving in pieces"
    );
}

#[test]
fn fail_matches_the_error_variant() {
    let trace = concat!(
        "S: +OK server ready\n",
        "C: HELLO\n",
        "S: ERR no\n",
        "FAIL Refused\n"
    );
    assert!(matches!(
        replay(&mut Echo::default(), trace).unwrap_err(),
        ProtoError::Refused { .. }
    ));
}

#[test]
fn end_of_stream_reaches_the_machine() {
    let trace = concat!(
        "S: +OK server ready\n",
        "C: HELLO\n",
        "EOF\n",
        "FAIL UnexpectedEof\n"
    );
    replay(&mut Echo::default(), trace).unwrap_err();
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let trace = concat!(
        "\n# a comment\n\n",
        "S: +OK server ready\n\n",
        "C: HELLO\n",
        "# another\n",
        "S: +OK done here\n",
        "DONE\n"
    );
    assert_eq!(
        replay(&mut Echo::default(), trace).unwrap(),
        "+OK done here"
    );
}

// --- the harness must REJECT these, or it proves nothing ------------------------------

fn must_reject(trace: &'static str, because: &str) {
    let result = std::panic::catch_unwind(|| replay(&mut Echo::default(), trace));
    assert!(
        result.is_err(),
        "the harness should have rejected this: {because}"
    );
}

#[test]
fn a_wrong_expected_write_is_rejected() {
    must_reject(
        concat!(
            "S: +OK server ready\n",
            "C: GOODBYE\n",
            "S: +OK done here\n",
            "DONE\n"
        ),
        "the machine writes HELLO, not GOODBYE",
    );
}

#[test]
fn a_trace_that_ends_early_is_rejected() {
    must_reject(
        concat!("S: +OK server ready\n", "C: HELLO\n", "DONE\n"),
        "the machine still wants to read",
    );
}

#[test]
fn a_trace_that_runs_past_the_end_is_rejected() {
    must_reject(
        concat!(
            "S: +OK server ready\n",
            "C: HELLO\n",
            "S: +OK done here\n",
            "S: extra\n",
            "DONE\n"
        ),
        "the machine finished before the trace did",
    );
}

#[test]
fn expecting_success_from_a_failing_machine_is_rejected() {
    must_reject(
        concat!(
            "S: +OK server ready\n",
            "C: HELLO\n",
            "S: ERR no\n",
            "DONE\n"
        ),
        "the machine failed but the trace said DONE",
    );
}

#[test]
fn the_wrong_failure_variant_is_rejected() {
    must_reject(
        concat!(
            "S: +OK server ready\n",
            "C: HELLO\n",
            "S: ERR no\n",
            "FAIL AuthRejected\n"
        ),
        "it failed with Refused, not AuthRejected",
    );
}

#[test]
fn a_trace_without_a_terminator_is_rejected() {
    must_reject(
        concat!("S: +OK server ready\n", "C: HELLO\n"),
        "no DONE or FAIL line",
    );
}

#[test]
fn an_unknown_directive_is_rejected() {
    must_reject(
        concat!("S: +OK\n", "WIGGLE\n", "DONE\n"),
        "WIGGLE is not a directive",
    );
}
