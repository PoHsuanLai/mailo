//! What a preset fills an [`AccountPlan`] with, and the capabilities we *expect*. Data, not types.
//!
//! Nothing here is a domain concept. The runtime replaces the expected capabilities with what
//! the server actually advertises. Which provider an address belongs to is not decided here:
//! that is porter's provider files (`porter_provider`, claimed by `matching`) and its discovery
//! (`porter_discover`), and `mail_core::discover` maps what they answer onto these builders.
//! What stays here is what a provider file has no place for: the capabilities we expect of a
//! server, the OAuth presets of the two issuers mailo reads mail through, the Graph policy, and
//! the plans for servers named by hand.

use crate::account::{
    AccountCaps, AccountPlan, ArchiveMeans, AuthPlan, Condstore, ConnectionBudget, ExpungeMeans,
    FolderRoles, Incoming, LeaveOnServer, MoveExt, Outgoing, SaslMech, ServerLabels, ServerThreads,
    Supported, Tls, Username, WatchMode,
};
use crate::state::MailboxRole;
use chrono::{DateTime, Utc};
use porter_provider::Issuer;
use std::time::Duration;

mod jmap;
pub use jmap::{jmap, well_known};

/// A preset's contribution: configuration, plus a starting guess at capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    pub plan: AccountPlan,
    /// Replaced on first connect. Only a starting value, so the first sync has something to
    /// work with before `CAPABILITY` comes back.
    pub expected_caps: AccountCaps,
}

// ---------------------------------------------------------------------------------------
// The table.
//
// Everything in `expected_caps` below is an EXPECTED value, not a fact. The runtime replaces
// the whole `AccountCaps` with what `CAPABILITY` / `CAPA` / `LIST (SPECIAL-USE)` report on
// the first connect; these exist only so the first sync has something to plan against before
// that round trip completes. A preset that turns out to be wrong is a stale guess, never a
// persisted lie — `AccountPlan` (configured) and `AccountCaps` (discovered) are separate
// types for exactly this reason.
//
// This module is also the ONLY place in the workspace that may name a provider's mail settings.
// Domain types stay vendor-neutral (`ArchiveMeans::DropInbox`, not `GmailStyle`); the single
// exception is `Issuer::Google`, which names an authorization server rather than a mail provider.
//
// `identities` is always empty here. An `Identity` needs an `IdentityId` and an `AccountId`,
// and a preset has neither: minting one would make a preset impure (a fresh UUID per
// call, so two calls with the same arguments differ) and would invent an account id that no
// store row matches. The account-creation flow mints the `AccountId`, builds the default
// `Identity` from the address the user typed, and pushes it onto this empty vector.
// ---------------------------------------------------------------------------------------

/// Gmail's IMAP/SMTP OAuth scope. `https://mail.google.com/` is the only scope that grants
/// IMAP and SMTP access; `email` is requested alongside it so the callback can confirm which
/// account actually consented, which is the address we key everything else off.
const GMAIL_SCOPES: [&str; 2] = ["https://mail.google.com/", "email"];

/// Delegated scopes for a managed Microsoft 365 mailbox.
///
/// One per protocol, which is the difference from Google's single `mail.google.com`: Microsoft
/// grants IMAP and SMTP separately, and asking for only the first produces an account that syncs
/// and cannot send. `offline_access` is what returns a refresh token — without it the account
/// stops working an hour after it is added, which looks like a bug in the client.
///
/// These are the **delegated** permissions, for the authorization-code flow where the user signs
/// in themselves. The `IMAP.AccessAsApp` family, which Microsoft's documentation surrounds with
/// admin consent and a `New-ServicePrincipal` registration, is for client credentials — an
/// application reaching mailboxes with nobody present. That is not this, and its requirements do
/// not apply here.
const MICROSOFT_SCOPES: [&str; 4] = [
    "https://outlook.office.com/IMAP.AccessAsUser.All",
    "https://outlook.office.com/SMTP.Send",
    "offline_access",
    "openid",
];

/// Gmail's special-use paths. Sent as `FolderRoles` so the first sync can address
/// `[Gmail]/All Mail` before `LIST (SPECIAL-USE)` comes back. Gmail localises these paths
/// for some accounts, which is one more reason the runtime overwrites them.
fn gmail_folders() -> FolderRoles {
    FolderRoles(vec![
        ("INBOX".to_owned(), MailboxRole::Inbox),
        ("[Gmail]/All Mail".to_owned(), MailboxRole::Archive),
        ("[Gmail]/Sent Mail".to_owned(), MailboxRole::Sent),
        ("[Gmail]/Drafts".to_owned(), MailboxRole::Drafts),
        ("[Gmail]/Trash".to_owned(), MailboxRole::Trash),
        ("[Gmail]/Spam".to_owned(), MailboxRole::Spam),
    ])
}

/// How often to poll a server with no `IDLE`. Five minutes: often enough to feel live,
/// rare enough not to look like abuse to a small institution's server.
const POLL_EVERY: Duration = Duration::from_secs(5 * 60);

