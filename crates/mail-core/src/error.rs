//! Core's failures: one error type for the crate, over the failures of the crates below it.
//!
//! [`CoreError`] wraps what the store, the runtime and the pure crates report, by `#[from]`, so a
//! function here that only passes a failure on writes `?`; and it names what core itself refuses
//! to do. Its `Display` is the sentence the command line has always printed: the words live in
//! the `#[error(...)]` attributes, not in a `String` carried about, so a window can match on the
//! variant and say it its own way.

use mail_domain::{Retry, Retryable};
use std::path::{Path, PathBuf};

/// What a failure carries as its cause, kept as the error it was rather than as its text.
pub type Source = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Something core was asked to do could not be done.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    // ---- the crates below
    #[error(transparent)]
    Store(#[from] mail_store::StoreError),
    #[error(transparent)]
    Runtime(#[from] mail_runtime::RuntimeError),
    #[error(transparent)]
    Mime(#[from] mail_mime::MimeError),
    #[error(transparent)]
    Proto(#[from] mail_proto::ProtoError),
    #[error(transparent)]
    Pim(#[from] mail_pim::PimError),
    #[error(transparent)]
    Time(#[from] TimeError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Regex(#[from] regex::Error),

    // ---- a failure with words about what was being done
    /// A file or directory that would not cooperate, named by its path.
    #[error("{}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// An I/O error on a stream or a file the caller already named.
    #[error(transparent)]
    Stream(#[from] std::io::Error),
    /// What was being done, and the error that stopped it: reads `cannot {doing}: {source}`.
    #[error("cannot {doing}: {source}")]
    Cannot {
        doing: String,
        #[source]
        source: Source,
    },
    /// Where it went wrong, and the error: reads `{what}: {source}`.
    #[error("{what}: {source}")]
    Context {
        what: String,
        #[source]
        source: Source,
    },

    // ---- attachments
    /// An attachment number the message does not have.
    #[error("that message has {have} attachment(s); there is no number {index}")]
    NoAttachment { have: usize, index: usize },
    /// The attachment was left on the server and is fetched when opened.
    #[error("{name} is still on the server; it downloads when opened")]
    AttachmentOnServer { name: String },

    // ---- conversations, accounts, places
    #[error("that conversation has no messages")]
    EmptyConversation,
    /// An account was named by its address and there is none like it.
    #[error("no account {0:?}")]
    UnknownAccount(String),
    #[error("no accounts. Add one with: mailo account add <address>")]
    NoAccounts,
    #[error("no config directory (neither XDG_CONFIG_HOME nor HOME is set)")]
    NoConfigDir,

    // ---- reminders
    #[error("the message has no Message-ID to be found by")]
    NoMessageId,

    // ---- messages, invitations, receipts
    /// Only the headers are stored; the body has not been fetched.
    #[error("only that message's headers are here yet; run mailo sync and try again")]
    HeadersOnly,
    #[error("the message's body is missing")]
    BodyMissing,
    #[error("that message carries no calendar invitation")]
    NoCalendar,
    #[error("that message carries no invitation to answer")]
    NoInvitation,
    #[error("that event was cancelled; there is nothing to answer")]
    EventCancelled,
    #[error("that message is someone's answer to an invitation, not an invitation")]
    IsAnAnswer,
    #[error(
        "that event was published to be added to a calendar and asks for no answer; save it \
         with: mailo invite <message-id> --ics FILE"
    )]
    PublishedEvent,
    #[error("you organised that event")]
    YouOrganised,
    #[error("none of this account's addresses is among that event's attendees")]
    NotInvited,
    #[error("the invitation names no organiser to answer")]
    NoOrganiser,
    #[error("the answer could not be queued")]
    AnswerNotQueued,
    #[error("a receipt for that message was already sent")]
    ReceiptSent,
    #[error("you already declined to send a receipt for that message")]
    ReceiptDeclined,
    #[error("that message did not ask for a read receipt")]
    NoReceiptAsked,
    #[error("the receipt could not be queued")]
    ReceiptNotQueued,
    #[error("{0:?} is not an address to block")]
    NotAnAddressToBlock(String),

    // ---- rules, and the server's filters
    #[error("add an account first: mailo account add <address>")]
    AddAnAccountFirst,
    #[error("name the account: --account you@example.com")]
    NameTheAccount,
    #[error("no rule {name:?} on {address}")]
    NoRuleNamed { name: String, address: String },
    #[error("--until has to be after --from")]
    UntilBeforeFrom,
    /// A Granted plan typed in by hand: presets never make one, the desktop's service does.
    #[error("this account is the desktop's account service's: add it there, in Add Account")]
    AddInAccountService,
    #[error("{address} is an account of the desktop's account service, which is not reachable")]
    AccountServiceUnreachable { address: String },
    #[error("{address}: the account service lists no ManageSieve server for it")]
    NoManageSieve { address: String },
    /// The account has no credential stored; what to do about it depends on how it signs in.
    #[error("{}", crate::account::no_credential(.address, .auth))]
    NoCredential {
        address: String,
        auth: mail_domain::AuthPlan,
    },
    #[error("the server answered a status request with something else")]
    StatusExpected,
    /// An account has no ManageSieve server to speak to, and the reason why.
    #[error("{address}: {why}")]
    NoSieve {
        address: String,
        why: mail_proto::sieve::NoSieve,
    },

    // ---- export and import
    #[error("say which messages: a search, or inbox, sent, all…")]
    NoSelection,
    /// An export would not create the file it was told to write.
    #[error("{}: {source}; choose a name that does not exist yet", .path.display())]
    NotCreated {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{} is a directory but not a Maildir: it has no cur, new or tmp inside", .0.display())]
    NotAMaildir(PathBuf),
    #[error("no account for {address:?}. `mailo account list` says which there are.")]
    NoAccountFor { address: String },
    #[error(
        "{address} is not an IMAP account, so it has no mailboxes to upload into; leave out \
         --to-mailbox to keep the mail on this computer"
    )]
    NotImap { address: String },
    #[error(
        "{address} has no folder called {folder:?}; `mailo folder list {address}` shows them, \
         and `mailo folder new {address} {folder}` makes one"
    )]
    NoFolder { address: String, folder: String },

    // ---- print, unsubscribe: an id that names a message or a thread
    #[error("{0} is neither a message nor a thread")]
    NeitherMessageNorThread(uuid::Uuid),
    #[error("thread {0} holds no messages")]
    EmptyThread(uuid::Uuid),
    #[error("{name} and 998 numbered copies of it already exist in {}", .dir.display())]
    NamesTaken { name: String, dir: PathBuf },
    #[error("that message has not been fetched in full yet; run `mailo sync` and try again")]
    NotFetchedInFull,
    #[error(
        "no message in that thread has been fetched in full yet; run `mailo sync` and try again"
    )]
    ThreadNotFetchedInFull,
    #[error("that message offers no way to unsubscribe (it has no usable List-Unsubscribe header)")]
    NothingOffered,

    // ---- the daemon
    #[error("the daemon closed the connection without answering")]
    DaemonSilent,
    #[error("a mailo daemon is already running; `mailo daemon --stop` will stop it")]
    DaemonRunning,
    #[error("the daemon said {said:?} where a change was expected")]
    DaemonSaid { said: crate::ipc::wire::Response },
    #[error("the daemon would not subscribe: {said:?}")]
    DaemonWouldNotSubscribe { said: crate::ipc::wire::Response },
    #[error(transparent)]
    Wire(#[from] crate::ipc::wire::Mismatch),
    #[error(transparent)]
    Door(#[from] latchkey::Error),

    // ---- discovery, server search
    #[error(transparent)]
    Discovery(#[from] crate::discover::Failed),
    #[error("that account is no longer configured")]
    AccountGone,
    #[error("{address} is POP3, which has one mailbox and no search")]
    Pop3NoSearch { address: String },
    #[error("{address} is kept on this computer: there is no server to search")]
    LocalNoSearch { address: String },

    // ---- accounts
    #[error("accountd names no address for {label}")]
    NoAddressNamed { label: String },
    #[error("the grant on {address} lists no IMAP, POP3, JMAP or Graph server")]
    GrantListsNoMail { address: String },
    #[error("no such account")]
    NoSuchAccount,
    #[error(
        "no JMAP session URL for {address}. Name it:\n\n                   mailo account add {address} --jmap https://jmap.example.com/.well-known/jmap"
    )]
    NoJmapSession { address: String },
    #[error(
        "no preset for {address:?}. Either it is one of the known domains \
         (gmail.com, googlemail.com), or name the servers:\n\n  \
         mailo account add {address} --imap imap.example.com --smtp smtp.example.com\n\n\
         Ports default to 993 and 465, both with implicit TLS. A server that offers \
         only POP3 takes --pop3 in place of --imap (port 995). Add --login NAME if \
         the server wants something other than the whole address."
    )]
    NoPreset { address: String },

    // ---- contacts
    #[error("no contact {0:?}")]
    NoContact(String),
    #[error("{url}: its account is no longer configured")]
    BookAccountGone { url: String },
    #[error("add an account first: an address book is kept under one")]
    AddAnAccountForBook,
    #[error("name the account it belongs to: --account you@example.com")]
    NameTheBookAccount,
    #[error("an address book needs the address of its server")]
    BookNeedsServer,
    #[error("{address} is not an account of the desktop's account service")]
    NotAServiceAccount { address: String },
    /// The account service would not let Mail read the account's contacts.
    #[error(
        "{address} is an account of the desktop's account service, and Mail has not been \
         allowed to read its contacts ({source})"
    )]
    ContactsNotAllowed {
        address: String,
        #[source]
        source: mail_runtime::link::LinkError,
    },
    #[error(
        "{address} is an account of the desktop's account service, and Mail has not been \
         allowed to read its contacts (the grant was given to another account)"
    )]
    ContactsGrantMisdirected { address: String },
    #[error(
        "{address} is an account of the desktop's account service, which is not reachable \
         from here"
    )]
    AccountServiceUnreachableFromHere { address: String },
    #[error(
        "{address} is an account of the desktop's account service whose contacts are not \
         CardDAV (Google's are served through People, which Mail does not read): no \
         contacts were synced"
    )]
    ContactsNotCardDav { address: String },
    #[error(
        "{start} is not the address book server of {address} ({relays}): the desktop's \
         account service relays that one only"
    )]
    NotTheBookServer {
        start: String,
        address: String,
        relays: String,
    },
    #[error("{address}: the address book server has no address")]
    BookServerNoAddress { address: String },
    #[error("{text:?} is not a URL: {source}")]
    NotAUrl {
        text: String,
        #[source]
        source: url::ParseError,
    },
    #[error("no password stored for {login}. Re-run with MAILO_PASSWORD set")]
    NoServicePassword { login: String },
    #[error("the credential stored for {address} is not a sign-in")]
    NotASignIn { address: String },

    // ---- OpenPGP and S/MIME
    #[error("that message could not be opened, so its attachments cannot be read")]
    NotOpened,
    #[error(transparent)]
    Pgp(#[from] crate::pgp::PgpError),
    #[error(transparent)]
    Smime(#[from] crate::smime::SmimeError),
    #[error(transparent)]
    Usage(#[from] UsageError),

    // ---- composing and sending
    #[error("the draft names identity {id}, which this account no longer has")]
    DraftIdentityGone { id: mail_domain::IdentityId },
    #[error(
        "this account has no identity to send as. It was added before identities were created \
         at setup; re-add it with: mailo account add <address>"
    )]
    NoIdentityToSendAs,
    #[error("no account {address}. This one has: {}", .known.join(", "))]
    NoSuchSender { address: String, known: Vec<String> },
    #[error(
        "which account should this be sent from? Say --from <address>: {}",
        .known.join(", ")
    )]
    WhichAccount { known: Vec<String> },
    #[error("that draft is on its way; what goes out was frozen when you sent it")]
    DraftOnItsWay,
    #[error(
        "that would make {total} of attachments, and most servers refuse above {limit}. Send a \
         link instead, or split the message"
    )]
    AttachmentsTooBig { total: String, limit: String },
    #[error("{} is {size}, and most servers refuse above {limit}", .path.display())]
    FileTooBig {
        path: PathBuf,
        size: String,
        limit: String,
    },
    #[error("that draft has {have} attachment(s); there is no number {index}")]
    NoDraftAttachment { have: usize, index: usize },
    #[error("that draft is queued for delivery; it cannot be discarded until the send settles")]
    DraftQueued,
    #[error("{stamp} has already passed; to send it now, leave out --at")]
    SendTimePassed { stamp: String },
    #[error("that draft was already sent at {at}")]
    DraftSentAt { at: chrono::DateTime<chrono::Utc> },
    #[error("that message is already being sent")]
    AlreadySending,
    #[error("that message was already sent")]
    AlreadySent,
    #[error("the submission could not be queued")]
    SubmissionNotQueued,
    /// A recipient the box could not read as an address.
    #[error(transparent)]
    Address(#[from] mail_domain::ParseAddressError),
    #[error(
        "that message has not been downloaded yet, so there is nothing to attach. `mailo sync` \
         downloads it; or forward it inline"
    )]
    NotDownloaded,
    #[error(
        "that message is large, so it was downloaded in parts and its attachments were left on \
         the server. What is here is rebuilt from those parts, with the attachments empty, and \
         attaching it would send that rather than the message as it was sent. Forward it inline \
         instead"
    )]
    RebuiltFromParts,
    #[error(
        "that message is {size}, and most servers refuse above {limit}. Forward it inline instead"
    )]
    MessageTooBig { size: String, limit: String },

    // ---- sync
    #[error("{address}: stored plan is unreadable: {why}")]
    PlanUnreadable { address: String, why: String },
    #[error("{address}: stored capabilities are unreadable: {why}")]
    CapsUnreadable { address: String, why: String },
    #[error("{address}: no capabilities recorded. Something has gone wrong with setup.")]
    NoCaps { address: String },
    /// A pass could not run, in the words the classifier gave it, with what to do about it.
    #[error("{why}")]
    PassFailed {
        retry: mail_domain::Retry,
        why: String,
    },
    #[error("the account this message belongs to is no longer configured")]
    MessageAccountGone,
    #[error("only IMAP leaves attachments on the server")]
    OnlyImapLeavesAttachments,
    #[error("only an IMAP account has mailboxes to upload into")]
    OnlyImapUploads,
    #[error("POP3 has one mailbox, and every sync fetches it")]
    Pop3OneMailbox,
    #[error("{address} is kept on this computer: there is no server to fetch from")]
    LocalNoServerToFetch { address: String },
    #[error("on {address} folders are labels: every sync fetches the whole account")]
    FoldersAreLabelsWhole { address: String },
    #[error("on {address} folders are labels: a folder's mail arrives with its label")]
    FoldersAreLabels { address: String },
}

