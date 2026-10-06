//! What the Add account sheet decides, as functions of what it is handed.
//!
//! The network, the keyring and the browser arrive as [`Seams`], so every rule here — look only when asked,
//! add only when told to, a password to the add and nowhere else — is tested without either.
//!
//! A domain can answer two ways: its autoconfig (or SRV, or MX) names IMAP or POP3 servers, and
//! its `/.well-known/jmap` names a JMAP session. When both answer, what the autoconfig named is
//! offered first and JMAP beside it, one press away: the autoconfig is the provider's own
//! statement of its mail settings and IMAP mailo's longest-tried path, while the JMAP answer says
//! only that a server is there. Both are said in words, and neither is used until Use these
//! settings.

use std::sync::Arc;

use mail_domain::presets::{self, Manual, ManualPop3, Preset};
use mail_domain::{AuthPlan, HttpAuth, Incoming, Retry};
use mail_proto::discover::Found;
use mail_store::SqliteStore;
use porter_core::AccountId;
use porter_provider::Issuer;

use crate::ui::space::{Scope, Space};
use mail_core::account::Setup;
use mail_core::discover::{Failed, Gap};
use mail_core::password::Password;

/// Where the sheet stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Stage {
    /// An address to type. Nothing has been asked of anyone.
    Blank,
    /// Looking the domain up.
    Looking,
    /// What was found, shown, and not used yet.
    Found(Offer),
    /// Nothing usable, or not an address; and what to do about it.
    Missed(Miss),
    /// Using the settings: saving the account, or waiting on a browser sign-in.
    Adding(Offer),
    /// Added: what that did, a sentence a line, and the account now in the store.
    Added {
        said: Vec<String>,
        account: Option<AccountId>,
    },
    /// The add refused, and why. The offer stays, so it can be tried again on purpose.
    Refused(Offer, Refusal),
    /// A JMAP server typed in by hand: nothing is looked up.
    ByHand(Hand),
}

/// Why a lookup found nothing to offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Miss {
    /// What was typed is not an address.
    NotAnAddress,
    /// The domain answered, and named no servers this client can use.
    NoServers { domain: String, gap: Gap },
    /// Nothing could be reached to ask.
    Unreachable {
        domain: String,
        retry: Retry,
        why: String,
    },
    /// The lookup could not even start.
    Broken(String),
}

impl Miss {
    /// What a failed search for `address` means.
    fn of(failed: Failed) -> Miss {
        let domain = |address: &str| domain_of(address).unwrap_or_else(|| address.to_owned());
        match failed {
            Failed::NoServers { address, gap, .. } => Miss::NoServers {
                domain: domain(&address),
                gap,
            },
            Failed::Unreachable {
                address,
                retry,
                why,
            } => Miss::Unreachable {
                domain: domain(&address),
                retry,
                why,
            },
            Failed::Broken(why) => Miss::Broken(why),
        }
    }
}

/// Which secret was left empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum What {
    Password,
    Token,
}

/// Why an add did not go ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Refusal {
    /// The password or token was not typed yet.
    Blank(What),
    /// An OAuth sign-in with no client id to make it with.
    NeedsClientId(Issuer),
    /// The add said no, in its own words. Those are written for a terminal; see
    /// [`super::copy::refused`] for what the sheet makes of them.
    Other(String),
}

impl Stage {
    /// Whether something is running that a second press must not start again.
    pub(in crate::ui) fn busy(&self) -> bool {
        matches!(self, Stage::Looking | Stage::Adding(_))
    }
}

/// Settings found for an address, as the sheet shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Offer {
    pub address: String,
    /// Where they came from: "from the built-in table", "via MX → google.com (…)",
    /// "at https://example.com/.well-known/jmap", "by hand".
    pub source: String,
    /// Incoming, outgoing and sign-in, each as `(what, how)`.
    pub rows: Vec<(String, String)>,
    /// What the add is handed: the servers discovery found, or a JMAP session.
    pub setup: Setup,
    pub sign_in: SignIn,
    /// The other way the same domain answered, when it answered two: JMAP beside what its
    /// autoconfig named. One is offered at a time; [`Offer::switched`] trades them.
    pub other: Option<Box<Offer>>,
}