fn gmail(address: &str, now: DateTime<Utc>) -> Preset {
    Preset {
        plan: AccountPlan {
            address: address.to_owned(),
            incoming: Incoming::Imap {
                host: "imap.gmail.com".to_owned(),
                port: 993,
                tls: Tls::Implicit,
            },
            outgoing: Outgoing::Smtp {
                host: "smtp.gmail.com".to_owned(),
                port: 465,
                tls: Tls::Implicit,
            },
            auth: AuthPlan::OAuth {
                issuer: Issuer::Google,
                scopes: GMAIL_SCOPES.iter().map(|s| (*s).to_owned()).collect(),
            },
            identities: Vec::new(),
        },
        expected_caps: AccountCaps {
            labels: ServerLabels::Supported,
            threads: ServerThreads::ProviderId,
            watch: WatchMode::Idle,
            archive: ArchiveMeans::DropInbox,
            folders: gmail_folders(),
            condstore: Condstore::Supported,
            move_ext: MoveExt::Supported,
            // Never, on any account with server-side labels: Gmail routes EXPUNGE through an
            // expungeBehavior setting that may be `deleteForever` and cannot be read over IMAP.
            expunge: ExpungeMeans::Forbidden,
            // POP3 capabilities; meaningless over IMAP.
            top: Supported::Absent,
            pipelining: Supported::Absent,
            // Gmail tolerates roughly fifteen simultaneous IMAP connections and punishes excess
            // with a lockout measured in hours. Stay well under it.
            connections: ConnectionBudget { max: 5 },
            observed_at: now,
        },
    }
}

/// The preset that signs in with `issuer`.
///
/// Each issuer mailo has a mail provider for serves exactly one, so naming the issuer names the
/// servers too. `None` for an issuer that hosts no mail mailo reads (porter's list is longer
/// than mailo's: Dropbox is not a mail provider).
///
/// Microsoft's is a work or school account's, which reads and sends through Graph
/// ([`microsoft_for`]): discovery refuses a personal Microsoft address before it gets here, so
/// every address that does is an organisation's. A caller that may hold a personal one, such as
/// a sign-in that learns the address only afterwards, asks [`microsoft_for`] with what it knows.
pub fn preset_for_issuer(issuer: Issuer, address: &str, now: DateTime<Utc>) -> Option<Preset> {
    match issuer {
        Issuer::Google => Some(gmail(address, now)),
        Issuer::Microsoft => Some(microsoft_for(MicrosoftAccount::Organisation, address, now)),
        Issuer::Dropbox | Issuer::Box | Issuer::Fastmail | Issuer::OpenRouter | Issuer::OpenAi => {
            None
        }
    }
}

/// The issuer an autoconfig document's `<oAuth2><issuer>` names, when it is one we have a
/// registration for. Any other issuer is somebody else's authorization server, and a client id
/// for it cannot be guessed.
pub fn issuer_named(issuer_host: &str) -> Option<Issuer> {
    match issuer_host.trim().to_ascii_lowercase().as_str() {
        "accounts.google.com" => Some(Issuer::Google),
        "login.microsoftonline.com" => Some(Issuer::Microsoft),
        _ => None,
    }
}

/// The issuer whose tokens a mail server at `host` takes, for a document that offers OAuth2 on a
/// server without naming the issuer.
pub fn issuer_for_server(host: &str) -> Option<Issuer> {
    let host = host.trim().to_ascii_lowercase();
    if under(&host, "gmail.com") || under(&host, "googlemail.com") {
        return Some(Issuer::Google);
    }
    if under(&host, "office365.com") || under(&host, "outlook.office.com") {
        return Some(Issuer::Microsoft);
    }
    None
}

/// Whether the submission server at `host` files a copy of what it accepts in the sender's Sent
/// folder itself.
///
/// Gmail and Microsoft's servers do, and a client that uploads a copy as well leaves two. Any
/// other server is taken not to: SMTP says only that a message was accepted (RFC 6409), and a
/// plain Dovecot or Postfix files nothing, so the client's own upload is the only copy there will
/// be. A wrong "no" here costs a duplicate in Sent; a wrong "yes" costs the sender's only record.
pub fn files_sent_itself(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    [
        "gmail.com",
        "googlemail.com",
        "office365.com",
        "outlook.office.com",
        "outlook.com",
    ]
    .iter()
    .any(|domain| under(&host, domain))
}

/// Hosts where a password will not authenticate, whatever the user types.
///
/// Both of these providers switched off password authentication for IMAP/POP/SMTP, and both fail
/// in the same unhelpful way: the credential is accepted by the keyring, stored, and rejected by
/// the server at the first sync with a message that says nothing about why. Saying it at setup
/// costs one line and saves the user diagnosing an authentication failure that is not theirs.
///
/// Advice, not a refusal. A tenant may have re-enabled something, an app password may exist, and
/// the user knows their own account better than a table does — so this explains and proceeds.
pub fn password_warning(host: &str) -> Option<PasswordWarning> {
    let host = host.trim().to_ascii_lowercase();
    let host = host.rsplit_once(':').map_or(host.as_str(), |(h, _)| h);

    if under(host, "office365.com") || under(host, "outlook.office.com") {
        return Some(PasswordWarning::Microsoft365);
    }
    if under(host, "gmail.com") || under(host, "googlemail.com") {
        return Some(PasswordWarning::Google);
    }
    None
}

/// Why a password will not authenticate at a host, as far as this table knows. The words for it
/// are the caller's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordWarning {
    /// Password authentication for IMAP, POP and SMTP is switched off; the mailbox needs OAuth.
    Microsoft365,
    /// Account passwords are refused, but an App Password (which needs two-factor
    /// authentication) still works.
    Google,
}

