//! IMAP4rev1 as a sans-I/O [`Machine`], parsing with `imap-proto`.
//!
//! Command serialisation is written here; response parsing is not, because parsing is the hard
//! half and `imap-proto` already does it including Gmail's `X-GM-*` attributes. `nom`'s
//! `Err::Incomplete` maps onto [`IoNeed::Read`], which is what makes a partial read a normal
//! step rather than a protocol failure.
//!
//! Three properties of real IMAP shape this design, and each cost some other client dearly:
//!
//! - **Untagged responses arrive unsolicited and out of order.** `EXISTS`, `EXPUNGE`, `FETCH`
//!   and `RECENT` may appear at any point, and RFC 3501 §7.4.1 explicitly permits `EXPUNGE`
//!   during a UID command. So every untagged response is collected wherever it lands rather
//!   than only where one is expected.
//! - **Sequence numbers renumber under you.** An untagged `EXPUNGE` shifts every later
//!   sequence number, which is how a client deletes the wrong message. Nothing here hands a
//!   sequence number to a caller; UIDs are the only identity that leaves this module.
//! - **`\Deleted` and `EXPUNGE` are absent on purpose.** Gmail routes `EXPUNGE` through a
//!   per-account setting that may be `deleteForever` and cannot be read over IMAP, so the
//!   commands simply do not exist here rather than being gated on something unobservable.

use crate::machine::{IoNeed, IoReady, Machine, Progress, ProtoError, Refusal};
use crate::mutf7;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use imap_proto::Response;
use mail_domain::{Credential, SaslMech};
use std::fmt;

/// One command to run, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImapCommand {
    Capability,
    /// `AUTHENTICATE XOAUTH2` with the session's credential.
    AuthenticateXoauth2,
    /// `LOGIN` with the session's credential.
    Login,
    /// `LIST "" "*"`, which is how folders and their special-use attributes are discovered.
    List,
    /// `SELECT`, or `EXAMINE` when read-only.
    Select {
        mailbox: String,
        read_only: bool,
    },
    /// `UID FETCH <set> <items>`.
    UidFetch {
        set: String,
        items: String,
    },
    /// `UID STORE <set> <what>`, e.g. `+FLAGS (\Seen)`.
    UidStore {
        set: String,
        what: String,
    },
    /// `UID SEARCH <criteria>`.
    UidSearch {
        criteria: String,
    },
    /// `UID COPY <set> <mailbox>`. Copy, never move-by-delete: see the module note.
    UidCopy {
        set: String,
        mailbox: String,
    },
    /// `UID MOVE <set> <mailbox>` (RFC 6851), where the server advertises `MOVE`.
    ///
    /// The only safe way to actually move a message. The alternative is `COPY`, then `\Deleted`,
    /// then `EXPUNGE` — and `EXPUNGE` is refused outright here, because Gmail may be configured
    /// to delete permanently and that setting cannot be read over IMAP. `MOVE` is atomic and
    /// names no flag, so it is not that dance in disguise; it is the primitive the dance was
    /// always a poor imitation of.
    UidMove {
        set: String,
        mailbox: String,
    },
    /// `APPEND <mailbox> (<flags>) {<n>}` followed by the message itself.
    ///
    /// The one command here that sends a literal, which is why it needs a phase of its own: the
    /// server answers `+` and only then may the bytes go. Writing them ahead of the
    /// continuation is how a client corrupts the next command, because a server that rejected
    /// the `APPEND` line is now reading a message as though it were commands.
    Append {
        mailbox: String,
        /// `\\Seen`, `\\Draft` and so on. Rendered verbatim inside the parentheses.
        flags: Vec<String>,
        raw: Vec<u8>,
    },
    /// `IDLE`, which parks until the server says something or the caller interrupts.
    Idle,
    Noop,
    Logout,
}

/// What a completed session saw.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImapTranscript {
    /// Every untagged response, in arrival order, with the command it arrived during.
    pub untagged: Vec<Untagged>,
    /// Capability atoms from the most recent `CAPABILITY` or capability status response.
    ///
    /// The *most recent*, because Gmail advertises a reduced list before authentication and a
    /// fuller one after: believing the first is how a CONDSTORE server looks like it has none.
    pub capabilities: Vec<String>,
}

