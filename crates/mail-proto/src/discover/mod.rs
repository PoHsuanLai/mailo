//! Finding an account's servers from its address, for a domain the preset table does not know.
//!
//! Four sources, tried by `mail-runtime` in this order, stopping at the first that yields
//! something usable:
//!
//! 1. the domain's own configuration document (`autoconfig.<domain>`, then
//!    `<domain>/.well-known/autoconfig/`);
//! 2. the public ISPDB's document for the domain;
//! 3. RFC 6186 SRV records for the implicit-TLS services (`_imaps`, `_submissions` from
//!    RFC 8314, `_pop3s`);
//! 4. the domain's MX record: its host's registered domain is either a provider the preset table
//!    knows, or has an ISPDB document of its own.
//!
//! This module is the pure half: what each answer means. Fetching them is the runtime's.
//!
//! Two rules hold whatever a source says, because a source is a stranger:
//!
//! - **Implicit TLS only.** An endpoint offered with `STARTTLS` or in plaintext is skipped,
//!   never used, even when it is the only one; the result then says that is what happened, so
//!   the user can decide to configure the account by hand. An upgrade is strippable and a
//!   cleartext password is a cleartext password.
//! - **Nothing here is a decision to connect.** The result is a [`Found`] naming its source, for
//!   the caller to show and have confirmed before any credential leaves.

pub mod autoconfig;

use autoconfig::{AuthMethod, ClientConfig, Server, ServerProtocol, SocketType};
use chrono::{DateTime, Utc};
use mail_domain::presets::{self, Manual, ManualPop3, Preset};
use mail_domain::{AuthPlan, SaslMech, Username};
use std::fmt;

/// Where a configuration came from, which is what the user is asked to trust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The domain's own document, from `autoconfig.<domain>` or its `.well-known` path.
    DomainAutoconfig,
    /// The public ISPDB's document for the address's domain.
    Ispdb,
    /// RFC 6186 SRV records in the domain's DNS.
    Srv,
    /// The domain's mail exchanger, whose host is in `provider`'s domain: either a provider the
    /// preset table knows, or one the ISPDB has a document for.
    Mx {
        /// The MX host itself, e.g. `aspmx.l.google.com`.
        exchanger: String,
        /// Its registered domain, e.g. `google.com`.
        provider: String,
    },
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::DomainAutoconfig => f.write_str("from the domain's autoconfig"),
            Source::Ispdb => f.write_str("from the Thunderbird ISPDB"),
            Source::Srv => f.write_str("from DNS SRV"),
            Source::Mx {
                exchanger,
                provider,
            } => write!(
                f,
                "via MX → {provider} (mail for the domain goes to {exchanger})"
            ),
        }
    }
}

/// A configuration, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub source: Source,
    pub preset: Preset,
}

/// Which half of an account an answer was about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Incoming,
    Outgoing,
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Side::Incoming => "incoming",
            Side::Outgoing => "outgoing",
        })
    }
}

/// A server a source offered and this skipped, for saying so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub socket: String,
}

impl fmt::Display for Offer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}:{} ({})",
            self.protocol, self.host, self.port, self.socket
        )
    }
}

/// Why a source's answer, though it arrived, cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unusable {
    /// Servers were offered for this side, but none with implicit TLS.
    #[error("the {side} servers offered use only STARTTLS or no TLS: {}", list(.offered))]
    NoImplicitTls { side: Side, offered: Vec<Offer> },
    /// No server at all for this side.
    #[error("no {0} server is offered")]
    Missing(Side),
    /// Implicit-TLS servers were offered, but only with sign-in methods this client lacks.
    #[error("the {side} servers want a sign-in this client does not do: {}", list(.offered))]
    Mechanisms { side: Side, offered: Vec<Offer> },
    /// The address is a personal Microsoft mailbox; see `OAuthIssuer::Microsoft`.
    #[error(
        "this is a personal Microsoft mailbox. Those no longer accept passwords, and recent ones \
         refuse client sending even with OAuth; this client supports work and school Microsoft \
         365 mailboxes only"
    )]
    PersonalMicrosoft,
}

