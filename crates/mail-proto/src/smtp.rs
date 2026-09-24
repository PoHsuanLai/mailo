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
    /// `SMTPUTF8` (RFC 6531): the server accepts a UTF-8 envelope.
    pub smtputf8: Advertised,
    /// `DSN` (RFC 3461): the server accepts delivery-status parameters.
    pub dsn: Advertised,
    /// `CHUNKING` (RFC 3030): the server accepts `BDAT` in place of `DATA`.
    pub chunking: Advertised,
}

/// A request for a delivery status notification (RFC 3461).
///
/// A request, not a requirement. When the server did not advertise `DSN` the
/// message is still submitted and these parameters are left off the commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    /// Which delivery events the sender wants reported.
    pub notify: Notify,
    /// How much of the message a notification should quote.
    pub ret: Return,
    /// Envelope identifier to echo in the notification, when the sender set one.
    pub envid: Option<String>,
}

/// Which delivery events a notification should report (`NOTIFY`).
///
/// `Never`, and an [`Notify::On`] with every flag false, are both sent as
/// `NOTIFY=NEVER`: RFC 3461 has no empty list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notify {
    /// The sender wants no notification.
    Never,
    /// The events to report. Only the true flags are listed, in the order
    /// success, failure, delay.
    On {
        /// Report a successful delivery.
        success: bool,
        /// Report a failed delivery.
        failure: bool,
        /// Report a delayed delivery.
        delay: bool,
    },
}

/// How much of the original message a notification includes (`RET`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Return {
    /// Headers only (`RET=HDRS`).
    Headers,
    /// The full message (`RET=FULL`).
    Full,
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
    /// Delivery status to request. `None` leaves the envelope commands unchanged.
    pub receipt: Option<Receipt>,
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
            .field("receipt", &self.receipt)
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
    /// `BDAT n LAST` and the raw message have been written. The reply is the acceptance:
    /// unlike `DATA`, `BDAT` has no intermediate `354` to wait for first.
    Bdat,
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
    /// Wire form of the envelope, chosen at the `EHLO` that precedes `AUTH`.
    ///
    /// Absent until then. The pre-TLS `EHLO` does not set it: the extensions
    /// that decide the encoding are the ones still in force once TLS is up,
    /// which is also when the transfer checks run. `MAIL FROM` and `RCPT TO`
    /// both read this, so the parameter and the address bytes are one decision.
    envelope: Option<PreparedEnvelope>,
}

impl fmt::Debug for SmtpSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpSession")
            .field("submission", &self.submission)
            .field("phase", &self.phase)
            .field("extensions", &self.extensions)
            .field("mechanism", &self.mechanism)
            .field("envelope", &self.envelope)
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
            envelope: None,
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
            self.envelope.as_ref(),
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
                envelope,
            } => {
                if let Some(mech) = mechanism {
                    self.mechanism = Some(mech);
                }
                // `None` leaves a decision already stored. RCPT and DATA also
                // come back as Continue, and clearing here would make the next
                // recipient forget the form chosen at EHLO.
                if let Some(envelope) = envelope {
                    self.envelope = Some(envelope);
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
        if let Some(prev) = expected
            && prev != parsed.code
        {
            return Err(ProtoError::Malformed(format!(
                "reply code changed from {prev} to {}",
                parsed.code
            )));
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
        // The keyword alone. A parameter, were a server to send one, is not
        // part of the token and must not hide the extension.
        "SMTPUTF8" => ext.smtputf8 = Advertised::Offered,
        "DSN" => ext.dsn = Advertised::Offered,
        "CHUNKING" => ext.chunking = Advertised::Offered,
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
        /// Set by the submitting `EHLO`. Later steps pass `None` so applying
        /// them does not discard the decision `MAIL FROM` is waiting to use.
        envelope: Option<PreparedEnvelope>,
    },
    Done(SmtpReply),
}

fn decide(
    phase: &Phase,
    reply: &ServerReply,
    sub: &Submission,
    ext: &EhloExtensions,
    mech: Option<SaslMech>,
    envelope: Option<&PreparedEnvelope>,
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
        | Phase::AuthXoauth2Sent => on_auth(phase, reply, sub, envelope),
        Phase::MailFrom => on_mail_from(reply, envelope),
        Phase::Rcpt(index) => on_rcpt(*index, reply, sub, envelope),
        Phase::Data => on_data(reply, sub),
        Phase::Body => on_body(reply),
        Phase::Bdat => on_bdat(reply),
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
    // Same moment as the transfer checks: the extensions are final, and no
    // envelope command has been written, so a refusal never becomes MAIL FROM.
    // DSN and CHUNKING are decided here too, from this EHLO and not the one
    // that preceded STARTTLS.
    let envelope = prepare_envelope(ext, &sub.mail_from, &sub.recipients, sub.receipt.as_ref())?;
    let mech = choose_mech(&ext.auth, &sub.sasl, &sub.credential)?;
    let (next, command) = auth_command(mech, sub)?;
    Ok(continue_with_envelope(
        next,
        vec![command],
        Some(mech),
        Some(envelope),
    ))
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
        envelope: None,
    })
}

