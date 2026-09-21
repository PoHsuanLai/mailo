//! POP3 client session (RFC 1939), as a [`Machine`].
//!
//! The caller queues the commands. This type only sequences them and parses
//! replies: it does not decide whether to delete mail, which mechanism to use,
//! or which message to fetch.
//!
//! Dialogue state is [`Phase`]. [`step`] is the only function that changes it.
//! A response that has not yet ended in CRLF stays in [`Buffer`] and the machine
//! asks for [`IoNeed::Read`] again.
//!
//! Multi-line replies (`CAPA`, `UIDL`, `LIST`, `RETR`) end at a line whose
//! content is `.`. Any other body line that begins with `.` loses that one dot
//! (RFC 1939 §3). `CAPA` answered with `-ERR` is a reply, not a failure; every
//! other `-ERR` aborts the session.
//!
//! The password is copied onto the wire for `PASS` and `AUTH PLAIN` and nowhere
//! else. [`Debug`] prints `<redacted>`.

use crate::machine::{IoNeed, IoReady, Machine, Progress, ProtoError};
use std::collections::VecDeque;
use std::fmt;

/// A response line longer than this is [`ProtoError::Malformed`].
///
/// RFC 1939's 512-octet limit is routinely exceeded by real message bodies.
/// One mebibyte still stops a missing CRLF from growing the buffer forever.
const MAX_LINE_BYTES: usize = 1024 * 1024;

/// One command in a caller-chosen POP3 sequence.
///
/// `USER`, `PASS`, and `AUTH PLAIN` take the username and password given to
/// [`Pop3Session::new`]. The command value itself holds no secret, so its
/// [`Debug`] is derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pop3Command {
    /// `CAPA` (RFC 2449). `-ERR` is a normal answer on older servers.
    Capa,
    /// `USER` with the session username.
    User,
    /// `PASS` with the session password.
    Pass,
    /// `AUTH PLAIN` with an initial response (RFC 5034, RFC 4616).
    ///
    /// The response is base64 of `\0username\0password`. A SASL continuation
    /// (`+`) is neither `+OK` nor `-ERR`, so it is [`ProtoError::Malformed`].
    AuthPlain,
    /// `STAT`.
    Stat,
    /// `UIDL` for the whole maildrop.
    Uidl,
    /// `LIST` for the whole maildrop.
    List,
    /// `RETR` of one server message number.
    Retr(u32),
    /// `DELE` of one server message number.
    Dele(u32),
    /// `QUIT`. A `+OK` ends the session even if further commands were queued.
    Quit,
}

/// One parsed POP3 reply.
///
/// A finished [`Pop3Session`] yields these in order: the greeting, then one
/// entry per command that completed. `QUIT` is the last entry when it was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pop3Reply {
    /// Banner text after `+OK`.
    Greeting(String),
    /// Capability lines from a successful `CAPA`, dot-unstuffed, without the terminator.
    Capabilities(Vec<String>),
    /// `CAPA` was answered `-ERR`. The session continues.
    CapaUnsupported(String),
    /// `USER` was accepted. The session is not authenticated yet.
    UserAccepted(String),
    /// `PASS` or `AUTH PLAIN` was accepted.
    Authenticated(String),
    /// `STAT`: message count and maildrop size in octets.
    Stat { messages: u32, octets: u64 },
    /// `UIDL` rows. Empty when the maildrop is empty.
    Uidl(Vec<UidlEntry>),
    /// `LIST` rows. Empty when the maildrop is empty.
    List(Vec<ListEntry>),
    /// `RETR` payload. Dot-stuffing is removed and each line keeps its CRLF.
    /// The terminating `.` line is not included.
    Retrieved(Vec<u8>),
    /// `DELE` was accepted. Text is the server's tail.
    Deleted(String),
    /// `QUIT` was accepted. The session is over.
    Quit(String),
}

/// One `UIDL` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UidlEntry {
    /// Server message number.
    pub number: u32,
    /// Unique-id token, printable ASCII (`0x21..=0x7E`).
    pub uidl: String,
}

/// One `LIST` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    /// Server message number.
    pub number: u32,
    /// Message size in octets.
    pub octets: u64,
}