/// One untagged response, kept as text.
///
/// Text rather than a typed tree because the caller needs the whole shape of a Gmail `FETCH`
/// including attributes this crate has no opinion about, and re-parsing at the edge beats a
/// lossy translation in the middle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Untagged {
    /// Which queued command was outstanding when this arrived.
    pub during: usize,
    /// The response as text, lossily, for the parts that are protocol vocabulary.
    ///
    /// Fine for `SEARCH`, `FETCH` attribute names and status codes — all ASCII. **Not** fine for
    /// a message body: `from_utf8_lossy` turns every 8-bit byte into U+FFFD, and `trim_end`
    /// eats trailing whitespace that is part of the message. Use [`Untagged::literal`] for
    /// anything that is mail rather than protocol.
    pub text: String,
    /// The response exactly as it arrived.
    pub raw: Vec<u8>,
}

impl Untagged {
    /// The bytes of this response's literal, if it has one.
    ///
    /// A `FETCH` carrying a body looks like `* 2 FETCH (UID 102 BODY[] {223}\r\n<223 bytes>)`:
    /// the count is authoritative and the trailing `)` closes the *response*, not the message.
    /// Handing the whole thing up as the body — which is what this crate used to do — appends
    /// that paren to every message fetched over IMAP and prepends the `* 2 FETCH (...)` header,
    /// which a lenient MIME parser then swallows without complaint.
    pub fn literal(&self) -> Option<&[u8]> {
        let open = self.raw.iter().rposition(|b| *b == b'{')?;
        let close = self.raw[open..].iter().position(|b| *b == b'}')? + open;
        let digits = std::str::from_utf8(&self.raw[open + 1..close]).ok()?;
        let len: usize = digits.parse().ok()?;
        // The literal begins after the CRLF that follows `{n}`.
        let start = close + 1;
        let start = match self.raw.get(start..start + 2) {
            Some(b"\r\n") => start + 2,
            _ => return None,
        };
        self.raw.get(start..start + len)
    }
}

/// The credential a session authenticates with.
#[derive(Clone, PartialEq, Eq)]
pub struct ImapAuth {
    pub username: String,
    pub credential: Credential,
    /// Mechanisms this account will use, most preferred first.
    pub sasl: Vec<SaslMech>,
}

// Hand-written: a derived Debug would put the password or bearer token into any log line that
// touches a session.
impl fmt::Debug for ImapAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImapAuth")
            .field("username", &self.username)
            .field("credential", &"<redacted>")
            .field("sasl", &self.sasl)
            .finish()
    }
}

/// Where the session is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// Waiting for the server greeting, which is itself an untagged response.
    Greeting,
    /// A command is outstanding; waiting for its tagged completion.
    Running {
        index: usize,
        tag: String,
    },
    /// `IDLE` was sent and the server has not yet answered with a continuation.
    IdlePending {
        index: usize,
        tag: String,
    },
    /// Parked in IDLE. Only bytes from the server or an interrupt move this.
    Idling {
        index: usize,
        tag: String,
    },
    /// `DONE` was sent; waiting for IDLE's tagged completion.
    IdleEnding {
        index: usize,
        tag: String,
    },
    /// `APPEND` was sent; the server has not yet asked for the literal.
    AppendPending {
        index: usize,
        tag: String,
        /// Held until the `+` arrives, then written in one go.
        body: Vec<u8>,
    },
    Finished,
}

/// A sans-I/O IMAP client.
pub struct ImapSession {
    auth: ImapAuth,
    commands: Vec<ImapCommand>,
    phase: Phase,
    buf: Vec<u8>,
    transcript: ImapTranscript,
    counter: u32,
}