fn on_auth(
    phase: &Phase,
    reply: &ServerReply,
    sub: &Submission,
    envelope: Option<&PreparedEnvelope>,
) -> Result<Outcome, ProtoError> {
    match phase {
        Phase::AuthPlain => {
            if reply.code == 334 {
                let pass = password(&sub.credential)?;
                let line = cmd(&b64(&plain_raw(&sub.username, pass)));
                return Ok(continue_with(Phase::AuthPlainSent, vec![line], None));
            }
            finish_auth(reply, sub, envelope)
        }
        Phase::AuthPlainSent => finish_auth(reply, sub, envelope),
        Phase::AuthXoauth2 => {
            if reply.code == 334 {
                // The server rejected the bearer token and is waiting for an empty line
                // before it sends the final 535. Sending another copy of the token here
                // would put it on the wire twice.
                return Ok(continue_with(Phase::AuthXoauth2Sent, vec![cmd("")], None));
            }
            finish_auth(reply, sub, envelope)
        }
        Phase::AuthXoauth2Sent => finish_auth(reply, sub, envelope),
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
        Phase::AuthLoginSent => finish_auth(reply, sub, envelope),
        _ => Err(ProtoError::Malformed(
            "authentication reply in a non-auth state".into(),
        )),
    }
}

fn finish_auth(
    reply: &ServerReply,
    sub: &Submission,
    envelope: Option<&PreparedEnvelope>,
) -> Result<Outcome, ProtoError> {
    if is_success(reply.code) {
        send_mail_from(sub, envelope)
    } else {
        Err(auth_negative(reply))
    }
}

fn on_mail_from(
    reply: &ServerReply,
    envelope: Option<&PreparedEnvelope>,
) -> Result<Outcome, ProtoError> {
    expect_success(reply)?;
    send_rcpt(require_envelope(envelope)?, 0)
}

fn on_rcpt(
    index: usize,
    reply: &ServerReply,
    sub: &Submission,
    envelope: Option<&PreparedEnvelope>,
) -> Result<Outcome, ProtoError> {
    expect_success(reply)?;
    let envelope = require_envelope(envelope)?;
    let next = index + 1;
    if next < envelope.recipients.len() {
        send_rcpt(envelope, next)
    } else {
        send_body(envelope, &sub.message)
    }
}

fn on_bdat(reply: &ServerReply) -> Result<Outcome, ProtoError> {
    // The reply to `BDAT LAST` is the acceptance, the same role as the reply
    // to the terminating dot. A 2xx is recorded on `SmtpReply.accepted`;
    // anything else is the refusal `on_body` already returns for a refused DATA.
    on_body(reply)
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
    continue_with_envelope(next, commands, mechanism, None)
}

