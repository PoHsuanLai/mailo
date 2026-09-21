//! SMTP submission of one message on an already-open connection.
//!
//! [`SmtpSession`] holds the dialogue position. [`decide`] is the step: a parsed reply goes
//! in, and the next command or a terminal result comes out. Commands are not pipelined.
//! Nothing here reads a socket.

use crate::machine::{IoNeed, IoReady, Machine, Progress, ProtoError, Refusal};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use mail_domain::{Credential, SaslMech, Tls};
use std::fmt;

/// A reply longer than this is rejected. Real EHLO banners are a few kilobytes.
const MAX_REPLY: usize = 64 * 1024;

/// Whether the server listed an extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Advertised {
    /// The keyword was absent.
    #[default]
    Absent,
    /// The keyword was present.
    Offered,
}

/// The `SIZE` extension, if the server mentioned it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SizeLimit {
    /// `SIZE` was not advertised.
    #[default]
    Absent,
    /// `SIZE` with no number: the server accepts the extension but states no limit.
    Unlimited,
    /// Maximum message size in octets, excluding the terminating dot.
    Limited(u64),
}

/// What the server advertised in the most recent `EHLO`.
///
/// The caller picks an authentication mechanism from [`EhloExtensions::auth`]. A mechanism
/// the server did not list is not attempted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EhloExtensions {
    /// `AUTH` mechanisms, in the order the server listed them.
    pub auth: Vec<SaslMech>,
    pub starttls: Advertised,
    pub size: SizeLimit,
    /// `8BITMIME`.
    pub eight_bit_mime: Advertised,
}

/// A complete SMTP reply, reduced to its status code and text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyText {
    pub code: u16,
    /// Text of each line, joined by newlines, without the status code.
    pub text: String,
}

/// The server accepted the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpReply {
    /// Extensions from the `EHLO` that preceded authentication.
    pub extensions: EhloExtensions,
    /// The mechanism the server accepted.
    pub mechanism: SaslMech,
    /// The positive reply to the terminating dot. This is the acceptance.
    pub accepted: ReplyText,
    /// The reply to `QUIT`. `None` when the connection closed after acceptance
    /// without a goodbye — the message still stands, and must not be sent again.
    pub closing: Option<ReplyText>,
}

/// One message to submit.
///
/// `username` is the login name already resolved from [`mail_domain::Username`].
/// The message is the RFC 5322 content, not yet dot-stuffed.
#[derive(Clone, PartialEq, Eq)]
pub struct Submission {
    /// Name sent in `EHLO`.
    pub ehlo: String,
    /// Server name. Used only for [`IoNeed::OpenTls`].
    pub host: String,
    pub port: u16,
    pub tls: Tls,
    pub username: String,
    pub credential: Credential,
    /// Mechanisms this account will use, most preferred first.
    pub sasl: Vec<SaslMech>,
    pub mail_from: String,
    pub recipients: Vec<String>,
    pub message: Vec<u8>,
}

// Hand-written: `credential` would otherwise ride along inside a derived `Debug`, and the
// message body is omitted so a secret pasted into it cannot leak through a log of the session.
impl fmt::Debug for Submission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Submission")
            .field("ehlo", &self.ehlo)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("tls", &self.tls)
            .field("username", &self.username)
            .field("credential", &self.credential)
            .field("sasl", &self.sasl)
            .field("mail_from", &self.mail_from)
            .field("recipients", &self.recipients)
            .field("message_len", &self.message.len())
            .finish()
    }
}

/// How far a submission has got.
///
/// Each variant is one wait: a reply to the command just sent, or the TLS handshake.
/// There is no "already authenticated" flag — that fact is which variant we are in.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Greeting,
    Ehlo,
    StartTls,
    AwaitTls,
    EhloAfterTls,
    AuthPlain,
    /// `AUTH PLAIN` drew a `334`; the initial response was sent on its own.
    AuthPlainSent,
    AuthLoginUser,
    AuthLoginPass,
    AuthLoginSent,
    AuthXoauth2,
    /// The server sent the XOAUTH2 failure `334`; an empty line was written.
    AuthXoauth2Sent,
    MailFrom,
    /// Waiting for the reply to `recipients[index]`.
    Rcpt(usize),
    Data,
    /// The stuffed body, including the terminating dot, has been written.
    Body,
    /// Waiting for `QUIT`. The value is the acceptance reply, which must survive a rude close.
    Quit(ReplyText),
    Finished,
}

/// A submission client. It speaks one message, then `QUIT`.
pub struct SmtpSession {
    submission: Submission,
    phase: Phase,
    buf: Vec<u8>,
    extensions: EhloExtensions,
    mechanism: Option<SaslMech>,
}

impl fmt::Debug for SmtpSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpSession")
            .field("submission", &self.submission)
            .field("phase", &self.phase)
            .field("extensions", &self.extensions)
            .field("mechanism", &self.mechanism)
            .finish()
    }
}

impl SmtpSession {
    /// A session that has not yet read the greeting.
    pub fn new(submission: Submission) -> Self {
        Self {
            submission,
            phase: Phase::Greeting,
            buf: Vec::new(),
            extensions: EhloExtensions::default(),
            mechanism: None,
        }
    }

    /// Extensions from the last completed `EHLO`, including after a later failure.
    ///
    /// Empty until that reply has been parsed. The caller uses this to choose a different
    /// mechanism and open another session when this one stops with [`ProtoError::Unsupported`].
    pub fn extensions(&self) -> &EhloExtensions {
        &self.extensions
    }

    /// The mechanism this session committed to, once `AUTH` has been sent.
    pub fn mechanism(&self) -> Option<SaslMech> {
        self.mechanism
    }

    fn fail(&self, err: ProtoError) -> Progress<SmtpReply> {
        Progress::Failed(scrub_error(err, &self.submission))
    }