impl ImapSession {
    /// A session that has not yet read the greeting.
    ///
    /// Rejects a credential containing CR, LF or NUL: those bytes would split a command, which
    /// is the shape of an IMAP injection.
    pub fn new(auth: ImapAuth, commands: Vec<ImapCommand>) -> Result<Self, ProtoError> {
        if forbidden(&auth.username) || credential_forbidden(&auth.credential) {
            return Err(ProtoError::Malformed(
                "credential contains CR, LF, or NUL".to_owned(),
            ));
        }
        Ok(Self {
            auth,
            commands,
            phase: Phase::Greeting,
            buf: Vec::new(),
            transcript: ImapTranscript::default(),
            counter: 0,
        })
    }

    /// Capabilities seen so far.
    pub fn capabilities(&self) -> &[String] {
        &self.transcript.capabilities
    }

    fn next_tag(&mut self) -> String {
        self.counter += 1;
        format!("a{:03}", self.counter)
    }

    /// Serialise the command at `index` and move to waiting for its completion.
    fn issue(&mut self, index: usize) -> Progress<ImapTranscript> {
        let Some(command) = self.commands.get(index).cloned() else {
            self.phase = Phase::Finished;
            return Progress::Done(std::mem::take(&mut self.transcript));
        };
        let tag = self.next_tag();
        let line = match self.render(&command, &tag) {
            Ok(line) => line,
            Err(e) => return self.fail(e),
        };
        self.phase = match command {
            ImapCommand::Idle => Phase::IdlePending {
                index,
                tag: tag.clone(),
            },
            // The bytes wait for the server's `+`; only the command line goes now.
            ImapCommand::Append { raw, .. } => Phase::AppendPending {
                index,
                tag: tag.clone(),
                body: raw,
            },
            _ => Phase::Running {
                index,
                tag: tag.clone(),
            },
        };
        Progress::Need(vec![IoNeed::Write(line), IoNeed::Read])
    }

    fn render(&self, command: &ImapCommand, tag: &str) -> Result<Vec<u8>, ProtoError> {
        let body = match command {
            ImapCommand::Capability => "CAPABILITY".to_owned(),
            ImapCommand::Noop => "NOOP".to_owned(),
            ImapCommand::Logout => "LOGOUT".to_owned(),
            ImapCommand::Idle => "IDLE".to_owned(),
            ImapCommand::List => "LIST \"\" \"*\"".to_owned(),
            ImapCommand::Login => {
                // RFC 3501 §6.2.3: a client MUST NOT issue LOGIN when the server advertises
                // LOGINDISABLED. Observed live on outlook.office365.com, which answers
                // `AUTH=XOAUTH2 LOGINDISABLED` — so this is not a hypothetical server, it is
                // the one two of this user's three accounts live on.
                //
                // The rule is worth more than protocol conformance. Without it the client sends
                // the user's password to something that has already said it will not accept
                // one, and then reports the rejection as though the credential were wrong.
                if has_capability(&self.transcript.capabilities, "LOGINDISABLED") {
                    return Err(ProtoError::Unsupported(
                        concat!(
                            "the server advertises LOGINDISABLED: it does not accept ",
                            "passwords on this connection, so one was not sent",
                        )
                        .to_owned(),
                    ));
                }
                let Credential::Password(password) = &self.auth.credential else {
                    return Err(ProtoError::Unsupported(
                        "LOGIN needs a password credential".to_owned(),
                    ));
                };
                format!("LOGIN {} {}", quoted(&self.auth.username), quoted(password))
            }
            ImapCommand::AuthenticateXoauth2 => {
                let Credential::OAuth { access, .. } = &self.auth.credential else {
                    return Err(ProtoError::Unsupported(
                        "XOAUTH2 needs an OAuth credential".to_owned(),
                    ));
                };
                if !self.auth.sasl.contains(&SaslMech::XOauth2) {
                    return Err(ProtoError::Unsupported(
                        "this account does not offer XOAUTH2".to_owned(),
                    ));
                }
                // Identical to Gmail's and to Exchange Online's, which is why adding Microsoft
                // is a preset row rather than a second code path.
                let initial = STANDARD.encode(format!(
                    "user={}\x01auth=Bearer {access}\x01\x01",
                    self.auth.username
                ));
                format!("AUTHENTICATE XOAUTH2 {initial}")
            }
            ImapCommand::Select { mailbox, read_only } => format!(
                "{} {}",
                if *read_only { "EXAMINE" } else { "SELECT" },
                // The wire name is modified UTF-7, and it is the identity: decode only for
                // display, never for addressing.
                quoted(&mutf7::encode(mailbox))
            ),
            ImapCommand::UidFetch { set, items } => {
                check_set(set)?;
                format!("UID FETCH {set} {items}")
            }
            ImapCommand::UidStore { set, what } => {
                check_set(set)?;
                format!("UID STORE {set} {what}")
            }
            ImapCommand::UidSearch { criteria } => format!("UID SEARCH {criteria}"),
            ImapCommand::UidCopy { set, mailbox } => {
                check_set(set)?;
                format!("UID COPY {set} {}", quoted(&mutf7::encode(mailbox)))
            }
            ImapCommand::Append {
                mailbox,
                flags,
                raw,
            } => {
                for flag in flags {
                    if forbidden(flag) {
                        return Err(ProtoError::Malformed(
                            "flag contains CR, LF, or NUL".to_owned(),
                        ));
                    }
                }
                // The length is of the bytes exactly as they will be written. A count that
                // disagrees with what follows desynchronises the connection: the server reads
                // the remainder of the message as commands, or waits for bytes never sent.
                format!(
                    "APPEND {} ({}) {{{}}}",
                    quoted(&mutf7::encode(mailbox)),
                    flags.join(" "),
                    raw.len()
                )
            }
            ImapCommand::UidMove { set, mailbox } => {
                check_set(set)?;
                format!("UID MOVE {set} {}", quoted(&mutf7::encode(mailbox)))
            }
        };
        Ok(format!("{tag} {body}\r\n").into_bytes())
    }