impl CoreError {
    /// `cannot {doing}: {source}`.
    pub fn cannot(doing: impl Into<String>, source: impl Into<Source>) -> Self {
        CoreError::Cannot {
            doing: doing.into(),
            source: source.into(),
        }
    }

    /// `{what}: {source}`.
    pub fn context(what: impl Into<String>, source: impl Into<Source>) -> Self {
        CoreError::Context {
            what: what.into(),
            source: source.into(),
        }
    }

    /// For `map_err` on a file operation: `{path}: {error}`.
    pub fn at(path: impl AsRef<Path>) -> impl FnOnce(std::io::Error) -> CoreError {
        let path = path.as_ref().to_path_buf();
        move |source| CoreError::Io { path, source }
    }
}

pub(crate) use mail_runtime::Logged;

impl Retryable for CoreError {
    fn retry(&self) -> Retry {
        match self {
            CoreError::Store(e) => e.retry(),
            CoreError::Runtime(e) => e.retry(),
            CoreError::Mime(e) => e.retry(),
            CoreError::Proto(e) => e.retry(),
            CoreError::PassFailed { retry, .. } => retry.clone(),
            // A refusal of ours, or a fact about bytes already in hand: asking again gets the
            // same answer.
            _ => Retry::Fatal(self.to_string()),
        }
    }
}