impl Offer {
    /// Servers discovery or the built-in table found, as `describe` says them.
    fn discovered(address: &str, source: String, preset: Preset, seams: &Seams) -> Offer {
        let shown = mail_core::discover::describe(address, &source, &preset);
        let sign_in = match &preset.plan.auth {
            AuthPlan::Password { .. } => SignIn::Password,
            AuthPlan::OAuth { issuer, .. } => SignIn::OAuth {
                issuer: *issuer,
                client: if (seams.client)(*issuer) {
                    Client::Ready
                } else {
                    Client::Missing
                },
            },
        };
        Offer {
            rows: rows(&shown),
            address: address.to_owned(),
            source,
            setup: Setup::Discovered(Box::new(preset)),
            sign_in,
            other: None,
        }
    }

    /// A JMAP session at `session`, from `source`, its secret sent as `auth`.
    pub(in crate::ui) fn jmap(
        address: &str,
        session: String,
        auth: HttpAuth,
        source: String,
    ) -> Offer {
        Offer {
            rows: jmap_rows(address, &session, auth),
            address: address.to_owned(),
            source,
            setup: Setup::Jmap {
                session: Some(session),
                login: None,
                auth,
            },
            sign_in: SignIn::Password,
            other: None,
        }
    }

    /// How a JMAP offer's secret travels; `None` for any other offer.
    pub(in crate::ui) fn jmap_auth(&self) -> Option<HttpAuth> {
        match &self.setup {
            Setup::Jmap { auth, .. } => Some(*auth),
            _ => None,
        }
    }

    /// Whether the secret is an API token rather than a password.
    pub(in crate::ui) fn token(&self) -> bool {
        self.jmap_auth() == Some(HttpAuth::Bearer)
    }

    /// How the offer reads mail, as a choice between two names it.
    pub(in crate::ui) fn way(&self) -> &'static str {
        let incoming = match &self.setup {
            Setup::Jmap { .. } => return "JMAP",
            Setup::Imap(_) => return "IMAP and SMTP",
            Setup::Pop3(_) => return "POP3 and SMTP",
            Setup::Discovered(preset) => &preset.plan.incoming,
        };
        match incoming {
            Incoming::Imap { .. } => "IMAP and SMTP",
            Incoming::Pop3 { .. } => "POP3 and SMTP",
            Incoming::Graph => "Microsoft Graph",
            Incoming::Jmap { .. } => "JMAP",
            Incoming::Local => "no server",
        }
    }

    /// The other way offered, with this one kept beside it. An offer with no other is itself.
    pub(in crate::ui) fn switched(mut self) -> Offer {
        match self.other.take() {
            Some(mut other) => {
                other.other = Some(Box::new(self));
                *other
            }
            None => self,
        }
    }

    /// The same JMAP offer with its secret sent as `auth`. Any other offer is unchanged.
    pub(in crate::ui) fn signing_with(mut self, auth: HttpAuth) -> Offer {
        if let Setup::Jmap {
            session: Some(session),
            auth: now,
            ..
        } = &mut self.setup
        {
            *now = auth;
            self.rows = jmap_rows(&self.address, session, auth);
        }
        self
    }
}

/// A JMAP offer's rows, in `describe`'s shape: one server both ways.
fn jmap_rows(address: &str, session: &str, auth: HttpAuth) -> Vec<(String, String)> {
    let sign_in = match auth {
        HttpAuth::Basic => format!("a password, logging in as {address:?} (the whole address)"),
        HttpAuth::Bearer => "an API token, sent as a bearer token".to_owned(),
    };
    vec![
        ("incoming".to_owned(), format!("JMAP at {session}")),
        (
            "outgoing".to_owned(),
            "JMAP submission, on the same server".to_owned(),
        ),
        ("sign-in".to_owned(), sign_in),
    ]
}

/// Which kind of server is being typed in, as the segmented control names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Kind {
    Imap,
    Pop3,
    Jmap,
}

impl Kind {
    /// The order the control shows them in.
    pub(in crate::ui) const ALL: [Kind; 3] = [Kind::Imap, Kind::Pop3, Kind::Jmap];

    pub(in crate::ui) fn label(self) -> &'static str {
        match self {
            Kind::Imap => "IMAP",
            Kind::Pop3 => "POP",
            Kind::Jmap => "JMAP",
        }
    }
}

/// Which server of an account a host and port belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Role {
    Imap,
    Pop3,
    Smtp,
}

impl Role {
    /// The port the server listens on for implicit TLS. The client refuses STARTTLS, so these
    /// are the only ports on offer, and the one a blank Port field means.
    pub(in crate::ui) fn default_port(self) -> Port {
        Port(match self {
            Role::Imap => 993,
            Role::Pop3 => 995,
            Role::Smtp => 465,
        })
    }

