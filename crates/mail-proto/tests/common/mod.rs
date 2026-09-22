//! The trace replay harness. See `../traces/FORMAT.md` for the grammar.
//!
//! Shared deliberately: four sessions written by four people must not invent four harnesses,
//! or a trace stops being a portable description of what a server did and becomes an input to
//! one particular test file.
//!
//! The harness is the only thing that knows how to drive a [`Machine`]. A test supplies a
//! machine and a trace and asserts on what comes back; it never writes a pump loop.

#![allow(dead_code)] // each test file uses a different subset

use mail_proto::{IoNeed, IoReady, Machine, Progress, ProtoError};

/// How a replay ended.
#[derive(Debug)]
pub enum Ended<T> {
    Done(T),
    Failed(ProtoError),
}

impl<T> Ended<T> {
    /// The value, or a panic naming the failure. For traces that must succeed.
    pub fn unwrap(self) -> T {
        match self {
            Ended::Done(value) => value,
            Ended::Failed(e) => panic!("trace expected to succeed, but the machine failed: {e}"),
        }
    }

    /// The failure, or a panic. For traces that must fail.
    pub fn unwrap_err(self) -> ProtoError {
        match self {
            Ended::Failed(e) => e,
            Ended::Done(_) => panic!("trace expected the machine to fail, but it completed"),
        }
    }
}

/// One directive from a trace file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// Feed these bytes.
    Serve(Vec<u8>),
    /// Feed these bytes one at a time, asserting the machine asks to read between each.
    ServeSplit(Vec<u8>),
    /// Assert the next write is exactly these bytes.
    Expect(Vec<u8>),
    Eof,
    Woke,
    Tls,
    Interrupt,
    Flush,
    Close,
    Sleep(u64),
    Done,
    Fail(String),
}

/// Where the machine is right now.
enum State<T> {
    Running(Vec<IoNeed>),
    Finished(Ended<T>),
}

/// Drive `machine` through `trace`, asserting every write and need along the way.
///
/// Panics with the offending line when the machine and the trace disagree: a protocol test that
/// fails should say which line of which transcript it stopped at, not merely that a boolean was
/// false.
pub fn replay<M: Machine>(machine: &mut M, trace: &str) -> Ended<M::Out> {
    let mut state = State::Running(Vec::new());
    let mut started = false;

    for (line_no, step) in parse(trace) {
        let at = format!("line {line_no}");

        // The machine does nothing until asked, so `start` is deferred to the first step that
        // needs it rather than run eagerly — a trace beginning with a server greeting is the
        // common case and must not require a bare `start` step to say so.
        if !started {
            started = true;
            state = advance(machine.start(), &at);
        }

        match step {
            Step::Done => return finish(state, &at, None),
            Step::Fail(variant) => return finish(state, &at, Some(variant)),
            _ => {}
        }

        let needs = match &mut state {
            State::Running(needs) => needs,
            State::Finished(_) => panic!("{at}: the machine has finished, but the trace continues"),
        };

        match step {
            Step::Expect(want) => match next_need(needs, &at) {
                IoNeed::Write(got) => assert_eq!(
                    show(&got),
                    show(&want),
                    "{at}: the machine wrote something else"
                ),
                other => panic!("{at}: expected a write of {:?}, got {other:?}", show(&want)),
            },
            Step::Flush | Step::Close | Step::Sleep(_) => {
                let need = next_need(needs, &at);
                let ok = matches!(
                    (&step, &need),
                    (Step::Flush, IoNeed::Flush)
                        | (Step::Close, IoNeed::Close)
                        | (Step::Sleep(_), IoNeed::Sleep(_))
                );
                assert!(ok, "{at}: expected {step:?}, machine needs {need:?}");
            }
            Step::Tls => {
                let need = next_need(needs, &at);
                assert!(
                    matches!(need, IoNeed::OpenTls { .. }),
                    "{at}: expected OpenTls, got {need:?}"
                );
                state = advance(machine.feed(IoReady::TlsOpen), &at);
            }
            Step::Serve(bytes) => {
                accept_bytes(needs, &at);
                state = advance(machine.feed(IoReady::Bytes(bytes)), &at);
            }
            Step::ServeSplit(bytes) => {
                accept_bytes(needs, &at);
                // Real sockets deliver a FETCH response across several reads. A machine that
                // only ever sees whole responses in tests deadlocks the first time one does not
                // arrive whole.
                let last = bytes.len().saturating_sub(1);
                for (i, byte) in bytes.iter().enumerate() {
                    state = advance(machine.feed(IoReady::Bytes(vec![*byte])), &at);
                    if i == last {
                        break;
                    }
                    match &mut state {
                        State::Running(needs) => {
                            assert!(
                                needs.iter().any(|n| matches!(n, IoNeed::Read)),
                                "{at}: after byte {i} of a SPLIT the machine must ask to read \
                                 again, not {needs:?}"
                            );
                            needs.retain(|n| !matches!(n, IoNeed::Read));
                        }
                        State::Finished(_) => panic!(
                            "{at}: the machine finished after byte {i} of {}, mid-response",
                            bytes.len()
                        ),
                    }
                }
            }
            Step::Eof => state = advance(machine.feed(IoReady::Eof), &at),
            Step::Woke => state = advance(machine.feed(IoReady::Woke), &at),
            Step::Interrupt => state = advance(machine.feed(IoReady::Interrupt), &at),
            Step::Done | Step::Fail(_) => unreachable!("handled above"),
        }
    }
    panic!("trace ended without DONE or FAIL");
}