// The window's older callers keep a `String` error, and the text is the same either way.
impl From<CoreError> for String {
    fn from(error: CoreError) -> Self {
        error.to_string()
    }
}

impl From<TimeError> for String {
    fn from(error: TimeError) -> Self {
        error.to_string()
    }
}

impl From<mail_runtime::unsubscribe::UnsubscribeFailure> for CoreError {
    fn from(failure: mail_runtime::unsubscribe::UnsubscribeFailure) -> Self {
        CoreError::Runtime(mail_runtime::RuntimeError::Unsubscribe(failure))
    }
}

/// A time that was typed and could not be read, or cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeError {
    #[error("{hour}:00 is not a time")]
    NotAnHour { hour: u32 },
    #[error("no tomorrow")]
    NoTomorrow,
    /// A daylight-saving jump skipped it, or the zone has no such instant.
    #[error("that local time does not exist")]
    NoLocalTime,
    #[error(
        "{phrase:?} is not a time I know. Try: later, tonight, tomorrow, tomorrow 9, weekend, \
         monday…sunday, fri 17:00, +2h, +3d, 2026-09-25, or \"2026-09-25 14:30\""
    )]
    Unknown { phrase: String },
    #[error("{rest:?} is not a number of minutes, hours or days")]
    NotACount { rest: String },
    #[error("a snooze goes forwards")]
    Backwards,
    #[error("{unit:?} is not m, h or d")]
    BadUnit { unit: String },
    #[error("{rest:?} is longer than a mail client can wait")]
    TooLong { rest: String },
    /// A reminder was asked for with no time given.
    #[error("Type a time: tomorrow 9, fri 17:00, +3d, 2026-10-02")]
    Blank,
    #[error("{stamp} has already passed. Pick a later time.")]
    Passed { stamp: String },
    #[error("{text:?} is not a date: write 2026-10-08 or 2026-10-08T09:00")]
    NotADate { text: String },
    #[error("{text} does not exist in this time zone")]
    NoSuchInstant { text: String },
}