    /// The first label of the host guessed for a domain: `imap` in `imap.example.com`.
    fn guess(self) -> &'static str {
        match self {
            Role::Imap => "imap",
            Role::Pop3 => "pop",
            Role::Smtp => "smtp",
        }
    }

    /// What the host field shows, greyed, before anything is typed: the guess for the domain
    /// of `address`, or for example.com when it has none yet. Never a value.
    pub(in crate::ui) fn placeholder(self, address: &str) -> String {
        let domain = domain_of(address).unwrap_or_else(|| "example.com".to_owned());
        format!("{}.{domain}", self.guess())
    }
}

/// A TCP port, which is never 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) struct Port(u16);

impl Port {
    /// A port as typed; a blank one is `default`.
    fn parse(typed: &str, default: Port) -> Option<Port> {
        match typed.trim() {
            "" => Some(default),
            text => text.parse::<u16>().ok().filter(|port| *port != 0).map(Port),
        }
    }

    pub(in crate::ui) fn get(self) -> u16 {
        self.0
    }
}

/// A mail server as it is being typed in: the host and port fields, as typed. Nothing here can
/// be sent anywhere until [`by_hand`] has made a [`Setup`] of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::ui) struct Endpoint {
    pub host: String,
    /// Blank means the role's default port.
    pub port: String,
}

/// An IMAP or POP3 account being typed in: where mail comes from, where it goes, and who signs in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::ui) struct ServerHand {
    pub incoming: Endpoint,
    pub outgoing: Endpoint,
    /// The user name, as typed. Blank means the whole address.
    pub login: String,
}

/// A JMAP server as it is being typed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct JmapHand {
    /// The session URL, as typed.
    pub session: String,
    pub auth: HttpAuth,
}

/// A server typed in by hand: what the Incoming and Outgoing Mail Server form holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Hand {
    Imap(ServerHand),
    Pop3(ServerHand),
    Jmap(JmapHand),
}

/// A field of the form a hint can be about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Field {
    Session,
    IncomingHost,
    IncomingPort,
    OutgoingHost,
    OutgoingPort,
    Login,
}

impl Hand {
    /// Nothing typed yet, for IMAP, the commonest kind.
    pub(in crate::ui) fn blank() -> Hand {
        Hand::Imap(ServerHand::default())
    }

    /// Nothing typed yet, for `kind`.
    pub(in crate::ui) fn blank_of(kind: Kind) -> Hand {
        match kind {
            Kind::Imap => Hand::Imap(ServerHand::default()),
            Kind::Pop3 => Hand::Pop3(ServerHand::default()),
            Kind::Jmap => Hand::Jmap(JmapHand {
                session: String::new(),
                auth: HttpAuth::Basic,
            }),
        }
    }

    pub(in crate::ui) fn kind(&self) -> Kind {
        match self {
            Hand::Imap(_) => Kind::Imap,
            Hand::Pop3(_) => Kind::Pop3,
            Hand::Jmap(_) => Kind::Jmap,
        }
    }

    /// How a JMAP secret travels; `None` for IMAP and POP.
    pub(in crate::ui) fn jmap_auth(&self) -> Option<HttpAuth> {
        match self {
            Hand::Jmap(jmap) => Some(jmap.auth),
            _ => None,
        }
    }

    /// The form for servers already named by `setup`, to edit them. `None` for a setup that
    /// names none the person could type: the table's, or a JMAP one with no session yet.
    fn of(setup: &Setup) -> Option<Hand> {
        let endpoint = |host: &str, port: u16| Endpoint {
            host: host.to_owned(),
            port: port.to_string(),
        };
        match setup {
            Setup::Imap(m) => Some(Hand::Imap(ServerHand {
                incoming: endpoint(&m.imap_host, m.imap_port),
                outgoing: endpoint(&m.smtp_host, m.smtp_port),
                login: m.login.clone().unwrap_or_default(),
            })),
            Setup::Pop3(m) => Some(Hand::Pop3(ServerHand {
                incoming: endpoint(&m.pop3_host, m.pop3_port),
                outgoing: endpoint(&m.smtp_host, m.smtp_port),
                login: m.login.clone().unwrap_or_default(),
            })),
            Setup::Jmap {
                session: Some(session),
                auth,
                ..
            } => Some(Hand::Jmap(JmapHand {
                session: session.clone(),
                auth: *auth,
            })),
            Setup::Jmap { session: None, .. } | Setup::Discovered(_) => None,
        }
    }

