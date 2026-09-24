//! Domain-to-configuration presets. Data, not types.
//!
//! Nothing here is a domain concept. A preset fills an [`AccountPlan`] and supplies the
//! capabilities we *expect*; the runtime replaces those with what the server actually
//! advertises. This is the only place in the workspace that may mention a specific provider,
//! and even then only as a table key.

use crate::account::{
    AccountCaps, AccountPlan, ArchiveMeans, AuthPlan, Condstore, ConnectionBudget, ExpungeMeans,
    FolderRoles, Incoming, LeaveOnServer, MoveExt, OAuthIssuer, Outgoing, SaslMech, ServerLabels,
    ServerThreads, Supported, Tls, Username, WatchMode,
};
use crate::state::MailboxRole;
use chrono::{DateTime, Utc};
use std::time::Duration;

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
// This module is also the ONLY place in the workspace that may name a provider. Domain types
// stay vendor-neutral (`ArchiveMeans::DropInbox`, not `GmailStyle`); the single exception is
// `OAuthIssuer::Google`, which names an authorization server rather than a mail provider.
//
// `identities` is always empty here. An `Identity` needs an `IdentityId` and an `AccountId`,
// and a preset has neither: minting one would make `preset_for` impure (a fresh UUID per
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

/// Look up configuration for an address.
///
/// Returns `None` for an unknown domain, which the UI turns into a manual setup form.
///
/// The domain is matched case-insensitively (ASCII: mail domains are IDNA-encoded by the
/// time they reach us). An address with no `@`, or with an empty local part or domain, has no
/// preset and returns `None` rather than panicking — this string comes from a text field.
pub fn preset_for(address: &str, now: DateTime<Utc>) -> Option<Preset> {
    // Split at the LAST `@`: a quoted local part may legally contain one.
    let (local, domain) = address.rsplit_once('@')?;
    if local.is_empty() || domain.is_empty() {
        return None;
    }
    let domain = domain.to_ascii_lowercase();
    // `contoso.onmicrosoft.com` — matched as a domain rather than a suffix, so that
    // `notonmicrosoft.com` is not treated as a tenant.
    if under(&domain, "onmicrosoft.com") {
        return Some(microsoft(address, now));
    }

    match domain.as_str() {
        "gmail.com" | "googlemail.com" => Some(gmail(address, now)),
        // The tenant fallback domain, which is the only Microsoft 365 address that can be
        // recognised from the address alone. A work or school mailbox almost always uses its
        // organisation's own domain — `you@yourcompany.com` — and nothing about that string
        // says Microsoft. Those are configured with `--microsoft`, or found by discovery
        // (`mail_proto::discover`) through the domain's MX record. Discovery never acts on what
        // it found until the user has seen it and said yes, because guessing wrong sends a
        // password to a host the user never named; this table, which needs no confirmation,
        // stays limited to what the address itself proves.
        //
        // Personal outlook.com and hotmail.com are deliberately absent: see `OAuthIssuer::
        // Microsoft`.
        "onmicrosoft.com" => Some(microsoft(address, now)),
        _ => None,
    }
}

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
                issuer: OAuthIssuer::Google,
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

/// The preset for a provider recognised by the registered domain of an address's mail
/// exchanger.
///
/// This is how a mailbox on its organisation's own domain is recognised: nothing about
/// `you@yourcompany.example` says who hosts it, but its MX record does, and a hosted domain's MX
/// points into the provider's own domain. Answering with the preset rather than with whatever a
/// database lists for that domain is what makes such an account sign in with OAuth, which is the
/// only thing either provider still accepts, instead of a password it will refuse.
///
/// `registered` is the registrable domain of the MX host (`google.com` for
/// `aspmx.l.google.com`), already lowercased by the caller's public-suffix lookup.
pub fn preset_for_mail_exchanger(
    registered: &str,
    address: &str,
    now: DateTime<Utc>,
) -> Option<Preset> {
    Some(preset_for_issuer(
        issuer_of_exchanger(registered)?,
        address,
        now,
    ))
}