fn continue_with_envelope(
    next: Phase,
    commands: Vec<Vec<u8>>,
    mechanism: Option<SaslMech>,
    envelope: Option<PreparedEnvelope>,
) -> Outcome {
    let mut needs: Vec<_> = commands.into_iter().map(IoNeed::Write).collect();
    needs.push(IoNeed::Read);
    Outcome::Continue {
        next,
        mechanism,
        needs,
        envelope,
    }
}

fn send_mail_from(
    sub: &Submission,
    envelope: Option<&PreparedEnvelope>,
) -> Result<Outcome, ProtoError> {
    let envelope = require_envelope(envelope)?;
    let line = mail_from_command(envelope, &sub.message);
    Ok(continue_with(Phase::MailFrom, vec![cmd(&line)], None))
}

fn send_rcpt(envelope: &PreparedEnvelope, index: usize) -> Result<Outcome, ProtoError> {
    let Some(addr) = envelope.recipients.get(index) else {
        return Err(ProtoError::Refused {
            kind: Refusal::Permanent,
            text: "no recipients".into(),
        });
    };
    Ok(continue_with(
        Phase::Rcpt(index),
        vec![cmd(&rcpt_command(envelope, addr))],
        None,
    ))
}

/// After the last recipient, `BDAT` when the submitting EHLO offered `CHUNKING`,
/// otherwise the `DATA` command this client has always sent.
fn send_body(envelope: &PreparedEnvelope, message: &[u8]) -> Result<Outcome, ProtoError> {
    if envelope.chunking == Advertised::Offered {
        Ok(send_bdat(message))
    } else {
        Ok(continue_with(Phase::Data, vec![cmd("DATA")], None))
    }
}

fn send_bdat(message: &[u8]) -> Outcome {
    continue_with(Phase::Bdat, bdat_writes(message), None)
}

/// The envelope was prepared at `EHLO`, which is before `AUTH` and therefore
/// before either envelope command. Reaching those commands without one means
/// the phase machine skipped that step.
fn require_envelope(envelope: Option<&PreparedEnvelope>) -> Result<&PreparedEnvelope, ProtoError> {
    envelope
        .ok_or_else(|| ProtoError::Malformed("envelope was not prepared before MAIL FROM".into()))
}

fn check_transfer(ext: &EhloExtensions, message: &[u8]) -> Result<(), ProtoError> {
    if let SizeLimit::Limited(max) = ext.size {
        let size = transfer_octets(ext, message);
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

/// Whether `MAIL FROM` carries the `SMTPUTF8` parameter.
///
/// An ASCII envelope omits it even when the server offered the extension.
/// Adding it would change bytes that servers already accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Utf8Parameter {
    Omit,
    Send,
}

/// Envelope addresses in the form they will be written.
///
/// Fixed once, at the `EHLO` that precedes submission, and then used for both
/// `MAIL FROM` and every `RCPT TO`. `receipt` and `chunking` are part of that
/// same decision: the parameters and the body path cannot drift apart from
/// the SIZE check, which runs at that EHLO.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedEnvelope {
    mail_from: String,
    recipients: Vec<String>,
    utf8: Utf8Parameter,
    /// DSN parameters to write. `None` when the caller asked for none, and
    /// also when they asked and the server did not offer `DSN`.
    receipt: Option<Receipt>,
    /// Whether this EHLO offered `CHUNKING`. `BDAT` is used only then.
    chunking: Advertised,
}