    /// The same form with `typed` in `field`; a field this kind has none of changes nothing.
    pub(in crate::ui) fn with(self, field: Field, typed: String) -> Hand {
        match (self, field) {
            (Hand::Jmap(jmap), Field::Session) => Hand::Jmap(JmapHand {
                session: typed,
                ..jmap
            }),
            (Hand::Imap(mut s), field) => {
                s.set(field, typed);
                Hand::Imap(s)
            }
            (Hand::Pop3(mut s), field) => {
                s.set(field, typed);
                Hand::Pop3(s)
            }
            (hand, _) => hand,
        }
    }

    /// The same form for another kind. IMAP and POP keep what was typed, as they ask the same
    /// questions; JMAP asks different ones, so a switch to or from it starts blank.
    pub(in crate::ui) fn into_kind(self, kind: Kind) -> Hand {
        match (self, kind) {
            (Hand::Imap(s) | Hand::Pop3(s), Kind::Imap) => Hand::Imap(s),
            (Hand::Imap(s) | Hand::Pop3(s), Kind::Pop3) => Hand::Pop3(s),
            (hand, kind) if hand.kind() == kind => hand,
            (_, kind) => Hand::blank_of(kind),
        }
    }

    /// Whether anything has been typed, which is when a missing field is worth saying.
    fn started(&self) -> bool {
        match self {
            Hand::Jmap(jmap) => !jmap.session.trim().is_empty(),
            Hand::Imap(s) | Hand::Pop3(s) => [
                &s.incoming.host,
                &s.incoming.port,
                &s.outgoing.host,
                &s.outgoing.port,
                &s.login,
            ]
            .iter()
            .any(|typed| !typed.trim().is_empty()),
        }
    }
}

impl ServerHand {
    fn set(&mut self, field: Field, typed: String) {
        match field {
            Field::IncomingHost => self.incoming.host = typed,
            Field::IncomingPort => self.incoming.port = typed,
            Field::OutgoingHost => self.outgoing.host = typed,
            Field::OutgoingPort => self.outgoing.port = typed,
            Field::Login => self.login = typed,
            Field::Session => {}
        }
    }
}

/// Why a field is not usable yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Why {
    /// Nothing there. Said only once something has been typed somewhere in the form.
    Missing,
    /// Typed, and not right.
    Wrong,
}

/// What is wrong with one field, in a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Hint {
    /// The field it is about; `None` for the address, which is the sheet's own field.
    pub field: Option<Field>,
    pub why: Why,
    pub said: String,
}

impl Hint {
    fn new(field: Option<Field>, why: Why, said: impl Into<String>) -> Hint {
        Hint {
            field,
            why,
            said: said.into(),
        }
    }
}

/// How the account signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum SignIn {
    /// A password, typed into the sheet.
    Password,
    /// In a browser, with `issuer`.
    OAuth { issuer: Issuer, client: Client },
}

/// Whether an OAuth client id is at hand: from the environment or an earlier sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Client {
    Ready,
    Missing,
}

/// What the add is handed. The password, or the token, is moved in and dropped with it.
#[derive(Debug)]
pub(in crate::ui) struct Request {
    pub address: String,
    pub setup: Setup,
    pub password: Option<Password>,
}

/// Look an address up. Blocks: run it off the thread that draws.
pub(in crate::ui) type Lookup = dyn Fn(&str) -> Result<Found, Failed> + Send + Sync;
/// Follow a domain's `/.well-known/jmap` to its session URL, sending nothing but the request.
/// Blocks.
pub(in crate::ui) type FindJmap = dyn Fn(&str) -> Result<String, String> + Send + Sync;
/// Add the account. Blocks, and for OAuth waits on a browser; the address to open goes to the
/// third argument first.
pub(in crate::ui) type Add =
    dyn Fn(&SqliteStore, Request, &dyn Fn(&str)) -> Result<String, String> + Send + Sync;
/// Open an address in the system browser.
pub(in crate::ui) type Browse = dyn Fn(&str) -> Result<(), String> + Send + Sync;
/// Whether an OAuth client id is at hand for an issuer.
pub(in crate::ui) type HasClient = dyn Fn(Issuer) -> bool + Send + Sync;

/// The sheet's reach into the world, handed in so tests reach nothing.
#[derive(Clone)]
pub(in crate::ui) struct Seams {
    pub lookup: Arc<Lookup>,
    pub jmap: Arc<FindJmap>,
    pub add: Arc<Add>,
    pub client: Arc<HasClient>,
    pub browse: Arc<Browse>,
}

