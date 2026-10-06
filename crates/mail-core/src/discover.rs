//! `account add` for a domain the built-in table does not know, and `account discover`.
//!
//! What a domain's servers are, and where to look, is `porter-discover`'s, and which provider an
//! address belongs to is porter's provider files' (`matching`): [`providers`] ships them. The
//! lookup runs over mailo's own resolver and HTTP client (`mail_runtime::lookup`, which
//! implements the two seams `porter-discover` asks through). This is the part between porter's
//! answer and the account: [`map`] turns it into mailo's [`Found`] (an account plan and where it
//! came from), and what was found is said as words ([`describe`]) and a miss as a typed
//! [`Failed`]. Getting a yes before any of it is used is the front-end's: at the terminal,
//! `mail_app::cli::discover`.
//!
//! What stays here, because a provider file or a discovery answer has no place for it: the OAuth
//! preset an issuer's mail servers take ([`mail_domain::presets::preset_for_issuer`], found from
//! the host an answer names), the capabilities expected of a server, the refusal of a personal
//! Microsoft mailbox, and the rule that only implicit TLS is used for a server found by
//! discovery.

mod jmap;
pub mod providers;
use chrono::{DateTime, Utc};
pub use jmap::find as find_jmap;
use mail_domain::presets::{self, Manual, Preset};
use mail_domain::{AuthPlan, Incoming, Outgoing, Retry, SaslMech, Tls, Username};
use porter_core::{Family, ServiceEndpoint, Tls as Wire, UrlScheme};
use porter_discover::{Found as Servers, NotFound, Outcome, ProviderLead, Source as Via};
use porter_provider::{DomainMatch, DomainName, Issuer, ProviderSpec};
use std::fmt::Write as _;

/// Where a configuration came from, which is what the user is asked to trust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A provider file lists the address's domain.
    Provider { label: String },
    /// A provider file claims the domain's mail exchanger.
    ProviderMx {
        label: String,
        /// The MX host that gave it away.
        exchanger: Option<String>,
    },
    /// The domain's own autoconfig document, or the public ISPDB's for it.
    Autoconfig,
    /// RFC 6186 SRV records in the domain's DNS.
    Srv,
    /// The ISPDB's document for the operator of the domain's mail exchanger.
    MxAutoconfig,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Provider { label } => write!(f, "from the provider list ({label})"),
            Source::ProviderMx {
                label,
                exchanger: Some(exchanger),
            } => write!(
                f,
                "via MX → {label} (mail for the domain goes to {exchanger})"
            ),
            Source::ProviderMx {
                label,
                exchanger: None,
            } => write!(f, "via MX → {label}"),
            Source::Autoconfig => f.write_str("from autoconfig"),
            Source::Srv => f.write_str("from DNS SRV"),
            Source::MxAutoconfig => f.write_str("via MX, from the Thunderbird ISPDB"),
        }
    }
}

/// A configuration, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub source: Source,
    pub preset: Preset,
}

/// What was found, for the user to judge.
pub fn describe(address: &str, origin: &str, found: &Preset) -> String {
    let plan = &found.plan;
    let mut out = format!("for {address}, {origin}:\n");
    let incoming = match &plan.incoming {
        Incoming::Imap { host, port, tls } => format!("IMAP {host}:{port}, {}", tls_said(*tls)),
        Incoming::Pop3 {
            host, port, tls, ..
        } => format!(
            "POP3 {host}:{port}, {} (mail is left on the server)",
            tls_said(*tls)
        ),
        Incoming::Local => "none".to_owned(),
        Incoming::Graph => "Microsoft Graph".to_owned(),
        Incoming::Jmap { session, auth } => format!(
            "JMAP at {session}, {}",
            match auth {
                mail_domain::HttpAuth::Basic => "signing in with a password",
                mail_domain::HttpAuth::Bearer => "signing in with a token",
            }
        ),
    };
    let outgoing = match &plan.outgoing {
        Outgoing::Smtp { host, port, tls } => format!("SMTP {host}:{port}, {}", tls_said(*tls)),
        Outgoing::Graph => "Microsoft Graph".to_owned(),
        Outgoing::Jmap => "JMAP submission, on the same server".to_owned(),
        Outgoing::Nowhere => "none".to_owned(),
    };
    let sign_in = match &plan.auth {
        AuthPlan::Password { username, .. } => format!(
            "a password, logging in as {:?}{}",
            username.resolve(address),
            match username {
                Username::SameAsAddress => " (the whole address)",
                Username::LocalPart => " (the part before the @)",
                Username::Literal(_) => "",
            }
        ),
        AuthPlan::OAuth { issuer, .. } => format!(
            "OAuth, signing in with {} in a browser; no password is stored",
            match issuer {
                Issuer::Google => "Google",
                Issuer::Microsoft => "Microsoft",
                _ => "an issuer mailo does not read mail through",
            }
        ),
    };
    let _ = writeln!(out, "  incoming  {incoming}");
    let _ = writeln!(out, "  outgoing  {outgoing}");
    let _ = writeln!(out, "  sign-in   {sign_in}");
    out
}