fn list(offers: &[Offer]) -> String {
    offers
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The domain of an address: after the last `@`, lowercased. `None` for something that is not
/// an address.
pub fn domain_of(address: &str) -> Option<String> {
    let (local, domain) = address.rsplit_once('@')?;
    (!local.is_empty() && !domain.is_empty()).then(|| domain.to_ascii_lowercase())
}

/// A configuration document's answer for `address`.
///
/// OAuth2 is taken only where it leads to an issuer this workspace has a registration for —
/// named by the document's `oAuth2/issuer` or implied by the server's host — and then the
/// provider's own preset is used whole, because its scopes and capabilities were measured and a
/// document's were not. Otherwise the account signs in with a password, on the first
/// implicit-TLS IMAP server that takes one (a POP3 server only when there is no such IMAP one),
/// and the first implicit-TLS SMTP server that takes one.
pub fn from_autoconfig(
    config: &ClientConfig,
    address: &str,
    now: DateTime<Utc>,
) -> Result<Preset, Unusable> {
    let preset = match oauth(config, address, now) {
        Some(preset) => preset,
        None => password(config, address, now)?,
    };
    guard(address, preset)
}

/// The provider preset, when the document offers OAuth2 through an issuer we know.
fn oauth(config: &ClientConfig, address: &str, now: DateTime<Utc>) -> Option<Preset> {
    let named = config
        .oauth_issuer
        .as_deref()
        .and_then(presets::issuer_named);
    config
        .incoming
        .iter()
        .chain(&config.outgoing)
        .filter(|s| s.auth.contains(&AuthMethod::OAuth2))
        .find_map(|s| named.or_else(|| presets::issuer_for_server(&expand(&s.hostname, address))))
        .map(|issuer| presets::preset_for_issuer(issuer, address, now))
}

fn password(config: &ClientConfig, address: &str, now: DateTime<Utc>) -> Result<Preset, Unusable> {
    let imap = pick(&config.incoming, &ServerProtocol::Imap);
    let pop3 = pick(&config.incoming, &ServerProtocol::Pop3);
    let smtp = pick(&config.outgoing, &ServerProtocol::Smtp);
    let incoming = match (imap, pop3) {
        (Ok(imap), _) => Ok((imap, ServerProtocol::Imap)),
        (Err(_), Ok(pop3)) => Ok((pop3, ServerProtocol::Pop3)),
        (Err(imap), Err(pop3)) => Err(imap.max(pop3)),
    };
    let incoming = incoming.map_err(|why| why.unusable(Side::Incoming, &config.incoming))?;
    let smtp = smtp.map_err(|why| why.unusable(Side::Outgoing, &config.outgoing))?;
    let (server, protocol) = incoming;
    let host = expand(&server.hostname, address);
    let smtp_host = expand(&smtp.hostname, address);
    let mut preset = match protocol {
        ServerProtocol::Pop3 => presets::manual_pop3(
            address,
            &ManualPop3 {
                pop3_host: host,
                pop3_port: server.port,
                smtp_host,
                smtp_port: smtp.port,
                login: None,
            },
            now,
        ),
        _ => presets::manual(
            address,
            &Manual {
                imap_host: host,
                imap_port: server.port,
                smtp_host,
                smtp_port: smtp.port,
                login: None,
            },
            now,
        ),
    };
    // One login for both directions, which is what `AuthPlan` holds. The incoming server's, since
    // that is the one used first and the one a wrong name shows up on.
    preset.plan.auth = AuthPlan::Password {
        username: username(&server.username, address),
        sasl: vec![SaslMech::Plain],
    };
    Ok(preset)
}

/// Why no server of one protocol was picked. Ordered: a later variant is more informative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Skip {
    None,
    NotTls,
    Mechanism,
}