impl Seams {
    /// The network, the keyring and the recorded OAuth clients. In this crate's tests, a set
    /// that refuses everything: a window drawn by a test has no business with any of them.
    pub(in crate::ui) fn real() -> Seams {
        if cfg!(test) {
            return Seams {
                lookup: Arc::new(|_| Err(Failed::Broken("no lookups in tests".to_owned()))),
                jmap: Arc::new(|_| Err("no lookups in tests".to_owned())),
                add: Arc::new(|_, _, _| Err("no accounts are added in tests".to_owned())),
                client: Arc::new(|_| false),
                browse: Arc::new(|_| Err("no browser is opened in tests".to_owned())),
            };
        }
        Seams {
            lookup: Arc::new(|address| mail_core::discover::search(address, chrono::Utc::now())),
            jmap: Arc::new(mail_core::discover::find_jmap),
            add: Arc::new(|store, request, on_url| {
                let Request {
                    address,
                    setup,
                    password,
                } = request;
                mail_core::account::add_with_password(
                    store,
                    &address,
                    Some(&setup),
                    false,
                    false,
                    chrono::Utc::now(),
                    mail_core::account::Credentials {
                        password: password.as_ref(),
                        saved: &mail_core::account::saved_clients(),
                        secrets: &mail_runtime::KeyringSecrets,
                        on_url,
                    },
                )
            }),
            client: Arc::new(|issuer| {
                std::env::var("MAILO_OAUTH_CLIENT_ID").is_ok_and(|id| !id.is_empty())
                    || mail_core::account::saved_clients().get(issuer).is_some()
            }),
            browse: Arc::new(|url| webbrowser::open(url).map_err(|e| e.to_string())),
        }
    }
}

