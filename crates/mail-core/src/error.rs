//! Core's failures: one error type for the crate, over the failures of the crates below it.
//!
//! [`CoreError`] wraps what the store, the runtime and the pure crates report, by `#[from]`, so a
//! function here that only passes a failure on writes `?`; and it names what core itself refuses
//! to do. Its `Display` is the sentence the command line has always printed: the words live in
//! the `#[error(...)]` attributes, not in a `String` carried about, so a window can match on the
//! variant and say it its own way.

use mail_domain::{Retry, Retryable};
use mail_logic::remedy::{Remedy, SignInWith};
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
    Time(#[from] mail_logic::snooze::TimeError),
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
    #[error("no accounts")]
    NoAccounts,
    /// A push relay's address that cannot be one (`sync::graph_push::set_relay`).
    #[error(transparent)]
    Relay(#[from] mail_runtime::graph::push::RelayRefused),
    #[error("no config directory (neither XDG_CONFIG_HOME nor HOME is set)")]
    NoConfigDir,
    /// A setting's file that is there and cannot be read, left as it is rather than written over.
    #[error("{} cannot be read: {why}", .path.display())]
    Damaged { path: PathBuf, why: String },

    // ---- profiles
    #[error(transparent)]
    ProfileId(#[from] crate::profile::NotAProfileId),
    /// A Profile was named by its id or its name and there is none like it.
    #[error("no profile {0:?}")]
    NoProfile(String),
    #[error("{0:?} cannot name a profile: a name is not blank and not a profile id")]
    ProfileNameUnusable(String),
    #[error("another profile is already called {0:?}")]
    ProfileNameTaken(String),
    #[error("the default profile is the install in place, and cannot be removed")]
    DefaultProfileStays,
    /// `profiles.json` of a later version, which this build would write back without what the
    /// version added.
    #[error("{}: written by a newer mailo (version {version})", .path.display())]
    ProfilesTooNew { path: PathBuf, version: u32 },

    // ---- reminders
    #[error("the message has no Message-ID to be found by")]
    NoMessageId,

    // ---- messages, invitations, receipts
    /// Only the headers are stored; the body has not been fetched.
    #[error("only that message's headers are here yet")]
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
    #[error("that event was published to be added to a calendar and asks for no answer")]
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
    #[error("add an account first")]
    AddAnAccountFirst,
    #[error("name the account: --account you@example.com")]
    NameTheAccount,
    #[error("no rule {name:?} on {address}")]
    NoRuleNamed { name: String, address: String },
    #[error("--until has to be after --from")]
    UntilBeforeFrom,
    /// The account has no credential stored; what to do about it depends on how it signs in.
    #[error("{}", crate::account::no_credential(.auth))]
    NoCredential {
        address: String,
        auth: mail_domain::AuthPlan,
    },
    /// The keyring that holds the account's credential did not answer: it is locked, or not
    /// reachable. The credential may be perfectly good, so this is a wait, not a sign-in.
    #[error("{address}: {why}")]
    KeyringUnreachable { address: String, why: &'static str },
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
    #[error(
        "{} is a directory but not a Maildir (it has no cur, new or tmp inside), Apple Mail's \
         mailboxes or a Thunderbird profile",
        .0.display()
    )]
    NotMailFolder(PathBuf),
    #[error("{} is not an Apple Mail message: it does not begin with its length", .0.display())]
    NotEmlx(PathBuf),
    #[error("no account for {address:?}")]
    NoAccountFor { address: String },
    #[error(
        "{address} is not an IMAP account, so it has no mailboxes to upload into; leave out \
         --to-mailbox to keep the mail on this computer"
    )]
    NotImap { address: String },
    #[error("{address} has no folder called {folder:?}")]
    NoFolder { address: String, folder: String },

    // ---- print, unsubscribe: an id that names a message or a thread
    #[error("{0} is neither a message nor a thread")]
    NeitherMessageNorThread(uuid::Uuid),
    #[error("thread {0} holds no messages")]
    EmptyThread(uuid::Uuid),
    #[error("{name} and 998 numbered copies of it already exist in {}", .dir.display())]
    NamesTaken { name: String, dir: PathBuf },
    #[error("that message has not been fetched in full yet")]
    NotFetchedInFull,
    #[error("no message in that thread has been fetched in full yet")]
    ThreadNotFetchedInFull,
    #[error("that message offers no way to unsubscribe (it has no usable List-Unsubscribe header)")]
    NothingOffered,
    #[error("that message names no address to write to its list (it has no List-Post header)")]
    NoListPost,
    #[error("that message has not been downloaded yet, so there is nothing to redirect")]
    NothingToRedirect,
    #[error(
        "that message is large, so it was downloaded in parts and its attachments were left on \
         the server. What is here is rebuilt from those parts, and redirecting it would send that \
         rather than the message as it came. Forward it instead"
    )]
    RebuiltNotRedirected,
    #[error(
        "{address} sends through Microsoft Graph, which delivers a message to the people its own \
         headers name, so a redirect would go back to whoever it first went to. Forward it instead"
    )]
    RedirectThroughGraph { address: String },

    // ---- the daemon
    #[error("the daemon closed the connection without answering")]
    DaemonSilent,
    #[error("a daemon is already running")]
    DaemonRunning,
    /// What the daemon's wire said was wrong with a message (`mail_agent::ipc::wire::Mismatch`),
    /// in its words: the crate that speaks the wire turns its mismatch into this.
    #[error("{0}")]
    Wire(String),
    #[error(transparent)]
    Door(#[from] latchkey::Error),

    // ---- discovery, server search
    #[error(transparent)]
    Discovery(#[from] crate::discover::Failed),
    #[error("that account is no longer configured")]
    AccountGone,

    // ---- accounts
    #[error("no such account")]
    NoSuchAccount,
    #[error(
        "{address} is not a work or school Microsoft account reading over IMAP, so there is \
         nothing to move to Microsoft Graph"
    )]
    NotMicrosoftImap { address: String },
    #[error("no JMAP session URL for {address}")]
    NoJmapSession { address: String },
    #[error(
        "no preset for {address:?}: it is not one of the known domains (gmail.com, googlemail.com), \
         so the servers have to be named"
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

    // ---- composing and sending
    #[error("the draft names identity {id}, which this account no longer has")]
    DraftIdentityGone { id: mail_domain::IdentityId },
    #[error(
        "this account has no identity to send as. It was added before identities were created \
         at setup, so it has to be added again"
    )]
    NoIdentityToSendAs,
    #[error("the account's own address stays: remove the account to stop sending as it")]
    OwnAddressStays,
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
    #[error(
        "{name} is not a picture the body can show. PNG, JPEG, GIF and WebP can; attach it instead"
    )]
    NotAnInlineImage { name: String },
    #[error("the pictures in the body would come to more than {limit}. Attach this one instead")]
    InlineImagesTooBig { limit: String },
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
        "that message has not been downloaded yet, so there is nothing to attach; it can be \
         forwarded inline instead"
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
    /// The incoming server was reached and would not take the sign-in, in its own words.
    /// `hint` is empty, or a sentence (with its space) about the likeliest thing to try.
    #[error("{host} did not accept the sign-in as {login}: {said}{hint}")]
    SignInRefused {
        host: String,
        login: String,
        said: String,
        hint: String,
    },
    /// The incoming server could not be reached to try the sign-in: nothing answered, or it would
    /// not finish the handshake. Not a refusal, so the person is not told the password was wrong.
    #[error("{host} could not be reached: {why}")]
    ServerUnreachable { host: String, why: String },
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
    #[error("on {address} folders are labels: a folder's mail arrives with its label")]
    FoldersAreLabels { address: String },
}