/// Username and password. `Debug` redacts the password.
#[derive(Clone, PartialEq, Eq)]
struct Auth {
    username: String,
    password: String,
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Auth")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Where the session is in the dialogue.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Greeting,
    /// Waiting for a one-line `+OK` or `-ERR`.
    Single(Pop3Command),
    /// Waiting for the status line of a dot-terminated reply.
    MultiStatus(Pop3Command),
    /// Status was `+OK`. `lines` are body lines with dot-stuffing already removed.
    MultiBody {
        cmd: Pop3Command,
        lines: Vec<Vec<u8>>,
    },
    /// `step` has already returned [`Progress::Done`] or [`Progress::Failed`].
    Closed,
}

/// How the next command is chosen once the response in flight is complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindDown {
    /// Issue the caller's queue.
    Run,
    /// [`IoReady::Interrupt`] arrived. The next command is `QUIT`.
    QuitNext,
}

/// Which shape of reply a command produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseShape {
    Single,
    Multi,
}

/// Bytes received so far and not yet consumed as CRLF-terminated lines.
#[derive(Clone, Debug, Default)]
struct Buffer {
    bytes: Vec<u8>,
    pos: usize,
}

impl PartialEq for Buffer {
    fn eq(&self, other: &Self) -> bool {
        self.bytes[self.pos..] == other.bytes[other.pos..]
    }
}

impl Eq for Buffer {}

impl Buffer {
    fn extend(&mut self, more: &[u8]) {
        self.bytes.extend_from_slice(more);
    }

    /// The next CRLF-terminated line, without the CRLF.
    ///
    /// `Ok(None)` means the line has not arrived yet. A line longer than
    /// [`MAX_LINE_BYTES`] is an error and is not copied into the message.
    fn take_line(&mut self) -> Result<Option<Vec<u8>>, ProtoError> {
        let end = {
            let rest = &self.bytes[self.pos..];
            match rest.windows(2).position(|pair| pair == b"\r\n") {
                Some(end) if end > MAX_LINE_BYTES => return Err(line_too_long()),
                Some(end) => end,
                None if rest.len() > MAX_LINE_BYTES => return Err(line_too_long()),
                None => return Ok(None),
            }
        };
        let start = self.pos;
        let line = self.bytes[start..start + end].to_vec();
        self.pos = start + end + 2;
        if self.pos == self.bytes.len() {
            self.bytes.clear();
            self.pos = 0;
        } else if self.pos > 8 * 1024 {
            self.bytes.drain(..self.pos);
            self.pos = 0;
        }
        Ok(Some(line))
    }
}

/// Memory that survives from one [`Machine::feed`] to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    commands: VecDeque<Pop3Command>,
    phase: Phase,
    buf: Buffer,
    replies: Vec<Pop3Reply>,
    wind_down: WindDown,
}

/// What [`step`] was just told.
enum Event {
    Start,
    Bytes(Vec<u8>),
    Eof,
    Interrupt,
    /// `TlsOpen` or `Woke`. The session never asks for either; keep reading.
    Continue,
}

/// What one complete line did to the phase.
#[derive(Debug)]
enum Action {
    /// Stay in this response and pull another line if the buffer has one.
    Continue,
    /// The response is finished. Queue the next command, or finish if none remain.
    Next(Pop3Reply),
    /// `QUIT` succeeded.
    Done(Pop3Reply),
    Fail(ProtoError),
}

/// A POP3 session that speaks only after the runtime has handed it a byte stream.
///
/// TLS is the runtime's job, before [`Machine::start`]. Implicit POP3S (port 995)
/// never appears here. `STLS` is not a command this session sends.
///
/// `Out` is [`Vec<Pop3Reply>`] rather than one reply: [`Progress::Done`] happens
/// once, at `QUIT` or when the queue runs out, and the caller needs every reply
/// the sequence produced (the `UIDL` table and the `RETR` body, not only the last
/// `+OK`).
///
/// [`Debug`] redacts the password. `IoNeed::Write` of `PASS` / `AUTH PLAIN`
/// contains it, because that is the protocol; nothing in this type formats those
/// bytes into an error or a panic.
#[derive(Clone, PartialEq, Eq)]
pub struct Pop3Session {
    auth: Auth,
    state: State,
}

impl fmt::Debug for Pop3Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pop3Session")
            .field("auth", &self.auth)
            .field("state", &self.state)
            .finish()
    }
}