    fn on_bytes(&mut self, bytes: Vec<u8>) -> Progress<SmtpReply> {
        if matches!(self.phase, Phase::AwaitTls | Phase::Finished) {
            let why = if matches!(self.phase, Phase::AwaitTls) {
                "server bytes arrived while TLS was starting"
            } else {
                "bytes after the session finished"
            };
            return self.fail(ProtoError::Malformed(why.into()));
        }
        if self.buf.len().saturating_add(bytes.len()) > MAX_REPLY {
            return self.fail(ProtoError::Malformed("reply exceeded 65536 bytes".into()));
        }
        self.buf.extend(bytes);
        let (reply, consumed) = match take_reply(&self.buf) {
            Ok(None) => return Progress::Need(vec![IoNeed::Read]),
            Ok(Some(pair)) => pair,
            Err(err) => return self.fail(err),
        };
        self.buf.drain(..consumed);
        if matches!(self.phase, Phase::Ehlo | Phase::EhloAfterTls) && is_success(reply.code) {
            self.extensions = parse_ehlo(&reply);
        }
        let outcome = match decide(
            &self.phase,
            &reply,
            &self.submission,
            &self.extensions,
            self.mechanism,
        ) {
            Ok(outcome) => outcome,
            Err(err) => return self.fail(err),
        };
        self.apply(outcome)
    }

    fn apply(&mut self, outcome: Outcome) -> Progress<SmtpReply> {
        match outcome {
            Outcome::Continue {
                next,
                mechanism,
                needs,
            } => {
                if let Some(mech) = mechanism {
                    self.mechanism = Some(mech);
                }
                self.phase = next;
                Progress::Need(needs)
            }
            Outcome::Done(reply) => {
                self.phase = Phase::Finished;
                Progress::Done(reply)
            }
        }
    }

    fn on_tls_open(&mut self) -> Progress<SmtpReply> {
        if !matches!(self.phase, Phase::AwaitTls) {
            return self.fail(ProtoError::Malformed("unexpected TLS open".into()));
        }
        // Anything buffered was plaintext from before the handshake.
        self.buf.clear();
        self.phase = Phase::EhloAfterTls;
        Progress::Need(vec![
            IoNeed::Write(ehlo_cmd(&self.submission.ehlo)),
            IoNeed::Read,
        ])
    }

    fn on_eof(&mut self) -> Progress<SmtpReply> {
        let Phase::Quit(accepted) = &self.phase else {
            return self.fail(ProtoError::UnexpectedEof);
        };
        // The terminating dot was already accepted. Retrying would submit a second copy.
        let accepted = accepted.clone();
        self.finish_ok(accepted, None)
    }

    fn finish_ok(
        &mut self,
        accepted: ReplyText,
        closing: Option<ReplyText>,
    ) -> Progress<SmtpReply> {
        let Some(mechanism) = self.mechanism else {
            return self.fail(ProtoError::Malformed(
                "finished without authenticating".into(),
            ));
        };
        let extensions = self.extensions.clone();
        self.phase = Phase::Finished;
        Progress::Done(SmtpReply {
            extensions,
            mechanism,
            accepted,
            closing,
        })
    }
}

impl Machine for SmtpSession {
    type Out = SmtpReply;

    fn start(&mut self) -> Progress<SmtpReply> {
        if let Err(err) = validate(&self.submission) {
            return self.fail(err);
        }
        Progress::Need(vec![IoNeed::Read])
    }

    fn feed(&mut self, ready: IoReady) -> Progress<SmtpReply> {
        match ready {
            IoReady::Bytes(bytes) => self.on_bytes(bytes),
            IoReady::Eof => self.on_eof(),
            IoReady::TlsOpen => self.on_tls_open(),
            IoReady::Woke => self.fail(ProtoError::Malformed("unexpected wake".into())),
            // The runtime wants the connection back. Giving up is permanent: resubmitting
            // is the caller's decision, not an automatic retry of a half-sent dialogue.
            IoReady::Interrupt => {
                if let Phase::Quit(accepted) = &self.phase {
                    let accepted = accepted.clone();
                    return self.finish_ok(accepted, None);
                }
                self.fail(ProtoError::Refused {
                    kind: Refusal::Permanent,
                    text: "submission interrupted".into(),
                })
            }
        }
    }
}

/// 4xx and 5xx both use [`ProtoError::Refused`]. This says which one it was.
///
// --- replies ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerReply {
    code: u16,
    lines: Vec<String>,
}

struct ParsedLine {
    code: u16,
    last: bool,
    text: String,
}

enum ReplyClass {
    Success,
    Intermediate,
    Transient,
    Permanent,
}

fn classify(code: u16) -> Option<ReplyClass> {
    match code / 100 {
        2 => Some(ReplyClass::Success),
        3 => Some(ReplyClass::Intermediate),
        4 => Some(ReplyClass::Transient),
        5 => Some(ReplyClass::Permanent),
        _ => None,
    }
}

fn is_success(code: u16) -> bool {
    matches!(classify(code), Some(ReplyClass::Success))
}

/// Pull one complete reply off the front of `buf`.
///
/// `Ok(None)` means the reply is still incomplete — the caller must read again. Continuation
/// lines (`NNN-`) do not finish a reply; only `NNN ` does. Bytes after that line stay in the
/// buffer for the next reply.
fn take_reply(buf: &[u8]) -> Result<Option<(ServerReply, usize)>, ProtoError> {
    let mut pos = 0;
    let mut lines = Vec::new();
    let mut expected: Option<u16> = None;
    while let Some(rel) = buf[pos..].windows(2).position(|w| w == b"\r\n") {
        let parsed = parse_line(&buf[pos..pos + rel])?;
        if let Some(prev) = expected {
            if prev != parsed.code {
                return Err(ProtoError::Malformed(format!(
                    "reply code changed from {prev} to {}",
                    parsed.code
                )));
            }
        }
        expected = Some(parsed.code);
        let last = parsed.last;
        let code = parsed.code;
        lines.push(parsed.text);
        pos += rel + 2;
        if last {
            return Ok(Some((ServerReply { code, lines }, pos)));
        }
    }
    Ok(None)
}