impl CoreError {
    /// The step that mends this failure, when there is one: the front end words it.
    pub fn remedy(&self) -> Option<Remedy> {
        Some(match self {
            CoreError::NoAccounts | CoreError::AddAnAccountFirst => Remedy::AddAccount,
            CoreError::HeadersOnly
            | CoreError::NotFetchedInFull
            | CoreError::ThreadNotFetchedInFull
            | CoreError::NotDownloaded
            | CoreError::NothingToRedirect => Remedy::Sync,
            CoreError::PublishedEvent => Remedy::SaveInvitation,
            CoreError::NoAccountFor { .. } => Remedy::ListAccounts,
            CoreError::NoFolder { address, folder } => Remedy::Folders {
                address: address.clone(),
                folder: folder.clone(),
            },
            CoreError::DaemonRunning => Remedy::StopDaemon,
            CoreError::NoJmapSession { address } => Remedy::NameJmapSession {
                address: address.clone(),
            },
            CoreError::NoPreset { address } => Remedy::NameServers {
                address: address.clone(),
            },
            CoreError::NoIdentityToSendAs => Remedy::ReAddAccount,
            CoreError::NoCredential { address, auth } => Remedy::SignIn {
                address: address.clone(),
                with: SignInWith::of(auth),
            },
            CoreError::Pgp(why) => return why.remedy(),
            CoreError::Smime(why) => return why.remedy(),
            _ => return None,
        })
    }

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