/// Whether `host` is `domain` or a subdomain of it.
///
/// `ends_with(domain)` is the version of this that treats `evil-office365.com` as Microsoft. The
/// dot is what makes a suffix match mean "inside that domain" rather than "spelled similarly",
/// and getting it wrong here only produces a misleading warning — but the same mistake in a
/// security decision is how the wrong host gets trusted.
fn under(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// Where a manually configured account's servers are.
///
/// A separate type from [`AccountPlan`] because the caller has typed a host and maybe a port,
/// and nothing else: the auth plan, the identities and the capabilities are this module's to
/// decide, not the command line's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manual {
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    /// The login name, when it is not the whole address.
    pub login: Option<String>,
}

/// A plan for a server the preset table has never heard of.
///
/// Every preset here was written from a measured probe of a specific host. This one cannot
/// be, so it assumes only what is nearly universal and safe to be wrong about:
///
/// - **Implicit TLS.** Never `StartTlsRequired` and never `Plaintext`. An opportunistic upgrade
///   is strippable and a cleartext password is a cleartext password; 993 and 465 are what a
///   server offering implicit TLS uses, and one that does not will refuse the connection rather
///   than quietly downgrade.
/// - **`LOGIN`-shaped auth with `PLAIN` offered.** `AuthPlan::Password`, because a manually
///   configured account is one the user has a password for — an OAuth account needs an issuer,
///   a client id and a scope list, none of which can be guessed from a hostname.
/// - **Capabilities at their safe end.** Labels local, no CONDSTORE, no MOVE, and expunging
///   forbidden. The first connection replaces these with what the server actually advertises;
///   until then, every default is the one that does the least.
pub fn manual(address: &str, manual: &Manual, now: DateTime<Utc>) -> Preset {
    Preset {
        plan: AccountPlan {
            address: address.to_owned(),
            incoming: Incoming::Imap {
                host: manual.imap_host.clone(),
                port: manual.imap_port,
                tls: Tls::Implicit,
            },
            outgoing: Outgoing::Smtp {
                host: manual.smtp_host.clone(),
                port: manual.smtp_port,
                tls: Tls::Implicit,
            },
            auth: AuthPlan::Password {
                username: match &manual.login {
                    Some(name) => Username::Literal(name.clone()),
                    None => Username::SameAsAddress,
                },
                sasl: vec![SaslMech::Plain],
            },
            identities: Vec::new(),
        },
        expected_caps: AccountCaps {
            labels: ServerLabels::LocalOnly,
            threads: ServerThreads::Jwz,
            watch: WatchMode::Poll {
                every: std::time::Duration::from_secs(300),
            },
            // Not `MoveToFolder`: we do not yet know this server has an Archive folder, and
            // archiving into one that does not exist loses the message.
            archive: ArchiveMeans::LocalOnly,
            folders: FolderRoles(Vec::new()),
            condstore: Condstore::Absent,
            move_ext: MoveExt::Absent,
            expunge: ExpungeMeans::Forbidden,
            // POP3 vocabulary; on IMAP the equivalent is BODY.PEEK, which is always available.
            top: Supported::Absent,
            pipelining: Supported::Absent,
            connections: ConnectionBudget { max: 1 },
            observed_at: now,
        },
    }
}

/// [`Manual`], for a server that receives over POP3 instead of IMAP.
///
/// A sibling rather than a field on [`Manual`], whose fields are named for IMAP and are part of
/// the frozen interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPop3 {
    pub pop3_host: String,
    pub pop3_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    /// The login name, when it is not the whole address.
    pub login: Option<String>,
}

/// A plan for a POP3 server the preset table has never heard of.
///
/// The same assumptions as [`manual`], plus two that only POP3 needs:
///
/// - **Mail stays on the server.** [`LeaveOnServer::Keep`], always. With delete-after-fetch this
///   client becomes the only copy of the user's mail, and a first sync interrupted halfway loses
///   whatever it had already deleted.
/// - **Capabilities are asked for before anything else.** `expected_caps` is dated at the epoch,
///   so the first pass counts it as stale and reads `CAPA` before fetching. The field that
///   matters is `TOP`: without it headers cannot be fetched without `RETR`, which marks mail
///   read on the server, and the backend refuses rather than do that. Assuming `TOP` would be a
///   guess; asking costs one round trip.
pub fn manual_pop3(address: &str, manual: &ManualPop3, _now: DateTime<Utc>) -> Preset {
    Preset {
        plan: AccountPlan {
            address: address.to_owned(),
            incoming: Incoming::Pop3 {
                host: manual.pop3_host.clone(),
                port: manual.pop3_port,
                tls: Tls::Implicit,
                leave: LeaveOnServer::Keep,
            },
            outgoing: Outgoing::Smtp {
                host: manual.smtp_host.clone(),
                port: manual.smtp_port,
                tls: Tls::Implicit,
            },
            auth: AuthPlan::Password {
                username: match &manual.login {
                    Some(name) => Username::Literal(name.clone()),
                    None => Username::SameAsAddress,
                },
                sasl: vec![SaslMech::Plain],
            },
            identities: Vec::new(),
        },
        expected_caps: AccountCaps {
            // POP3 has no keywords and no folders at all, so every one of these is local.
            labels: ServerLabels::LocalOnly,
            threads: ServerThreads::Jwz,
            watch: WatchMode::Poll { every: POLL_EVERY },
            archive: ArchiveMeans::LocalOnly,
            folders: FolderRoles(Vec::new()),
            condstore: Condstore::Absent,
            move_ext: MoveExt::Absent,
            // POP3 has no EXPUNGE; deletion is DELE, governed by LeaveOnServer.
            expunge: ExpungeMeans::Forbidden,
            top: Supported::Absent,
            pipelining: Supported::Absent,
            // One maildrop, and many POP3 servers lock it against a second session.
            connections: ConnectionBudget { max: 1 },
            observed_at: DateTime::<Utc>::UNIX_EPOCH,
        },
    }
}