fn parse_line(line: &[u8]) -> Result<ParsedLine, ProtoError> {
    let Some(code) = status_code(line) else {
        return Err(ProtoError::Malformed(format!(
            "not a reply line: {}",
            preview(line)
        )));
    };
    if line.len() == 3 {
        return Ok(ParsedLine {
            code,
            last: true,
            text: String::new(),
        });
    }
    let text = std::str::from_utf8(&line[4..])
        .map_err(|_| ProtoError::Malformed("reply line is not utf-8".into()))?
        .to_owned();
    match line[3] {
        b' ' => Ok(ParsedLine {
            code,
            last: true,
            text,
        }),
        b'-' => Ok(ParsedLine {
            code,
            last: false,
            text,
        }),
        _ => Err(ProtoError::Malformed(format!(
            "bad reply separator: {}",
            preview(line)
        ))),
    }
}

fn status_code(line: &[u8]) -> Option<u16> {
    if line.len() < 3 {
        return None;
    }
    let mut code = 0u16;
    for byte in &line[..3] {
        if !byte.is_ascii_digit() {
            return None;
        }
        code = code * 10 + u16::from(byte - b'0');
    }
    Some(code)
}

fn preview(line: &[u8]) -> String {
    let shown = String::from_utf8_lossy(line);
    let mut out = String::new();
    for (i, ch) in shown.chars().enumerate() {
        if i == 60 {
            out.push('…');
            break;
        }
        if ch.is_control() {
            out.push(' ')
        } else {
            out.push(ch)
        }
    }
    out
}

fn show_reply(reply: &ServerReply) -> String {
    let text = reply
        .lines
        .iter()
        .filter(|line| !line.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" | ");
    if text.is_empty() {
        format!("{}", reply.code)
    } else {
        format!("{} {text}", reply.code)
    }
}

fn reply_text(reply: &ServerReply) -> ReplyText {
    ReplyText {
        code: reply.code,
        text: reply.lines.join("\n"),
    }
}

fn refusal(reply: &ServerReply) -> ProtoError {
    // 421 is "service not available, closing transmission channel", which servers use for rate
    // limiting. Backing off is the remedy; treating it as a refusal would discard the message.
    if reply.code == 421 {
        return ProtoError::Throttled {
            reason: show_reply(reply),
            retry_after: None,
        };
    }
    let kind = match classify(reply.code) {
        Some(ReplyClass::Transient) => Refusal::Transient,
        _ => Refusal::Permanent,
    };
    ProtoError::Refused {
        kind,
        text: show_reply(reply),
    }
}

fn expect_success(reply: &ServerReply) -> Result<(), ProtoError> {
    match classify(reply.code) {
        Some(ReplyClass::Success) => Ok(()),
        Some(ReplyClass::Transient | ReplyClass::Permanent) => Err(refusal(reply)),
        _ => Err(ProtoError::Malformed(format!(
            "expected a completion reply, got {}",
            show_reply(reply)
        ))),
    }
}

fn expect_intermediate(reply: &ServerReply) -> Result<(), ProtoError> {
    match classify(reply.code) {
        Some(ReplyClass::Intermediate) => Ok(()),
        Some(ReplyClass::Transient | ReplyClass::Permanent) => Err(refusal(reply)),
        _ => Err(ProtoError::Malformed(format!(
            "expected an intermediate reply, got {}",
            show_reply(reply)
        ))),
    }
}

/// A negative answer while authenticating.
///
/// 535 and any other 5xx, and a challenge we cannot answer, are [`ProtoError::AuthRejected`].
/// A 4xx stays a transient [`ProtoError::Refused`] so the caller can tell a locked account
/// from a server that is merely busy.
fn auth_negative(reply: &ServerReply) -> ProtoError {
    match classify(reply.code) {
        Some(ReplyClass::Transient) => refusal(reply),
        Some(ReplyClass::Permanent) => ProtoError::AuthRejected(show_reply(reply)),
        _ => ProtoError::AuthRejected(format!("rejected challenge {}", show_reply(reply))),
    }
}

fn parse_ehlo(reply: &ServerReply) -> EhloExtensions {
    let mut ext = EhloExtensions::default();
    for line in &reply.lines {
        apply_ehlo_line(&mut ext, line);
    }
    ext
}

fn apply_ehlo_line(ext: &mut EhloExtensions, line: &str) {
    let mut parts = line.split_whitespace();
    let Some(raw) = parts.next() else {
        return;
    };
    let keyword = raw.to_ascii_uppercase();
    if let Some(mech) = keyword.strip_prefix("AUTH=") {
        push_mech(&mut ext.auth, mech);
        for extra in parts {
            push_mech(&mut ext.auth, extra);
        }
        return;
    }
    match keyword.as_str() {
        "AUTH" => {
            for mech in parts {
                push_mech(&mut ext.auth, mech);
            }
        }
        "STARTTLS" => ext.starttls = Advertised::Offered,
        "8BITMIME" => ext.eight_bit_mime = Advertised::Offered,
        "SIZE" => match parts.next() {
            Some(number) => match number.parse::<u64>() {
                Ok(limit) => ext.size = SizeLimit::Limited(limit),
                Err(_) => ext.size = SizeLimit::Unlimited,
            },
            None => ext.size = SizeLimit::Unlimited,
        },
        _ => {}
    }
}

fn push_mech(auth: &mut Vec<SaslMech>, raw: &str) {
    let Some(mech) = parse_mech(raw) else {
        return;
    };
    if !auth.contains(&mech) {
        auth.push(mech);
    }
}

fn parse_mech(raw: &str) -> Option<SaslMech> {
    match raw.to_ascii_uppercase().as_str() {
        "PLAIN" => Some(SaslMech::Plain),
        "LOGIN" => Some(SaslMech::Login),
        // CRAM-MD5 is parsed as unknown on purpose: SaslMech no longer has it, since no
        // account we target offers it and we cannot exercise it against a real server.
        "XOAUTH2" => Some(SaslMech::XOauth2),
        _ => None,
    }
}

// --- the step --------------------------------------------------------------------

/// What [`decide`] wants done. Not `Debug`: `needs` holds the AUTH command, which is the secret.
enum Outcome {
    Continue {
        next: Phase,
        mechanism: Option<SaslMech>,
        needs: Vec<IoNeed>,
    },
    Done(SmtpReply),
}