    fn fail(&mut self, e: ProtoError) -> Progress<ImapTranscript> {
        self.phase = Phase::Finished;
        Progress::Failed(e)
    }

    /// Consume whole responses from the buffer.
    fn drain(&mut self) -> Progress<ImapTranscript> {
        loop {
            if self.buf.is_empty() {
                return Progress::Need(vec![IoNeed::Read]);
            }
            // A continuation is not a response in imap-proto's grammar in every position, so
            // handle it before parsing: IDLE's `+` is the signal that we are actually idling.
            if self.buf.starts_with(b"+") {
                let Some(end) = find_line(&self.buf) else {
                    return Progress::Need(vec![IoNeed::Read]);
                };
                self.buf.drain(..end);
                match self.phase.clone() {
                    Phase::IdlePending { index, tag } => {
                        self.phase = Phase::Idling { index, tag };
                        return Progress::Need(vec![IoNeed::Read]);
                    }
                    // The server asked for the literal. Now, and not before.
                    Phase::AppendPending { index, tag, body } => {
                        self.phase = Phase::Running { index, tag };
                        let mut bytes = body;
                        bytes.extend_from_slice(b"\r\n");
                        return Progress::Need(vec![IoNeed::Write(bytes), IoNeed::Read]);
                    }
                    // A continuation anywhere else means the server wants data we did not
                    // plan to send. Saying so beats hanging.
                    _ => {
                        return self.fail(ProtoError::Malformed(
                            "unexpected continuation request".to_owned(),
                        ));
                    }
                }
            }

            let snapshot = self.buf.clone();
            let consumed = match imap_proto::parser::parse_response(&snapshot) {
                Ok((rest, response)) => {
                    let used = snapshot.len() - rest.len();
                    if let Some(progress) = self.absorb(&response, &snapshot[..used]) {
                        return progress;
                    }
                    used
                }
                // Not a failure: the rest is still on the wire.
                Err(nom::Err::Incomplete(_)) => return Progress::Need(vec![IoNeed::Read]),
                Err(_) => {
                    // The response as text, not as `nom`'s error. `{e:?}` prints the remaining
                    // input as a `Vec<u8>` of decimal numbers — five hundred integers where the
                    // answer is one line of IMAP, which is a diagnostic nobody can read and the
                    // only thing the field ever sees.
                    return self.fail(ProtoError::Malformed(format!(
                        "could not parse a response: {}",
                        excerpt(&snapshot)
                    )));
                }
            };
            self.buf.drain(..consumed);
        }
    }