impl Skip {
    fn unusable(self, side: Side, servers: &[Server]) -> Unusable {
        let offered = servers.iter().map(offer).collect();
        match self {
            Skip::None => Unusable::Missing(side),
            Skip::NotTls => Unusable::NoImplicitTls { side, offered },
            Skip::Mechanism => Unusable::Mechanisms { side, offered },
        }
    }
}

/// The first server of `protocol` with implicit TLS and a password sign-in this client does.
fn pick<'a>(servers: &'a [Server], protocol: &ServerProtocol) -> Result<&'a Server, Skip> {
    let mut skipped = Skip::None;
    for server in servers.iter().filter(|s| &s.protocol == protocol) {
        if server.socket != SocketType::Ssl {
            skipped = skipped.max(Skip::NotTls);
            continue;
        }
        // No `authentication` element means the format's default, a password.
        if server.auth.is_empty() || server.auth.contains(&AuthMethod::PasswordCleartext) {
            return Ok(server);
        }
        skipped = skipped.max(Skip::Mechanism);
    }
    Err(skipped)
}

fn offer(server: &Server) -> Offer {
    Offer {
        protocol: match &server.protocol {
            ServerProtocol::Imap => "IMAP".to_owned(),
            ServerProtocol::Pop3 => "POP3".to_owned(),
            ServerProtocol::Smtp => "SMTP".to_owned(),
            ServerProtocol::Other(other) => other.clone(),
        },
        host: server.hostname.clone(),
        port: server.port,
        socket: match &server.socket {
            SocketType::Ssl => "SSL".to_owned(),
            SocketType::StartTls => "STARTTLS".to_owned(),
            SocketType::Plain => "no TLS".to_owned(),
            SocketType::Other(other) => other.clone(),
        },
    }
}

/// A `username` element, as a [`Username`].
///
/// The three placeholders the format defines map onto the forms `Username` already has: the
/// whole address, and the local part. Anything else — a fixed name, or placeholders combined
/// some other way — is resolved against the address now and kept as a literal. An empty element
/// is the address, which is what a login almost always is.
pub fn username(raw: &str, address: &str) -> Username {
    let raw = raw.trim();
    if raw.is_empty() || raw == "%EMAILADDRESS%" {
        return Username::SameAsAddress;
    }
    if raw == "%EMAILLOCALPART%" {
        return Username::LocalPart;
    }
    let resolved = expand(raw, address);
    if resolved.eq_ignore_ascii_case(address) {
        Username::SameAsAddress
    } else {
        Username::Literal(resolved)
    }
}

/// Substitute the format's address placeholders.
fn expand(raw: &str, address: &str) -> String {
    let (local, domain) = address.rsplit_once('@').unwrap_or((address, ""));
    raw.replace("%EMAILADDRESS%", address)
        .replace("%EMAILLOCALPART%", local)
        .replace("%EMAILDOMAIN%", domain)
}

/// One SRV record (RFC 2782), as DNS answered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrvRecord {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    /// With or without the trailing dot. `.` alone means the service is decidedly not offered.
    pub target: String,
}

/// The answer RFC 6186 SRV records give for `address`.
///
/// Only the implicit-TLS service names are asked about, so everything found here is implicit
/// TLS by construction. IMAP is preferred to POP3 when both are published, and the login is the
/// whole address, since SRV says nothing about it.
///
/// Among several records the lowest priority wins, then the highest weight, then the name —
/// deterministic, where RFC 2782 would pick among equal priorities at random: the result is
/// shown to the user and asked about, and it should not change between asking and answering.
pub fn from_srv(
    address: &str,
    imaps: &[SrvRecord],
    pop3s: &[SrvRecord],
    submissions: &[SrvRecord],
    now: DateTime<Utc>,
) -> Result<Preset, Unusable> {
    let smtp = best(submissions).ok_or(Unusable::Missing(Side::Outgoing))?;
    let preset = if let Some(imap) = best(imaps) {
        presets::manual(
            address,
            &Manual {
                imap_host: imap.0,
                imap_port: imap.1,
                smtp_host: smtp.0,
                smtp_port: smtp.1,
                login: None,
            },
            now,
        )
    } else if let Some(pop3) = best(pop3s) {
        presets::manual_pop3(
            address,
            &ManualPop3 {
                pop3_host: pop3.0,
                pop3_port: pop3.1,
                smtp_host: smtp.0,
                smtp_port: smtp.1,
                login: None,
            },
            now,
        )
    } else {
        return Err(Unusable::Missing(Side::Incoming));
    };
    guard(address, preset)
}