fn decide(
    phase: &Phase,
    reply: &ServerReply,
    sub: &Submission,
    ext: &EhloExtensions,
    mech: Option<SaslMech>,
) -> Result<Outcome, ProtoError> {
    match phase {
        Phase::Greeting => on_greeting(reply, sub),
        Phase::Ehlo | Phase::EhloAfterTls => on_ehlo(phase, reply, sub, ext),
        Phase::StartTls => on_starttls(reply, sub),
        Phase::AwaitTls => Err(ProtoError::Malformed(
            "server bytes arrived while TLS was starting".into(),
        )),
        Phase::AuthPlain
        | Phase::AuthPlainSent
        | Phase::AuthLoginUser
        | Phase::AuthLoginPass
        | Phase::AuthLoginSent
        | Phase::AuthXoauth2
        | Phase::AuthXoauth2Sent => on_auth(phase, reply, sub),
        Phase::MailFrom => on_mail_from(reply, sub),
        Phase::Rcpt(index) => on_rcpt(*index, reply, sub),
        Phase::Data => on_data(reply, sub),
        Phase::Body => on_body(reply),
        Phase::Quit(accepted) => on_quit(accepted, reply, ext, mech),
        Phase::Finished => Err(ProtoError::Malformed(
            "reply after the session finished".into(),
        )),
    }
}

fn on_greeting(reply: &ServerReply, sub: &Submission) -> Result<Outcome, ProtoError> {
    expect_success(reply)?;
    Ok(continue_with(Phase::Ehlo, vec![ehlo_cmd(&sub.ehlo)], None))
}

fn on_ehlo(
    phase: &Phase,
    reply: &ServerReply,
    sub: &Submission,
    ext: &EhloExtensions,
) -> Result<Outcome, ProtoError> {
    expect_success(reply)?;
    let after_tls = matches!(phase, Phase::EhloAfterTls);
    if sub.tls == Tls::StartTlsRequired && !after_tls {
        if ext.starttls != Advertised::Offered {
            return Err(ProtoError::Unsupported("STARTTLS".into()));
        }
        return Ok(continue_with(Phase::StartTls, vec![cmd("STARTTLS")], None));
    }
    check_transfer(ext, &sub.message)?;
    let mech = choose_mech(&ext.auth, &sub.sasl, &sub.credential)?;
    let (next, command) = auth_command(mech, sub)?;
    Ok(continue_with(next, vec![command], Some(mech)))
}

fn on_starttls(reply: &ServerReply, sub: &Submission) -> Result<Outcome, ProtoError> {
    expect_success(reply)?;
    Ok(Outcome::Continue {
        next: Phase::AwaitTls,
        mechanism: None,
        needs: vec![IoNeed::OpenTls {
            host: sub.host.clone(),
            port: sub.port,
            mode: sub.tls,
        }],
    })
}

fn on_auth(phase: &Phase, reply: &ServerReply, sub: &Submission) -> Result<Outcome, ProtoError> {
    match phase {
        Phase::AuthPlain => {
            if reply.code == 334 {
                let pass = password(&sub.credential)?;
                let line = cmd(&b64(&plain_raw(&sub.username, pass)));
                return Ok(continue_with(Phase::AuthPlainSent, vec![line], None));
            }
            finish_auth(reply, sub)
        }
        Phase::AuthPlainSent => finish_auth(reply, sub),
        Phase::AuthXoauth2 => {
            if reply.code == 334 {
                // The server rejected the bearer token and is waiting for an empty line
                // before it sends the final 535. Sending another copy of the token here
                // would put it on the wire twice.
                return Ok(continue_with(Phase::AuthXoauth2Sent, vec![cmd("")], None));
            }
            finish_auth(reply, sub)
        }
        Phase::AuthXoauth2Sent => finish_auth(reply, sub),
        Phase::AuthLoginUser => {
            if reply.code != 334 {
                return Err(auth_negative(reply));
            }
            let line = cmd(&b64(sub.username.as_bytes()));
            Ok(continue_with(Phase::AuthLoginPass, vec![line], None))
        }
        Phase::AuthLoginPass => {
            if reply.code != 334 {
                return Err(auth_negative(reply));
            }
            let pass = password(&sub.credential)?;
            let line = cmd(&b64(pass.as_bytes()));
            Ok(continue_with(Phase::AuthLoginSent, vec![line], None))
        }
        Phase::AuthLoginSent => finish_auth(reply, sub),
        _ => Err(ProtoError::Malformed(
            "authentication reply in a non-auth state".into(),
        )),
    }
}

fn finish_auth(reply: &ServerReply, sub: &Submission) -> Result<Outcome, ProtoError> {
    if is_success(reply.code) {
        send_mail_from(sub)
    } else {
        Err(auth_negative(reply))
    }
}

fn on_mail_from(reply: &ServerReply, sub: &Submission) -> Result<Outcome, ProtoError> {
    expect_success(reply)?;
    send_rcpt(sub, 0)
}

fn on_rcpt(index: usize, reply: &ServerReply, sub: &Submission) -> Result<Outcome, ProtoError> {
    expect_success(reply)?;
    let next = index + 1;
    if next < sub.recipients.len() {
        send_rcpt(sub, next)
    } else {
        Ok(continue_with(Phase::Data, vec![cmd("DATA")], None))
    }
}

fn on_data(reply: &ServerReply, sub: &Submission) -> Result<Outcome, ProtoError> {
    expect_intermediate(reply)?;
    Ok(continue_with(Phase::Body, data_lines(&sub.message), None))
}

fn on_body(reply: &ServerReply) -> Result<Outcome, ProtoError> {
    expect_success(reply)?;
    Ok(continue_with(
        Phase::Quit(reply_text(reply)),
        vec![cmd("QUIT")],
        None,
    ))
}

fn on_quit(
    accepted: &ReplyText,
    reply: &ServerReply,
    ext: &EhloExtensions,
    mech: Option<SaslMech>,
) -> Result<Outcome, ProtoError> {
    let Some(mechanism) = mech else {
        return Err(ProtoError::Malformed(
            "finished without authenticating".into(),
        ));
    };
    // Any reply to QUIT ends the session. The message was accepted at the dot; failing
    // here would make the outbox undo a delivery the server has already taken.
    Ok(Outcome::Done(SmtpReply {
        extensions: ext.clone(),
        mechanism,
        accepted: accepted.clone(),
        closing: Some(reply_text(reply)),
    }))
}