impl Pop3Session {
    /// Queue `commands` to run after the greeting.
    ///
    /// `username` is the login name, already resolved (`Username::resolve`).
    /// Neither credential may contain CR, LF, or NUL: those bytes would split a
    /// command or a SASL PLAIN message. The rejection text does not echo them.
    pub fn new(
        username: impl Into<String>,
        password: impl Into<String>,
        commands: Vec<Pop3Command>,
    ) -> Result<Self, ProtoError> {
        let username = username.into();
        let password = password.into();
        if has_forbidden(&username) {
            return Err(ProtoError::Malformed(
                "username contains CR, LF, or NUL".into(),
            ));
        }
        if has_forbidden(&password) {
            return Err(ProtoError::Malformed(
                "password contains CR, LF, or NUL".into(),
            ));
        }
        Ok(Self {
            auth: Auth { username, password },
            state: State {
                commands: commands.into(),
                phase: Phase::Greeting,
                buf: Buffer::default(),
                replies: Vec::new(),
                wind_down: WindDown::Run,
            },
        })
    }
}

impl Machine for Pop3Session {
    type Out = Vec<Pop3Reply>;

    fn start(&mut self) -> Progress<Self::Out> {
        step(&mut self.state, &self.auth, Event::Start)
    }

    fn feed(&mut self, ready: IoReady) -> Progress<Self::Out> {
        let event = match ready {
            IoReady::Bytes(bytes) => Event::Bytes(bytes),
            IoReady::Eof => Event::Eof,
            IoReady::Interrupt => Event::Interrupt,
            IoReady::TlsOpen | IoReady::Woke => Event::Continue,
        };
        step(&mut self.state, &self.auth, event)
    }
}

/// Advance `state` by one external event.
fn step(state: &mut State, auth: &Auth, event: Event) -> Progress<Vec<Pop3Reply>> {
    match event {
        Event::Start => Progress::Need(vec![IoNeed::Read]),
        Event::Bytes(bytes) => {
            state.buf.extend(&bytes);
            pump(state, auth)
        }
        Event::Eof => on_eof(state, auth),
        Event::Interrupt => {
            // The response in flight must be read first. Sending QUIT into the
            // middle of a multi-line RETR would desynchronise the stream.
            state.wind_down = WindDown::QuitNext;
            pump(state, auth)
        }
        Event::Continue => pump(state, auth),
    }
}

fn on_eof(state: &mut State, auth: &Auth) -> Progress<Vec<Pop3Reply>> {
    match pump(state, auth) {
        Progress::Need(needs) if needs.iter().any(|need| matches!(need, IoNeed::Read)) => {
            state.phase = Phase::Closed;
            Progress::Failed(ProtoError::UnexpectedEof)
        }
        other => other,
    }
}

/// Pull every complete line already buffered. Stop when a line is missing,
/// a command must be written, or the session ends.
fn pump(state: &mut State, auth: &Auth) -> Progress<Vec<Pop3Reply>> {
    if matches!(state.phase, Phase::Closed) {
        return Progress::Failed(ProtoError::Malformed("session is already finished".into()));
    }
    loop {
        let line = match state.buf.take_line() {
            Ok(Some(line)) => line,
            Ok(None) => return Progress::Need(vec![IoNeed::Read]),
            Err(err) => {
                state.phase = Phase::Closed;
                return Progress::Failed(err);
            }
        };
        let phase = std::mem::replace(&mut state.phase, Phase::Closed);
        let (phase, action) = decide(phase, &line);
        state.phase = phase;
        match action {
            Action::Continue => {}
            Action::Next(reply) => {
                state.replies.push(reply);
                return dispatch_next(state, auth);
            }
            Action::Done(reply) => {
                state.replies.push(reply);
                state.phase = Phase::Closed;
                return Progress::Done(std::mem::take(&mut state.replies));
            }
            Action::Fail(err) => {
                state.phase = Phase::Closed;
                return Progress::Failed(err);
            }
        }
    }
}

fn dispatch_next(state: &mut State, auth: &Auth) -> Progress<Vec<Pop3Reply>> {
    if state.wind_down == WindDown::QuitNext {
        state.commands.clear();
        state.commands.push_back(Pop3Command::Quit);
        state.wind_down = WindDown::Run;
    }
    let Some(cmd) = state.commands.pop_front() else {
        state.phase = Phase::Closed;
        return Progress::Done(std::mem::take(&mut state.replies));
    };
    let wire = encode(&cmd, auth);
    state.phase = match shape(&cmd) {
        ResponseShape::Single => Phase::Single(cmd),
        ResponseShape::Multi => Phase::MultiStatus(cmd),
    };
    Progress::Need(vec![IoNeed::Write(wire), IoNeed::Read])
}