/// The OAuth issuer that hosts mail whose exchanger is in `registered`.
fn issuer_of_exchanger(registered: &str) -> Option<OAuthIssuer> {
    match registered.to_ascii_lowercase().as_str() {
        "google.com" | "googlemail.com" | "gmail.com" => Some(OAuthIssuer::Google),
        // Microsoft 365's inbound hosts are `<tenant>.mail.protection.outlook.com`.
        "outlook.com" => Some(OAuthIssuer::Microsoft),
        _ => None,
    }
}

/// The preset that signs in with `issuer`.
///
/// Each issuer this workspace knows serves exactly one mail provider, so naming the issuer names
/// the servers too.
pub fn preset_for_issuer(issuer: OAuthIssuer, address: &str, now: DateTime<Utc>) -> Preset {
    match issuer {
        OAuthIssuer::Google => gmail(address, now),
        OAuthIssuer::Microsoft => microsoft(address, now),
    }
}

/// The issuer an autoconfig document's `<oAuth2><issuer>` names, when it is one we have a
/// registration for. Any other issuer is somebody else's authorization server, and a client id
/// for it cannot be guessed.
pub fn issuer_named(issuer_host: &str) -> Option<OAuthIssuer> {
    match issuer_host.trim().to_ascii_lowercase().as_str() {
        "accounts.google.com" => Some(OAuthIssuer::Google),
        "login.microsoftonline.com" => Some(OAuthIssuer::Microsoft),
        _ => None,
    }
}

/// The issuer whose tokens a mail server at `host` takes, for a document that offers OAuth2 on a
/// server without naming the issuer.
pub fn issuer_for_server(host: &str) -> Option<OAuthIssuer> {
    let host = host.trim().to_ascii_lowercase();
    if under(&host, "gmail.com") || under(&host, "googlemail.com") {
        return Some(OAuthIssuer::Google);
    }
    if under(&host, "office365.com") || under(&host, "outlook.office.com") {
        return Some(OAuthIssuer::Microsoft);
    }
    None
}