fn tls_said(tls: Tls) -> &'static str {
    match tls {
        Tls::Implicit => "TLS from the first byte",
        Tls::StartTlsRequired => "STARTTLS, required",
        Tls::Plaintext => "no TLS",
    }
}

/// What a domain's servers were missing, as far as the search could tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// Nothing usable, and no more to say.
    Nothing,
    /// Servers were published, but only ones that upgrade with STARTTLS, which this client
    /// does not use.
    StartTlsOnly,
    /// A personal Microsoft mailbox, which no longer takes a password.
    PersonalMicrosoft,
}

/// Why a lookup found no servers: what a caller can act on, before any wording.
///
/// [`Failed::said`] is the command line's prose; a window matches on the variants instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// The sources answered, and none named servers this client can use.
    NoServers {
        address: String,
        gap: Gap,
        /// Every step tried and what came of it, as `NotFound` prints them.
        tried: String,
    },
    /// Nothing answered at all: most likely this computer is offline.
    Unreachable {
        address: String,
        /// Whether trying again can help.
        retry: Retry,
        why: String,
    },
    /// The lookup could not be set up: no async runtime, no HTTP client, no resolver.
    Broken(String),
}

impl Failed {
    /// What to say at a terminal: why, and the way to configure the account by hand.
    pub fn said(&self) -> String {
        let (address, why, gap) = match self {
            Failed::Broken(why) => return why.clone(),
            Failed::NoServers {
                address,
                gap,
                tried,
            } => (address, tried, *gap),
            Failed::Unreachable { address, why, .. } => (address, why, Gap::Nothing),
        };
        let mut out = format!("could not find servers for {address}: {why}\n");
        match gap {
            Gap::PersonalMicrosoft => return out,
            Gap::StartTlsOnly => out.push_str(
                "\nThis domain publishes servers that use STARTTLS, which this client does not \
                 use: the upgrade can be stripped by anyone on the path, and the password then \
                 crosses in the clear. Ask the provider whether it also offers IMAP on port 993 \
                 (or POP3 on 995) and submission on port 465; if it does, name them:\n",
            ),
            Gap::Nothing => out.push_str("\nName the servers yourself:\n"),
        }
        let _ = write!(
            out,
            "\n  mailo account add {address} --imap HOST[:993] --smtp HOST[:465] [--login NAME]\n\
             \nor --pop3 HOST[:995] in place of --imap for a POP3-only server."
        );
        out
    }
}

/// What a personal Microsoft mailbox is told: it no longer takes a password, and the client
/// supports work and school Microsoft 365 mailboxes only.
const PERSONAL_MICROSOFT: &str = "this is a personal Microsoft mailbox. Those no longer accept \
    passwords, and recent ones refuse client sending even with OAuth; this client supports work \
    and school Microsoft 365 mailboxes only";

/// The address's domain, when it is one: what follows the last `@` (a quoted local part may
/// contain one), with something before it.
fn domain_of(address: &str) -> Option<DomainName> {
    let (local, domain) = address.rsplit_once('@')?;
    (!local.is_empty()).then(|| DomainName::parse(domain).ok())?
}

/// Whether `spec` is Microsoft's and claims `domain` as a personal one (`outlook.com`), not as a
/// tenant's (`contoso.onmicrosoft.com`): the file lists a tenant's as a suffix.
///
/// Basic authentication was retired on personal Outlook.com on 2024-09-16, and recently created
/// personal mailboxes are reported to have SMTP client authentication permanently off, failing
/// even under OAuth. porter's `microsoft.toml` claims both through one multi-tenant client;
/// mailo's preset signs in with the work and school scopes, so it says why it will not, rather
/// than configure an account that cannot send.
fn personal_microsoft(spec: &ProviderSpec, domain: &DomainName) -> bool {
    spec.auth.issuer == Some(Issuer::Microsoft)
        && !spec
            .matching
            .domain_suffixes
            .iter()
            .any(|suffix| domain.is_within(suffix))
}

/// The built-in table: what an address alone proves, so that nothing is looked up and nothing
/// need be confirmed.
///
/// A provider file that lists the address's domain and signs in with OAuth gives its issuer's
/// preset: Gmail's (`gmail.com`, `googlemail.com`) and a Microsoft 365 tenant's
/// (`<tenant>.onmicrosoft.com`). A domain that merely has an MX at a provider is not proved by
/// its address and goes through discovery, which shows what it found and asks. Nor does a
/// provider that signs in with a password: its hosts are shown for a yes first. A personal
/// Microsoft domain is not here either, and discovery says why.
///
/// The address is kept exactly as typed; only the lookup folds case.
pub fn known(address: &str, now: DateTime<Utc>) -> Option<Preset> {
    let domain = domain_of(address)?;
    let (spec, via) = providers::set().claiming(&domain, &[]).into_iter().next()?;
    if via != DomainMatch::Domain || personal_microsoft(spec, &domain) {
        return None;
    }
    presets::preset_for_issuer(spec.auth.issuer?, address, now)
}