/// A command line that could not be read, with the words that say what to type instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UsageError {
    /// A command with nothing after it: the synopsis alone.
    #[error("{0}")]
    Synopsis(&'static str),
    #[error("{command} {verb} needs {what}\n\n{usage}")]
    Missing {
        command: &'static str,
        verb: String,
        what: &'static str,
        usage: &'static str,
    },
    #[error("unknown option {option:?}\n\n{usage}")]
    UnknownOption { option: String, usage: &'static str },
    #[error("unknown {command} command {verb:?}\n\n{usage}")]
    UnknownCommand {
        command: &'static str,
        verb: String,
        usage: &'static str,
    },
    #[error("{raw:?} is not a message id\n\n{usage}")]
    NotAMessageId { raw: String, usage: &'static str },
    /// The domain's own words about a fingerprint that would not read.
    #[error("{0}; `mailo pgp keys` lists them")]
    PgpFingerprint(mail_domain::ParseFingerprintError),
    #[error("{0}; `mailo smime list` lists them")]
    SmimeFingerprint(mail_domain::ParseFingerprintError),
    #[error("contacts add needs an address: mailo contacts add ada@example.com Ada")]
    ContactsAdd,
    #[error("contacts remove needs an address")]
    ContactsRemove,
    #[error("contacts import needs a .vcf file")]
    ContactsImport,
    /// A flag that takes a value was the last word.
    #[error("{flag} needs {what}")]
    FlagNeeds {
        flag: &'static str,
        what: &'static str,
    },
    #[error("unknown option {0:?}")]
    UnknownFlag(String),
    #[error("unexpected {0:?}")]
    Unexpected(String),
}