/// A Microsoft mailbox over IMAP and SMTP, for an address the table cannot recognise.
///
/// Public because a custom tenant domain is the common case and `--microsoft --receive imap` is
/// how the user says so. A work or school account is otherwise set up over Graph
/// ([`microsoft_for`]); this is what [`send_through_graph`] and [`receive_through_graph`] start
/// from, and what a personal account keeps.
pub fn microsoft_preset(address: &str, now: DateTime<Utc>) -> Preset {
    microsoft(address, now)
}

/// Microsoft's tenant for personal accounts (outlook.com, hotmail.com, live.com, msn.com): the
/// `tid` a token or ID token carries for every one of them. A work or school account carries its
/// organisation's own.
pub const MICROSOFT_CONSUMER_TENANT: &str = "9188040d-6c67-4c5b-b112-36a304b66dad";

/// Which kind of Microsoft account an address is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrosoftAccount {
    /// A personal Microsoft account: outlook.com, hotmail.com, live.com, msn.com and the rest of
    /// Microsoft's consumer domains.
    Personal,
    /// A work or school account, in an organisation's Microsoft Entra ID tenant.
    Organisation,
}

/// Which kind of Microsoft account signed in: the tenant the sign-in named when it named one,
/// otherwise what the address's domain says (`by_domain`).
///
/// The tenant first because it is the issuer's own word and the domain is only a guess from
/// a name: a personal account may sign in with any address it was created with, and an
/// organisation may use any domain it owns. Microsoft gives every personal account the one
/// consumer tenant ([`MICROSOFT_CONSUMER_TENANT`]); any other tenant is an organisation's.
pub fn microsoft_account(tenant: Option<&str>, by_domain: MicrosoftAccount) -> MicrosoftAccount {
    match tenant.map(str::trim) {
        Some(tid) if tid.eq_ignore_ascii_case(MICROSOFT_CONSUMER_TENANT) => {
            MicrosoftAccount::Personal
        }
        Some(tid) if !tid.is_empty() => MicrosoftAccount::Organisation,
        _ => by_domain,
    }
}

/// How a Microsoft account of `kind` is set up.
///
/// A work or school account reads and sends through Microsoft Graph, as Thunderbird and
/// Evolution set up Microsoft 365: an organisation can switch IMAP off per mailbox, and it is
/// off by default on some plans, while Graph is what Microsoft keeps for mail clients. A personal
/// account keeps IMAP and SMTP.
pub fn microsoft_for(kind: MicrosoftAccount, address: &str, now: DateTime<Utc>) -> Preset {
    match kind {
        MicrosoftAccount::Personal => microsoft(address, now),
        MicrosoftAccount::Organisation => receive_through_graph(microsoft(address, now)),
    }
}

/// What a Microsoft sign-in asks for when the kind of account is not known until it is done:
/// both plans' scopes, IMAP's first, so either plan finds its consent in hand afterwards.
///
/// IMAP's resource first because the code is redeemed for the first one named: a personal
/// account then holds an IMAP token at once, and an organisation's mints its Graph token from
/// the refresh token as every Graph account does.
pub fn microsoft_either_scopes() -> Vec<String> {
    let mut scopes: Vec<String> = MICROSOFT_SCOPES.iter().map(|s| (*s).to_owned()).collect();
    for scope in GRAPH_SCOPES {
        if !scopes.iter().any(|s| s == scope) {
            scopes.push(scope.to_owned());
        }
    }
    scopes
}

/// The delegated permission Graph's `sendMail` needs.
pub const GRAPH_SEND_SCOPE: &str = "https://graph.microsoft.com/Mail.Send";

/// The delegated permission a message over Graph's 4 MB request limit needs besides
/// [`GRAPH_SEND_SCOPE`]: it is created as a draft in the mailbox, its attachments uploaded to
/// the draft, and the draft sent.
pub const GRAPH_WRITE_SCOPE: &str = "https://graph.microsoft.com/Mail.ReadWrite";

/// The delegated permission setting the mailbox's automatic replies needs: the vacation reply of
/// an account that reads through Graph.
pub const GRAPH_SETTINGS_SCOPE: &str = "https://graph.microsoft.com/MailboxSettings.ReadWrite";

