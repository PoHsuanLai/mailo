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
//! - **`\Deleted` and `EXPUNGE` serve two purposes only.** Gmail routes `EXPUNGE` through a
//!   per-account setting that may be `deleteForever` and cannot be read over IMAP, so nothing
//!   here moves or cleans up by expunging. One use is the user's own "Delete forever" of mail
//!   in the Trash or Spam folder (`ProtoOp::Destroy`), where deleting for good is the point:
//!   `UID STORE +FLAGS.SILENT (\Deleted)` then `UID EXPUNGE` of the same UIDs (RFC 4315), behind
//!   a [`ImapCommand::RequireCapability`] guard for `UIDPLUS`. The other is this client's own
//!   copies of a draft in the Drafts folder, replaced or sent (`ProtoOp::KeepDraft`,
//!   `ProtoOp::DropDraft`): the same two commands, on the UIDs a search for the draft's
//!   `Message-ID` found, and the expunge left out without `UIDPLUS`. A bare `EXPUNGE` does not
//!   exist here, because it removes every `\Deleted` message in the mailbox, other clients'
//!   included.

use crate::machine::{IoNeed, IoReady, Machine, Progress, ProtoError, Refusal};
use crate::mutf7;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use imap_proto::Response;
use mail_domain::SaslMech;
use porter_core::Credential;
use std::fmt;

/// The most a response may hold before it is complete: a message's bytes in a `FETCH`, with room
/// to spare. A server that sends more without ending the response is refused rather than let
/// grow the client's memory until it is killed.
pub const MAX_RESPONSE: usize = 256 * 1024 * 1024;

/// The most the answers to one command may hold together, as the transcript keeps them: a whole
/// batch of bodies in one `UID FETCH`, with room to spare. A server that keeps sending responses,
/// each under [`MAX_RESPONSE`], before it completes the command is refused at this rather than let
/// the transcript grow the client's memory until it is killed.
pub const MAX_TRANSCRIPT: usize = 1024 * 1024 * 1024;

/// How a mailbox is opened (RFC 3501 sections 6.3.1 and 6.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// `SELECT`: the session may change flags and expunge.
    ReadWrite,
    /// `EXAMINE`: the session cannot, and so cannot set `\Recent` or expunge by accident.
    ReadOnly,
}

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
    /// `ENABLE <capability>` (RFC 5161). Per connection, after authentication, before any
    /// command that relies on it.
    Enable(String),
    /// `SELECT`, or `EXAMINE` when read-only.
    Select {
        mailbox: String,
        access: Access,
        /// `(QRESYNC (<uidvalidity> <modseq>))`, only after `ENABLE QRESYNC` succeeded.
        qresync: Option<mail_domain::Resync>,
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
        /// A `NO` or `BAD` to it is passed over and the walk goes on: for a keyword that only
        /// informs (`$Junk`), which some servers refuse (Exchange answers `BAD`), and which must
        /// not cancel the move it travels with.
        tolerate: bool,
    },
    /// `UID SEARCH <criteria>`.
    UidSearch {
        criteria: String,
    },
    /// `UID SEARCH` for a search typed here ([`crate::search::imap`]): with `RETURN (COUNT ALL)`
    /// where the server has `ESEARCH` (RFC 4731), `CHARSET UTF-8` where a string is not ASCII,
    /// and each such string a literal. Rendered when it is issued, from the capabilities the
    /// session has seen by then, so a walk that wants either asks for `CAPABILITY` first.
    Search {
        keys: Vec<crate::search::imap::SearchKey>,
    },
    /// `UID COPY <set> <mailbox>`. Copy, never move-by-delete: see the module note.
    UidCopy {
        set: String,
        mailbox: String,
    },
    /// `UID MOVE <set> <mailbox>` (RFC 6851), where the server advertises `MOVE`.
    ///
    /// The only safe way to actually move a message. The alternative is `COPY`, then `\Deleted`,
    /// then `EXPUNGE` — and expunging is never used to move, because Gmail may be configured
    /// to delete permanently and that setting cannot be read over IMAP. `MOVE` is atomic and
    /// names no flag, so it is not that dance in disguise; it is the primitive the dance was
    /// always a poor imitation of.
    UidMove {
        set: String,
        mailbox: String,
    },
    /// `APPEND <mailbox> (<flags>) ["<date>"] {<n>}` followed by the message itself.
    ///
    /// The one command here that sends a literal, which is why it needs a phase of its own: the
    /// server answers `+` and only then may the bytes go. Writing them ahead of the
    /// continuation is how a client corrupts the next command, because a server that rejected
    /// the `APPEND` line is now reading a message as though it were commands.
    ///
    /// Unless the server has said it will not ask: with `LITERAL+` (RFC 7888), or `LITERAL-`
    /// and a message of at most 4096 bytes, the literal is written `{n+}` and follows the line
    /// at once. That saves a round trip per message. Decided from the capabilities this session
    /// has seen, so a walk that wants it asks for `CAPABILITY` first.
    Append {
        mailbox: String,
        /// `\\Seen`, `\\Draft` and so on. Rendered verbatim inside the parentheses.
        flags: Vec<String>,
        /// The internal date to give the message; the server uses now when absent.
        date: Option<chrono::DateTime<chrono::Utc>>,
        raw: Vec<u8>,
    },
    /// `UID EXPUNGE <set>` (RFC 4315 §2.1): expunge exactly these UIDs, and no other message
    /// marked `\Deleted` in the mailbox. Only after `\Deleted` was set on the same set for a
    /// "Delete forever" in Trash or Spam, and only behind [`ImapCommand::RequireCapability`] for
    /// `UIDPLUS`; or as [`ImapCommand::UidExpungeFound`] decides. See the module note.
    UidExpunge {
        set: String,
    },
    /// `UID STORE <found> <what>`, where `found` is every UID the last
    /// [`ImapCommand::UidSearch`] before it in the walk found; nothing at all when it found none.
    ///
    /// Decided inside the walk for the reason [`ImapCommand::RequireEmpty`] is: what to act on is
    /// what the search on this connection answered, and a later walk on another would ask again
    /// and might be answered differently. Only the search's own `* SEARCH` lines are read, so
    /// nothing a message says can add a UID to the set.
    UidStoreFound {
        what: String,
    },
    /// `UID EXPUNGE <found>`, for the same UIDs as [`ImapCommand::UidStoreFound`], where the
    /// server has `UIDPLUS` or IMAP4rev2; nothing at all where it has neither, or the search
    /// found none.
    ///
    /// Not a refusal without them, unlike a walk behind [`ImapCommand::RequireCapability`]: the
    /// UIDs are this client's own copies of a draft, marked `\Deleted` already, and are left so
    /// for the server or another client to expunge. A bare `EXPUNGE`, which would take every
    /// other `\Deleted` message in the mailbox with them, is never the way round it.
    UidExpungeFound,
    /// Sends nothing: the walk stops with [`ProtoError::Unsupported`] unless the capabilities this
    /// session has seen include one of these atoms.
    ///
    /// A guard, like [`ImapCommand::RequireEmpty`], for a walk whose later commands are only safe
    /// on a server that has the extension, and whose earlier ones must not be sent without it:
    /// `\Deleted` set on a server without `UIDPLUS` could only be cleared by a bare `EXPUNGE`,
    /// which would take other clients' deleted mail with it. Put [`ImapCommand::Capability`]
    /// before it, since the list a server gives before sign-in is often short.
    RequireCapability(Vec<String>),
    /// Sends nothing: the walk stops unless the `SELECT` before it reported this `UIDVALIDITY`.
    ///
    /// A UID means the message it was given to only while the mailbox keeps its `UIDVALIDITY`
    /// (RFC 9051 §2.3.1.1). A flag set on a renumbered mailbox lands on the wrong message and can
    /// be set right; an expunge cannot, so a walk that destroys checks first. A server that
    /// reported none is taken as renumbered.
    RequireUidValidity(u32),
    /// `LSUB "" "*"`: the mailboxes the user follows (RFC 3501 §6.3.9).
    Lsub,
    /// `CREATE <mailbox>`.
    Create {
        mailbox: String,
    },
    /// `RENAME <from> <to>`. The server moves everything beneath `from` too.
    Rename {
        from: String,
        to: String,
    },
    /// `DELETE <mailbox>`.
    ///
    /// Not `\Deleted` and `EXPUNGE`, which are only for "Delete forever" (see the module note):
    /// this removes a mailbox, and on Gmail, where a mailbox is a label, it removes the label and no message.
    Delete {
        mailbox: String,
    },
    /// `SUBSCRIBE <mailbox>`.
    Subscribe {
        mailbox: String,
    },
    /// `UNSUBSCRIBE <mailbox>`.
    Unsubscribe {
        mailbox: String,
    },
    /// `STATUS <mailbox> (MESSAGES)`, and the walk stops with a refusal unless it says zero.
    ///
    /// A guard, not a query: the one way to make "delete this folder only if it is empty" hold
    /// at the moment of deletion. A walk is a fixed list of commands on one connection, and a
    /// second connection cannot share this one's greeting, so the decision has to be made
    /// inside the walk. A server that answers no `MESSAGES` count is taken as not empty.
    RequireEmpty {
        mailbox: String,
    },
    /// `IDLE`, which parks until the server says something or the caller interrupts.
    Idle,
    /// `IDLE`, unless the `SELECT` before it reported a `UIDNEXT` above `uidnext`: then mail
    /// arrived after the caller last looked, and the walk ends without idling.
    ///
    /// Decided inside the walk for the reason [`ImapCommand::RequireEmpty`] is: the `SELECT`
    /// that says so is on this connection, and a server announces during `IDLE` only what
    /// arrives while it runs.
    IdleAfter {
        uidnext: u32,
    },
    Noop,
    Logout,
}