    /// Record one response. Returns `Some` when the session should stop or move on.
    fn absorb(&mut self, response: &Response<'_>, raw: &[u8]) -> Option<Progress<ImapTranscript>> {
        let text = String::from_utf8_lossy(raw).trim_end().to_owned();
        let bytes = raw.to_vec();

        if let Response::Capabilities(atoms) = response {
            // Replace rather than extend: a later CAPABILITY describes the session we have now,
            // and Gmail's pre-auth list is a strict subset of its post-auth one.
            self.transcript.capabilities = atoms.iter().map(|c| format!("{c:?}")).collect();
        }

        match response {
            Response::Done { tag, status, .. } => {
                let tag = tag.0.clone();
                let expected = match &self.phase {
                    Phase::Running { tag, .. }
                    | Phase::IdlePending { tag, .. }
                    | Phase::Idling { tag, .. }
                    | Phase::IdleEnding { tag, .. }
                    // A tagged reply while waiting for the literal's `+` is the server
                    // refusing the APPEND — no such mailbox, over quota — which is an ordinary
                    // command failure and must read as one rather than as a desynchronised
                    // connection.
                    | Phase::AppendPending { tag, .. } => tag.clone(),
                    _ => String::new(),
                };
                if tag != expected {
                    return Some(self.fail(ProtoError::Malformed(format!(
                        "tagged {tag} while waiting for {expected}"
                    ))));
                }
                let index = match &self.phase {
                    Phase::Running { index, .. }
                    | Phase::IdlePending { index, .. }
                    | Phase::Idling { index, .. }
                    | Phase::IdleEnding { index, .. }
                    | Phase::AppendPending { index, .. } => *index,
                    _ => 0,
                };
                match status {
                    imap_proto::Status::Ok => {
                        // Consume before issuing the next command, or these bytes would be
                        // re-parsed on the next pass.
                        self.buf.drain(..raw.len());
                        Some(self.issue(index + 1))
                    }
                    imap_proto::Status::No => {
                        let refused = self.commands.get(index);
                        Some(self.fail(classify(&text, refused)))
                    }
                    _ => Some(self.fail(ProtoError::Malformed(format!("BAD: {text}")))),
                }
            }
            _ => {
                // Everything else is untagged and may arrive at any moment, solicited or not.
                let during = match &self.phase {
                    Phase::Running { index, .. }
                    | Phase::IdlePending { index, .. }
                    | Phase::Idling { index, .. }
                    | Phase::IdleEnding { index, .. } => *index,
                    _ => 0,
                };
                let news = is_news(&text);
                self.transcript.untagged.push(Untagged {
                    during,
                    text,
                    raw: bytes,
                });

                // The half of IDLE's contract that was documented and not implemented. `IDLE`
                // "parks until the server says something or the caller interrupts", and only
                // the interrupt was wired — so an `EXISTS` announcing new mail was filed away
                // and the session went on parking. IDLE was a sleep with extra steps.
                //
                // Ending it in protocol, with `DONE`, rather than dropping the socket: the
                // connection stays reusable and the server is not left wondering.
                if news {
                    if let Phase::Idling { index, tag } = self.phase.clone() {
                        self.phase = Phase::IdleEnding { index, tag };
                        return Some(Progress::Need(vec![
                            IoNeed::Write(b"DONE\r\n".to_vec()),
                            IoNeed::Read,
                        ]));
                    }
                }
                None
            }
        }
    }
}