fn continue_with(next: Phase, commands: Vec<Vec<u8>>, mechanism: Option<SaslMech>) -> Outcome {
    let mut needs: Vec<_> = commands.into_iter().map(IoNeed::Write).collect();
    needs.push(IoNeed::Read);
    Outcome::Continue {
        next,
        mechanism,
        needs,
    }
}

fn send_mail_from(sub: &Submission) -> Result<Outcome, ProtoError> {
    let line = if has_high_bit(&sub.message) {
        format!("MAIL FROM:<{}> BODY=8BITMIME", sub.mail_from)
    } else {
        format!("MAIL FROM:<{}>", sub.mail_from)
    };
    Ok(continue_with(Phase::MailFrom, vec![cmd(&line)], None))
}

fn send_rcpt(sub: &Submission, index: usize) -> Result<Outcome, ProtoError> {
    let Some(addr) = sub.recipients.get(index) else {
        return Err(ProtoError::Refused {
            kind: Refusal::Permanent,
            text: "no recipients".into(),
        });
    };
    Ok(continue_with(
        Phase::Rcpt(index),
        vec![cmd(&format!("RCPT TO:<{addr}>"))],
        None,
    ))
}

fn check_transfer(ext: &EhloExtensions, message: &[u8]) -> Result<(), ProtoError> {
    if let SizeLimit::Limited(max) = ext.size {
        let size = transmitted_octets(message);
        if size > max {
            return Err(ProtoError::Refused {
                kind: Refusal::Permanent,
                text: format!("message is {size} octets, above SIZE {max}"),
            });
        }
    }
    if has_high_bit(message) && ext.eight_bit_mime != Advertised::Offered {
        return Err(ProtoError::Unsupported("8BITMIME".into()));
    }
    Ok(())
}

fn choose_mech(
    offered: &[SaslMech],
    wanted: &[SaslMech],
    cred: &Credential,
) -> Result<SaslMech, ProtoError> {
    let acceptable: Vec<SaslMech> = wanted
        .iter()
        .copied()
        .filter(|mech| compatible(*mech, cred))
        .collect();
    if acceptable.is_empty() {
        return Err(ProtoError::Unsupported(
            "AUTH PLAIN, AUTH LOGIN, or AUTH XOAUTH2 for this credential".into(),
        ));
    }
    if let Some(mech) = acceptable.into_iter().find(|mech| offered.contains(mech)) {
        return Ok(mech);
    }
    let offered_list = if offered.is_empty() {
        "nothing".to_owned()
    } else {
        offered
            .iter()
            .copied()
            .map(mech_label)
            .collect::<Vec<_>>()
            .join(", ")
    };
    Err(ProtoError::Unsupported(format!(
        "a compatible AUTH mechanism (server offered {offered_list})"
    )))
}

/// Whether this mechanism can carry this credential.
///
/// `CramMd5` is absent on purpose: neither account we target offers it (the NTU spike found
/// `SASL PLAIN` only), and a mechanism we cannot test against a real server is one we should not
/// claim to support.
fn compatible(mech: SaslMech, cred: &Credential) -> bool {
    matches!(
        (mech, cred),
        (SaslMech::Plain | SaslMech::Login, Credential::Password(_))
            | (SaslMech::XOauth2, Credential::OAuth { .. })
    )
}

fn mech_label(mech: SaslMech) -> &'static str {
    match mech {
        SaslMech::Plain => "PLAIN",
        SaslMech::Login => "LOGIN",
        SaslMech::XOauth2 => "XOAUTH2",
    }
}

fn auth_command(mech: SaslMech, sub: &Submission) -> Result<(Phase, Vec<u8>), ProtoError> {
    match mech {
        SaslMech::Plain => {
            let pass = password(&sub.credential)?;
            let line = cmd(&format!(
                "AUTH PLAIN {}",
                b64(&plain_raw(&sub.username, pass))
            ));
            Ok((Phase::AuthPlain, line))
        }
        SaslMech::Login => Ok((Phase::AuthLoginUser, cmd("AUTH LOGIN"))),
        SaslMech::XOauth2 => {
            let token = access_token(&sub.credential)?;
            let line = cmd(&format!(
                "AUTH XOAUTH2 {}",
                b64(&xoauth2_raw(&sub.username, token))
            ));
            Ok((Phase::AuthXoauth2, line))
        }
    }
}

fn password(cred: &Credential) -> Result<&str, ProtoError> {
    match cred {
        Credential::Password(pass) => Ok(pass.as_str()),
        Credential::OAuth { .. } => Err(ProtoError::Unsupported(
            "a password mechanism with an OAuth credential".into(),
        )),
    }
}

fn access_token(cred: &Credential) -> Result<&str, ProtoError> {
    match cred {
        Credential::OAuth { access, .. } => Ok(access.as_str()),
        Credential::Password(_) => Err(ProtoError::Unsupported(
            "XOAUTH2 with a password credential".into(),
        )),
    }
}

// --- bytes on the wire -----------------------------------------------------------

fn cmd(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 2);
    out.extend_from_slice(text.as_bytes());
    out.extend_from_slice(b"\r\n");
    out
}

fn ehlo_cmd(name: &str) -> Vec<u8> {
    cmd(&format!("EHLO {name}"))
}

fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

/// `\0 username \0 password`, as RFC 4616 defines the PLAIN initial response.
fn plain_raw(user: &str, pass: &str) -> Vec<u8> {
    let mut raw = Vec::with_capacity(user.len() + pass.len() + 2);
    raw.push(0);
    raw.extend_from_slice(user.as_bytes());
    raw.push(0);
    raw.extend_from_slice(pass.as_bytes());
    raw
}

/// `user=<addr>\x01auth=Bearer <token>\x01\x01`.
fn xoauth2_raw(user: &str, token: &str) -> Vec<u8> {
    let mut raw = Vec::with_capacity(user.len() + token.len() + 20);
    raw.extend_from_slice(b"user=");
    raw.extend_from_slice(user.as_bytes());
    raw.extend_from_slice(b"\x01auth=Bearer ");
    raw.extend_from_slice(token.as_bytes());
    raw.extend_from_slice(b"\x01\x01");
    raw
}