/// Pure transition: one complete line in, the next phase and action out.
fn decide(phase: Phase, line: &[u8]) -> (Phase, Action) {
    match phase {
        Phase::Greeting => decide_greeting(line),
        Phase::Single(cmd) => decide_single(cmd, line),
        Phase::MultiStatus(cmd) => decide_multi_status(cmd, line),
        Phase::MultiBody { cmd, lines } => decide_multi_body(cmd, lines, line),
        Phase::Closed => (
            Phase::Closed,
            Action::Fail(ProtoError::Malformed("session is already finished".into())),
        ),
    }
}

fn decide_greeting(line: &[u8]) -> (Phase, Action) {
    let action = match status_of(line) {
        Ok(Status::Ok(text)) => Action::Next(Pop3Reply::Greeting(text)),
        Ok(Status::Err(text)) => Action::Fail(ProtoError::Refused(bounded(&text))),
        Err(err) => Action::Fail(err),
    };
    (Phase::Closed, action)
}

fn decide_single(cmd: Pop3Command, line: &[u8]) -> (Phase, Action) {
    let action = match status_of(line) {
        Ok(Status::Ok(text)) => match interpret_single(&cmd, &text) {
            Ok(reply) if matches!(cmd, Pop3Command::Quit) => Action::Done(reply),
            Ok(reply) => Action::Next(reply),
            Err(err) => Action::Fail(err),
        },
        Ok(Status::Err(text)) => Action::Fail(refused(&cmd, text)),
        Err(err) => Action::Fail(err),
    };
    (Phase::Closed, action)
}

fn decide_multi_status(cmd: Pop3Command, line: &[u8]) -> (Phase, Action) {
    match status_of(line) {
        Ok(Status::Ok(_)) => (
            Phase::MultiBody {
                cmd,
                lines: Vec::new(),
            },
            Action::Continue,
        ),
        Ok(Status::Err(text)) if matches!(cmd, Pop3Command::Capa) => (
            Phase::Closed,
            Action::Next(Pop3Reply::CapaUnsupported(bounded(&text))),
        ),
        Ok(Status::Err(text)) => (Phase::Closed, Action::Fail(refused(&cmd, text))),
        Err(err) => (Phase::Closed, Action::Fail(err)),
    }
}

fn decide_multi_body(cmd: Pop3Command, mut lines: Vec<Vec<u8>>, line: &[u8]) -> (Phase, Action) {
    if line != b"." {
        lines.push(unstuff(line));
        return (Phase::MultiBody { cmd, lines }, Action::Continue);
    }
    let action = match interpret_multi(&cmd, &lines) {
        Ok(reply) => Action::Next(reply),
        Err(err) => Action::Fail(err),
    };
    (Phase::Closed, action)
}

fn shape(cmd: &Pop3Command) -> ResponseShape {
    match cmd {
        Pop3Command::Capa | Pop3Command::Uidl | Pop3Command::List | Pop3Command::Retr(_) => {
            ResponseShape::Multi
        }
        Pop3Command::User
        | Pop3Command::Pass
        | Pop3Command::AuthPlain
        | Pop3Command::Stat
        | Pop3Command::Dele(_)
        | Pop3Command::Quit => ResponseShape::Single,
    }
}

fn interpret_single(cmd: &Pop3Command, text: &str) -> Result<Pop3Reply, ProtoError> {
    match cmd {
        Pop3Command::Stat => interpret_stat(text),
        Pop3Command::User => Ok(Pop3Reply::UserAccepted(text.to_owned())),
        Pop3Command::Pass | Pop3Command::AuthPlain => Ok(Pop3Reply::Authenticated(text.to_owned())),
        Pop3Command::Dele(_) => Ok(Pop3Reply::Deleted(text.to_owned())),
        Pop3Command::Quit => Ok(Pop3Reply::Quit(text.to_owned())),
        Pop3Command::Capa | Pop3Command::Uidl | Pop3Command::List | Pop3Command::Retr(_) => {
            // invariant: those commands are Phase::MultiStatus, never Phase::Single
            panic!("multi-line command reached the single-line parser");
        }
    }
}