/// Whether the server advertised a capability, matched as a whole atom.
///
/// `capabilities` holds `imap-proto`'s atoms rendered with `Debug`, so an entry looks like
/// `Atom("LOGINDISABLED")`, `Auth("XOAUTH2")` or `Imap4rev1`. Matching that with `contains` is
/// the substring mistake this codebase has now made three times in three disguises (FINDINGS
/// F67): `contains("MOVE")` is also true of a hypothetical `REMOVE`, and `contains("UID")` of
/// `UIDPLUS`.
///
/// So the name is compared against the whole atom, with the `Debug` wrapper accounted for
/// rather than matched through.
pub fn has_capability(capabilities: &[String], name: &str) -> bool {
    capabilities.iter().any(|c| {
        let c = c.trim();
        // `Atom("NAME")` / `Auth("NAME")` → NAME; anything else is compared as it stands.
        let inner = c
            .split_once('(')
            .and_then(|(_, rest)| rest.strip_suffix(')'))
            .map(|v| v.trim_matches('"'))
            .unwrap_or(c);
        inner.eq_ignore_ascii_case(name)
    })
}

/// Whether an untagged response is the server announcing something worth waking for.
///
/// `EXISTS` and `RECENT` are new mail; `EXPUNGE` and `FETCH` are a change made elsewhere, which
/// a watcher wants just as much — a message read on a phone should not wait for the next poll.
///
/// Matched on the response text rather than the parsed shape because that is how the rest of
/// this module treats untagged responses, and for the same reason: the caller needs shapes this
/// crate has no opinion about.
fn is_news(text: &str) -> bool {
    let upper = text.trim().to_uppercase();
    let Some(rest) = upper.strip_prefix("* ") else {
        return false;
    };
    // `* <n> EXISTS`, `* <n> EXPUNGE`, `* <n> FETCH (...)`, `* <n> RECENT`.
    let mut parts = rest.split_whitespace();
    let Some(first) = parts.next() else {
        return false;
    };
    if first.parse::<u64>().is_err() {
        return false;
    }
    matches!(
        parts.next(),
        Some("EXISTS") | Some("RECENT") | Some("EXPUNGE") | Some("FETCH")
    )
}

impl Machine for ImapSession {
    type Out = ImapTranscript;

    fn start(&mut self) -> Progress<ImapTranscript> {
        // The server speaks first. Reading rather than writing is the whole difference between
        // this and a protocol where the client opens.
        Progress::Need(vec![IoNeed::Read])
    }

    fn feed(&mut self, ready: IoReady) -> Progress<ImapTranscript> {
        match ready {
            IoReady::Eof => self.fail(ProtoError::UnexpectedEof),
            IoReady::TlsOpen | IoReady::Woke => Progress::Need(vec![IoNeed::Read]),
            IoReady::Interrupt => match self.phase.clone() {
                // The cancellation path IDLE exists for: wind down in protocol so the
                // connection can be reused, rather than being dropped mid-command.
                Phase::Idling { index, tag } => {
                    self.phase = Phase::IdleEnding { index, tag };
                    Progress::Need(vec![IoNeed::Write(b"DONE\r\n".to_vec()), IoNeed::Read])
                }
                // Never send DONE if we never entered IDLE: a server that advertised IDLE and
                // answered NO wedged Thunderbird for years on exactly this.
                _ => {
                    self.phase = Phase::Finished;
                    Progress::Done(std::mem::take(&mut self.transcript))
                }
            },
            IoReady::Bytes(bytes) => {
                self.buf.extend_from_slice(&bytes);
                if matches!(self.phase, Phase::Greeting) {
                    // The greeting is an untagged response; absorb it, then start commanding.
                    return match imap_proto::parser::parse_response(&self.buf) {
                        Ok((rest, response)) => {
                            let used = self.buf.len() - rest.len();
                            let text = String::from_utf8_lossy(&self.buf[..used])
                                .trim_end()
                                .to_owned();
                            if let Response::Capabilities(atoms) = &response {
                                self.transcript.capabilities =
                                    atoms.iter().map(|c| format!("{c:?}")).collect();
                            }
                            self.transcript.untagged.push(Untagged {
                                during: 0,
                                text,
                                raw: self.buf[..used].to_vec(),
                            });
                            self.buf.drain(..used);
                            self.issue(0)
                        }
                        Err(nom::Err::Incomplete(_)) => Progress::Need(vec![IoNeed::Read]),
                        Err(e) => self.fail(ProtoError::Malformed(format!(
                            "could not parse the greeting: {e:?}"
                        ))),
                    };
                }
                self.drain()
            }
        }
    }
}