/// Dot-stuff `message` and terminate it.
///
/// Each buffer is one transmitted line, including its CRLF. The last line is the
/// terminating dot. A message line whose first octet is `.` gains an extra `.`.
/// A lone LF is sent as CRLF. When the message does not already end on a line break,
/// a CRLF is added first so the dot occupies a line of its own.
fn data_lines(message: &[u8]) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut ended_on_break = false;
    let mut i = 0;
    while i < message.len() {
        if message[i] == b'\n' {
            lines.push(stuff_line(std::mem::take(&mut current)));
            ended_on_break = true;
            i += 1;
            continue;
        }
        if message[i] == b'\r' && message.get(i + 1) == Some(&b'\n') {
            lines.push(stuff_line(std::mem::take(&mut current)));
            ended_on_break = true;
            i += 2;
            continue;
        }
        current.push(message[i]);
        ended_on_break = false;
        i += 1;
    }
    if !ended_on_break {
        lines.push(stuff_line(current));
    }
    lines.push(b".\r\n".to_vec());
    lines
}

fn stuff_line(line: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len() + 3);
    if line.first() == Some(&b'.') {
        out.push(b'.');
    }
    out.extend_from_slice(&line);
    out.extend_from_slice(b"\r\n");
    out
}

/// Octets that will be sent, excluding the terminating dot line.
fn transmitted_octets(message: &[u8]) -> u64 {
    let lines = data_lines(message);
    let body: usize = lines.iter().rev().skip(1).map(Vec::len).sum();
    body as u64
}

fn has_high_bit(message: &[u8]) -> bool {
    message.iter().any(|byte| *byte >= 0x80)
}

fn validate(sub: &Submission) -> Result<(), ProtoError> {
    if !is_token(&sub.ehlo) {
        return Err(invalid("EHLO name is empty or contains whitespace"));
    }
    if !is_token(&sub.host) {
        return Err(invalid("SMTP host is empty or contains whitespace"));
    }
    if !is_mailbox(&sub.mail_from) {
        return Err(invalid("mail from is empty or contains a line break"));
    }
    if sub.recipients.is_empty() {
        return Err(invalid("no recipients"));
    }
    if sub.recipients.iter().any(|addr| !is_mailbox(addr)) {
        return Err(invalid("a recipient is empty or contains a line break"));
    }
    if sub.username.is_empty() || has_break(&sub.username) {
        return Err(invalid("username is empty or contains a line break"));
    }
    match &sub.credential {
        Credential::Password(pass) if pass.as_bytes().contains(&0) => {
            Err(invalid("password contains NUL"))
        }
        Credential::OAuth { access, .. } if access.is_empty() || access.as_bytes().contains(&0) => {
            Err(invalid("access token is empty or contains NUL"))
        }
        _ => Ok(()),
    }
}

/// A local precondition. Permanent, so the outbox does not retry it.
fn invalid(why: &str) -> ProtoError {
    ProtoError::Refused {
        kind: Refusal::Permanent,
        text: why.to_owned(),
    }
}

fn is_token(value: &str) -> bool {
    !value.is_empty()
        && !value
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
}

fn is_mailbox(value: &str) -> bool {
    !value.is_empty()
        && !value
            .bytes()
            .any(|b| matches!(b, b'\r' | b'\n' | b'\0' | b'<' | b'>' | b' '))
}

fn has_break(value: &str) -> bool {
    value.bytes().any(|b| matches!(b, b'\r' | b'\n' | b'\0'))
}

/// Remove anything we know is a secret from an error string.
///
/// Server text is forwarded to the caller. A reply that echoes a token must not carry it
/// into a log. The class is carried structurally, so redaction cannot disturb it, and
/// works when the secret happens to contain those words.
fn scrub_error(err: ProtoError, sub: &Submission) -> ProtoError {
    match err {
        ProtoError::Malformed(text) => ProtoError::Malformed(scrub_text(text, sub)),
        // The class survives scrubbing: it is a field, not a prefix in the text.
        ProtoError::Refused { kind, text } => ProtoError::Refused {
            kind,
            text: scrub_text(text, sub),
        },
        ProtoError::AuthRejected(text) => ProtoError::AuthRejected(scrub_text(text, sub)),
        ProtoError::Unsupported(text) => ProtoError::Unsupported(scrub_text(text, sub)),
        ProtoError::Throttled {
            reason,
            retry_after,
        } => ProtoError::Throttled {
            reason: scrub_text(reason, sub),
            retry_after,
        },
        ProtoError::UnexpectedEof => ProtoError::UnexpectedEof,
    }
}

fn scrub_text(text: String, sub: &Submission) -> String {
    let (prefix, mut rest) = if let Some(rest) = text.strip_prefix("transient ") {
        ("transient ", rest.to_owned())
    } else if let Some(rest) = text.strip_prefix("permanent ") {
        ("permanent ", rest.to_owned())
    } else {
        ("", text)
    };
    let mut secrets = secret_strings(sub);
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    secrets.dedup();
    for secret in secrets {
        if !secret.is_empty() {
            rest = rest.replace(&secret, "<redacted>");
        }
    }
    if prefix.is_empty() {
        rest
    } else {
        format!("{prefix}{rest}")
    }
}

fn secret_strings(sub: &Submission) -> Vec<String> {
    let mut out = Vec::new();
    match &sub.credential {
        Credential::Password(pass) => {
            push_secret(&mut out, pass);
            if !pass.is_empty() {
                out.push(b64(&plain_raw(&sub.username, pass)));
                out.push(b64(pass.as_bytes()));
            }
        }
        Credential::OAuth {
            access, refresh, ..
        } => {
            push_secret(&mut out, access);
            push_secret(&mut out, refresh);
            if !access.is_empty() {
                out.push(b64(&xoauth2_raw(&sub.username, access)));
                out.push(b64(access.as_bytes()));
            }
            if !refresh.is_empty() {
                out.push(b64(refresh.as_bytes()));
            }
        }
    }
    out
}

fn push_secret(out: &mut Vec<String>, secret: &str) {
    if !secret.is_empty() {
        out.push(secret.to_owned());
    }
}