fn advance<T>(progress: Progress<T>, at: &str) -> State<T> {
    match progress {
        Progress::Need(needs) => State::Running(needs),
        Progress::Done(value) => State::Finished(Ended::Done(value)),
        Progress::Failed(e) => {
            let _ = at;
            State::Finished(Ended::Failed(e))
        }
    }
}

fn finish<T>(state: State<T>, at: &str, want_variant: Option<String>) -> Ended<T> {
    match (state, want_variant) {
        (State::Running(needs), _) => {
            panic!("{at}: the trace ends here, but the machine still wants {needs:?}")
        }
        (State::Finished(Ended::Done(v)), None) => Ended::Done(v),
        (State::Finished(Ended::Done(_)), Some(v)) => {
            panic!("{at}: expected the machine to fail with {v}, but it completed")
        }
        (State::Finished(Ended::Failed(e)), None) => {
            panic!("{at}: expected the machine to complete, but it failed: {e}")
        }
        (State::Finished(Ended::Failed(e)), Some(want)) => {
            // A bare `FAIL` says only that the session must not survive this, leaving which
            // error it is to the test. That is what a test *about* the classification needs:
            // naming the variant in the trace would make the transcript assert the answer.
            if !want.is_empty() {
                let got = variant_of(&e);
                assert_eq!(got, want, "{at}: wrong failure variant ({e})");
            }
            Ended::Failed(e)
        }
    }
}

/// The variant name of a `ProtoError`, for `FAIL <variant>`. `FAIL` alone accepts any failure.
fn variant_of(e: &ProtoError) -> &'static str {
    match e {
        ProtoError::Malformed(_) => "Malformed",
        ProtoError::Refused { .. } => "Refused",
        ProtoError::AuthRejected(_) => "AuthRejected",
        ProtoError::UnexpectedEof => "UnexpectedEof",
        ProtoError::Unsupported(_) => "Unsupported",
        ProtoError::Throttled { .. } => "Throttled",
    }
}

fn next_need(needs: &mut Vec<IoNeed>, at: &str) -> IoNeed {
    if needs.is_empty() {
        panic!("{at}: the trace expects a need here, but the machine wants nothing");
    }
    needs.remove(0)
}

/// A pending `Read` is the machine saying "give me bytes", which is what a serve step does.
fn accept_bytes(needs: &mut Vec<IoNeed>, at: &str) {
    needs.retain(|n| !matches!(n, IoNeed::Read));
    if let Some(unexpected) = needs.first() {
        panic!("{at}: the machine wants {unexpected:?} before it will accept bytes");
    }
}

/// Bytes as a readable string, so a mismatch shows the protocol rather than a byte array.
fn show(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

fn parse(trace: &str) -> Vec<(usize, Step)> {
    let mut out = Vec::new();
    let mut split_next = false;
    for (i, raw) in trace.lines().enumerate() {
        let line_no = i + 1;
        let line = raw.trim_end();
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        // `S:`/`C:` carry their payload after a colon, and the payload may itself contain
        // colons, so only the FIRST one separates. Directives without a colon (`FAIL Refused`,
        // `EXPECT sleep 5`) separate on whitespace instead.
        let trimmed = line.trim_start();
        let (directive, rest) = match trimmed.split_once(':') {
            Some((d, r)) if !d.contains(char::is_whitespace) => {
                (d.trim(), r.strip_prefix(' ').unwrap_or(r))
            }
            _ => match trimmed.split_once(char::is_whitespace) {
                Some((d, r)) => (d.trim(), r.trim_start()),
                None => (trimmed.trim(), ""),
            },
        };
        let step = match directive {
            "S" => {
                let bytes = crlf(rest);
                if std::mem::take(&mut split_next) {
                    Step::ServeSplit(bytes)
                } else {
                    Step::Serve(bytes)
                }
            }
            "C" => Step::Expect(crlf(rest)),
            "S64" => {
                let bytes = b64(rest, line_no);
                if std::mem::take(&mut split_next) {
                    Step::ServeSplit(bytes)
                } else {
                    Step::Serve(bytes)
                }
            }
            "C64" => Step::Expect(b64(rest, line_no)),
            "SPLIT" => {
                split_next = true;
                continue;
            }
            "EOF" => Step::Eof,
            "WOKE" => Step::Woke,
            "TLS" => Step::Tls,
            "INTERRUPT" => Step::Interrupt,
            "EXPECT" => match rest.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["flush"] => Step::Flush,
                ["close"] => Step::Close,
                ["sleep", secs] => Step::Sleep(secs.parse().unwrap_or_else(|_| {
                    panic!("line {line_no}: EXPECT sleep needs a number of seconds")
                })),
                other => panic!("line {line_no}: unknown EXPECT {other:?}"),
            },
            "DONE" => Step::Done,
            "FAIL" => Step::Fail(rest.trim().to_owned()),
            other => panic!("line {line_no}: unknown directive {other:?}"),
        };
        if matches!(step, Step::Done | Step::Fail(_)) {
            out.push((line_no, step));
            break;
        }
        out.push((line_no, step));
    }
    out
}

/// Every text line carries an implicit CRLF: writing `\r\n` in a trace would be noise on every
/// line and an invitation to get one wrong.
fn crlf(text: &str) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.extend_from_slice(b"\r\n");
    bytes
}

fn b64(text: &str, line_no: usize) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .unwrap_or_else(|e| panic!("line {line_no}: bad base64: {e}"))
}