/// The domain of what is typed, when it reads as an address.
pub(in crate::ui) fn domain_of(typed: &str) -> Option<String> {
    let (local, domain) = typed.trim().rsplit_once('@')?;
    let domain = domain.to_ascii_lowercase();
    (!local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.'))
    .then_some(domain)
}

/// What the sheet says before anything is looked up: exactly what a lookup would send.
pub(in crate::ui) fn before_looking(typed: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let address = typed.trim().to_lowercase();
    match domain_of(&address) {
        Some(domain) if mail_domain::presets::preset_for(&address, now).is_some() => {
            format!("{domain} is known. Nothing is looked up.")
        }
        Some(domain) => format!("Only {domain} is looked up."),
        None => "Only the domain is looked up.".to_owned(),
    }
}

/// Look `typed` up: the built-in table first, which needs no lookup, then `seams.lookup` and
/// `seams.jmap` side by side. Each is handed only what it needs: the lookup the address, which
/// it reduces to the domain, and the JMAP search the domain's well-known URL.
pub(in crate::ui) fn look(typed: &str, seams: &Seams, now: chrono::DateTime<chrono::Utc>) -> Stage {
    let address = typed.trim().to_lowercase();
    let (Some(_), Some(well_known)) = (
        domain_of(&address),
        mail_domain::presets::well_known(&address),
    ) else {
        return Stage::Missed(Miss::NotAnAddress);
    };
    if let Some(preset) = mail_domain::presets::preset_for(&address, now) {
        let known = "from the built-in table".to_owned();
        return Stage::Found(Offer::discovered(&address, known, preset, seams));
    }
    let (found, jmap) = std::thread::scope(|scope| {
        let jmap = scope.spawn(|| (seams.jmap)(&well_known));
        let found = (seams.lookup)(&address);
        let jmap = jmap
            .join()
            .unwrap_or_else(|_| Err("the search stopped before it finished".to_owned()));
        (found, jmap)
    });
    let jmap_offer = |session: String| {
        let source = format!("at {well_known}");
        Offer::jmap(&address, session, HttpAuth::Basic, source)
    };
    let discovered =
        |found: Found| Offer::discovered(&address, found.source.to_string(), found.preset, seams);
    match (found, jmap) {
        (Ok(found), Ok(session)) => Stage::Found(Offer {
            other: Some(Box::new(jmap_offer(session))),
            ..discovered(found)
        }),
        (Ok(found), Err(_)) => Stage::Found(discovered(found)),
        (Err(_), Ok(session)) => Stage::Found(jmap_offer(session)),
        // What JMAP said is no help to the person: the domain has no JMAP server to name.
        (Err(why), Err(_)) => Stage::Missed(Miss::of(why)),
    }
}

/// When the domain offered two ways, both in one sentence.
pub(in crate::ui) fn both(offer: &Offer) -> Option<String> {
    let other = offer.other.as_deref()?;
    Some(format!(
        "This domain offers two ways in: {}; and {}. Nothing is sent to either until you use \
         one.",
        said_short(offer),
        said_short(other)
    ))
}

/// "JMAP at https://jmap.example.com/session, found at https://example.com/.well-known/jmap";
/// "IMAP and SMTP, found from the Thunderbird ISPDB".
fn said_short(offer: &Offer) -> String {
    match &offer.setup {
        Setup::Jmap {
            session: Some(session),
            ..
        } => format!("JMAP at {session}, found {}", offer.source),
        _ => format!("{}, found {}", offer.way(), offer.source),
    }
}

/// The ways a choice between two offers, as `(is JMAP, name)`, in a fixed order: the
/// autoconfig's first, JMAP second. Empty when there is no choice.
pub(in crate::ui) fn ways(offer: &Offer) -> Vec<(bool, String)> {
    let Some(other) = offer.other.as_deref() else {
        return Vec::new();
    };
    let (jmap, not) = if offer.jmap_auth().is_some() {
        (offer, other)
    } else {
        (other, offer)
    };
    vec![(false, not.way().to_owned()), (true, jmap.way().to_owned())]
}

/// The stage with the JMAP way picked or not, when that is not the one shown. A refusal goes
/// with the offer it was about.
pub(in crate::ui) fn pick_way(stage: &Stage, jmap: bool) -> Option<Stage> {
    match stage {
        Stage::Found(offer) | Stage::Refused(offer, _)
            if offer.other.is_some() && offer.jmap_auth().is_some() != jmap =>
        {
            Some(Stage::Found(offer.clone().switched()))
        }
        _ => None,
    }
}

/// The stage with a JMAP secret sent as `auth`, when it is not already.
pub(in crate::ui) fn pick_auth(stage: &Stage, auth: HttpAuth) -> Option<Stage> {
    match stage {
        Stage::Found(offer) | Stage::Refused(offer, _)
            if offer.jmap_auth().is_some_and(|now| now != auth) =>
        {
            Some(Stage::Found(offer.clone().signing_with(auth)))
        }
        Stage::ByHand(Hand::Jmap(jmap)) if jmap.auth != auth => {
            Some(Stage::ByHand(Hand::Jmap(JmapHand {
                auth,
                ..jmap.clone()
            })))
        }
        _ => None,
    }
}

/// The form with `kind` picked, when it is not the one shown.
pub(in crate::ui) fn pick_kind(stage: &Stage, kind: Kind) -> Option<Stage> {
    match stage {
        Stage::ByHand(hand) if hand.kind() != kind => {
            Some(Stage::ByHand(hand.clone().into_kind(kind)))
        }
        _ => None,
    }
}

/// From looking up to typing servers in, and from typing them in back to looking up.
///
/// The form starts from what is at hand: a JMAP session that was found (the domain said JMAP is
/// there, so JMAP is what is offered); else the servers of an offer that was typed in; else a
/// blank IMAP form.
pub(in crate::ui) fn alternate(stage: &Stage) -> Stage {
    let offer = match stage {
        Stage::ByHand(_) => return Stage::Blank,
        Stage::Found(offer) | Stage::Refused(offer, _) => offer,
        _ => return Stage::ByHand(Hand::blank()),
    };
    let offers = || [Some(offer), offer.other.as_deref()].into_iter().flatten();
    let found = offers()
        .filter_map(|offer| Hand::of(&offer.setup))
        .find(|hand| hand.kind() == Kind::Jmap)
        .or_else(|| offers().find_map(|offer| Hand::of(&offer.setup)));
    Stage::ByHand(found.unwrap_or_else(Hand::blank))
}

/// Every hint for a form, whether or not it is time to say it.
fn problems(typed: &str, hand: &Hand) -> Vec<Hint> {
    let mut hints = Vec::new();
    if domain_of(typed).is_none() {
        hints.push(Hint::new(
            None,
            Why::Wrong,
            "Enter a full address, like ada@example.com.",
        ));
    }
    match hand {
        Hand::Jmap(jmap) => {
            let session = jmap.session.trim();
            let usable = url::Url::parse(session).is_ok_and(|url| {
                url.scheme() == "https" && url.host_str().is_some_and(|h| !h.is_empty())
            });
            if session.is_empty() {
                hints.push(Hint::new(
                    Some(Field::Session),
                    Why::Missing,
                    "Enter the JMAP session URL.",
                ));
            } else if !usable {
                hints.push(Hint::new(
                    Some(Field::Session),
                    Why::Wrong,
                    "The session URL must start with https://.",
                ));
            }
        }
        Hand::Imap(s) | Hand::Pop3(s) => {
            let incoming = if matches!(hand, Hand::Imap(_)) {
                Role::Imap
            } else {
                Role::Pop3
            };
            let (a, b) = (Field::IncomingHost, Field::IncomingPort);
            endpoint_problems(&s.incoming, incoming, (a, b), &mut hints);
            let (a, b) = (Field::OutgoingHost, Field::OutgoingPort);
            endpoint_problems(&s.outgoing, Role::Smtp, (a, b), &mut hints);
        }
    }
    hints
}

fn endpoint_problems(
    endpoint: &Endpoint,
    role: Role,
    (host, port): (Field, Field),
    hints: &mut Vec<Hint>,
) {
    let typed = endpoint.host.trim();
    if typed.is_empty() {
        let what = match role {
            Role::Smtp => "outgoing",
            Role::Imap | Role::Pop3 => "incoming",
        };
        hints.push(Hint::new(
            Some(host),
            Why::Missing,
            format!("Enter the {what} mail server."),
        ));
    } else if typed.contains(|c: char| c.is_whitespace() || matches!(c, ':' | '/' | '@')) {
        hints.push(Hint::new(
            Some(host),
            Why::Wrong,
            format!(
                "Enter just the server name, like {}. The port goes in Port.",
                role.placeholder("")
            ),
        ));
    }
    if Port::parse(&endpoint.port, role.default_port()).is_none() {
        hints.push(Hint::new(
            Some(port),
            Why::Wrong,
            "A port is a number from 1 to 65535.",
        ));
    }
}

/// What the form has to say now: every wrong field, and a missing one once anything is typed.
pub(in crate::ui) fn hints(typed: &str, hand: &Hand) -> Vec<Hint> {
    let started = hand.started();
    problems(typed, hand)
        .into_iter()
        .filter(|hint| hint.why == Why::Wrong || started)
        .collect()
}

/// Servers typed in, as an offer, or what is wrong with them. The same [`Setup`] the
/// command line's `--imap`, `--pop3`, `--smtp`, `--login` and `--jmap` make. Only an `https://`
/// JMAP session is taken, since the password or token goes to that address; IMAP, POP3 and SMTP
/// are all implicit TLS, which is why their ports default to 993, 995 and 465 and there is no
/// choice of another.
pub(in crate::ui) fn by_hand(
    typed: &str,
    hand: &Hand,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Offer, Vec<Hint>> {
    let address = typed.trim().to_lowercase();
    let wrong = problems(&address, hand);
    if !wrong.is_empty() {
        return Err(wrong);
    }
    let source = "by hand".to_owned();
    let (s, incoming) = match hand {
        Hand::Jmap(jmap) => {
            return Ok(Offer::jmap(
                &address,
                jmap.session.trim().to_owned(),
                jmap.auth,
                source,
            ));
        }
        Hand::Imap(s) => (s, Role::Imap),
        Hand::Pop3(s) => (s, Role::Pop3),
    };
    // Checked above: every host is there and every port parses.
    let resolve = |e: &Endpoint, role: Role| {
        let port = Port::parse(&e.port, role.default_port()).unwrap_or(role.default_port());
        (e.host.trim().to_owned(), port.get())
    };
    let (incoming_host, incoming_port) = resolve(&s.incoming, incoming);
    let (smtp_host, smtp_port) = resolve(&s.outgoing, Role::Smtp);
    // `--login`: the whole address when none is given, or when what is given is that.
    let login = Some(s.login.trim())
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case(&address))
        .map(str::to_owned);
    let (setup, preset) = match incoming {
        Role::Pop3 => {
            let manual = ManualPop3 {
                pop3_host: incoming_host,
                pop3_port: incoming_port,
                smtp_host,
                smtp_port,
                login,
            };
            let preset = presets::manual_pop3(&address, &manual, now);
            (Setup::Pop3(manual), preset)
        }
        Role::Imap | Role::Smtp => {
            let manual = Manual {
                imap_host: incoming_host,
                imap_port: incoming_port,
                smtp_host,
                smtp_port,
                login,
            };
            let preset = presets::manual(&address, &manual, now);
            (Setup::Imap(manual), preset)
        }
    };
    let shown = mail_core::discover::describe(&address, &source, &preset);
    Ok(Offer {
        rows: rows(&shown),
        address,
        source,
        setup,
        sign_in: SignIn::Password,
        other: None,
    })
}

/// `describe`'s lines as `(what, how)`: "  incoming  IMAP …" is `("incoming", "IMAP …")`.
pub(in crate::ui) fn rows(shown: &str) -> Vec<(String, String)> {
    shown
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (what, how) = line.trim().split_once("  ")?;
            Some((what.to_owned(), how.trim().to_owned()))
        })
        .collect()
}