/// Choose the wire form of the envelope before `MAIL FROM` is written.
///
/// ASCII addresses are copied unchanged. A non-ASCII address is sent as UTF-8,
/// with the parameter, only when the server offered `SMTPUTF8`. Otherwise the
/// domain — everything after the last `@` — becomes an A-label. A non-ASCII
/// local part has no such encoding, and neither does a domain IDNA rejects:
/// both are [`ProtoError::Unsupported`] naming the address.
fn prepare_envelope(
    ext: &EhloExtensions,
    mail_from: &str,
    recipients: &[String],
    receipt: Option<&Receipt>,
) -> Result<PreparedEnvelope, ProtoError> {
    // A receipt is a request, never a precondition. When the server did not
    // offer `DSN` the parameters are omitted and submission continues;
    // refusing here would drop a message only because a notification could
    // not be asked for.
    let receipt = dsn_to_send(ext, receipt);
    let chunking = ext.chunking;
    if !envelope_is_ascii(mail_from, recipients) && ext.smtputf8 != Advertised::Offered {
        return Ok(PreparedEnvelope {
            mail_from: encode_without_smtputf8(mail_from)?,
            recipients: recipients
                .iter()
                .map(|addr| encode_without_smtputf8(addr))
                .collect::<Result<Vec<_>, _>>()?,
            utf8: Utf8Parameter::Omit,
            receipt,
            chunking,
        });
    }
    // One non-ASCII address puts the parameter on the whole transaction.
    // RFC 6531 attaches `SMTPUTF8` to `MAIL FROM`, not to each recipient.
    let utf8 = if envelope_is_ascii(mail_from, recipients) {
        Utf8Parameter::Omit
    } else {
        Utf8Parameter::Send
    };
    Ok(PreparedEnvelope {
        mail_from: mail_from.to_owned(),
        recipients: recipients.to_vec(),
        utf8,
        receipt,
        chunking,
    })
}

/// `Some` only when the caller asked for a receipt and this EHLO offered `DSN`.
fn dsn_to_send(ext: &EhloExtensions, receipt: Option<&Receipt>) -> Option<Receipt> {
    if ext.dsn == Advertised::Offered {
        receipt.cloned()
    } else {
        None
    }
}

/// True when every envelope address is ASCII.
///
/// The check is on the whole address. A match against a fragment would treat
/// an ASCII domain as permission to send a non-ASCII local part.
fn envelope_is_ascii(mail_from: &str, recipients: &[String]) -> bool {
    mail_from.is_ascii() && recipients.iter().all(|addr| addr.is_ascii())
}

/// `MAIL FROM`, without CRLF.
///
/// Parameters stay in a fixed order — `BODY=8BITMIME`, then `SMTPUTF8`, then
/// `RET`, then `ENVID` — so a command this client already sends is a prefix
/// of the same command once a later extension is added. `RET` and `ENVID`
/// are present only when [`dsn_to_send`] kept the receipt.
fn mail_from_command(envelope: &PreparedEnvelope, message: &[u8]) -> String {
    let mut line = format!("MAIL FROM:<{}>", envelope.mail_from);
    if has_high_bit(message) {
        line.push_str(" BODY=8BITMIME");
    }
    if envelope.utf8 == Utf8Parameter::Send {
        line.push_str(" SMTPUTF8");
    }
    if let Some(receipt) = &envelope.receipt {
        line.push(' ');
        line.push_str(ret_parameter(&receipt.ret));
        if let Some(envid) = &receipt.envid {
            line.push_str(" ENVID=");
            line.push_str(&xtext(envid.as_bytes()));
        }
    }
    line
}

/// `RET=HDRS` or `RET=FULL`.
fn ret_parameter(ret: &Return) -> &'static str {
    match ret {
        Return::Headers => "RET=HDRS",
        Return::Full => "RET=FULL",
    }
}

/// `RCPT TO`, without CRLF.
///
/// `NOTIFY` lists the requested events. `ORCPT` is added only when the
/// address is ASCII. A UTF-8 original recipient needs RFC 6533
/// `utf-8-addr-xtext`, which is a different encoding from the xtext used
/// here, so a non-ASCII recipient keeps `NOTIFY` and omits `ORCPT`.
fn rcpt_command(envelope: &PreparedEnvelope, addr: &str) -> String {
    let mut line = format!("RCPT TO:<{addr}>");
    if let Some(receipt) = &envelope.receipt {
        line.push(' ');
        line.push_str(&notify_parameter(&receipt.notify));
        if addr.is_ascii() {
            line.push_str(" ORCPT=rfc822;");
            line.push_str(&xtext(addr.as_bytes()));
        }
    }
    line
}