fn interpret_multi(cmd: &Pop3Command, lines: &[Vec<u8>]) -> Result<Pop3Reply, ProtoError> {
    match cmd {
        Pop3Command::Capa => {
            let mut caps = Vec::with_capacity(lines.len());
            for line in lines {
                caps.push(utf8(line)?);
            }
            Ok(Pop3Reply::Capabilities(caps))
        }
        Pop3Command::Uidl => lines
            .iter()
            .map(|line| uidl_entry(line))
            .collect::<Result<Vec<_>, _>>()
            .map(Pop3Reply::Uidl),
        Pop3Command::List => lines
            .iter()
            .map(|line| list_entry(line))
            .collect::<Result<Vec<_>, _>>()
            .map(Pop3Reply::List),
        Pop3Command::Retr(_) => Ok(Pop3Reply::Retrieved(join_lines(lines))),
        Pop3Command::User
        | Pop3Command::Pass
        | Pop3Command::AuthPlain
        | Pop3Command::Stat
        | Pop3Command::Dele(_)
        | Pop3Command::Quit => {
            // invariant: those commands are Phase::Single, never Phase::MultiBody
            panic!("single-line command reached the multi-line parser");
        }
    }
}

/// `STAT`'s first two fields are the count and the size. Servers append a tail
/// (`2 320 messages`); it is not part of the numbers.
fn interpret_stat(text: &str) -> Result<Pop3Reply, ProtoError> {
    let mut parts = text.split_whitespace();
    let messages = parse_u32(parts.next(), "STAT message count")?;
    let octets = parse_u64(parts.next(), "STAT octet count")?;
    Ok(Pop3Reply::Stat { messages, octets })
}

fn uidl_entry(line: &[u8]) -> Result<UidlEntry, ProtoError> {
    let (number, uidl) = two_tokens(line)?;
    if !uidl.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return Err(ProtoError::Malformed(format!(
            "UIDL is not printable ASCII: {}",
            snippet(line)
        )));
    }
    Ok(UidlEntry {
        number: parse_u32(Some(number), "UIDL message number")?,
        uidl: uidl.to_owned(),
    })
}

fn list_entry(line: &[u8]) -> Result<ListEntry, ProtoError> {
    let (number, octets) = two_tokens(line)?;
    Ok(ListEntry {
        number: parse_u32(Some(number), "LIST message number")?,
        octets: parse_u64(Some(octets), "LIST octet count")?,
    })
}

fn two_tokens(line: &[u8]) -> Result<(&str, &str), ProtoError> {
    let text = std::str::from_utf8(line).map_err(|_| {
        ProtoError::Malformed(format!("response line is not utf-8: {}", snippet(line)))
    })?;
    let mut parts = text.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (Some(first), Some(second), None) => Ok((first, second)),
        _ => Err(ProtoError::Malformed(format!(
            "expected two fields: {}",
            snippet(line)
        ))),
    }
}