/// What a completed session saw.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ImapTranscript {
    /// Every untagged response, in arrival order, with the command it arrived during.
    pub untagged: Vec<Untagged>,
    /// Capability atoms from the most recent `CAPABILITY` or capability status response.
    ///
    /// The *most recent*, because Gmail advertises a reduced list before authentication and a
    /// fuller one after: believing the first is how a CONDSTORE server looks like it has none.
    pub capabilities: Vec<String>,
    /// Every tagged `OK`, with the command it completed.
    ///
    /// Kept because some completions carry the answer: `APPEND`'s `[APPENDUID <validity> <uid>]`
    /// (RFC 4315) is the only place a server says where the message it just stored went.
    pub completed: Vec<Completed>,
}

/// A tagged `OK`, kept as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completed {
    /// Which queued command it completed.
    pub index: usize,
    pub text: String,
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
    ///
    /// The *first* `{n}` followed by CRLF is the marker. Searching from the end found the last
    /// `{` in the response — which is inside the message whenever the message has one, so every
    /// HTML mail with a stylesheet had no body, and was asked for again on every sync.
    pub fn literal(&self) -> Option<&[u8]> {
        let mut from = 0;
        while let Some(at) = self.raw[from..].iter().position(|b| *b == b'{') {
            let open = from + at;
            from = open + 1;
            let close = open + self.raw[open..].iter().position(|b| *b == b'}')?;
            let digits = &self.raw[open + 1..close];
            if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
                continue;
            }
            let Ok(len) = std::str::from_utf8(digits).unwrap_or("").parse::<usize>() else {
                continue;
            };
            // The literal begins after the CRLF that follows `{n}`.
            if self.raw.get(close + 1..close + 3) != Some(b"\r\n") {
                continue;
            }
            let start = close + 3;
            // A count near `usize::MAX` is a server's claim, not a length: past the end is none.
            return self.raw.get(start..start.checked_add(len)?);
        }
        None
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
    /// `STARTTLS` was sent; waiting for its tagged completion (RFC 9051 §6.2.1).
    StartingTls {
        tag: String,
    },
    /// The server agreed to `STARTTLS`, and the runtime is upgrading the connection.
    AwaitingTls,
    /// Upgraded: `CAPABILITY` was sent, because what the server said before TLS is not to be
    /// trusted (RFC 9051 §6.2.1), and the first command waits for its answer.
    Recapping {
        tag: String,
    },
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
    /// A command with literals in it was sent up to its first literal the server must ask for;
    /// each `+` sends the next of `rest` (RFC 3501 §7.5).
    LiteralPending {
        index: usize,
        tag: String,
        rest: Vec<Vec<u8>>,
    },
    /// Resumed on a connection with a mailbox still selected: `UNSELECT` was sent, and until its
    /// tagged completion whatever arrives is about that mailbox, not this session's
    /// ([`ImapSession::resuming`]).
    Deselecting {
        tag: String,
    },
    Finished,
}