/// What an account that reads and sends through Graph signs in for: Thunderbird's set, and the
/// mailbox's settings.
///
/// `Mail.ReadWrite` to read, flag and move, `MailboxFolder.ReadWrite` for the folders,
/// `Mail.Send` to send, `MailboxSettings.ReadWrite` for the vacation reply (Exchange's automatic
/// replies), `User.Read` for whose account it is, and `offline_access` for a refresh token,
/// without which the account stops an hour after it is added. All Graph's, so one token serves
/// both directions. `Mail.ReadWrite` first: nothing depends on it, but it is the one every other
/// permission here would be useless without.
pub const GRAPH_SCOPES: [&str; 6] = [
    GRAPH_WRITE_SCOPE,
    "https://graph.microsoft.com/MailboxFolder.ReadWrite",
    GRAPH_SEND_SCOPE,
    GRAPH_SETTINGS_SCOPE,
    "https://graph.microsoft.com/User.Read",
    "offline_access",
];

/// `preset`, sending through Microsoft Graph instead of SMTP.
///
/// For a tenant that refuses SMTP AUTH (`535 5.7.139`). The sign-in asks for Graph's
/// `Mail.Send` and `Mail.ReadWrite` in place of `SMTP.Send`: one consent covers both
/// resources, and the runtime exchanges the refresh token for a Graph token when it sends, since
/// one access token is only ever good for one of them.
pub fn send_through_graph(mut preset: Preset) -> Preset {
    preset.plan.outgoing = Outgoing::Graph;
    if let AuthPlan::OAuth { scopes, .. } = &mut preset.plan.auth {
        scopes.retain(|s| !s.ends_with("/SMTP.Send"));
        for scope in [GRAPH_SEND_SCOPE, GRAPH_WRITE_SCOPE] {
            if !scopes.iter().any(|s| s == scope) {
                scopes.push(scope.to_owned());
            }
        }
    }
    preset
}

/// How often an account that reads through Graph looks for new mail, at most.
///
/// Graph pushes change notifications only to a public HTTPS webhook, which a desktop client does
/// not have, so this polls. Fifteen seconds is cheap here and would not be over IMAP: a pass
/// that resumes is one request per followed folder, the delta link it stored last time, and one
/// with nothing to report is answered with no messages. A mailbox with many folders polls
/// slower than this ([`graph_poll_every`]).
pub const GRAPH_POLL_EVERY: Duration = Duration::from_secs(15);

/// The share of a Graph poll each folder of the mailbox takes.
///
/// Graph throttles a mailbox at about 10,000 requests in ten minutes from one application. A
/// pass asks one delta per folder, so at a quarter second a folder the polling alone stays
/// under 2,400 requests in ten minutes, a quarter of that, and the rest is left for bodies,
/// changes and searches. Sixty folders still poll every fifteen seconds; two hundred and forty
/// poll every minute, as every Graph account used to.
pub const GRAPH_POLL_PER_FOLDER: Duration = Duration::from_millis(250);

/// How often an account that reads through Graph and lists `folders` folders looks for new mail:
/// [`GRAPH_POLL_EVERY`], or slower where the folders would make that many requests too many
/// ([`GRAPH_POLL_PER_FOLDER`]).
pub fn graph_poll_every(folders: usize) -> Duration {
    let folders = u32::try_from(folders).unwrap_or(u32::MAX);
    GRAPH_POLL_PER_FOLDER
        .saturating_mul(folders)
        .max(GRAPH_POLL_EVERY)
}

/// `preset`, receiving and sending through Microsoft Graph instead of IMAP and SMTP.
///
/// How a work or school account is set up ([`microsoft_for`]), and the way out for a tenant that
/// switched IMAP off. Every scope the sign-in asks for is Graph's ([`GRAPH_SCOPES`]), so one
/// token serves both directions: there is no second resource to exchange the refresh token for.
///
/// The capabilities are what Graph offers every mailbox, not a guess about one: folders rather
/// than labels, a move that is a move, nothing ever expunged, and polling. The folder roles are
/// left for the first listing, and `observed_at` is the epoch so that listing happens before the
/// first sync rather than after it.
pub fn receive_through_graph(mut preset: Preset) -> Preset {
    preset.plan.incoming = Incoming::Graph;
    preset.plan.outgoing = Outgoing::Graph;
    if let AuthPlan::OAuth { scopes, .. } = &mut preset.plan.auth {
        *scopes = GRAPH_SCOPES.iter().map(|s| (*s).to_owned()).collect();
    }
    preset.expected_caps = AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: GRAPH_POLL_EVERY,
        },
        // Graph's well-known name, which every mailbox answers to whatever its language.
        archive: ArchiveMeans::MoveToFolder("archive".to_owned()),
        folders: FolderRoles(Vec::new()),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 4 },
        observed_at: DateTime::<Utc>::UNIX_EPOCH,
    };
    preset
}

/// The name the local-only account is stored under.
///
/// Not an address, on purpose: it contains a space, so no real mailbox can ever collide with it,
/// and it is what `account list` prints and what `--account` would be given.
pub const LOCAL_FOLDERS: &str = "local folders";

/// The local-only account imported mail lands in: no server in either direction.
///
/// `auth` is a password plan with no mechanisms because the plan must name one and this account
/// never authenticates to anything: sync skips an [`Incoming::Local`] account before any
/// credential is asked for. The capabilities say everything is local, which is the truth rather
/// than an expectation a connection will replace — there is no connection.
pub fn local_folders(now: DateTime<Utc>) -> Preset {
    Preset {
        plan: AccountPlan {
            address: LOCAL_FOLDERS.to_owned(),
            incoming: Incoming::Local,
            outgoing: Outgoing::Nowhere,
            auth: AuthPlan::Password {
                username: Username::SameAsAddress,
                sasl: Vec::new(),
            },
            identities: Vec::new(),
        },
        expected_caps: AccountCaps {
            labels: ServerLabels::LocalOnly,
            threads: ServerThreads::Jwz,
            watch: WatchMode::Poll { every: POLL_EVERY },
            archive: ArchiveMeans::LocalOnly,
            folders: FolderRoles(Vec::new()),
            condstore: Condstore::Absent,
            move_ext: MoveExt::Absent,
            expunge: ExpungeMeans::Forbidden,
            top: Supported::Absent,
            pipelining: Supported::Absent,
            connections: ConnectionBudget { max: 1 },
            observed_at: now,
        },
    }
}