/// A search's answer as an account plan, and where it came from.
///
/// Pure: what the network said is already in `outcome`.
pub fn map(outcome: Outcome, address: &str, now: DateTime<Utc>) -> Result<Found, Failed> {
    match outcome {
        Outcome::Provider(lead) => from_provider(&lead, address, now),
        Outcome::Servers(servers) => from_servers(&servers, address, now),
    }
}

/// A provider file that claims the address: its issuer's preset when it signs in with OAuth,
/// otherwise a password plan at the servers the file names.
fn from_provider(lead: &ProviderLead, address: &str, now: DateTime<Utc>) -> Result<Found, Failed> {
    let Some(spec) = providers::set().get(&lead.provider) else {
        return Err(no_servers(address, Gap::Nothing, "no such provider"));
    };
    let personal = lead.via == DomainMatch::Domain
        && domain_of(address).is_some_and(|domain| personal_microsoft(spec, &domain));
    if personal {
        return Err(no_servers(
            address,
            Gap::PersonalMicrosoft,
            PERSONAL_MICROSOFT,
        ));
    }
    let preset = match spec.auth.issuer {
        Some(issuer) => presets::preset_for_issuer(issuer, address, now)
            .ok_or_else(|| no_servers(address, Gap::Nothing, "its mail is not read here"))?,
        None => from_file(spec, address, now)?,
    };
    let label = spec.label.clone();
    let source = match lead.via {
        DomainMatch::Domain => Source::Provider { label },
        DomainMatch::Mx => Source::ProviderMx {
            label,
            exchanger: lead.exchanger.as_ref().map(ToString::to_string),
        },
    };
    Ok(Found { source, preset })
}

/// A password plan at the IMAP and SMTP servers a provider file names.
///
/// The file is curated, so a submission server that upgrades with STARTTLS (`smtp://`) is used,
/// required, as Microsoft's is; an IMAP server that does so is not, because mailo's IMAP has no
/// STARTTLS.
fn from_file(spec: &ProviderSpec, address: &str, now: DateTime<Utc>) -> Result<Preset, Failed> {
    let origin = |family: Family| {
        spec.capabilities
            .iter()
            .find(|row| row.family == family)
            .and_then(|row| porter_core::EndpointUrl::parse(&row.endpoint.as_ref()?.0).ok())
            .map(|url| url.origin())
    };
    let (Some(imap), Some(smtp)) = (origin(Family::Imap), origin(Family::Smtp)) else {
        return Err(no_servers(
            address,
            Gap::Nothing,
            "the file names no IMAP and SMTP servers",
        ));
    };
    let smtp_tls = match smtp.scheme {
        UrlScheme::Smtps => Tls::Implicit,
        UrlScheme::Smtp => Tls::StartTlsRequired,
        _ => {
            return Err(no_servers(
                address,
                Gap::Nothing,
                "the file's SMTP server is not SMTP",
            ));
        }
    };
    if imap.scheme != UrlScheme::Imaps {
        return Err(no_servers(
            address,
            Gap::StartTlsOnly,
            &format!(
                "the file's IMAP server is {}:{} without TLS from the first byte",
                imap.host, imap.port
            ),
        ));
    }
    let mut preset = presets::manual(
        address,
        &Manual {
            imap_host: imap.host,
            imap_port: imap.port,
            smtp_host: smtp.host,
            smtp_port: smtp.port,
            login: None,
        },
        now,
    );
    if let Outgoing::Smtp { tls, .. } = &mut preset.plan.outgoing {
        *tls = smtp_tls;
    }
    Ok(preset)
}