fn join_lines(lines: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for line in lines {
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Drop one leading `.`. The caller has already treated a line that is exactly
/// `.` as the end of the multi-line reply, so this function does not see it.
fn unstuff(line: &[u8]) -> Vec<u8> {
    debug_assert!(
        line != b".",
        "the terminator is handled before unstuff" // invariant: decide_multi_body checks `line == b"."` first
    );
    line.strip_prefix(b".").unwrap_or(line).to_vec()
}

enum Status {
    Ok(String),
    Err(String),
}

fn status_of(line: &[u8]) -> Result<Status, ProtoError> {
    if let Some(rest) = line.strip_prefix(b"+OK") {
        return Ok(Status::Ok(status_tail(rest)?));
    }
    if let Some(rest) = line.strip_prefix(b"-ERR") {
        return Ok(Status::Err(status_tail(rest)?));
    }
    Err(ProtoError::Malformed(format!(
        "response is neither +OK nor -ERR: {}",
        snippet(line)
    )))
}

fn status_tail(rest: &[u8]) -> Result<String, ProtoError> {
    match rest {
        [] => Ok(String::new()),
        [b' ', text @ ..] => utf8(text),
        _ => Err(ProtoError::Malformed(format!(
            "status keyword is not followed by a space: {}",
            snippet(rest)
        ))),
    }
}

fn utf8(bytes: &[u8]) -> Result<String, ProtoError> {
    std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| {
        ProtoError::Malformed(format!("response line is not utf-8: {}", snippet(bytes)))
    })
}

fn refused(cmd: &Pop3Command, text: String) -> ProtoError {
    let text = bounded(&text);
    match cmd {
        Pop3Command::User | Pop3Command::Pass | Pop3Command::AuthPlain => {
            ProtoError::AuthRejected(text)
        }
        _ => ProtoError::Refused(text),
    }
}

fn encode(cmd: &Pop3Command, auth: &Auth) -> Vec<u8> {
    let mut out = Vec::new();
    match cmd {
        Pop3Command::Capa => out.extend_from_slice(b"CAPA"),
        Pop3Command::User => {
            out.extend_from_slice(b"USER ");
            out.extend_from_slice(auth.username.as_bytes());
        }
        Pop3Command::Pass => {
            out.extend_from_slice(b"PASS ");
            out.extend_from_slice(auth.password.as_bytes());
        }
        Pop3Command::AuthPlain => {
            out.extend_from_slice(b"AUTH PLAIN ");
            let initial = sasl_plain(&auth.username, &auth.password);
            out.extend_from_slice(initial.as_bytes());
        }
        Pop3Command::Stat => out.extend_from_slice(b"STAT"),
        Pop3Command::Uidl => out.extend_from_slice(b"UIDL"),
        Pop3Command::List => out.extend_from_slice(b"LIST"),
        Pop3Command::Retr(number) => {
            out.extend_from_slice(b"RETR ");
            out.extend_from_slice(number.to_string().as_bytes());
        }
        Pop3Command::Dele(number) => {
            out.extend_from_slice(b"DELE ");
            out.extend_from_slice(number.to_string().as_bytes());
        }
        Pop3Command::Quit => out.extend_from_slice(b"QUIT"),
    }
    out.extend_from_slice(b"\r\n");
    out
}

/// SASL PLAIN initial response: base64(`\0` username `\0` password).
/// The authorization identity is empty, so the server uses the username.
fn sasl_plain(username: &str, password: &str) -> String {
    let mut raw = Vec::with_capacity(username.len() + password.len() + 2);
    raw.push(0);
    raw.extend_from_slice(username.as_bytes());
    raw.push(0);
    raw.extend_from_slice(password.as_bytes());
    base64_encode(&raw)
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut index = 0;
    while index < data.len() {
        let b0 = u32::from(data[index]);
        let b1 = u32::from(data.get(index + 1).copied().unwrap_or(0));
        let b2 = u32::from(data.get(index + 2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(TABLE[((n >> 18) & 0x3f) as usize]));
        out.push(char::from(TABLE[((n >> 12) & 0x3f) as usize]));
        if index + 1 < data.len() {
            out.push(char::from(TABLE[((n >> 6) & 0x3f) as usize]));
        } else {
            out.push('=');
        }
        if index + 2 < data.len() {
            out.push(char::from(TABLE[(n & 0x3f) as usize]));
        } else {
            out.push('=');
        }
        index += 3;
    }
    out
}

fn parse_u32(token: Option<&str>, what: &str) -> Result<u32, ProtoError> {
    parse_int(token, what, str::parse)
}

fn parse_u64(token: Option<&str>, what: &str) -> Result<u64, ProtoError> {
    parse_int(token, what, str::parse)
}

fn parse_int<T>(
    token: Option<&str>,
    what: &str,
    parse: fn(&str) -> Result<T, std::num::ParseIntError>,
) -> Result<T, ProtoError> {
    let Some(token) = token else {
        return Err(ProtoError::Malformed(format!("missing {what}")));
    };
    parse(token)
        .map_err(|_| ProtoError::Malformed(format!("bad {what}: {}", snippet(token.as_bytes()))))
}

fn has_forbidden(value: &str) -> bool {
    value.bytes().any(|byte| matches!(byte, b'\n' | b'\r' | 0))
}

fn line_too_long() -> ProtoError {
    ProtoError::Malformed("response line exceeds 1 MiB".into())
}

fn snippet(bytes: &[u8]) -> String {
    clip(&String::from_utf8_lossy(bytes), 80)
}

fn bounded(text: &str) -> String {
    clip(text, 200)
}

fn clip(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        Action, Buffer, MAX_LINE_BYTES, Phase, Pop3Command, Pop3Reply, Pop3Session, base64_encode,
        decide, interpret_stat, sasl_plain, unstuff,
    };
    use crate::machine::{IoNeed, IoReady, Machine, Progress, ProtoError};

    #[test]
    fn base64_matches_known_vectors() {
        let cases: &[(&[u8], &str)] = &[
            (b"", ""),
            (b"M", "TQ=="),
            (b"Ma", "TWE="),
            (b"Man", "TWFu"),
            (b"hello", "aGVsbG8="),
            (b"\0user\0pass", "AHVzZXIAcGFzcw=="),
        ];
        for (input, want) in cases {
            assert_eq!(base64_encode(input), *want);
        }
        assert_eq!(sasl_plain("user", "pass"), "AHVzZXIAcGFzcw==");
        assert_eq!(sasl_plain("student", "s3cret"), "AHN0dWRlbnQAczNjcmV0");
    }

    #[test]
    fn unstuff_strips_exactly_one_leading_dot() {
        // A line that is exactly `.` is the terminator and is not passed to unstuff.
        let cases: &[(&[u8], &[u8])] = &[
            (b"hello", b"hello"),
            (b"", b""),
            (b".hello", b"hello"),
            (b"..hello", b".hello"),
            (b"..", b"."),
        ];
        for (input, want) in cases {
            assert_eq!(
                unstuff(input).as_slice(),
                *want,
                "{}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn a_crlf_split_across_chunks_is_one_line() {
        let mut buf = Buffer::default();
        buf.extend(b"+OK hel");
        assert_eq!(buf.take_line().unwrap(), None);
        buf.extend(b"lo\r");
        assert_eq!(buf.take_line().unwrap(), None);
        buf.extend(b"\nNEXT");
        assert_eq!(buf.take_line().unwrap().unwrap(), b"+OK hello");
        assert_eq!(buf.take_line().unwrap(), None);
        buf.extend(b"\r\n");
        assert_eq!(buf.take_line().unwrap().unwrap(), b"NEXT");
        assert_eq!(buf.take_line().unwrap(), None);
    }

    #[test]
    fn lines_longer_than_the_cap_are_malformed_and_not_echoed() {
        let mut bare = Buffer::default();
        bare.extend(&vec![b'A'; MAX_LINE_BYTES + 1]);
        let err = bare.take_line().unwrap_err();
        assert_eq!(
            err.to_string(),
            "malformed response: response line exceeds 1 MiB"
        );

        let mut terminated = Buffer::default();
        let mut bytes = vec![b'B'; MAX_LINE_BYTES + 1];
        bytes.extend_from_slice(b"\r\n");
        terminated.extend(&bytes);
        let err = terminated.take_line().unwrap_err();
        assert_eq!(
            err.to_string(),
            "malformed response: response line exceeds 1 MiB"
        );

        let mut exact = Buffer::default();
        let mut ok = vec![b'C'; MAX_LINE_BYTES];
        ok.extend_from_slice(b"\r\n");
        exact.extend(&ok);
        let line = exact.take_line().unwrap().unwrap();
        assert_eq!(line.len(), MAX_LINE_BYTES);
    }

    #[test]
    fn stat_reads_the_first_two_numbers() {
        assert_eq!(
            interpret_stat("2 320 messages").unwrap(),
            Pop3Reply::Stat {
                messages: 2,
                octets: 320
            }
        );
        assert!(interpret_stat("2").is_err());
        assert!(interpret_stat("many 1").is_err());
        assert!(interpret_stat("").is_err());
    }

    #[test]
    fn status_lines_are_strict() {
        let (_, ok) = decide(Phase::Greeting, b"+OK POP3 server ready");
        assert!(
            matches!(ok, Action::Next(Pop3Reply::Greeting(text)) if text == "POP3 server ready")
        );

        let (_, empty) = decide(Phase::Greeting, b"+OK");
        assert!(matches!(empty, Action::Next(Pop3Reply::Greeting(text)) if text.is_empty()));

        let (_, glued) = decide(Phase::Greeting, b"+OKglued");
        assert!(matches!(glued, Action::Fail(ProtoError::Malformed(_))));

        let (_, garbage) = decide(Phase::Greeting, b"NOPE");
        assert!(matches!(garbage, Action::Fail(ProtoError::Malformed(_))));

        let (_, down) = decide(Phase::Greeting, b"-ERR shutting down");
        assert!(matches!(down, Action::Fail(ProtoError::Refused(_))));
    }

    #[test]
    fn refusals_follow_the_command_and_capa_does_not_fail() {
        let cases = [
            (Pop3Command::User, "AuthRejected"),
            (Pop3Command::Pass, "AuthRejected"),
            (Pop3Command::AuthPlain, "AuthRejected"),
            (Pop3Command::Stat, "Refused"),
            (Pop3Command::Dele(1), "Refused"),
            (Pop3Command::Quit, "Refused"),
            (Pop3Command::Retr(1), "Refused"),
            (Pop3Command::Uidl, "Refused"),
            (Pop3Command::List, "Refused"),
        ];
        for (cmd, want) in cases {
            let phase = match super::shape(&cmd) {
                super::ResponseShape::Multi => Phase::MultiStatus(cmd.clone()),
                super::ResponseShape::Single => Phase::Single(cmd.clone()),
            };
            let (_, action) = decide(phase, b"-ERR no");
            let got = match &action {
                Action::Fail(ProtoError::AuthRejected(text)) => {
                    assert_eq!(text, "no");
                    "AuthRejected"
                }
                Action::Fail(ProtoError::Refused(text)) => {
                    assert_eq!(text, "no");
                    "Refused"
                }
                other => panic!("{cmd:?} produced {other:?}"),
            };
            assert_eq!(got, want, "{cmd:?}");
        }

        let (_, action) = decide(
            Phase::MultiStatus(Pop3Command::Capa),
            b"-ERR unknown command",
        );
        match action {
            Action::Next(Pop3Reply::CapaUnsupported(text)) => {
                assert_eq!(text, "unknown command");
            }
            other => panic!("CAPA -ERR must continue, got {other:?}"),
        }
    }

    #[test]
    fn a_dot_stuffed_body_line_is_unstuffed_and_the_bare_dot_ends_it() {
        let phase = Phase::MultiBody {
            cmd: Pop3Command::Retr(1),
            lines: Vec::new(),
        };
        let (phase, action) = decide(phase, b"Hello");
        assert!(matches!(action, Action::Continue));
        let (phase, action) = decide(phase, b"..dot");
        assert!(matches!(action, Action::Continue));
        match &phase {
            Phase::MultiBody { lines, .. } => {
                assert_eq!(lines, &[b"Hello".to_vec(), b".dot".to_vec()]);
            }
            other => panic!("expected a body, got {other:?}"),
        }
        let (_, action) = decide(phase, b".");
        match action {
            Action::Next(Pop3Reply::Retrieved(body)) => {
                assert_eq!(body, b"Hello\r\n.dot\r\n");
            }
            other => panic!("expected the message, got {other:?}"),
        }
    }

    #[test]
    fn credentials_cannot_break_a_command_line() {
        let cases = ["has\rcr", "has\nlf", "has\0nul"];
        for bad in cases {
            let user = Pop3Session::new(bad, "secret", vec![]).unwrap_err();
            assert_eq!(
                user.to_string(),
                "malformed response: username contains CR, LF, or NUL"
            );
            let pass = Pop3Session::new("student", bad, vec![]).unwrap_err();
            assert_eq!(
                pass.to_string(),
                "malformed response: password contains CR, LF, or NUL"
            );
        }
    }

    #[test]
    fn debug_redacts_the_password_even_after_pass_is_written() {
        let mut session = Pop3Session::new(
            "student",
            "s3cret-hunter2",
            vec![Pop3Command::Pass, Pop3Command::Quit],
        )
        .expect("credentials");
        assert!(matches!(session.start(), Progress::Need(_)));
        let progress = session.feed(IoReady::Bytes(b"+OK ready\r\n".to_vec()));
        let Progress::Need(needs) = progress else {
            panic!("greeting should be followed by PASS");
        };
        let carries = needs.iter().any(|need| match need {
            IoNeed::Write(bytes) => {
                bytes.starts_with(b"PASS ")
                    && bytes
                        .windows(b"s3cret-hunter2".len())
                        .any(|window| window == b"s3cret-hunter2")
            }
            _ => false,
        });
        assert!(carries, "PASS line did not contain the configured password");
        let shown = format!("{session:?}");
        assert!(
            !shown.contains("s3cret-hunter2"),
            "Debug contained the password"
        );
        assert!(
            shown.contains("redacted"),
            "Debug did not mark the password"
        );
    }
}