/// A sans-I/O IMAP client.
///
/// It is fed what the server sent and says what it wants done next; nothing here touches a
/// socket. Asked for one `CAPABILITY`:
///
/// ```
/// use mail_domain::SaslMech;
/// use mail_proto::machine::{IoNeed, IoReady, Machine, Progress};
/// use mail_proto::{ImapAuth, ImapCommand, ImapSession, has_capability};
/// use porter_core::{Credential, SecretText};
///
/// let auth = ImapAuth {
///     username: "me@example.test".to_owned(),
///     credential: Credential::Password(SecretText::new("secret".to_owned())),
///     sasl: vec![SaslMech::Plain],
/// };
/// let mut session = ImapSession::new(auth, vec![ImapCommand::Capability]).unwrap();
///
/// // The server speaks first.
/// assert!(matches!(session.start(), Progress::Need(needs) if needs == [IoNeed::Read]));
///
/// // After its greeting the command goes out, tagged.
/// let Progress::Need(needs) = session.feed(IoReady::Bytes(b"* OK ready\r\n".to_vec())) else {
///     panic!("the greeting should be answered with the command");
/// };
/// assert!(matches!(&needs[0], IoNeed::Write(line) if line.starts_with(b"a001 CAPABILITY")));
///
/// // Its answer ends the session, with what it said.
/// let reply = b"* CAPABILITY IMAP4rev1 IDLE\r\na001 OK done\r\n".to_vec();
/// let Progress::Done(transcript) = session.feed(IoReady::Bytes(reply)) else {
///     panic!("the session should be finished");
/// };
/// assert!(has_capability(&transcript.capabilities, "IDLE"));
/// ```
pub struct ImapSession {
    auth: ImapAuth,
    commands: Vec<ImapCommand>,
    phase: Phase,
    buf: Vec<u8>,
    transcript: ImapTranscript,
    counter: u32,
    max_response: usize,
    /// The most the answers to one command may hold ([`MAX_TRANSCRIPT`] unless
    /// [`Self::with_max_transcript`] set another), and how much the one outstanding holds.
    max_transcript: usize,
    held: usize,
    /// On a connection an earlier session finished on, so there is no greeting to wait for.
    resumed: bool,
    /// The mailbox the connection has selected, and how: from the last `SELECT` or `EXAMINE`
    /// that completed, this session's or the one it resumed from.
    selected: Option<Access>,
    /// The server's name and port, when the connection is to be upgraded with `STARTTLS` before
    /// anything else is said ([`Self::with_starttls`]).
    starttls: Option<(String, u16)>,
}

/// What a session that finished leaves for the next one on its connection.
///
/// The capabilities, because the next session decides on them (`LITERAL+`, a search charset)
/// and would otherwise know none until it asked; the tags, because each command on a
/// connection has a tag of its own (RFC 9051 §2.2.1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resume {
    /// The capability atoms the earlier session last heard, as [`ImapTranscript::capabilities`].
    pub capabilities: Vec<String>,
    /// How many tags the connection has used.
    pub tags: u32,
    /// Whether a mailbox is still selected, and how ([`ImapSession::selected`]).
    pub selected: Option<Access>,
    /// What the server sent after the earlier session's last tagged response, read already:
    /// the next session reads it first, so it neither starts in the middle of a response nor
    /// loses one ([`ImapSession::take_unread`]).
    pub unread: Vec<u8>,
}

impl Resume {
    /// Whether the next session can go on this connection: no mailbox is selected, or the
    /// server has `UNSELECT` (RFC 3691, and in IMAP4rev2) to leave it with. Without it the
    /// connection is not resumed, and the next operation has a new one: `CLOSE` would leave an
    /// examined mailbox as cleanly, but costs the same round trip to keep one connection on the
    /// rare server without `UNSELECT`, and on a selected one it expunges.
    pub fn can_go_on(&self) -> bool {
        self.selected.is_none() || unselects(&self.capabilities)
    }

    /// `LOGOUT`, tagged after the connection's last command: what ends it politely.
    pub fn logout(&self) -> Vec<u8> {
        format!("a{:03} LOGOUT\r\n", self.tags + 1).into_bytes()
    }
}