/// Whether an address's domain is one of Microsoft's personal (consumer) mail domains.
///
/// [`OAuthIssuer::Microsoft`] is for managed tenants only, and [`preset_for`] leaves these out
/// on purpose. Discovery finds them anyway — their MX is Microsoft's and databases list them —
/// so it asks this before handing out the tenant preset, and says why it will not rather than
/// configuring an account that cannot send.
pub fn is_personal_microsoft(domain: &str) -> bool {
    matches!(
        domain.trim().to_ascii_lowercase().as_str(),
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com"
    )
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
pub fn password_warning(host: &str) -> Option<&'static str> {
    let host = host.trim().to_ascii_lowercase();
    let host = host.rsplit_once(':').map_or(host.as_str(), |(h, _)| h);

    if under(host, "office365.com") || under(host, "outlook.office.com") {
        return Some(
            "Microsoft 365 turned off password authentication for IMAP, POP and SMTP, so a \
             password will be rejected however it is stored. These mailboxes need OAuth, which \
             is queued in plan.md and not written yet.",
        );
    }
    if under(host, "gmail.com") || under(host, "googlemail.com") {
        return Some(
            "Google stopped accepting account passwords for IMAP and SMTP. An App Password (which \
             needs two-factor authentication switched on) still works here; the account's own \
             password will not.",
        );
    }
    None
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
/// Every preset here was written from a measured spike against a specific host. This one cannot
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

/// A managed Microsoft 365 mailbox, work or school, for an address the table cannot recognise.
///
/// Public because a custom tenant domain is the common case and `--microsoft` is how the user
/// says so.
pub fn microsoft_preset(address: &str, now: DateTime<Utc>) -> Preset {
    microsoft(address, now)
}

/// The delegated permission Graph's `sendMail` needs.
pub const GRAPH_SEND_SCOPE: &str = "https://graph.microsoft.com/Mail.Send";

/// The delegated permission a message over Graph's 4 MB request limit needs besides
/// [`GRAPH_SEND_SCOPE`]: it is created as a draft in the mailbox, its attachments uploaded to
/// the draft, and the draft sent.
pub const GRAPH_WRITE_SCOPE: &str = "https://graph.microsoft.com/Mail.ReadWrite";

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

/// How often an account that reads through Graph looks for new mail.
///
/// Graph pushes change notifications only to a public HTTPS webhook, which a desktop client does
/// not have, so this polls. A minute is cheap here and would not be over IMAP: a delta query
/// that has nothing to report is one small request per folder, answered with no messages.
pub const GRAPH_POLL_EVERY: Duration = Duration::from_secs(60);

/// `preset`, receiving and sending through Microsoft Graph instead of IMAP and SMTP.
///
/// For a tenant that switched IMAP off. Every scope the sign-in asks for is Graph's —
/// `Mail.ReadWrite` to read, flag and move, `Mail.Send` to send — so one token serves both
/// directions: there is no second resource to exchange the refresh token for.
///
/// The capabilities are what Graph offers every mailbox, not a guess about one: folders rather
/// than labels, a move that is a move, nothing ever expunged, and polling. The folder roles are
/// left for the first listing, and `observed_at` is the epoch so that listing happens before the
/// first sync rather than after it.
pub fn receive_through_graph(mut preset: Preset) -> Preset {
    preset.plan.incoming = Incoming::Graph;
    preset.plan.outgoing = Outgoing::Graph;
    if let AuthPlan::OAuth { scopes, .. } = &mut preset.plan.auth {
        *scopes = [
            GRAPH_WRITE_SCOPE,
            GRAPH_SEND_SCOPE,
            "offline_access",
            "openid",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
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
/// `OAuthIssuer` variant, an endpoints row and this function. `Incoming::Imap`, `Outgoing::Smtp`
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
                issuer: OAuthIssuer::Microsoft,
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

    #[test]
    fn gmail_domains_and_case() {
        for address in [
            "someone@gmail.com",
            "someone@GMAIL.com",
            "someone@googlemail.com",
            "Someone@GoogleMail.COM",
        ] {
            let preset = preset_for(address, at()).unwrap_or_else(|| panic!("{address}"));
            assert_eq!(
                preset.plan.incoming,
                Incoming::Imap {
                    host: "imap.gmail.com".to_owned(),
                    port: 993,
                    tls: Tls::Implicit,
                },
                "{address}"
            );
            // The address is kept exactly as typed; only the lookup folds case.
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
                every: GRAPH_POLL_EVERY
            }
        );
        assert!(
            (at() - caps.observed_at) > chrono::TimeDelta::try_days(1).unwrap(),
            "stale, so the folders are listed before the first sync"
        );
    }

    #[test]
    fn unknown_and_malformed() {
        const CASES: &[&str] = &[
            "someone@example.com",
            "someone@sub.gmail.com",
            "someone@gmail.com.evil.test",
            "not-an-address",
            "",
            "@gmail.com",
            "someone@",
            "@",
        ];
        for address in CASES {
            assert!(
                preset_for(address, at()).is_none(),
                "{address} should have no preset"
            );
        }
    }

    /// A custom domain whose mail exchanger is in Google's or Microsoft's own domain is that
    /// provider's, and signs in the way the provider's preset does.
    #[test]
    fn a_mail_exchanger_in_a_providers_domain_gives_that_providers_preset() {
        let google = preset_for_mail_exchanger("google.com", "me@firm.example", at())
            .expect("google.com is known");
        assert!(matches!(
            google.plan.auth,
            AuthPlan::OAuth {
                issuer: OAuthIssuer::Google,
                ..
            }
        ));
        assert_eq!(google.plan.address, "me@firm.example");
        let microsoft = preset_for_mail_exchanger("OUTLOOK.com", "me@firm.example", at())
            .expect("outlook.com is known");
        assert!(matches!(
            microsoft.plan.auth,
            AuthPlan::OAuth {
                issuer: OAuthIssuer::Microsoft,
                ..
            }
        ));
        for other in [
            "example.net",
            "notgoogle.com",
            "google.com.example.test",
            "",
        ] {
            assert_eq!(
                preset_for_mail_exchanger(other, "me@firm.example", at()),
                None,
                "{other}"
            );
        }
    }

    #[test]
    fn only_the_two_known_issuers_are_recognised() {
        assert_eq!(
            issuer_named("accounts.google.com"),
            Some(OAuthIssuer::Google)
        );
        assert_eq!(
            issuer_named("login.microsoftonline.com"),
            Some(OAuthIssuer::Microsoft)
        );
        assert_eq!(issuer_named("auth.example.net"), None);
        assert_eq!(
            issuer_for_server("imap.gmail.com"),
            Some(OAuthIssuer::Google)
        );
        assert_eq!(
            issuer_for_server("outlook.office365.com"),
            Some(OAuthIssuer::Microsoft)
        );
        assert_eq!(issuer_for_server("imap.evil-gmail.com"), None);
        assert!(is_personal_microsoft("Hotmail.com"));
        assert!(!is_personal_microsoft("firm.example"));
    }

    #[test]
    fn last_at_wins() {
        // A quoted local part may contain `@`; the domain is what follows the last one.
        let preset = preset_for("\"odd@name\"@gmail.com", at());
        assert!(preset.is_some());
    }
}

#[cfg(test)]
mod password_warning_tests {
    use super::*;

    #[test]
    fn microsoft_365_hosts_are_flagged() {
        // The user has a work Outlook mailbox and a school one, both managed tenants. Storing a
        // password for either produces an authentication failure at the first sync that says
        // nothing about the cause.
        for host in [
            "outlook.office365.com",
            "smtp.office365.com",
            "outlook.office.com",
            "OUTLOOK.OFFICE365.COM",
            "outlook.office365.com:993",
        ] {
            let warning = password_warning(host).unwrap_or_else(|| panic!("{host} not flagged"));
            assert!(warning.contains("OAuth"), "{host}: {warning}");
        }
    }

    #[test]
    fn google_hosts_say_an_app_password_is_the_one_that_works() {
        // Different advice, because the outcome is different: Google still accepts an App
        // Password, so telling the user "use OAuth" would send them to build something they do
        // not need.
        let warning = password_warning("imap.gmail.com").expect("flagged");
        assert!(warning.contains("App Password"), "{warning}");
        assert!(password_warning("smtp.googlemail.com").is_some());
    }

    #[test]
    fn a_server_with_no_known_restriction_is_left_alone() {
        // Advice, not a gate. Anything not known to have switched passwords off gets no warning,
        // because inventing one teaches the user to ignore them.
        for host in [
            "pop.example.edu",
            "imap.example.com",
            "mail.fastmail.com",
            "",
            "notahost",
        ] {
            assert_eq!(password_warning(host), None, "{host} was warned about");
        }
    }

    #[test]
    fn a_lookalike_domain_is_not_flagged() {
        // Suffix matching on a hostname is how `evil-office365.com.attacker.test` gets treated
        // as Microsoft. These must not match.
        assert_eq!(password_warning("office365.com.example.test"), None);
        assert_eq!(password_warning("notgmail.com.example.test"), None);
        // `ends_with` alone treats both of these as the real thing.
        assert_eq!(password_warning("evil-office365.com"), None);
        assert_eq!(password_warning("notgmail.com"), None);
        // And the genuine article still matches, bare or as a subdomain.
        assert!(password_warning("office365.com").is_some());
        assert!(password_warning("imap.gmail.com").is_some());
    }
}