// --- tests of the pure pieces ----------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::Credential;

    const PASSWORD: &str = "s3cr3t-password";

    fn submission(message: &str) -> Submission {
        Submission {
            ehlo: "client.example".into(),
            host: "smtp.example".into(),
            port: 465,
            tls: Tls::Implicit,
            username: "ada@example.com".into(),
            credential: Credential::Password(PASSWORD.into()),
            sasl: vec![SaslMech::Plain, SaslMech::Login],
            mail_from: "ada@example.com".into(),
            recipients: vec!["bob@example.com".into()],
            message: message.as_bytes().to_vec(),
        }
    }

    fn session(message: &str) -> SmtpSession {
        SmtpSession::new(submission(message))
    }

    #[test]
    fn plain_initial_response_matches_rfc4616() {
        let raw = plain_raw("tim", "tanstaaftanstaaf");
        assert_eq!(raw, b"\0tim\0tanstaaftanstaaf");
        assert_eq!(b64(&raw), "AHRpbQB0YW5zdGFhZnRhbnN0YWFm");
    }

    #[test]
    fn xoauth2_initial_response_is_the_bearer_form() {
        let raw = xoauth2_raw("user@example.com", "tok");
        assert_eq!(raw, b"user=user@example.com\x01auth=Bearer tok\x01\x01");
    }

    #[test]
    fn dot_stuff_cases() {
        let cases: &[(&[u8], &[&[u8]])] = &[
            (b"hello\r\n", &[b"hello\r\n", b".\r\n"]),
            (b"hello", &[b"hello\r\n", b".\r\n"]),
            (b".dot\r\n", &[b"..dot\r\n", b".\r\n"]),
            (b".\r\n", &[b"..\r\n", b".\r\n"]),
            (b"", &[b"\r\n", b".\r\n"]),
            (b"a\n.b\n", &[b"a\r\n", b"..b\r\n", b".\r\n"]),
            (
                b"Hi.\r\n.this starts with a dot\r\nBye\r\n",
                &[
                    b"Hi.\r\n",
                    b"..this starts with a dot\r\n",
                    b"Bye\r\n",
                    b".\r\n",
                ],
            ),
        ];
        for (input, want) in cases {
            let got = data_lines(input);
            let expect: Vec<Vec<u8>> = want.iter().map(|line| line.to_vec()).collect();
            assert_eq!(got, expect, "input {input:?}");
        }
    }

    #[test]
    fn transmitted_size_excludes_the_terminator_and_counts_stuffing() {
        assert_eq!(transmitted_octets(b"hello\r\n"), 7);
        assert_eq!(transmitted_octets(b"hello"), 7);
        assert_eq!(transmitted_octets(b".x\r\n"), 5);
        assert_eq!(transmitted_octets(b""), 2);
    }

    #[test]
    fn an_incomplete_reply_asks_for_more_and_a_final_line_finishes_it() {
        assert!(take_reply(b"250-hello\r\n250 AUTH").unwrap().is_none());
        let (reply, n) = take_reply(b"250-hello\r\n250 AUTH PLAIN\r\n")
            .unwrap()
            .expect("complete");
        assert_eq!(n, b"250-hello\r\n250 AUTH PLAIN\r\n".len());
        assert_eq!(reply.code, 250);
        assert_eq!(
            reply.lines,
            vec!["hello".to_owned(), "AUTH PLAIN".to_owned()]
        );
    }

    #[test]
    fn a_changed_reply_code_is_malformed() {
        let err = take_reply(b"250-hello\r\n251 end\r\n").unwrap_err();
        assert!(matches!(err, ProtoError::Malformed(_)));
    }

    #[test]
    fn ehlo_extensions_keep_known_mechanisms_in_server_order() {
        let reply = ServerReply {
            code: 250,
            lines: vec![
                "smtp.example Hello".into(),
                "SIZE 100".into(),
                "8BITMIME".into(),
                "STARTTLS".into(),
                "AUTH PLAIN GSSAPI LOGIN XOAUTH2".into(),
                "AUTH LOGIN".into(),
                "PIPELINING".into(),
            ],
        };
        let ext = parse_ehlo(&reply);
        assert_eq!(
            ext.auth,
            vec![SaslMech::Plain, SaslMech::Login, SaslMech::XOauth2]
        );
        assert_eq!(ext.size, SizeLimit::Limited(100));
        assert_eq!(ext.starttls, Advertised::Offered);
        assert_eq!(ext.eight_bit_mime, Advertised::Offered);
    }

    #[test]
    fn legacy_auth_equals_and_bare_size_are_recognised() {
        let reply = ServerReply {
            code: 250,
            lines: vec!["AUTH=PLAIN LOGIN".into(), "SIZE".into()],
        };
        let ext = parse_ehlo(&reply);
        assert_eq!(ext.auth, vec![SaslMech::Plain, SaslMech::Login]);
        assert_eq!(ext.size, SizeLimit::Unlimited);
    }

    #[test]
    fn mechanism_choice_follows_preference_and_the_credential() {
        let password = Credential::Password(PASSWORD.into());
        let offered = [SaslMech::Plain, SaslMech::XOauth2, SaslMech::Login];
        assert_eq!(
            choose_mech(&offered, &[SaslMech::XOauth2, SaslMech::Plain], &password).unwrap(),
            SaslMech::Plain
        );
        let token = Credential::OAuth {
            access: "tok".into(),
            refresh: "ref".into(),
            expires_at: chrono_expiry(),
        };
        assert_eq!(
            choose_mech(&offered, &[SaslMech::Plain, SaslMech::XOauth2], &token).unwrap(),
            SaslMech::XOauth2
        );
        let err = choose_mech(&[SaslMech::Login], &[SaslMech::Plain], &password).unwrap_err();
        assert!(matches!(err, ProtoError::Unsupported(_)));
        assert!(!err.to_string().contains(PASSWORD));
    }

    #[test]
    fn plaintext_and_implicit_do_not_send_starttls() {
        for tls in [Tls::Plaintext, Tls::Implicit] {
            let mut sub = submission("hi\r\n");
            sub.tls = tls;
            let ext = EhloExtensions {
                auth: vec![SaslMech::Plain],
                starttls: Advertised::Offered,
                ..EhloExtensions::default()
            };
            let reply = ServerReply {
                code: 250,
                lines: vec!["AUTH PLAIN".into()],
            };
            let outcome = on_ehlo(&Phase::Ehlo, &reply, &sub, &ext).expect("ehlo");
            let Outcome::Continue { next, needs, .. } = outcome else {
                panic!("EHLO ended the session");
            };
            assert!(matches!(next, Phase::AuthPlain), "{tls:?}");
            let IoNeed::Write(bytes) = &needs[0] else {
                panic!("expected a write");
            };
            assert!(
                bytes.starts_with(b"AUTH PLAIN "),
                "{tls:?} sent a different command"
            );
        }
    }

    #[test]
    fn starttls_required_upgrades_before_auth_and_not_after() {
        let mut sub = submission("hi\r\n");
        sub.tls = Tls::StartTlsRequired;
        let ext = EhloExtensions {
            auth: vec![SaslMech::Login],
            starttls: Advertised::Offered,
            ..EhloExtensions::default()
        };
        let reply = ServerReply {
            code: 250,
            lines: vec!["STARTTLS".into(), "AUTH LOGIN".into()],
        };
        let before = on_ehlo(&Phase::Ehlo, &reply, &sub, &ext).expect("first ehlo");
        let Outcome::Continue { next, needs, .. } = before else {
            panic!("first EHLO ended the session");
        };
        assert!(matches!(next, Phase::StartTls));
        assert_eq!(needs[0], IoNeed::Write(b"STARTTLS\r\n".to_vec()));

        let after = on_ehlo(&Phase::EhloAfterTls, &reply, &sub, &ext).expect("second ehlo");
        let Outcome::Continue { next, needs, .. } = after else {
            panic!("second EHLO ended the session");
        };
        assert!(matches!(next, Phase::AuthLoginUser));
        assert_eq!(needs[0], IoNeed::Write(b"AUTH LOGIN\r\n".to_vec()));
    }

    #[test]
    fn xoauth2_challenge_is_answered_with_an_empty_line() {
        let mut sub = submission("hi\r\n");
        sub.credential = Credential::OAuth {
            access: "tok".into(),
            refresh: "ref".into(),
            expires_at: chrono_expiry(),
        };
        sub.sasl = vec![SaslMech::XOauth2];
        let reply = ServerReply {
            code: 334,
            lines: vec!["eyJzdGF0dXMiOiI0MDAifQ==".into()],
        };
        let outcome = on_auth(&Phase::AuthXoauth2, &reply, &sub).expect("challenge");
        let Outcome::Continue { next, needs, .. } = outcome else {
            panic!("challenge ended the session");
        };
        assert!(matches!(next, Phase::AuthXoauth2Sent));
        assert_eq!(needs[0], IoNeed::Write(b"\r\n".to_vec()));
    }

    #[test]
    fn a_server_line_that_echoes_the_password_is_scrubbed() {
        let sub = submission("hi\r\n");
        let err = ProtoError::AuthRejected(format!("535 bad {PASSWORD}"));
        let cleaned = scrub_error(err, &sub);
        let rendered = format!("{cleaned} {cleaned:?}");
        assert!(!rendered.contains(PASSWORD), "password survived scrubbing");
        assert!(rendered.contains("<redacted>"));
        // The class is a field, so scrubbing the text cannot disturb it — which is the point
        // of making it structural rather than a prefix.
        let refused = scrub_error(
            ProtoError::Refused {
                kind: Refusal::Transient,
                text: format!("{PASSWORD} later"),
            },
            &sub,
        );
        assert!(
            matches!(
                &refused,
                ProtoError::Refused {
                    kind: Refusal::Transient,
                    ..
                }
            ),
            "{refused:?}"
        );
        assert!(!refused.to_string().contains(PASSWORD));
    }

    #[test]
    fn debug_of_a_session_redacts_the_password() {
        let session = session("hi\r\n");
        let rendered = format!("{session:?}");
        assert!(rendered.contains("SmtpSession"));
        assert!(rendered.contains("ada@example.com"));
        assert!(rendered.contains("redacted"));
        assert!(!rendered.contains(PASSWORD));
        let encoded = b64(&plain_raw("ada@example.com", PASSWORD));
        assert!(!rendered.contains(&encoded));
    }

    #[test]
    fn a_line_break_in_a_recipient_is_refused_without_echoing_it() {
        let mut sub = submission("hi\r\n");
        sub.recipients = vec!["bob@example.com\r\nRCPT TO:<eve@example.com>".into()];
        let mut session = SmtpSession::new(sub);
        match session.start() {
            Progress::Failed(err) => {
                assert!(
                    matches!(
                        &err,
                        ProtoError::Refused {
                            kind: Refusal::Permanent,
                            ..
                        }
                    ),
                    "{err:?}"
                );
                let rendered = format!("{err} {err:?}");
                assert!(!rendered.contains("eve@example.com"));
                assert!(!rendered.contains(PASSWORD));
            }
            Progress::Need(_) | Progress::Done(_) => panic!("invalid submission was accepted"),
        }
    }

    #[test]
    fn a_split_greeting_is_buffered_until_crlf() {
        let mut session = session("hi\r\n");
        assert!(matches!(session.start(), Progress::Need(_)));
        let partial = session.feed(IoReady::Bytes(b"22".to_vec()));
        assert_eq!(partial, Progress::Need(vec![IoNeed::Read]));
        let done = session.feed(IoReady::Bytes(b"0 ready\r\n".to_vec()));
        match done {
            Progress::Need(needs) => {
                assert_eq!(needs[0], IoNeed::Write(b"EHLO client.example\r\n".to_vec()));
                assert_eq!(needs[1], IoNeed::Read);
            }
            Progress::Done(_) | Progress::Failed(_) => panic!("greeting did not produce EHLO"),
        }
    }

    #[test]
    fn an_oversized_reply_is_malformed() {
        let mut session = session("hi\r\n");
        let _ = session.start();
        let big = vec![b'A'; MAX_REPLY + 1];
        let err = session.feed(IoReady::Bytes(big));
        assert!(matches!(err, Progress::Failed(ProtoError::Malformed(_))));
    }

    fn chrono_expiry() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp in range")
    }
}