/// `NOTIFY=NEVER`, or the true events in the order success, failure, delay.
///
/// An [`Notify::On`] with every flag false is `NEVER`. RFC 3461's value is
/// a non-empty list or `NEVER`, and an empty list is not a value on the wire.
fn notify_parameter(notify: &Notify) -> String {
    let Notify::On {
        success,
        failure,
        delay,
    } = notify
    else {
        return "NOTIFY=NEVER".to_owned();
    };
    let mut events = Vec::new();
    if *success {
        events.push("SUCCESS");
    }
    if *failure {
        events.push("FAILURE");
    }
    if *delay {
        events.push("DELAY");
    }
    if events.is_empty() {
        return "NOTIFY=NEVER".to_owned();
    }
    format!("NOTIFY={}", events.join(","))
}

/// RFC 3461 xtext.
///
/// Bytes from `!` to `~` except `+` and `=` pass through. Every other byte,
/// including those two and each octet of a UTF-8 character, is `+` and two
/// uppercase hex digits. Characters are not the unit: the RFC encodes bytes.
fn xtext(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        if is_xchar(byte) {
            out.push(char::from(byte));
        } else {
            push_hex(&mut out, byte);
        }
    }
    out
}

/// Printable ASCII except `+` and `=`, which introduce the hex form.
fn is_xchar(byte: u8) -> bool {
    byte.is_ascii_graphic() && byte != b'+' && byte != b'='
}

fn push_hex(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('+');
    out.push(char::from(HEX[usize::from(byte >> 4)]));
    out.push(char::from(HEX[usize::from(byte & 0x0f)]));
}

/// Rewrite one address for a server that did not offer `SMTPUTF8`.
///
/// An address with no `@` has no domain to encode. ASCII is already safe to
/// send; anything else is the same refusal as a non-ASCII local part.
fn encode_without_smtputf8(addr: &str) -> Result<String, ProtoError> {
    let Some((local, domain)) = split_mailbox(addr) else {
        if addr.is_ascii() {
            return Ok(addr.to_owned());
        }
        return Err(local_part_needs_smtputf8(addr));
    };
    if !local.is_ascii() {
        return Err(local_part_needs_smtputf8(addr));
    }
    let ascii = idna::domain_to_ascii(domain).map_err(|_| domain_not_idna(addr))?;
    Ok(format!("{local}@{ascii}"))
}

/// Split `local@domain` at the last `@`.
///
/// The domain is what IDNA applies to, and a stray `@` earlier in the local
/// part must not be read as the start of that domain.
fn split_mailbox(addr: &str) -> Option<(&str, &str)> {
    let at = addr.rfind('@')?;
    Some((&addr[..at], &addr[at + 1..]))
}

fn local_part_needs_smtputf8(addr: &str) -> ProtoError {
    ProtoError::Unsupported(format!(
        "SMTPUTF8, which {addr} needs: its local part is not ASCII"
    ))
}

fn domain_not_idna(addr: &str) -> ProtoError {
    ProtoError::Unsupported(format!(
        "the address {addr}, whose domain is not valid IDNA"
    ))
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
/// `CramMd5` is absent on purpose: neither account we target offers it (the POP3 spike found
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
        Credential::OpenPgp(_) => Err(ProtoError::Unsupported(
            "an OpenPGP key is not a sign-in credential".into(),
        )),
    }
}