/// The provider an issuer is, as a button says it.
pub(in crate::ui) fn provider(issuer: Issuer) -> &'static str {
    match issuer {
        Issuer::Google => "Google",
        Issuer::Microsoft => "Microsoft",
        _ => "another issuer",
    }
}

/// A browser sign-in under way: the address it waits on, and whether a browser was opened there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct SigningIn {
    pub url: String,
    pub opened: Opened,
}

/// Whether the system browser took the sign-in's address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Opened {
    Browser,
    /// It could not be opened, and why; the address is still shown, to open by hand.
    Not(String),
}

/// Open the sign-in's `url` with `browse`, and say what the sheet should show for it.
pub(in crate::ui) fn signing_in(url: &str, browse: &Browse) -> SigningIn {
    SigningIn {
        url: url.to_owned(),
        opened: match browse(url) {
            Ok(()) => Opened::Browser,
            Err(why) => Opened::Not(why),
        },
    }
}

/// Use `offer`: the one call that sends anything, made only from Use these settings.
///
/// A password account needs its password, and an OAuth account a client id; without them
/// nothing is added, rather than an account that cannot sign in.
pub(in crate::ui) fn confirm(
    store: &SqliteStore,
    offer: Offer,
    password: Password,
    add: &Add,
    on_url: &dyn Fn(&str),
) -> Stage {
    let password = match offer.sign_in {
        SignIn::Password if password.is_empty() => {
            let what = if offer.token() {
                What::Token
            } else {
                What::Password
            };
            return Stage::Refused(offer, Refusal::Blank(what));
        }
        SignIn::Password => Some(password),
        SignIn::OAuth {
            issuer,
            client: Client::Missing,
        } => {
            return Stage::Refused(offer, Refusal::NeedsClientId(issuer));
        }
        // Nothing to hand over; dropped here, empty.
        SignIn::OAuth { .. } => None,
    };
    let request = Request {
        address: offer.address.clone(),
        setup: offer.setup.clone(),
        password,
    };
    match add(store, request, on_url) {
        Ok(said) => Stage::Added {
            said: in_words(&said),
            account: super::super::data::account_rows(store)
                .into_iter()
                .find(|row| row.address == offer.address)
                .map(|row| row.id),
        },
        Err(why) => Stage::Refused(offer, Refusal::Other(why)),
    }
}