    /// A credential of `address` that could not be read: a sign-in to ask for when it is not
    /// there, a wait when the keyring did not answer ([`mail_secrets::SecretTrouble`] decides).
    pub(crate) fn unread_credential(
        address: &str,
        auth: &mail_domain::AuthPlan,
        trouble: mail_secrets::SecretTrouble,
    ) -> Self {
        match trouble {
            mail_secrets::SecretTrouble::Absent => CoreError::NoCredential {
                address: address.to_owned(),
                auth: auth.clone(),
            },
            mail_secrets::SecretTrouble::Unreachable(why) => CoreError::KeyringUnreachable {
                address: address.to_owned(),
                why,
            },
        }
    }

    /// For `map_err` on a file operation: `{path}: {error}`.
    pub fn at(path: impl AsRef<Path>) -> impl FnOnce(std::io::Error) -> CoreError {
        let path = path.as_ref().to_path_buf();
        move |source| CoreError::Io { path, source }
    }
}

pub(crate) use mail_runtime::Logged;

impl From<mail_secrets::CredentialError> for CoreError {
    fn from(error: mail_secrets::CredentialError) -> Self {
        CoreError::Runtime(error.into())
    }
}

impl Retryable for CoreError {
    fn retry(&self) -> Retry {
        match self {
            CoreError::Store(e) => e.retry(),
            CoreError::Runtime(e) => e.retry(),
            CoreError::Mime(e) => e.retry(),
            CoreError::Proto(e) => e.retry(),
            CoreError::PassFailed { retry, .. } => retry.clone(),
            CoreError::KeyringUnreachable { why, .. } => {
                mail_secrets::SecretTrouble::Unreachable(why).retry()
            }
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

impl From<mail_runtime::unsubscribe::UnsubscribeFailure> for CoreError {
    fn from(failure: mail_runtime::unsubscribe::UnsubscribeFailure) -> Self {
        CoreError::Runtime(mail_runtime::RuntimeError::Unsubscribe(failure))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_net::Failure;
    use mail_secrets::{CredentialError, SecretTrouble};
    use porter_secrets::SecretsError;

    fn read_failed(why: SecretsError) -> SecretTrouble {
        SecretTrouble::of(&CredentialError::Secrets(Failure::caused(why)))
    }

    /// A locked keyring asked the person to sign in again, which fixed nothing: only a missing
    /// credential is a sign-in, and a keyring that did not answer is waited for, in its own words.
    #[test]
    fn a_locked_keyring_is_a_wait_and_only_a_missing_credential_is_a_sign_in() {
        let auth = mail_domain::AuthPlan::OAuth {
            issuer: porter_provider::Issuer::Google,
            scopes: Vec::new(),
        };
        let missing = CoreError::unread_credential(
            "a@example.test",
            &auth,
            read_failed(SecretsError::Missing),
        );
        assert!(
            matches!(missing, CoreError::NoCredential { .. }),
            "{missing:?}"
        );
        // A missing credential keeps the answer it had: no waiting, the sign-in is the way on.
        assert!(!matches!(missing.retry(), Retry::After(_)), "{missing:?}");

        let locked = CoreError::unread_credential(
            "a@example.test",
            &auth,
            read_failed(SecretsError::Locked),
        );
        assert_eq!(locked.to_string(), "a@example.test: the keyring is locked");
        assert!(matches!(locked.retry(), Retry::After(_)), "{locked:?}");
        assert_eq!(locked.remedy(), None);

        let unavailable = CoreError::unread_credential(
            "a@example.test",
            &auth,
            read_failed(SecretsError::Unavailable),
        );
        assert_eq!(
            unavailable.to_string(),
            "a@example.test: the keyring is not reachable"
        );
        assert!(matches!(unavailable.retry(), Retry::After(_)));
    }
}