/// Servers a source named. Implicit TLS only: an upgrade is strippable, and a cleartext password
/// is a cleartext password. A server that does not offer it is declined, and the result says that
/// is what happened, so the user can configure the account by hand.
///
/// OAuth is taken where the IMAP or SMTP host is one of an issuer mailo reads mail through
/// (`issuer_for_server`): the issuer's own preset is used whole, because its scopes and
/// capabilities were measured and a document's were not. Otherwise the account signs in with a
/// password.
fn from_servers(servers: &Servers, address: &str, now: DateTime<Utc>) -> Result<Found, Failed> {
    let by = |family: Family| servers.endpoints.iter().find(|e| e.family == family);
    let (Some(imap), Some(smtp)) = (by(Family::Imap), by(Family::Smtp)) else {
        return Err(no_servers(
            address,
            Gap::Nothing,
            "no IMAP and SMTP servers are named",
        ));
    };
    if [imap, smtp].iter().any(|e| e.tls != Wire::Implicit) {
        let offered = [imap, smtp].map(offer).join(", ");
        return Err(no_servers(
            address,
            Gap::StartTlsOnly,
            &format!("the servers offered use only STARTTLS or no TLS: {offered}"),
        ));
    }
    let (incoming, outgoing) = (imap.url.origin(), smtp.url.origin());
    let issuer = presets::issuer_for_server(&incoming.host)
        .or_else(|| presets::issuer_for_server(&outgoing.host));
    let preset = match issuer.and_then(|issuer| presets::preset_for_issuer(issuer, address, now)) {
        Some(preset) => preset,
        None => {
            let mut preset = presets::manual(
                address,
                &Manual {
                    imap_host: incoming.host,
                    imap_port: incoming.port,
                    smtp_host: outgoing.host,
                    smtp_port: outgoing.port,
                    login: None,
                },
                now,
            );
            // One login for both directions, which is what `AuthPlan` holds: the incoming
            // server's, since that is the one used first and the one a wrong name shows up on.
            preset.plan.auth = AuthPlan::Password {
                username: username(&imap.login.0, address),
                sasl: vec![SaslMech::Plain],
            };
            preset
        }
    };
    let source = match servers.source {
        Via::Srv => Source::Srv,
        Via::Mx => Source::MxAutoconfig,
        // `discover_mail` returns the others as `Outcome::Provider` or never.
        _ => Source::Autoconfig,
    };
    Ok(Found { source, preset })
}

/// A server declined, as the user is shown it: `IMAP host:port (STARTTLS)`.
fn offer(endpoint: &ServiceEndpoint) -> String {
    let origin = endpoint.url.origin();
    let protocol = match endpoint.family {
        Family::Imap => "IMAP",
        Family::Smtp => "SMTP",
        _ => "server",
    };
    let socket = match endpoint.tls {
        Wire::Implicit => "SSL",
        Wire::StartTls => "STARTTLS",
        Wire::Plain => "no TLS",
    };
    format!("{protocol} {}:{} ({socket})", origin.host, origin.port)
}

/// A login as a [`Username`]: the whole address, the part before the `@`, or a name of its own.
fn username(login: &str, address: &str) -> Username {
    let login = login.trim();
    let local = address.rsplit_once('@').map_or(address, |(local, _)| local);
    if login.is_empty() || login.eq_ignore_ascii_case(address) {
        Username::SameAsAddress
    } else if login == local {
        Username::LocalPart
    } else {
        Username::Literal(login.to_owned())
    }
}

fn no_servers(address: &str, gap: Gap, tried: &str) -> Failed {
    Failed::NoServers {
        address: address.to_owned(),
        gap,
        tried: tried.to_owned(),
    }
}

/// What a search that found nothing means for `address`.
pub fn classify(address: &str, why: &NotFound) -> Failed {
    if why.offline() {
        return Failed::Unreachable {
            address: address.to_owned(),
            retry: Retry::Now,
            why: why.to_string(),
        };
    }
    no_servers(address, Gap::Nothing, &why.to_string())
}

/// Look the address up, over the network.
pub fn lookup(address: &str, now: DateTime<Utc>) -> Result<Found, String> {
    search(address, now).map_err(|why| why.said())
}