/// What `account::add` printed, a sentence a line, in the sheet's words.
pub(in crate::ui) fn in_words(said: &str) -> Vec<String> {
    said.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            if let Some(rest) = line.strip_prefix("added ") {
                format!("Added {}.", before_as(rest))
            } else if let Some(rest) = line.strip_prefix("updated ") {
                format!(
                    "{} was already here; its settings are updated.",
                    before_as(rest)
                )
            } else if let Some(login) =
                line.strip_prefix("password stored in the keyring for login ")
            {
                format!("Password saved for {}.", login.trim_matches('"'))
            } else if line == "token stored in the keyring" {
                "Token saved.".to_owned()
            } else if let Some(rest) = line.strip_prefix("signed in; token stored in the keyring") {
                match rest.strip_prefix(", client id in ") {
                    Some(path) => format!("Signed in. The client id is kept in {path}."),
                    None => "Signed in.".to_owned(),
                }
            } else if let Some(rest) = line.strip_prefix("warning: ") {
                format!("Warning: {rest}")
            } else {
                capitalised(line)
            }
        })
        .collect()
}

/// "ada@example.com as 0b1c…" is "ada@example.com".
fn before_as(rest: &str) -> &str {
    rest.rsplit_once(" as ")
        .map_or(rest, |(address, _)| address)
}

fn capitalised(line: &str) -> String {
    let mut chars = line.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Put `account` in `space`'s scope when the Space is scoped and it is not there yet. Returns
/// whether the Space changed. A Space over every account already shows it.
pub(in crate::ui) fn widen(space: &mut Space, account: AccountId) -> bool {
    match &mut space.scope {
        Scope::Accounts(ids) if !ids.contains(&account) => {
            ids.push(account);
            true
        }
        _ => false,
    }
}