/// A managed Microsoft 365 mailbox, work or school.
///
/// The plan's first outside test of "a provider is a value, not a type": adding this required an
/// `Issuer` variant, an endpoints row and this function. `Incoming::Imap`, `Outgoing::Smtp`
/// and `ImapBackend` are untouched, and the compiler found the single site that had to change.
///
/// Capabilities are at their cautious end rather than guessed from documentation. Exchange
/// Online's answers vary by tenant — IMAP and POP can be disabled per mailbox, SMTP AUTH is off
/// by default in many tenants — and `refresh_caps` replaces all of this on the first connection
/// with what the server actually says.
fn microsoft(address: &str, now: DateTime<Utc>) -> Preset {
    Preset {
        plan: AccountPlan {
            address: address.to_owned(),
            incoming: Incoming::Imap {
                host: "outlook.office365.com".to_owned(),
                port: 993,
                tls: Tls::Implicit,
            },
            outgoing: Outgoing::Smtp {
                host: "smtp.office365.com".to_owned(),
                port: 587,
                // The one place Microsoft differs from Gmail's shape. Port 587 is submission
                // with a mandatory STARTTLS; Exchange Online does not offer implicit TLS on 465
                // for SMTP AUTH. `StartTlsRequired`, never opportunistic — a failure to upgrade
                // aborts rather than falling back to a cleartext bearer token.
                tls: Tls::StartTlsRequired,
            },
            auth: AuthPlan::OAuth {
                issuer: Issuer::Microsoft,
                scopes: MICROSOFT_SCOPES.iter().map(|s| (*s).to_owned()).collect(),
            },
            identities: Vec::new(),
        },
        expected_caps: AccountCaps {
            // Exchange Online has folders, not labels.
            labels: ServerLabels::LocalOnly,
            threads: ServerThreads::Jwz,
            watch: WatchMode::Idle,
            archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
            // Left empty on purpose: the folder names are localised per mailbox, and
            // `LIST (SPECIAL-USE)` is the only thing that knows them. Guessing "Sent Items"
            // files mail into a folder that may not exist.
            folders: FolderRoles(Vec::new()),
            condstore: Condstore::Absent,
            move_ext: MoveExt::Absent,
            // Same rule as everywhere: never inferred, never on by default.
            expunge: ExpungeMeans::Forbidden,
            top: Supported::Absent,
            pipelining: Supported::Absent,
            connections: ConnectionBudget { max: 4 },
            observed_at: now,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-03-01T12:00:00Z")
            .expect("literal is valid RFC 3339")
            .with_timezone(&Utc)
    }

    /// Google's preset keeps the address exactly as typed, whatever its case, and starts the
    /// first sync from Gmail's folder paths.
    #[test]
    fn the_google_preset_keeps_the_address_as_typed() {
        for address in [
            "someone@gmail.com",
            "someone@GMAIL.com",
            "Someone@GoogleMail.COM",
            "me@firm.example",
        ] {
            let preset = preset_for_issuer(Issuer::Google, address, at()).expect(address);
            assert_eq!(
                preset.plan.incoming,
                Incoming::Imap {
                    host: "imap.gmail.com".to_owned(),
                    port: 993,
                    tls: Tls::Implicit,
                },
                "{address}"
            );
            assert_eq!(preset.plan.address, address, "{address}");
            assert_eq!(preset.expected_caps.observed_at, at(), "{address}");
            assert_eq!(
                preset.expected_caps.folders.path(MailboxRole::Archive),
                Some("[Gmail]/All Mail"),
                "{address}"
            );
        }
    }

    /// A POP3 server named by hand keeps mail on the server, and asks what it can do first.
    #[test]
    fn a_manual_pop3_server_keeps_mail_and_asks_before_fetching() {
        let preset = manual_pop3(
            "someone@example.edu",
            &ManualPop3 {
                pop3_host: "pop.example.edu".to_owned(),
                pop3_port: 995,
                smtp_host: "smtp.example.edu".to_owned(),
                smtp_port: 465,
                login: Some("someone".to_owned()),
            },
            at(),
        );
        assert_eq!(
            preset.plan.incoming,
            Incoming::Pop3 {
                host: "pop.example.edu".to_owned(),
                port: 995,
                tls: Tls::Implicit,
                leave: LeaveOnServer::Keep,
            }
        );
        assert_eq!(preset.plan.username(), "someone");
        assert!(
            (at() - preset.expected_caps.observed_at) > chrono::TimeDelta::try_days(1).unwrap(),
            "expected capabilities must read as stale, so CAPA runs before the first fetch"
        );
        assert_eq!(preset.expected_caps.top, Supported::Absent, "not assumed");
    }

    /// Graph replaces SMTP for sending, and the sign-in asks for Graph's permission in place of
    /// SMTP's. Receiving is untouched.
    #[test]
    fn sending_through_graph_swaps_the_smtp_permission_for_graphs() {
        let smtp = microsoft_preset("me@contoso.example", at());
        let graph = send_through_graph(smtp.clone());
        assert_eq!(graph.plan.outgoing, Outgoing::Graph);
        assert_eq!(graph.plan.incoming, smtp.plan.incoming);
        let AuthPlan::OAuth { scopes, .. } = &graph.plan.auth else {
            panic!("{:?}", graph.plan.auth)
        };
        assert!(scopes.iter().any(|s| s == GRAPH_SEND_SCOPE));
        assert!(
            scopes.iter().any(|s| s == GRAPH_WRITE_SCOPE),
            "a message over 4 MB is sent as a draft, which needs Mail.ReadWrite"
        );
        assert!(!scopes.iter().any(|s| s.ends_with("/SMTP.Send")));
        assert!(
            scopes[0].ends_with("/IMAP.AccessAsUser.All"),
            "the sign-in's own token is for the first resource named, and it must be IMAP's"
        );
        assert_eq!(
            send_through_graph(graph.clone()),
            graph,
            "applying it twice changes nothing"
        );
    }

    /// Reading through Graph asks for Graph's permissions only, so the sign-in's own token is a
    /// Graph token and serves both directions; the capabilities are asked for before any fetch.
    #[test]
    fn receiving_through_graph_asks_for_graph_alone() {
        let graph = receive_through_graph(microsoft_preset("me@contoso.example", at()));
        assert_eq!(graph.plan.incoming, Incoming::Graph);
        assert_eq!(graph.plan.outgoing, Outgoing::Graph);
        let AuthPlan::OAuth { scopes, .. } = &graph.plan.auth else {
            panic!("{:?}", graph.plan.auth)
        };
        assert_eq!(scopes[0], GRAPH_WRITE_SCOPE);
        assert!(scopes.iter().any(|s| s == GRAPH_SEND_SCOPE));
        assert!(
            scopes
                .iter()
                .filter(|s| s.starts_with("https://"))
                .all(|s| s.starts_with("https://graph.microsoft.com/")),
            "one resource, so one token: {scopes:?}"
        );
        assert!(scopes.iter().any(|s| s == "offline_access"));
        let caps = &graph.expected_caps;
        assert_eq!(caps.expunge, ExpungeMeans::Forbidden);
        assert_eq!(caps.labels, ServerLabels::LocalOnly);
        assert_eq!(
            caps.watch,
            WatchMode::Poll {
                every: Duration::from_secs(15)
            },
            "every fifteen seconds until the folders are listed"
        );
        assert!(
            (at() - caps.observed_at) > chrono::TimeDelta::try_days(1).unwrap(),
            "stale, so the folders are listed before the first sync"
        );
    }

    /// A Graph mailbox polls every fifteen seconds until its folders would make that too many
    /// requests, and then a quarter second a folder: never faster, and slower only for size.
    #[test]
    fn a_graph_mailbox_polls_slower_only_for_its_folders() {
        let secs = Duration::from_secs;
        let table = [
            (0, secs(15)),
            (1, secs(15)),
            (60, secs(15)),
            (61, Duration::from_millis(15_250)),
            (120, secs(30)),
            (240, secs(60)),
            (2000, secs(500)),
        ];
        for (folders, every) in table {
            assert_eq!(graph_poll_every(folders), every, "{folders} folders");
        }
    }

    /// The server names a mail host signs in with an issuer by, and no lookalike does.
    #[test]
    fn a_server_names_the_issuer_whose_tokens_it_takes() {
        assert_eq!(issuer_named("accounts.google.com"), Some(Issuer::Google));
        assert_eq!(
            issuer_named("login.microsoftonline.com"),
            Some(Issuer::Microsoft)
        );
        assert_eq!(issuer_named("auth.example.net"), None);
        assert_eq!(issuer_for_server("imap.gmail.com"), Some(Issuer::Google));
        assert_eq!(
            issuer_for_server("outlook.office365.com"),
            Some(Issuer::Microsoft)
        );
        assert_eq!(issuer_for_server("imap.evil-gmail.com"), None);
        assert_eq!(issuer_for_server("imap.example.net"), None);
    }

    /// Personal or organisational, from the tenant when the sign-in named one and from the
    /// domain when it did not; and how each is set up.
    #[test]
    fn a_microsoft_account_is_told_apart_by_its_tenant_then_its_domain() {
        use MicrosoftAccount::{Organisation, Personal};
        let tenant = "0b5f3c2e-1d4a-4f6b-9c8d-7e6f5a4b3c2d";
        // (tenant the sign-in named, what the domain says, what it is)
        let cases = [
            (Some(MICROSOFT_CONSUMER_TENANT), Organisation, Personal),
            (Some(MICROSOFT_CONSUMER_TENANT), Personal, Personal),
            (
                Some("9188040D-6C67-4C5B-B112-36A304B66DAD"),
                Organisation,
                Personal,
            ),
            (Some(tenant), Personal, Organisation),
            (Some(tenant), Organisation, Organisation),
            (None, Personal, Personal),
            (None, Organisation, Organisation),
            (Some(""), Personal, Personal),
            (Some("  "), Organisation, Organisation),
        ];
        for (tid, by_domain, want) in cases {
            assert_eq!(
                microsoft_account(tid, by_domain),
                want,
                "{tid:?} {by_domain:?}"
            );
        }

        // (kind, incoming, outgoing)
        let imap = Incoming::Imap {
            host: "outlook.office365.com".to_owned(),
            port: 993,
            tls: Tls::Implicit,
        };
        let smtp = Outgoing::Smtp {
            host: "smtp.office365.com".to_owned(),
            port: 587,
            tls: Tls::StartTlsRequired,
        };
        let plans = [
            (Personal, imap, smtp),
            (Organisation, Incoming::Graph, Outgoing::Graph),
        ];
        for (kind, incoming, outgoing) in plans {
            let preset = microsoft_for(kind, "me@contoso.example", at());
            assert_eq!(preset.plan.incoming, incoming, "{kind:?}");
            assert_eq!(preset.plan.outgoing, outgoing, "{kind:?}");
            assert_eq!(preset.plan.address, "me@contoso.example");
        }
        assert_eq!(
            preset_for_issuer(Issuer::Microsoft, "me@contoso.example", at()),
            Some(microsoft_for(Organisation, "me@contoso.example", at())),
            "discovery refuses a personal address, so what reaches the issuer's preset is work"
        );
    }

    /// A work or school account signs in for Thunderbird's five Graph permissions, and nothing of
    /// IMAP's or SMTP's.
    #[test]
    fn an_organisations_account_signs_in_for_graph_alone() {
        let preset = microsoft_for(MicrosoftAccount::Organisation, "me@contoso.example", at());
        let AuthPlan::OAuth { issuer, scopes } = &preset.plan.auth else {
            panic!("{:?}", preset.plan.auth)
        };
        assert_eq!(*issuer, Issuer::Microsoft);
        let mut got: Vec<&str> = scopes.iter().map(String::as_str).collect();
        got.sort_unstable();
        assert_eq!(
            got,
            [
                "https://graph.microsoft.com/Mail.ReadWrite",
                "https://graph.microsoft.com/Mail.Send",
                "https://graph.microsoft.com/MailboxFolder.ReadWrite",
                "https://graph.microsoft.com/MailboxSettings.ReadWrite",
                "https://graph.microsoft.com/User.Read",
                "offline_access",
            ]
        );
    }

    /// A sign-in that learns the kind of account only afterwards consents to both plans once
    /// each, IMAP's resource first.
    #[test]
    fn either_kinds_scopes_are_both_plans_once_each() {
        let either = microsoft_either_scopes();
        assert_eq!(
            either[0],
            "https://outlook.office.com/IMAP.AccessAsUser.All"
        );
        for kind in [MicrosoftAccount::Personal, MicrosoftAccount::Organisation] {
            let AuthPlan::OAuth { scopes, .. } = microsoft_for(kind, "", at()).plan.auth else {
                panic!("an OAuth plan");
            };
            assert!(
                scopes.iter().all(|s| either.contains(s)),
                "{kind:?}: {scopes:?}"
            );
        }
        let mut unique = either.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), either.len(), "{either:?}");
    }

    /// Only the issuers mailo reads mail through have a preset.
    #[test]
    fn an_issuer_with_no_mail_has_no_preset() {
        assert!(preset_for_issuer(Issuer::Dropbox, "me@firm.example", at()).is_none());
    }

    /// `(host, the warning it gets, or None for no warning)`.
    const WARNINGS: &[(&str, Option<PasswordWarning>)] = &[
        // The user has a work Outlook mailbox and a school one, both managed tenants. Storing a
        // password for either produces an authentication failure at the first sync that says
        // nothing about the cause.
        ("outlook.office365.com", Some(PasswordWarning::Microsoft365)),
        ("smtp.office365.com", Some(PasswordWarning::Microsoft365)),
        ("outlook.office.com", Some(PasswordWarning::Microsoft365)),
        ("OUTLOOK.OFFICE365.COM", Some(PasswordWarning::Microsoft365)),
        (
            "outlook.office365.com:993",
            Some(PasswordWarning::Microsoft365),
        ),
        // Different advice, because the outcome is different: Google still accepts an App
        // Password, so telling the user "use OAuth" would send them to build something they do
        // not need.
        ("imap.gmail.com", Some(PasswordWarning::Google)),
        ("smtp.googlemail.com", Some(PasswordWarning::Google)),
        // Advice, not a gate. Anything not known to have switched passwords off gets no warning,
        // because inventing one teaches the user to ignore them.
        ("pop.example.edu", None),
        ("imap.example.com", None),
        ("mail.fastmail.com", None),
        ("", None),
        ("notahost", None),
        // Suffix matching on a hostname is how `evil-office365.com.attacker.test` gets treated
        // as Microsoft. These lookalikes must not match.
        ("office365.com.example.test", None),
        ("notgmail.com.example.test", None),
        // `ends_with` alone treats both of these as the real thing.
        ("evil-office365.com", None),
        ("notgmail.com", None),
        // And the genuine article still matches bare (`imap.gmail.com` above is a subdomain).
        ("office365.com", Some(PasswordWarning::Microsoft365)),
    ];

    #[test]
    fn password_warning_by_host() {
        for (host, want) in WARNINGS {
            assert_eq!(password_warning(host), *want, "{host:?}");
        }
    }
}