/// The preferred target of a service, as `(host, port)`.
fn best(records: &[SrvRecord]) -> Option<(String, u16)> {
    records
        .iter()
        .map(|r| (r, r.target.trim_end_matches('.').to_ascii_lowercase()))
        .filter(|(r, host)| !host.is_empty() && r.port != 0)
        .min_by(|(a, ah), (b, bh)| {
            a.priority
                .cmp(&b.priority)
                .then(b.weight.cmp(&a.weight))
                .then(ah.cmp(bh))
        })
        .map(|(r, host)| (host, r.port))
}

/// One MX record, as DNS answered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MxRecord {
    pub preference: u16,
    pub exchange: String,
}

/// What an address's MX records point to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MxLead {
    /// A provider the preset table knows; nothing more to fetch.
    Known(Box<Found>),
    /// Another domain, whose ISPDB document should be asked for. The [`Source`] is what to
    /// report if that document is usable.
    AskIspdb { domain: String, source: Source },
    /// Nothing more to learn: no MX, or it is in the address's own domain (whose ISPDB
    /// document was already asked for), or its host has no registrable domain.
    Nothing,
}

/// Where the MX records lead, given `registered`, the public-suffix lookup of a host
/// (`aspmx.l.google.com` → `google.com`), which is data the caller holds.
pub fn from_mx(
    address: &str,
    records: &[MxRecord],
    registered: impl Fn(&str) -> Option<String>,
    now: DateTime<Utc>,
) -> Result<MxLead, Unusable> {
    let Some(domain) = domain_of(address) else {
        return Ok(MxLead::Nothing);
    };
    let Some(exchanger) = records
        .iter()
        .map(|r| {
            (
                r.preference,
                r.exchange.trim_end_matches('.').to_ascii_lowercase(),
            )
        })
        // A null MX (RFC 7505) is "." and says the domain takes no mail.
        .filter(|(_, host)| !host.is_empty())
        .min()
        .map(|(_, host)| host)
    else {
        return Ok(MxLead::Nothing);
    };
    let Some(provider) = registered(&exchanger) else {
        return Ok(MxLead::Nothing);
    };
    let source = Source::Mx {
        exchanger,
        provider: provider.clone(),
    };
    if let Some(preset) = presets::preset_for_mail_exchanger(&provider, address, now) {
        return Ok(MxLead::Known(Box::new(Found {
            source,
            preset: guard(address, preset)?,
        })));
    }
    if provider == domain {
        return Ok(MxLead::Nothing);
    }
    Ok(MxLead::AskIspdb {
        domain: provider,
        source,
    })
}

/// Refuse the managed-tenant preset for a personal Microsoft address, however it was reached.
fn guard(address: &str, preset: Preset) -> Result<Preset, Unusable> {
    let tenant = matches!(
        preset.plan.auth,
        AuthPlan::OAuth {
            issuer: mail_domain::OAuthIssuer::Microsoft,
            ..
        }
    );
    let personal = domain_of(address).is_some_and(|d| presets::is_personal_microsoft(&d));
    if tenant && personal {
        return Err(Unusable::PersonalMicrosoft);
    }
    Ok(preset)
}