/// [`lookup`], with the reason a miss was left typed.
pub fn search(address: &str, now: DateTime<Utc>) -> Result<Found, Failed> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Failed::Broken(format!("cannot start the async runtime: {e}")))?;
    runtime.block_on(async {
        let http = mail_runtime::lookup::ReqwestHttp::new()
            .map_err(|e| Failed::Broken(format!("cannot build an HTTP client: {e}")))?;
        let dns =
            mail_runtime::lookup::SystemDns::new().map_err(|e| Failed::Broken(e.to_string()))?;
        let search = porter_discover::discover_mail(&http, &dns, providers::set(), address);
        match tokio::time::timeout(mail_runtime::lookup::TOTAL, search).await {
            Ok(Ok(outcome)) => map(outcome, address, now),
            Ok(Err(why)) => Err(classify(address, &why)),
            Err(_) => Err(Failed::Unreachable {
                address: address.to_owned(),
                retry: Retry::Now,
                why: format!(
                    "no answer in {} seconds",
                    mail_runtime::lookup::TOTAL.as_secs()
                ),
            }),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use porter_core::{EndpointUrl, LoginName};
    use porter_discover::{
        Dns, DnsFault, Miss, MxRecord, SrvRecord, Tried, discover_mail, parse_autoconfig,
    };
    use porter_http::{Http, HttpError, HttpRequest, HttpResponse, Status};
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn name(text: &str) -> DomainName {
        DomainName::parse(text).unwrap()
    }

    /// Both seams of discovery, answering from tables: no test asks a real resolver or server.
    #[derive(Default)]
    struct Net {
        /// URL to document; any other URL is a 404.
        documents: HashMap<String, String>,
        mx: HashMap<String, Vec<MxRecord>>,
        asked: Mutex<Vec<String>>,
    }

    impl Http for Net {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            let url = request.url.to_string();
            self.asked.lock().unwrap().push(url.clone());
            let (status, body) = match self.documents.get(&url) {
                Some(body) => (200, body.clone()),
                None => (404, String::new()),
            };
            Ok(HttpResponse {
                status: Status(status),
                headers: Vec::new(),
                body: body.into_bytes(),
            })
        }
    }

    impl Dns for Net {
        async fn srv(&self, _: &str) -> Result<Vec<SrvRecord>, DnsFault> {
            Err(DnsFault::NoRecords)
        }

        async fn mx(&self, domain: &DomainName) -> Result<Vec<MxRecord>, DnsFault> {
            self.mx
                .get(domain.as_str())
                .cloned()
                .ok_or(DnsFault::NoRecords)
        }
    }

    impl Net {
        fn with_mx(domain: &str, host: &str) -> Net {
            let mut net = Net::default();
            net.mx.insert(
                domain.to_owned(),
                vec![MxRecord {
                    preference: 10,
                    host: name(host),
                }],
            );
            net
        }

        /// The discovery search for `address`, mapped as `search` maps it.
        async fn found(&self, address: &str) -> Result<Found, Failed> {
            match discover_mail(self, self, providers::set(), address).await {
                Ok(outcome) => map(outcome, address, now()),
                Err(why) => Err(classify(address, &why)),
            }
        }
    }

    fn document(servers: &str) -> String {
        format!(
            r#"<?xml version="1.0"?>
    <clientConfig version="1.1"><emailProvider id="example.test">{servers}</emailProvider></clientConfig>"#
        )
    }

    fn server(
        side: &str,
        kind: &str,
        (host, port, socket): (&str, u16, &str),
        user: &str,
        auth: &[&str],
    ) -> String {
        let auth: String = auth
            .iter()
            .map(|a| format!("<authentication>{a}</authentication>"))
            .collect();
        format!(
            "<{side}Server type=\"{kind}\"><hostname>{host}</hostname><port>{port}</port>\
             <socketType>{socket}</socketType><username>{user}</username>{auth}</{side}Server>"
        )
    }

    fn imap_smtp(
        imap: (&str, u16, &str),
        smtp: (&str, u16, &str),
        user: &str,
        auth: &[&str],
    ) -> String {
        document(&format!(
            "{}{}",
            server("incoming", "imap", imap, user, auth),
            server("outgoing", "smtp", smtp, user, auth)
        ))
    }

    fn found_in(xml: &str, address: &str) -> Result<Found, Failed> {
        let servers = parse_autoconfig(xml, address).expect("a readable document");
        map(Outcome::Servers(servers), address, now())
    }

    fn endpoint(family: Family, url: &str, tls: Wire, login: &str) -> ServiceEndpoint {
        ServiceEndpoint {
            family,
            url: EndpointUrl::parse(url).unwrap(),
            tls,
            login: LoginName(login.to_owned()),
        }
    }

    fn imap_host(found: &Found) -> &str {
        match &found.preset.plan.incoming {
            Incoming::Imap { host, .. } => host,
            other => panic!("{other:?}"),
        }
    }

    // ---- the built-in table ---------------------------------------------------------------------

    /// What an address alone proves, and no more: Gmail's domains and a Microsoft 365 tenant's
    /// fallback domain. The preset keeps the address as typed.
    #[test]
    fn the_built_in_table_is_what_the_address_alone_proves() {
        for (address, issuer) in [
            ("someone@gmail.com", Issuer::Google),
            ("someone@GMAIL.com", Issuer::Google),
            ("someone@googlemail.com", Issuer::Google),
            ("Someone@GoogleMail.COM", Issuer::Google),
            ("me@onmicrosoft.com", Issuer::Microsoft),
            ("me@contoso.onmicrosoft.com", Issuer::Microsoft),
            // A quoted local part may contain an `@`; the domain is what follows the last one.
            ("\"odd@name\"@gmail.com", Issuer::Google),
        ] {
            let preset = known(address, now()).unwrap_or_else(|| panic!("{address}"));
            assert!(
                matches!(preset.plan.auth, AuthPlan::OAuth { issuer: got, .. } if got == issuer),
                "{address}: {:?}",
                preset.plan.auth
            );
            assert_eq!(preset.plan.address, address);
            assert_eq!(preset.expected_caps.observed_at, now(), "{address}");
        }
    }

    #[test]
    fn the_built_in_table_leaves_the_rest_to_discovery() {
        for address in [
            "someone@example.com",
            "someone@sub.gmail.com",
            "someone@gmail.com.evil.test",
            "me@notonmicrosoft.com",
            "me@onmicrosoft.com.evil.test",
            // A domain with an MX at a provider is not proved by its address.
            "me@firm.example",
            // A provider that signs in with a password shows its hosts for a yes first.
            "me@fastmail.com",
            "me@icloud.com",
            // A personal Microsoft mailbox is refused, with the reason, by discovery.
            "me@outlook.com",
            "me@Hotmail.com",
            "not-an-address",
            "",
            "@gmail.com",
            "someone@",
            "@",
        ] {
            assert_eq!(known(address, now()), None, "{address:?}");
        }
    }

    // ---- provider files ---------------------------------------------------------------------------

    /// A Fastmail, iCloud, Yahoo or GMX address resolves to its file with no request at all, and the
    /// plan is a password at the hosts the file names. The hosts are the providers' published ones,
    /// never checked against the servers (`tests/live_presets.rs`).
    #[tokio::test]
    async fn an_address_at_a_known_provider_resolves_to_its_file_without_asking_the_network() {
        use Tls::{Implicit, StartTlsRequired};
        type Row = (
            &'static str,
            Option<&'static str>,
            &'static str,
            (&'static str, u16),
            (&'static str, u16, Tls),
        );
        const CASES: &[Row] = &[
            (
                "me@fastmail.com",
                None,
                "Fastmail",
                ("imap.fastmail.com", 993),
                ("smtp.fastmail.com", 465, Implicit),
            ),
            (
                "me@fastmail.fm",
                None,
                "Fastmail",
                ("imap.fastmail.com", 993),
                ("smtp.fastmail.com", 465, Implicit),
            ),
            (
                "me@icloud.com",
                None,
                "iCloud",
                ("imap.mail.me.com", 993),
                ("smtp.mail.me.com", 587, StartTlsRequired),
            ),
            (
                "me@me.com",
                None,
                "iCloud",
                ("imap.mail.me.com", 993),
                ("smtp.mail.me.com", 587, StartTlsRequired),
            ),
            (
                "me@mac.com",
                None,
                "iCloud",
                ("imap.mail.me.com", 993),
                ("smtp.mail.me.com", 587, StartTlsRequired),
            ),
            (
                "me@yahoo.com",
                None,
                "Yahoo Mail",
                ("imap.mail.yahoo.com", 993),
                ("smtp.mail.yahoo.com", 465, Implicit),
            ),
            (
                "me@ymail.com",
                None,
                "Yahoo Mail",
                ("imap.mail.yahoo.com", 993),
                ("smtp.mail.yahoo.com", 465, Implicit),
            ),
            (
                "me@gmx.de",
                None,
                "GMX",
                ("imap.gmx.com", 993),
                ("mail.gmx.com", 465, Implicit),
            ),
            (
                "me@gmx.net",
                None,
                "GMX",
                ("imap.gmx.com", 993),
                ("mail.gmx.com", 465, Implicit),
            ),
            // A domain of their own, found by its MX.
            (
                "me@firm.example",
                Some("in1-smtp.messagingengine.com"),
                "Fastmail",
                ("imap.fastmail.com", 993),
                ("smtp.fastmail.com", 465, Implicit),
            ),
            (
                "me@firm.example",
                Some("mx01.mail.icloud.com"),
                "iCloud",
                ("imap.mail.me.com", 993),
                ("smtp.mail.me.com", 587, StartTlsRequired),
            ),
            (
                "me@firm.example",
                Some("mta5.am0.yahoodns.net"),
                "Yahoo Mail",
                ("imap.mail.yahoo.com", 993),
                ("smtp.mail.yahoo.com", 465, Implicit),
            ),
            (
                "me@firm.example",
                Some("mx00.gmx.net"),
                "GMX",
                ("imap.gmx.com", 993),
                ("mail.gmx.com", 465, Implicit),
            ),
        ];
        for (address, mx, label, imap, smtp) in CASES {
            let net = match mx {
                Some(host) => Net::with_mx("firm.example", host),
                None => Net::default(),
            };
            let found = net
                .found(address)
                .await
                .unwrap_or_else(|e| panic!("{address}: {e:?}"));
            let plan = &found.preset.plan;
            assert_eq!(
                plan.incoming,
                Incoming::Imap {
                    host: imap.0.to_owned(),
                    port: imap.1,
                    tls: Implicit
                },
                "{address} via {mx:?}"
            );
            assert_eq!(
                plan.outgoing,
                Outgoing::Smtp {
                    host: smtp.0.to_owned(),
                    port: smtp.1,
                    tls: smtp.2
                },
                "{address} via {mx:?}"
            );
            assert!(
                matches!(&plan.auth, AuthPlan::Password { username: Username::SameAsAddress, sasl } if sasl == &[SaslMech::Plain]),
                "{address}: {:?}",
                plan.auth
            );
            assert_eq!(plan.address, *address);
            assert!(
                found.source.to_string().contains(label),
                "{address}: {}",
                found.source
            );
            let via_mx = matches!(found.source, Source::ProviderMx { .. });
            assert_eq!(via_mx, mx.is_some(), "{address}");
            // A listed domain answers before any fetch; an MX is read after the documents.
            assert_eq!(
                net.asked.lock().unwrap().is_empty(),
                mx.is_none(),
                "{address}"
            );
        }
    }

    /// A custom domain whose mail exchanger is in Google's or Microsoft's own domain is that
    /// provider's, and signs in the way the provider's preset does.
    #[tokio::test]
    async fn a_mail_exchanger_in_a_providers_domain_gives_that_providers_preset() {
        let google = Net::with_mx("firm.example", "aspmx.l.google.com")
            .found("me@firm.example")
            .await
            .expect("google.com is known");
        assert!(matches!(
            google.preset.plan.auth,
            AuthPlan::OAuth {
                issuer: Issuer::Google,
                ..
            }
        ));
        assert_eq!(google.preset.plan.address, "me@firm.example");
        let text = describe(
            "me@firm.example",
            &google.source.to_string(),
            &google.preset,
        );
        assert!(
            text.contains("via MX → Google (mail for the domain goes to aspmx.l.google.com)"),
            "{text}"
        );
        assert!(text.contains("imap.gmail.com:993"), "{text}");
        assert!(text.contains("OAuth"), "{text}");

        for host in ["firm-example.mail.protection.outlook.com", "mx.outlook.com"] {
            let microsoft = Net::with_mx("firm.example", host)
                .found("me@firm.example")
                .await
                .expect(host);
            assert!(
                matches!(
                    microsoft.preset.plan.auth,
                    AuthPlan::OAuth {
                        issuer: Issuer::Microsoft,
                        ..
                    }
                ),
                "{host}"
            );
        }

        // A lookalike is nobody's: the search goes on to the ISPDB, finds nothing, and says so.
        for host in [
            "notgoogle.com",
            "google.com.example.test",
            "evil-outlook.com",
        ] {
            let net = Net::with_mx("firm.example", host);
            let failed = net.found("me@firm.example").await.unwrap_err();
            assert!(
                matches!(failed, Failed::NoServers { .. }),
                "{host}: {failed:?}"
            );
        }
    }

    /// A personal Microsoft mailbox no longer takes a password and recent ones cannot send even with
    /// OAuth, so it is told why instead of being configured; a tenant's mailbox is not.
    #[tokio::test]
    async fn a_personal_microsoft_mailbox_is_refused_and_a_tenants_is_not() {
        for address in [
            "me@outlook.com",
            "me@Hotmail.com",
            "me@live.com",
            "me@msn.com",
        ] {
            let failed = Net::default().found(address).await.unwrap_err();
            let Failed::NoServers { gap, .. } = &failed else {
                panic!("{address}: {failed:?}")
            };
            assert_eq!(*gap, Gap::PersonalMicrosoft, "{address}");
            assert!(!failed.said().contains("mailo account add"), "{address}");
        }
        let tenant = Net::default()
            .found("me@contoso.onmicrosoft.com")
            .await
            .expect("a tenant");
        assert!(matches!(
            tenant.preset.plan.auth,
            AuthPlan::OAuth {
                issuer: Issuer::Microsoft,
                ..
            }
        ));
        assert_eq!(
            tenant.source,
            Source::Provider {
                label: "Microsoft".to_owned()
            }
        );
    }

    // ---- servers a document or DNS named ---------------------------------------------------------

    #[tokio::test]
    async fn the_domains_own_document_is_asked_before_the_ispdb_and_carries_the_address() {
        let mut net = Net::default();
        net.documents.insert(
            "https://autoconfig.example.test/mail/config-v1.1.xml?emailaddress=me%40example.test"
                .to_owned(),
            imap_smtp(
                ("imap.example.test", 993, "SSL"),
                ("smtp.example.test", 465, "SSL"),
                "%EMAILLOCALPART%",
                &["password-cleartext"],
            ),
        );
        let found = net.found("me@example.test").await.expect("found");
        assert_eq!(found.source, Source::Autoconfig);
        assert_eq!(imap_host(&found), "imap.example.test");
        assert!(matches!(
            &found.preset.plan.auth,
            AuthPlan::Password {
                username: Username::LocalPart,
                ..
            }
        ));
        assert_eq!(net.asked.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_login_is_the_address_the_local_part_or_a_name_of_its_own() {
        for (login, want) in [
            ("", Username::SameAsAddress),
            ("me@example.test", Username::SameAsAddress),
            ("ME@example.test", Username::SameAsAddress),
            ("me", Username::LocalPart),
            ("mailbox17", Username::Literal("mailbox17".to_owned())),
        ] {
            assert_eq!(username(login, "me@example.test"), want, "{login:?}");
        }
    }

    /// OAuth is taken where the server's host is an issuer's, the issuer's own preset used whole,
    /// and not for a lookalike.
    #[test]
    fn a_server_at_an_issuers_host_signs_in_with_that_issuer() {
        let both = ["OAuth2", "password-cleartext"];
        let google = found_in(
            &imap_smtp(
                ("imap.gmail.com", 993, "SSL"),
                ("smtp.gmail.com", 465, "SSL"),
                "%EMAILADDRESS%",
                &both,
            ),
            "me@firm.example",
        )
        .expect("google");
        assert!(matches!(
            google.preset.plan.auth,
            AuthPlan::OAuth {
                issuer: Issuer::Google,
                ..
            }
        ));
        let microsoft = found_in(
            &imap_smtp(
                ("outlook.office365.com", 993, "SSL"),
                ("smtp.office365.com", 587, "STARTTLS"),
                "%EMAILADDRESS%",
                &both,
            ),
            "me@firm.example",
        );
        // Microsoft's document offers submission with STARTTLS, which a found server may not use; the
        // preset for its address comes from the provider file instead (`an_address_at_...`).
        assert!(matches!(
            microsoft,
            Err(Failed::NoServers {
                gap: Gap::StartTlsOnly,
                ..
            })
        ));
        let lookalike = found_in(
            &imap_smtp(
                ("imap.evil-gmail.com", 993, "SSL"),
                ("smtp.evil-gmail.com", 465, "SSL"),
                "%EMAILADDRESS%",
                &both,
            ),
            "me@firm.example",
        )
        .expect("lookalike");
        assert!(matches!(
            lookalike.preset.plan.auth,
            AuthPlan::Password { .. }
        ));
    }

    #[test]
    fn a_starttls_only_domain_is_told_how_to_configure_it_by_hand() {
        let xml = imap_smtp(
            ("imap.example.test", 143, "STARTTLS"),
            ("smtp.example.test", 587, "STARTTLS"),
            "%EMAILADDRESS%",
            &["password-cleartext"],
        );
        let failed = found_in(&xml, "me@example.test").unwrap_err();
        assert!(matches!(
            failed,
            Failed::NoServers {
                gap: Gap::StartTlsOnly,
                ..
            }
        ));
        let said = failed.said();
        assert!(said.contains("STARTTLS"), "{said}");
        assert!(said.contains("imap.example.test:143"), "{said}");
        assert!(said.contains("--imap HOST[:993]"), "{said}");
    }

    #[test]
    fn servers_found_by_dns_are_named_for_the_confirmation() {
        let servers = Servers {
            endpoints: vec![
                endpoint(
                    Family::Imap,
                    "imaps://mail.example.test:993",
                    Wire::Implicit,
                    "me@example.test",
                ),
                endpoint(
                    Family::Smtp,
                    "smtps://mail.example.test:465",
                    Wire::Implicit,
                    "me@example.test",
                ),
            ],
            claims: Vec::new(),
            source: Via::Srv,
        };
        let found = map(Outcome::Servers(servers), "me@example.test", now()).expect("found");
        assert_eq!(found.source, Source::Srv);
        assert_eq!(found.source.to_string(), "from DNS SRV");
        assert_eq!(imap_host(&found), "mail.example.test");
    }

    #[test]
    fn every_source_names_itself_the_way_the_confirmation_prints_it() {
        for (source, want) in [
            (Source::Autoconfig, "from autoconfig"),
            (Source::Srv, "from DNS SRV"),
            (Source::MxAutoconfig, "via MX, from the Thunderbird ISPDB"),
            (
                Source::Provider {
                    label: "Fastmail".to_owned(),
                },
                "from the provider list (Fastmail)",
            ),
            (
                Source::ProviderMx {
                    label: "Google".to_owned(),
                    exchanger: None,
                },
                "via MX → Google",
            ),
        ] {
            assert_eq!(source.to_string(), want);
        }
    }

    // ---- a miss ----------------------------------------------------------------------------------

    #[test]
    fn a_miss_is_typed_for_the_window_and_unchanged_at_the_terminal() {
        let tried = |miss| Tried {
            what: "https://example.test/".to_owned(),
            miss,
        };
        let offline = NotFound {
            tried: vec![tried(Miss::Unreachable)],
        };
        assert!(matches!(
            classify("me@example.test", &offline),
            Failed::Unreachable {
                retry: Retry::Now,
                ..
            }
        ));
        let nothing = NotFound {
            tried: vec![tried(Miss::Absent)],
        };
        let failed = classify("me@example.test", &nothing);
        assert!(matches!(
            failed,
            Failed::NoServers {
                gap: Gap::Nothing,
                ..
            }
        ));
        assert_eq!(
            failed.said(),
            format!(
                "could not find servers for me@example.test: {nothing}\n\nName the servers \
                 yourself:\n\n  mailo account add me@example.test --imap HOST[:993] --smtp \
                 HOST[:465] [--login NAME]\n\nor --pop3 HOST[:995] in place of --imap for a \
                 POP3-only server."
            )
        );
    }
}