/// Whether `capabilities` include `UNSELECT`, alone or as part of IMAP4rev2 (RFC 9051 §6.4.2).
fn unselects(capabilities: &[String]) -> bool {
    has_capability(capabilities, "UNSELECT") || has_capability(capabilities, "IMAP4rev2")
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
            max_response: MAX_RESPONSE,
            max_transcript: MAX_TRANSCRIPT,
            held: 0,
            resumed: false,
            selected: None,
            starttls: None,
        })
    }

    /// The same session, upgrading the connection with `STARTTLS` straight after the greeting,
    /// before it signs in or sends anything else ([`mail_domain::Tls::StartTlsRequired`]).
    ///
    /// The upgrade is required, never opportunistic: a server that refuses it, greets with
    /// `PREAUTH` (signed in already, so `STARTTLS` is no longer allowed), or sends anything after
    /// agreeing to it ends the session before a credential is written.
    #[must_use]
    pub fn with_starttls(mut self, host: impl Into<String>, port: u16) -> Self {
        self.starttls = Some((host.into(), port));
        self
    }

    /// The same session, failing a response that grows past `bytes` without ending rather than
    /// past [`MAX_RESPONSE`].
    #[must_use]
    pub fn with_max_response(mut self, bytes: usize) -> Self {
        self.max_response = bytes;
        self
    }

    /// The same session, failing when the answers to one command hold more than `bytes`
    /// together rather than more than [`MAX_TRANSCRIPT`].
    #[must_use]
    pub fn with_max_transcript(mut self, bytes: usize) -> Self {
        self.max_transcript = bytes;
        self
    }

    /// This session, on a connection an earlier one finished on: greeted and signed in already,
    /// so it begins with its first command, tagged after the earlier session's last, reading
    /// first what the earlier one left unread.
    ///
    /// When the earlier session left a mailbox selected, this one leaves it before its first
    /// command, with `UNSELECT`, and keeps nothing that arrives until that completes. Until then
    /// the server may still send news of that mailbox (`FETCH`, `EXPUNGE`, `EXISTS`), and without
    /// the break it would read as this session's, about the mailbox it selects next. `UNSELECT`
    /// is the one break every server that has it gives: QRESYNC's `[CLOSED]` (RFC 7162 §3.2.11)
    /// marks it only where QRESYNC is enabled, and a bare `SELECT` leaves the old mailbox's news
    /// and the new one's indistinguishable.
    pub fn resuming(mut self, from: Resume) -> Self {
        self.transcript.capabilities = from.capabilities;
        self.counter = from.tags;
        self.selected = from.selected;
        self.buf = from.unread;
        self.resumed = true;
        // The connection was upgraded when it was opened.
        self.starttls = None;
        self
    }

    /// The mailbox this session left selected, if any, and how.
    pub fn selected(&self) -> Option<Access> {
        self.selected
    }

    /// What the server sent after the last response this session read, which the next session
    /// on the connection is to read first ([`Resume::unread`]).
    pub fn take_unread(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }

    /// Leave the mailbox an earlier session selected, before this session's first command.
    fn deselect(&mut self) -> Progress<ImapTranscript> {
        if !unselects(&self.transcript.capabilities) {
            return self.fail(ProtoError::Unsupported(
                "the server has no UNSELECT, so a connection with a mailbox selected is not \
                 reused"
                    .to_owned(),
            ));
        }
        let tag = self.next_tag();
        let line = format!("{tag} UNSELECT\r\n").into_bytes();
        self.phase = Phase::Deselecting { tag };
        Progress::Need(vec![IoNeed::Write(line), IoNeed::Read])
    }

    /// The greeting was read: upgrade the connection first when it is to be, else begin.
    fn greeted(&mut self, preauth: bool) -> Progress<ImapTranscript> {
        if self.starttls.is_none() {
            return self.issue(0);
        }
        if preauth {
            return self.fail(ProtoError::Unsupported(
                "the server signed in before STARTTLS, so the connection cannot be secured"
                    .to_owned(),
            ));
        }
        let tag = self.next_tag();
        let line = format!("{tag} STARTTLS\r\n").into_bytes();
        self.phase = Phase::StartingTls { tag };
        Progress::Need(vec![IoNeed::Write(line), IoNeed::Read])
    }

    /// The connection is upgraded: forget what the server said before, and ask again.
    fn upgraded(&mut self) -> Progress<ImapTranscript> {
        self.transcript.capabilities.clear();
        let tag = self.next_tag();
        let line = format!("{tag} CAPABILITY\r\n").into_bytes();
        self.phase = Phase::Recapping { tag };
        Progress::Need(vec![IoNeed::Write(line), IoNeed::Read])
    }

    /// How many tags the connection has used, this session's and any it resumed from.
    pub fn tags(&self) -> u32 {
        self.counter
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
        if let ImapCommand::IdleAfter { uidnext } = command
            && reported_uidnext(&self.transcript.untagged).is_some_and(|now| now > uidnext)
        {
            self.phase = Phase::Finished;
            return Progress::Done(std::mem::take(&mut self.transcript));
        }
        if let ImapCommand::RequireUidValidity(expected) = command {
            return match reported_uidvalidity(&self.transcript.untagged) {
                Some(now) if now == expected => self.issue(index + 1),
                now => self.fail(ProtoError::Refused {
                    kind: Refusal::Permanent,
                    text: format!(
                        "the mailbox's UIDVALIDITY is {} where {expected} was expected: its \
                         UIDs may name other messages now, so nothing was deleted",
                        now.map_or_else(|| "unreported".to_owned(), |n| n.to_string())
                    ),
                }),
            };
        }
        // Acted on what the search before it found, or passed over.
        let command = match command {
            ImapCommand::UidStoreFound { what } => match self.found_before(index) {
                Some(set) => ImapCommand::UidStore {
                    set,
                    what,
                    tolerate: false,
                },
                None => return self.issue(index + 1),
            },
            ImapCommand::UidExpungeFound => match self.found_before(index) {
                Some(set)
                    if has_capability(&self.transcript.capabilities, "UIDPLUS")
                        || has_capability(&self.transcript.capabilities, "IMAP4rev2") =>
                {
                    ImapCommand::UidExpunge { set }
                }
                _ => return self.issue(index + 1),
            },
            other => other,
        };
        if let ImapCommand::RequireCapability(any) = &command {
            if any
                .iter()
                .any(|atom| has_capability(&self.transcript.capabilities, atom))
            {
                return self.issue(index + 1);
            }
            return self.fail(ProtoError::Unsupported(format!(
                "the server does not offer {}, so nothing was sent",
                any.join(" or ")
            )));
        }
        let tag = self.next_tag();
        self.held = 0;
        if let ImapCommand::Search { keys } = &command {
            let mut parts =
                match crate::search::imap::command(&tag, keys, &self.transcript.capabilities) {
                    Ok(parts) => parts,
                    Err(e) => return self.fail(e),
                };
            let first = parts.remove(0);
            self.phase = if parts.is_empty() {
                Phase::Running { index, tag }
            } else {
                Phase::LiteralPending {
                    index,
                    tag,
                    rest: parts,
                }
            };
            return Progress::Need(vec![IoNeed::Write(first), IoNeed::Read]);
        }
        let line = match self.render(&command, &tag) {
            Ok(line) => line,
            Err(e) => return self.fail(e),
        };
        let mut line = line;
        self.phase = match command {
            ImapCommand::Idle | ImapCommand::IdleAfter { .. } => Phase::IdlePending {
                index,
                tag: tag.clone(),
            },
            // Promised not to ask: the bytes follow the line in the same write.
            ImapCommand::Append { raw, .. } if self.non_synchronizing(raw.len()) => {
                line.extend_from_slice(&raw);
                line.extend_from_slice(b"\r\n");
                Phase::Running {
                    index,
                    tag: tag.clone(),
                }
            }
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

    /// The UIDs the last [`ImapCommand::UidSearch`] before `index` found, as a set; `None` when
    /// there was no such search or it found nothing.
    fn found_before(&self, index: usize) -> Option<String> {
        let search = (0..index)
            .rev()
            .find(|at| matches!(self.commands.get(*at), Some(ImapCommand::UidSearch { .. })))?;
        let uids: Vec<String> = self
            .transcript
            .untagged
            .iter()
            .filter(|u| u.during == search)
            .filter_map(|u| {
                let text = u.text.trim();
                let rest = text
                    .get(..8)
                    .filter(|head| head.eq_ignore_ascii_case("* SEARCH"))
                    .map(|_| &text[8..])?;
                (rest.is_empty() || rest.starts_with(' ')).then_some(rest)
            })
            .flat_map(str::split_whitespace)
            // A number past a UID's range is not a UID the server could have given.
            .filter_map(|n| n.parse::<u32>().ok())
            .filter(|uid| *uid > 0)
            .map(|uid| uid.to_string())
            .collect();
        (!uids.is_empty()).then(|| uids.join(","))
    }

    /// Whether a literal of `len` bytes may be sent without waiting for `+` (RFC 7888).
    fn non_synchronizing(&self, len: usize) -> bool {
        let caps = &self.transcript.capabilities;
        has_capability(caps, "LITERAL+") || (has_capability(caps, "LITERAL-") && len <= 4096)
    }

    fn render(&self, command: &ImapCommand, tag: &str) -> Result<Vec<u8>, ProtoError> {
        let body = match command {
            ImapCommand::Capability => "CAPABILITY".to_owned(),
            ImapCommand::Noop => "NOOP".to_owned(),
            ImapCommand::Logout => "LOGOUT".to_owned(),
            ImapCommand::Idle | ImapCommand::IdleAfter { .. } => "IDLE".to_owned(),
            ImapCommand::List => "LIST \"\" \"*\"".to_owned(),
            ImapCommand::Lsub => "LSUB \"\" \"*\"".to_owned(),
            // Every name goes out as modified UTF-7, quoted: the wire name is the identity.
            ImapCommand::Create { mailbox } => format!("CREATE {}", wire(mailbox)),
            ImapCommand::Rename { from, to } => format!("RENAME {} {}", wire(from), wire(to)),
            ImapCommand::Delete { mailbox } => format!("DELETE {}", wire(mailbox)),
            ImapCommand::Subscribe { mailbox } => format!("SUBSCRIBE {}", wire(mailbox)),
            ImapCommand::Unsubscribe { mailbox } => format!("UNSUBSCRIBE {}", wire(mailbox)),
            ImapCommand::RequireEmpty { mailbox } => {
                format!("STATUS {} (MESSAGES)", wire(mailbox))
            }
            // A server that offers SASL PLAIN with an initial response is signed in to with it:
            // the password goes base64-encoded, so one in any script arrives as typed, where
            // LOGIN's quoted string is 7-bit (RFC 3501 §4.3) and an accented letter in it is
            // the server's to refuse. One round trip either way.
            // IMAP4rev2 takes an initial response without naming SASL-IR (RFC 9051 §6.2.2).
            ImapCommand::Login
                if offers_sasl(&self.transcript.capabilities, "PLAIN")
                    && (has_capability(&self.transcript.capabilities, "SASL-IR")
                        || has_capability(&self.transcript.capabilities, "IMAP4rev2")) =>
            {
                let Credential::Password(password) = &self.auth.credential else {
                    return Err(ProtoError::Unsupported(
                        "PLAIN needs a password credential".to_owned(),
                    ));
                };
                let raw = crate::smtp::plain_raw(&self.auth.username, password.expose());
                format!("AUTHENTICATE PLAIN {}", crate::smtp::b64(&raw))
            }
            ImapCommand::Login => {
                // RFC 3501 §6.2.3: a client MUST NOT issue LOGIN when the server advertises
                // LOGINDISABLED. Observed live on outlook.office365.com, which answers
                // `AUTH=XOAUTH2 LOGINDISABLED` — so this is not a hypothetical server, it is
                // the one every Microsoft 365 account lives on.
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
                format!(
                    "LOGIN {} {}",
                    quoted(&self.auth.username),
                    quoted(password.expose())
                )
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
                    "user={}\x01auth=Bearer {}\x01\x01",
                    self.auth.username,
                    access.expose()
                ));
                format!("AUTHENTICATE XOAUTH2 {initial}")
            }
            ImapCommand::Enable(capability) => format!("ENABLE {capability}"),
            ImapCommand::Select {
                mailbox,
                access,
                qresync,
            } => {
                let verb = match access {
                    Access::ReadOnly => "EXAMINE",
                    Access::ReadWrite => "SELECT",
                };
                // The wire name is modified UTF-7, and it is the identity: decode only for
                // display, never for addressing.
                let name = quoted(&mutf7::encode(mailbox));
                match qresync {
                    Some(r) => format!("{verb} {name} (QRESYNC ({} {}))", r.uidvalidity, r.modseq),
                    None => format!("{verb} {name}"),
                }
            }
            ImapCommand::UidFetch { set, items } => {
                check_set(set)?;
                format!("UID FETCH {set} {items}")
            }
            ImapCommand::UidStore { set, what, .. } => {
                check_set(set)?;
                format!("UID STORE {set} {what}")
            }
            ImapCommand::UidSearch { criteria } => format!("UID SEARCH {criteria}"),
            // Never rendered here: `issue` writes it in parts, around its literals.
            ImapCommand::Search { .. } => {
                return Err(ProtoError::Malformed(
                    "a search is written by search::imap".to_owned(),
                ));
            }
            ImapCommand::UidExpunge { set } => {
                check_set(set)?;
                format!("UID EXPUNGE {set}")
            }
            // Never rendered: `issue` either passes over it or stops the walk.
            ImapCommand::RequireCapability(_) | ImapCommand::RequireUidValidity(_) => {
                return Err(ProtoError::Malformed("a guard is not a command".to_owned()));
            }
            // Never rendered: `issue` writes what the search found in its place, or passes over it.
            ImapCommand::UidStoreFound { .. } | ImapCommand::UidExpungeFound => {
                return Err(ProtoError::Malformed(
                    "a command on what a search found is decided in the walk".to_owned(),
                ));
            }
            ImapCommand::UidCopy { set, mailbox } => {
                check_set(set)?;
                format!("UID COPY {set} {}", quoted(&mutf7::encode(mailbox)))
            }
            ImapCommand::Append {
                mailbox,
                flags,
                date,
                raw,
            } => {
                // An atom, so a flag cannot close the list it is in and add to the command.
                if let Some(flag) = flags.iter().find(|flag| !is_flag(flag)) {
                    return Err(ProtoError::Malformed(format!("{flag:?} is not a flag")));
                }
                // RFC 3501's date-time, always in UTC so the zone is never a guess.
                let date = date
                    .map(|d| format!(" \"{}\"", d.format("%d-%b-%Y %H:%M:%S +0000")))
                    .unwrap_or_default();
                let plus = if self.non_synchronizing(raw.len()) {
                    "+"
                } else {
                    ""
                };
                // The length is of the bytes exactly as they will be written. A count that
                // disagrees with what follows desynchronises the connection: the server reads
                // the remainder of the message as commands, or waits for bytes never sent.
                format!(
                    "APPEND {} ({}){date} {{{}{plus}}}",
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
        // Whatever field a value came in through, a line break in it would end this command
        // and start another of the value's choosing.
        if forbidden(&body) {
            return Err(ProtoError::Malformed(
                "a command contains CR, LF or NUL".to_owned(),
            ));
        }
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
                    return self.read_more();
                };
                self.buf.drain(..end);
                match self.phase.clone() {
                    Phase::IdlePending { index, tag } => {
                        self.phase = Phase::Idling { index, tag };
                        // The continuation and the news that ends the idle can arrive in one
                        // read. Returning here would leave that news in the buffer until the
                        // socket speaks again, and the server is waiting for DONE, so it never
                        // does. The session then parks until the watch times out.
                        if self.buf.is_empty() {
                            return Progress::Need(vec![IoNeed::Read]);
                        }
                        continue;
                    }
                    // The server asked for the literal. Now, and not before.
                    Phase::AppendPending { index, tag, body } => {
                        self.phase = Phase::Running { index, tag };
                        let mut bytes = body;
                        bytes.extend_from_slice(b"\r\n");
                        return Progress::Need(vec![IoNeed::Write(bytes), IoNeed::Read]);
                    }
                    // The server asked for a search string's literal: the next part goes.
                    Phase::LiteralPending {
                        index,
                        tag,
                        mut rest,
                    } => {
                        let part = rest.remove(0);
                        self.phase = if rest.is_empty() {
                            Phase::Running { index, tag }
                        } else {
                            Phase::LiteralPending { index, tag, rest }
                        };
                        return Progress::Need(vec![IoNeed::Write(part), IoNeed::Read]);
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

            // `imap-proto` has no grammar for ESEARCH (RFC 4731 §3.1). The response is one line
            // with no literal in it, so it is kept as text, as every untagged response is.
            if self.buf.len() < b"* ESEARCH".len()
                && b"* ESEARCH"[..self.buf.len()].eq_ignore_ascii_case(&self.buf)
            {
                return Progress::Need(vec![IoNeed::Read]);
            }
            if starts_with_ignore_case(&self.buf, b"* ESEARCH") {
                let Some(end) = find_line(&self.buf) else {
                    return self.read_more();
                };
                let raw: Vec<u8> = self.buf.drain(..end).collect();
                if matches!(self.phase, Phase::Deselecting { .. }) {
                    continue;
                }
                let during = match &self.phase {
                    Phase::Running { index, .. } => *index,
                    _ => 0,
                };
                let kept = Untagged {
                    during,
                    text: String::from_utf8_lossy(&raw).trim_end().to_owned(),
                    raw,
                };
                if let Some(failed) = self.keep(kept) {
                    return failed;
                }
                continue;
            }

            // Taken out of `self` while it is parsed, since the response borrows it and recording
            // the response needs `self`; put back before anything returns. Moved, not copied: a
            // copy of a large `FETCH` on every read is quadratic in the message's size.
            let buf = std::mem::take(&mut self.buf);
            let parsed = imap_proto::parser::parse_response(&buf);
            let (consumed, progress) = match parsed {
                Ok((rest, response)) => {
                    let used = buf.len() - rest.len();
                    (used, self.absorb(&response, &buf[..used]))
                }
                // Not a failure: the rest is still on the wire.
                Err(nom::Err::Incomplete(_)) => {
                    self.buf = buf;
                    return self.read_more();
                }
                // `imap-proto` takes a capability list only with IMAP4rev1 in it, so the list of
                // an IMAP4rev2 server that does not also name IMAP4rev1 (RFC 9051 §7.2.2) is
                // read here, as a line of text.
                Err(_) if starts_with_ignore_case(&buf, b"* CAPABILITY ") => {
                    let Some(end) = find_line(&buf) else {
                        self.buf = buf;
                        return self.read_more();
                    };
                    let line = String::from_utf8_lossy(&buf[..end]).trim_end().to_owned();
                    self.transcript.capabilities = capability_atoms(&line["* CAPABILITY ".len()..]);
                    (end, None)
                }
                Err(_) => {
                    // The response as text, not as `nom`'s error. `{e:?}` prints the remaining
                    // input as a `Vec<u8>` of decimal numbers — five hundred integers where the
                    // answer is one line of IMAP, which is a diagnostic nobody can read and the
                    // only thing the field ever sees.
                    let said = excerpt(&buf);
                    self.buf = buf;
                    return self.fail(ProtoError::Malformed(format!(
                        "could not parse a response: {said}"
                    )));
                }
            };
            // Consumed whatever was recorded: left in the buffer, the next read would parse it
            // again. A response that ended the session leaves nothing that matters behind it.
            self.buf = buf;
            self.buf.drain(..consumed);
            // Anything the server sent after its go-ahead came before TLS, so an attacker on the
            // path may have written it to be read as said under TLS (CVE-2011-0411's shape).
            if matches!(self.phase, Phase::AwaitingTls) && !self.buf.is_empty() {
                return self.fail(ProtoError::Malformed(
                    "the server sent more after agreeing to STARTTLS".to_owned(),
                ));
            }
            if let Some(progress) = progress {
                return progress;
            }
        }
    }

    /// Keep `untagged` in the transcript, unless the answers to the command outstanding would then
    /// hold more than the session's limit: then the session fails, and that is the answer.
    fn keep(&mut self, untagged: Untagged) -> Option<Progress<ImapTranscript>> {
        self.held = self
            .held
            .saturating_add(untagged.raw.len() + untagged.text.len());
        if self.held > self.max_transcript {
            return Some(self.fail(ProtoError::Malformed(format!(
                "the answers to one command grew past {} bytes",
                self.max_transcript
            ))));
        }
        self.transcript.untagged.push(untagged);
        None
    }

    /// Ask for more of a response that has not ended, unless it is already over the session's
    /// limit ([`MAX_RESPONSE`] unless [`Self::with_max_response`] set another).
    fn read_more(&mut self) -> Progress<ImapTranscript> {
        if self.buf.len() > self.max_response {
            return self.fail(ProtoError::Malformed(format!(
                "a response grew past {} bytes without ending",
                self.max_response
            )));
        }
        Progress::Need(vec![IoNeed::Read])
    }

    /// Record one response. Returns `Some` when the session should stop or move on. Its bytes are
    /// consumed by [`Self::drain`], which owns the buffer while this runs.
    fn absorb(&mut self, response: &Response<'_>, raw: &[u8]) -> Option<Progress<ImapTranscript>> {
        let text = String::from_utf8_lossy(raw).trim_end().to_owned();
        let bytes = raw.to_vec();

        if let Some(atoms) =
            capabilities_in(response).or_else(|| capabilities_in_code(response, &text))
        {
            // Replace rather than extend: a later CAPABILITY describes the session we have now,
            // and Gmail's pre-auth list is a strict subset of its post-auth one.
            self.transcript.capabilities = atoms;
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
                    | Phase::AppendPending { tag, .. }
                    // The same for a search refused before its literal: a bad charset, say.
                    | Phase::LiteralPending { tag, .. }
                    | Phase::Deselecting { tag }
                    | Phase::StartingTls { tag }
                    | Phase::Recapping { tag } => tag.clone(),
                    _ => String::new(),
                };
                if tag != expected {
                    return Some(self.fail(ProtoError::Malformed(format!(
                        "tagged {tag} while waiting for {expected}"
                    ))));
                }
                if matches!(self.phase, Phase::StartingTls { .. }) {
                    return Some(match status {
                        imap_proto::Status::Ok => {
                            // `StartingTls` is only entered with a server to upgrade to.
                            let Some((host, port)) = self.starttls.clone() else {
                                return Some(self.fail(ProtoError::Malformed(
                                    "STARTTLS without a server".to_owned(),
                                )));
                            };
                            self.phase = Phase::AwaitingTls;
                            Progress::Need(vec![IoNeed::OpenTls {
                                host,
                                port,
                                mode: mail_domain::Tls::StartTlsRequired,
                            }])
                        }
                        _ => self.fail(ProtoError::Unsupported(format!("STARTTLS: {text}"))),
                    });
                }
                if matches!(self.phase, Phase::Recapping { .. }) {
                    return Some(match status {
                        imap_proto::Status::Ok => self.issue(0),
                        _ => self.fail(ProtoError::Malformed(format!("CAPABILITY: {text}"))),
                    });
                }
                if matches!(self.phase, Phase::Deselecting { .. }) {
                    return Some(match status {
                        imap_proto::Status::Ok => {
                            self.selected = None;
                            self.issue(0)
                        }
                        _ => self.fail(ProtoError::Malformed(format!(
                            "the mailbox could not be left: {text}"
                        ))),
                    });
                }
                let index = match &self.phase {
                    Phase::Running { index, .. }
                    | Phase::IdlePending { index, .. }
                    | Phase::Idling { index, .. }
                    | Phase::IdleEnding { index, .. }
                    | Phase::AppendPending { index, .. }
                    | Phase::LiteralPending { index, .. } => *index,
                    _ => 0,
                };
                match status {
                    imap_proto::Status::Ok => {
                        if let Some(ImapCommand::Select { access, .. }) = self.commands.get(index) {
                            self.selected = Some(*access);
                        }
                        self.transcript.completed.push(Completed {
                            index,
                            text: text.clone(),
                        });
                        if let Some(ImapCommand::RequireEmpty { mailbox }) =
                            self.commands.get(index)
                            && let Some(refusal) =
                                not_empty(&self.transcript.untagged, index, mailbox)
                        {
                            return Some(self.fail(refusal));
                        }
                        Some(self.issue(index + 1))
                    }
                    _ if matches!(
                        self.commands.get(index),
                        Some(ImapCommand::UidStore { tolerate: true, .. })
                    ) =>
                    {
                        Some(self.issue(index + 1))
                    }
                    imap_proto::Status::No => {
                        let refused = self.commands.get(index);
                        Some(self.fail(classify(&text, refused)))
                    }
                    _ => Some(self.fail(ProtoError::Malformed(format!("BAD: {text}")))),
                }
            }
            // News of the mailbox being left, not of this session's; or what the server says
            // while the connection is being secured, which no command asked for.
            _ if matches!(
                self.phase,
                Phase::Deselecting { .. }
                    | Phase::StartingTls { .. }
                    | Phase::AwaitingTls
                    | Phase::Recapping { .. }
            ) =>
            {
                None
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
                if let Some(failed) = self.keep(Untagged {
                    during,
                    text,
                    raw: bytes,
                }) {
                    return Some(failed);
                }

                // The half of IDLE's contract that was documented and not implemented. `IDLE`
                // "parks until the server says something or the caller interrupts", and only
                // the interrupt was wired — so an `EXISTS` announcing new mail was filed away
                // and the session went on parking. IDLE was a sleep with extra steps.
                //
                // Ending it in protocol, with `DONE`, rather than dropping the socket: the
                // connection stays reusable and the server is not left wondering.
                if news && let Phase::Idling { index, tag } = self.phase.clone() {
                    self.phase = Phase::IdleEnding { index, tag };
                    return Some(Progress::Need(vec![
                        IoNeed::Write(b"DONE\r\n".to_vec()),
                        IoNeed::Read,
                    ]));
                }
                None
            }
        }
    }
}