/// An IMAP `NO` classified into something the outbox can act on.
///
/// `refused` is the command the server said `NO` to, and for a sign-in it decides the answer on
/// its own. Reading the prose instead only works on servers that word it the way Dovecot and
/// Gmail do: Exchange, Courier, UW-imapd and Zimbra all answer a wrong password with plain
/// `NO LOGIN failed.`, which matches none of the phrases below. That used to come out
/// `Refusal::Permanent`, so `retry()` said `Fatal` rather than `NeedsReauth`, the sync pass
/// never raised `needs_reauth`, and the poll loop treated the pass as a success and came back in
/// five minutes — 288 failed sign-ins a day against the user's own mail server, which is how an
/// account gets locked. A `NO` to a sign-in is a rejected credential whatever the wording.
fn classify(text: &str, refused: Option<&ImapCommand>) -> ProtoError {
    let upper = text.to_ascii_uppercase();
    // Checked before the command, because a server refusing a sign-in for rate limiting wants
    // backing off and not a new password — and it is the one thing that says so in the text.
    if upper.contains("[LIMIT]") || upper.contains("[OVERQUOTA]") || upper.contains("TOO MANY") {
        // Backing off is the remedy and hammering lengthens the lockout.
        return ProtoError::Throttled {
            reason: text.to_owned(),
            retry_after: None,
        };
    }
    let signing_in = matches!(
        refused,
        Some(ImapCommand::Login | ImapCommand::AuthenticateXoauth2)
    );
    if signing_in
        || upper.contains("[AUTHENTICATIONFAILED]")
        || upper.contains("[EXPIRED]")
        || upper.contains("[AUTHORIZATIONFAILED]")
        || upper.contains("INVALID CREDENTIALS")
    {
        return ProtoError::AuthRejected(text.to_owned());
    }
    ProtoError::Refused {
        kind: Refusal::Permanent,
        text: text.to_owned(),
    }
}

/// A UID set must be digits, ranges and commas — never a sequence number and never free text.
///
/// Checked because a set is built from stored values, and an unchecked one is a way to append
/// arbitrary command text.
fn check_set(set: &str) -> Result<(), ProtoError> {
    let ok = !set.is_empty()
        && set
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b':' || b == b',' || b == b'*');
    if ok {
        Ok(())
    } else {
        Err(ProtoError::Malformed(format!("{set:?} is not a UID set")))
    }
}

/// An IMAP quoted string, with `\` and `"` escaped.
/// The beginning of a response, printable, for an error a person has to read.
///
/// Lossy on purpose: bytes that are not text are exactly what one wants to see in a message
/// about bytes that did not parse. Truncated because a `FETCH` can be megabytes and an error is
/// not the place for them.
fn excerpt(bytes: &[u8]) -> String {
    const LIMIT: usize = 400;
    let head = &bytes[..bytes.len().min(LIMIT)];
    let text = String::from_utf8_lossy(head)
        .replace("\r\n", "\\r\\n")
        .replace('\r', "\\r")
        .replace('\n', "\\n");
    if bytes.len() > LIMIT {
        format!("{text}… ({} bytes)", bytes.len())
    } else {
        text
    }
}

fn quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if ch == '\\' || ch == '"' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

fn forbidden(value: &str) -> bool {
    value.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0))
}

fn credential_forbidden(credential: &Credential) -> bool {
    match credential {
        Credential::Password(p) => forbidden(p),
        Credential::OAuth {
            access, refresh, ..
        } => forbidden(access) || forbidden(refresh),
    }
}

fn find_line(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\r\n").map(|i| i + 2)
}

// The session never derives Debug: it holds ImapAuth.
impl fmt::Debug for ImapSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImapSession")
            .field("phase", &self.phase)
            .field("queued", &self.commands.len())
            .field("untagged", &self.transcript.untagged.len())
            .finish()
    }
}