fn access_token(cred: &Credential) -> Result<&str, ProtoError> {
    match cred {
        Credential::OAuth { access, .. } => Ok(access.as_str()),
        Credential::Password(_) => Err(ProtoError::Unsupported(
            "XOAUTH2 with a password credential".into(),
        )),
        Credential::OpenPgp(_) => Err(ProtoError::Unsupported(
            "an OpenPGP key is not a sign-in credential".into(),
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

/// Octets `SIZE` is compared with, for the path the submitting EHLO chose.
///
/// `CHUNKING` transmits the raw message, so the figure is its length. Without
/// it the figure is the dot-stuffed body, which is what `DATA` actually sends
/// and can be longer than the message the caller handed us.
fn transfer_octets(ext: &EhloExtensions, message: &[u8]) -> u64 {
    if ext.chunking == Advertised::Offered {
        message.len() as u64
    } else {
        transmitted_octets(message)
    }
}

/// The `BDAT` command and the message that follows it.
///
/// RFC 3030 makes the octet count authoritative: the server reads exactly
/// that many bytes and does not look for a terminating dot. The message is
/// therefore copied unchanged. A missing final CRLF stays missing — adding
/// one would change both the count and the content — and a line that begins
/// with `.` is not stuffed, because stuffing would make the count describe
/// different bytes from the ones the server will store.
fn bdat_writes(message: &[u8]) -> Vec<Vec<u8>> {
    let line = cmd(&format!("BDAT {} LAST", message.len()));
    vec![line, message.to_vec()]
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
        Credential::OpenPgp(key) => push_secret(&mut out, key),
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
            receipt: None,
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
        assert_eq!(ext.smtputf8, Advertised::Absent);
        assert_eq!(ext.dsn, Advertised::Absent);
        assert_eq!(ext.chunking, Advertised::Absent);
    }

    #[test]
    fn ehlo_dsn_and_chunking_are_whole_keywords() {
        let offered = parse_ehlo(&ServerReply {
            code: 250,
            lines: vec!["hello".into(), "dsn".into(), "Chunking extra".into()],
        });
        assert_eq!(offered.dsn, Advertised::Offered);
        assert_eq!(offered.chunking, Advertised::Offered);
        // A longer token that merely contains the word is a different extension.
        let absent = parse_ehlo(&ServerReply {
            code: 250,
            lines: vec!["XDSN".into(), "CHUNKINGS".into(), "BDAT".into()],
        });
        assert_eq!(absent.dsn, Advertised::Absent);
        assert_eq!(absent.chunking, Advertised::Absent);
    }

    #[test]
    fn ehlo_smtputf8_is_recognised_without_regard_to_case() {
        let offered = parse_ehlo(&ServerReply {
            code: 250,
            lines: vec!["hello".into(), "Smtputf8".into(), "SMTPUTF8 ignored".into()],
        });
        assert_eq!(offered.smtputf8, Advertised::Offered);
        let absent = parse_ehlo(&ServerReply {
            code: 250,
            lines: vec!["8BITMIME".into()],
        });
        assert_eq!(absent.smtputf8, Advertised::Absent);
    }

    #[test]
    fn envelope_decision_is_ascii_utf8_or_an_a_label() {
        let offered = EhloExtensions {
            smtputf8: Advertised::Offered,
            ..EhloExtensions::default()
        };
        let absent = EhloExtensions::default();
        let cases: &[(&str, &EhloExtensions, &str, &[&str], PreparedEnvelope)] = &[
            (
                "ascii stays byte for byte even when the extension is offered",
                &offered,
                "Ada@Example.COM",
                &["bob@example.com"],
                PreparedEnvelope {
                    mail_from: "Ada@Example.COM".into(),
                    recipients: vec!["bob@example.com".into()],
                    utf8: Utf8Parameter::Omit,
                    receipt: None,
                    chunking: Advertised::Absent,
                },
            ),
            (
                "a utf-8 address is sent unchanged when SMTPUTF8 is offered",
                &offered,
                "用户@例子.广告",
                &["bob@example.com"],
                PreparedEnvelope {
                    mail_from: "用户@例子.广告".into(),
                    recipients: vec!["bob@example.com".into()],
                    utf8: Utf8Parameter::Send,
                    receipt: None,
                    chunking: Advertised::Absent,
                },
            ),
            (
                "one utf-8 recipient puts the parameter on the whole transaction",
                &offered,
                "ada@example.com",
                &["bob@bücher.example"],
                PreparedEnvelope {
                    mail_from: "ada@example.com".into(),
                    recipients: vec!["bob@bücher.example".into()],
                    utf8: Utf8Parameter::Send,
                    receipt: None,
                    chunking: Advertised::Absent,
                },
            ),
            (
                "a utf-8 domain becomes an A-label, split at the last @",
                &absent,
                "ann@bob@bücher.example",
                &["cara@例子.广告"],
                PreparedEnvelope {
                    mail_from: "ann@bob@xn--bcher-kva.example".into(),
                    recipients: vec!["cara@xn--fsqu00a.xn--4rr70v".into()],
                    utf8: Utf8Parameter::Omit,
                    receipt: None,
                    chunking: Advertised::Absent,
                },
            ),
        ];
        for (name, ext, mail_from, recipients, expect) in cases {
            let got = prepare_envelope(
                ext,
                mail_from,
                &recipients
                    .iter()
                    .map(|addr| (*addr).to_owned())
                    .collect::<Vec<_>>(),
                None,
            )
            .unwrap_or_else(|err| panic!("{name}: {err}"));
            assert_eq!(&got, expect, "{name}");
        }

        let local = prepare_envelope(
            &absent,
            "jörg@example.com",
            &["bob@example.com".into()],
            None,
        )
        .unwrap_err();
        assert_eq!(
            local.to_string(),
            "server does not support SMTPUTF8, which jörg@example.com needs: its local part is not ASCII"
        );
        // U+11C3A is disallowed by IDNA. The local part is ASCII, so the
        // refusal is the domain and not the local-part case. An all-ASCII
        // address is not put through IDNA: that path stays byte-for-byte.
        let rejected = "ada@\u{11C3A}";
        let domain =
            prepare_envelope(&absent, rejected, &["bob@example.com".into()], None).unwrap_err();
        assert_eq!(
            domain.to_string(),
            format!(
                "server does not support the address {rejected}, whose domain is not valid IDNA"
            )
        );

        let both = PreparedEnvelope {
            mail_from: "用户@例子.广告".into(),
            recipients: vec!["bob@example.com".into()],
            utf8: Utf8Parameter::Send,
            receipt: None,
            chunking: Advertised::Absent,
        };
        assert_eq!(
            mail_from_command(&both, "café".as_bytes()),
            "MAIL FROM:<用户@例子.广告> BODY=8BITMIME SMTPUTF8"
        );
    }

    #[test]
    fn xtext_escapes_plus_equals_space_and_non_ascii_bytes() {
        let cases: &[(&[u8], &str)] = &[
            (b"bob@example.com", "bob@example.com"),
            (b"q+1=x", "q+2B1+3Dx"),
            (b"+", "+2B"),
            (b"=", "+3D"),
            (b" ", "+20"),
            ("é".as_bytes(), "+C3+A9"),
        ];
        for (input, expect) in cases {
            assert_eq!(xtext(input), *expect, "{input:?}");
        }
    }

    #[test]
    fn notify_lists_only_the_true_events_and_an_empty_set_is_never() {
        let cases = [
            (Notify::Never, "NOTIFY=NEVER"),
            (
                Notify::On {
                    success: false,
                    failure: false,
                    delay: false,
                },
                "NOTIFY=NEVER",
            ),
            (
                Notify::On {
                    success: true,
                    failure: true,
                    delay: false,
                },
                "NOTIFY=SUCCESS,FAILURE",
            ),
            (
                Notify::On {
                    success: true,
                    failure: false,
                    delay: true,
                },
                "NOTIFY=SUCCESS,DELAY",
            ),
            (
                Notify::On {
                    success: false,
                    failure: true,
                    delay: true,
                },
                "NOTIFY=FAILURE,DELAY",
            ),
            (
                Notify::On {
                    success: true,
                    failure: true,
                    delay: true,
                },
                "NOTIFY=SUCCESS,FAILURE,DELAY",
            ),
        ];
        for (notify, expect) in cases {
            assert_eq!(notify_parameter(&notify), expect, "{notify:?}");
        }
    }

    #[test]
    fn mail_from_parameters_follow_body_utf8_ret_envid() {
        let envelope = PreparedEnvelope {
            mail_from: "用户@例子.广告".into(),
            recipients: vec!["bob@example.com".into()],
            utf8: Utf8Parameter::Send,
            receipt: Some(Receipt {
                notify: Notify::On {
                    success: true,
                    failure: false,
                    delay: false,
                },
                ret: Return::Headers,
                envid: Some("q+1=x".into()),
            }),
            chunking: Advertised::Absent,
        };
        assert_eq!(
            mail_from_command(&envelope, "café".as_bytes()),
            "MAIL FROM:<用户@例子.广告> BODY=8BITMIME SMTPUTF8 RET=HDRS ENVID=q+2B1+3Dx"
        );
        assert_eq!(
            rcpt_command(&envelope, "bob+tag@example.com"),
            "RCPT TO:<bob+tag@example.com> NOTIFY=SUCCESS ORCPT=rfc822;bob+2Btag@example.com"
        );
        // The address itself is not ASCII, so ORCPT is omitted rather than
        // encoded with the RFC 3461 rules.
        assert_eq!(
            rcpt_command(&envelope, "收件人@例子.广告"),
            "RCPT TO:<收件人@例子.广告> NOTIFY=SUCCESS"
        );
    }

    #[test]
    fn a_receipt_without_dsn_is_dropped_and_does_not_fail() {
        let receipt = Receipt {
            notify: Notify::Never,
            ret: Return::Full,
            envid: Some("id".into()),
        };
        let dropped = prepare_envelope(
            &EhloExtensions::default(),
            "ada@example.com",
            &["bob@example.com".into()],
            Some(&receipt),
        )
        .expect("a receipt is not a precondition");
        assert_eq!(dropped.receipt, None);
        assert_eq!(
            mail_from_command(&dropped, b"hi"),
            "MAIL FROM:<ada@example.com>"
        );
        assert_eq!(
            rcpt_command(&dropped, "bob@example.com"),
            "RCPT TO:<bob@example.com>"
        );

        let offered = EhloExtensions {
            dsn: Advertised::Offered,
            ..EhloExtensions::default()
        };
        let kept = prepare_envelope(
            &offered,
            "ada@example.com",
            &["bob@example.com".into()],
            Some(&receipt),
        )
        .expect("ascii");
        assert_eq!(kept.receipt, Some(receipt));
    }

    #[test]
    fn bdat_copies_the_message_and_counts_raw_octets() {
        let message = b"Hi.\r\n.dot\r\nno final break";
        let writes = bdat_writes(message);
        assert_eq!(
            writes[0],
            format!("BDAT {} LAST\r\n", message.len()).into_bytes()
        );
        assert_eq!(writes[1], message);
        // Stuffing would have turned the second line into "..dot".
        assert!(writes[1].windows(5).any(|window| window == b"\n.dot"));
        assert!(!writes[1].windows(6).any(|window| window == b"\n..dot"));

        let dotted = b".x\r\n";
        assert_eq!(transmitted_octets(dotted), 5);
        let chunked = EhloExtensions {
            chunking: Advertised::Offered,
            size: SizeLimit::Limited(4),
            ..EhloExtensions::default()
        };
        assert!(check_transfer(&chunked, dotted).is_ok());
        let data_path = EhloExtensions {
            size: SizeLimit::Limited(4),
            ..EhloExtensions::default()
        };
        let err = check_transfer(&data_path, dotted).unwrap_err();
        assert!(
            err.to_string().contains("5 octets"),
            "DATA must count the stuffed body, got {err}"
        );
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
        let outcome = on_auth(&Phase::AuthXoauth2, &reply, &sub, None).expect("challenge");
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
        assert!(rendered.contains("receipt"));
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