/// The capabilities `response` lists: a `CAPABILITY` response, or a `[CAPABILITY ...]` code on a
/// greeting or a completion (RFC 9051 §7.1), which is how Dovecot and others say them without
/// being asked. Rendered as [`ImapTranscript::capabilities`] holds them.
fn capabilities_in(response: &Response<'_>) -> Option<Vec<String>> {
    let atoms = match response {
        Response::Capabilities(atoms)
        | Response::Data {
            code: Some(imap_proto::ResponseCode::Capabilities(atoms)),
            ..
        }
        | Response::Done {
            code: Some(imap_proto::ResponseCode::Capabilities(atoms)),
            ..
        } => atoms,
        _ => return None,
    };
    Some(atoms.iter().map(|c| format!("{c:?}")).collect())
}

/// The atoms of a capability list in words, rendered as `imap-proto`'s are in
/// [`ImapTranscript::capabilities`].
fn capability_atoms(list: &str) -> Vec<String> {
    list.split_ascii_whitespace()
        .map(|atom| {
            if atom.eq_ignore_ascii_case("IMAP4rev1") {
                "Imap4rev1".to_owned()
            } else if atom
                .get(..5)
                .is_some_and(|p| p.eq_ignore_ascii_case("AUTH="))
            {
                format!("Auth({:?})", &atom[5..])
            } else {
                format!("Atom({atom:?})")
            }
        })
        .collect()
}

/// The `[CAPABILITY ...]` code of a greeting or completion that `imap-proto` left as text
/// because the list does not name IMAP4rev1 (see the `* CAPABILITY` case in `drain`).
fn capabilities_in_code(response: &Response<'_>, text: &str) -> Option<Vec<String>> {
    if !matches!(response, Response::Data { .. } | Response::Done { .. }) {
        return None;
    }
    let line = text.lines().next()?;
    let start = line.to_ascii_uppercase().find("[CAPABILITY ")? + "[CAPABILITY ".len();
    let list = line.get(start..)?.split(']').next()?;
    Some(capability_atoms(list))
}

/// Whether the server offers the SASL mechanism `mechanism` (`AUTH=<mechanism>`, RFC 9051 §6.2.2).
fn offers_sasl(capabilities: &[String], mechanism: &str) -> bool {
    capabilities.iter().any(|c| {
        c.trim()
            .strip_prefix("Auth(")
            .and_then(|rest| rest.strip_suffix(')'))
            .is_some_and(|name| name.trim_matches('"').eq_ignore_ascii_case(mechanism))
    })
}

/// Whether the server advertised a capability, matched as a whole atom.
///
/// `capabilities` holds `imap-proto`'s atoms rendered with `Debug`, so an entry looks like
/// `Atom("LOGINDISABLED")`, `Auth("XOAUTH2")` or `Imap4rev1`. Matching that with `contains` is
/// a substring standing in for a token: `contains("MOVE")` is also true of a hypothetical
/// `REMOVE`, and `contains("UID")` of `UIDPLUS`.
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
        // Resumed, the greeting was an earlier session's to read.
        if self.resumed {
            return match self.selected {
                Some(_) => self.deselect(),
                None => self.issue(0),
            };
        }
        // The server speaks first. Reading rather than writing is the whole difference between
        // this and a protocol where the client opens.
        Progress::Need(vec![IoNeed::Read])
    }

    fn feed(&mut self, ready: IoReady) -> Progress<ImapTranscript> {
        match ready {
            IoReady::Eof => self.fail(ProtoError::UnexpectedEof),
            IoReady::TlsOpen if matches!(self.phase, Phase::AwaitingTls) => self.upgraded(),
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
                            if let Some(atoms) = capabilities_in(&response)
                                .or_else(|| capabilities_in_code(&response, &text))
                            {
                                self.transcript.capabilities = atoms;
                            }
                            let preauth = matches!(
                                response,
                                Response::Data {
                                    status: imap_proto::Status::PreAuth,
                                    ..
                                }
                            );
                            self.transcript.untagged.push(Untagged {
                                during: 0,
                                text,
                                raw: self.buf[..used].to_vec(),
                            });
                            self.buf.drain(..used);
                            self.greeted(preauth)
                        }
                        Err(nom::Err::Incomplete(_)) => self.read_more(),
                        Err(_) => {
                            let said = excerpt(&self.buf);
                            self.fail(ProtoError::Malformed(format!(
                                "could not parse the greeting: {said}"
                            )))
                        }
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

/// The last `UIDNEXT` the server reported on this connection, from `SELECT`'s response code.
fn reported_uidnext(untagged: &[Untagged]) -> Option<u32> {
    // A UIDNEXT past u32 is not one a server may send: taken as unreported.
    response_code(untagged, "UIDNEXT").and_then(|n| u32::try_from(n).ok())
}

/// The `UIDVALIDITY` the last `SELECT` or `EXAMINE` reported, if any did.
fn reported_uidvalidity(untagged: &[Untagged]) -> Option<u32> {
    // Out of range is taken as unreported, which a destructive walk refuses.
    response_code(untagged, "UIDVALIDITY").and_then(|n| u32::try_from(n).ok())
}

/// The number the last `* OK [NAME n]` status response gave for `name`, if one did.
///
/// Only status responses are read: the same words in a message's subject or body, which arrive
/// in a `FETCH`, are the message's and not the server's, and taking them for the server's would
/// let a message decide which mailbox a destructive command thinks it is in.
pub(crate) fn response_code(untagged: &[Untagged], name: &str) -> Option<u64> {
    untagged
        .iter()
        .filter_map(|u| {
            let code = u
                .text
                .get(..6)
                .filter(|head| head.eq_ignore_ascii_case("* OK ["))?;
            let rest = &u.text[code.len()..];
            let end = rest.find(']')?;
            let (key, value) = rest[..end].split_once(' ')?;
            key.eq_ignore_ascii_case(name)
                // A value that is not a number is not that code's.
                .then(|| value.trim().parse().ok())
                .flatten()
        })
        .next_back()
}

/// Why a [`ImapCommand::RequireEmpty`] guard stops the walk, if it does.
///
/// Read from the typed `STATUS` response that arrived during the guard, not from its text:
/// the count is the one number that decides whether mail is about to be destroyed.
fn not_empty(untagged: &[Untagged], index: usize, mailbox: &str) -> Option<ProtoError> {
    let count = untagged
        .iter()
        .filter(|u| u.during == index)
        .filter_map(|u| match imap_proto::parser::parse_response(&u.raw) {
            Ok((_, Response::MailboxData(imap_proto::MailboxDatum::Status { status, .. }))) => {
                status.iter().find_map(|a| match a {
                    imap_proto::StatusAttribute::Messages(n) => Some(*n),
                    _ => None,
                })
            }
            _ => None,
        })
        .next_back();
    let why = match count {
        Some(0) => return None,
        Some(n) => format!("{mailbox} holds {n} message(s) on the server"),
        None => format!("the server did not say how many messages {mailbox} holds"),
    };
    Some(ProtoError::Refused {
        kind: Refusal::Permanent,
        text: format!("{why}; not deleted"),
    })
}

/// A mailbox name as it travels: modified UTF-7, quoted.
fn wire(mailbox: &str) -> String {
    quoted(&mutf7::encode(mailbox))
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

/// An IMAP quoted string, with `\` and `"` escaped.
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

/// A flag as RFC 3501 writes one: an atom, `\` before it for a system flag.
fn is_flag(flag: &str) -> bool {
    let atom = flag.strip_prefix('\\').unwrap_or(flag);
    !atom.is_empty()
        && atom
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"(){%*\"\\]".contains(&b))
}

fn forbidden(value: &str) -> bool {
    value.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0))
}

fn credential_forbidden(credential: &Credential) -> bool {
    match credential {
        Credential::Password(p) => forbidden(p.expose()),
        Credential::OAuth {
            access, refresh, ..
        } => forbidden(access.expose()) || forbidden(refresh.expose()),
        // Not a sign-in credential at all: refused the same way as one that would break a line.
        // A bearer token signs in to JMAP only; IMAP has no mechanism that presents one.
        Credential::ApiKey(_) | Credential::Bearer(_) | Credential::KeyPair { .. } => true,
    }
}

fn starts_with_ignore_case(buf: &[u8], prefix: &[u8]) -> bool {
    buf.len() >= prefix.len() && buf[..prefix.len()].eq_ignore_ascii_case(prefix)
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
