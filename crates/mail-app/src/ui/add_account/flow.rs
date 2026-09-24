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

use mail_domain::presets::Preset;
use mail_domain::{AccountId, AuthPlan, HttpAuth, Incoming, OAuthIssuer};
use mail_proto::discover::Found;
use mail_store::SqliteStore;

use crate::cli::Setup;
use crate::password::Password;
use crate::space::{Scope, Space};

/// Where the sheet stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Stage {
    /// An address to type. Nothing has been asked of anyone.
    Blank,
    /// Looking the domain up.
    Looking,
    /// What was found, shown, and not used yet.
    Found(Offer),
    /// Nothing usable, or not an address; in words.
    Missed(String),
    /// Using the settings: saving the account, or waiting on a browser sign-in.
    Adding(Offer),
    /// Added: what that did, a sentence a line, and the account now in the store.
    Added {
        said: Vec<String>,
        account: Option<AccountId>,
    },
    /// The add refused, and why. The offer stays, so it can be tried again on purpose.
    Refused(Offer, String),
    /// A JMAP server typed in by hand: nothing is looked up.
    ByHand(Hand),
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
        let shown = crate::discover::describe(address, &source, &preset);
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

/// A JMAP server as it is being typed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Hand {
    /// The session URL, as typed.
    pub session: String,
    pub auth: HttpAuth,
}

impl Hand {
    /// Nothing typed yet, signing in with a password.
    pub(in crate::ui) fn blank() -> Hand {
        Hand {
            session: String::new(),
            auth: HttpAuth::Basic,
        }
    }
}

/// How the account signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum SignIn {
    /// A password, typed into the sheet.
    Password,
    /// In a browser, with `issuer`.
    OAuth { issuer: OAuthIssuer, client: Client },
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
pub(in crate::ui) type Lookup = dyn Fn(&str) -> Result<Found, String> + Send + Sync;
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
pub(in crate::ui) type HasClient = dyn Fn(OAuthIssuer) -> bool + Send + Sync;

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
                lookup: Arc::new(|_| Err("no lookups in tests".to_owned())),
                jmap: Arc::new(|_| Err("no lookups in tests".to_owned())),
                add: Arc::new(|_, _, _| Err("no accounts are added in tests".to_owned())),
                client: Arc::new(|_| false),
                browse: Arc::new(|_| Err("no browser is opened in tests".to_owned())),
            };
        }
        Seams {
            lookup: Arc::new(|address| crate::discover::lookup(address, chrono::Utc::now())),
            jmap: Arc::new(crate::discover::find_jmap),
            add: Arc::new(|store, request, on_url| {
                let Request {
                    address,
                    setup,
                    password,
                } = request;
                crate::account::add_with_password(
                    store,
                    &address,
                    Some(&setup),
                    false,
                    false,
                    chrono::Utc::now(),
                    crate::account::Credentials {
                        password: password.as_ref(),
                        saved: &crate::account::saved_clients(),
                        secrets: &mail_runtime::KeyringSecrets,
                        on_url,
                    },
                )
            }),
            client: Arc::new(|issuer| {
                std::env::var("MAILO_OAUTH_CLIENT_ID").is_ok_and(|id| !id.is_empty())
                    || crate::account::saved_clients().get(issuer).is_some()
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
        Some(domain) if mail_domain::presets::preset_for(&address, now).is_some() => format!(
            "mailo knows {domain}: nothing is looked up, and nothing is sent until you use \
             the settings."
        ),
        Some(domain) => format!(
            "Looking up sends only the domain, {domain}, to DNS, and over HTTPS to its \
             autoconfig servers and to https://{domain}/.well-known/jmap. Your address and \
             password stay here until you use what is found."
        ),
        None => "Looking up sends only the domain, to DNS, and over HTTPS to its autoconfig \
                 servers and its /.well-known/jmap. Your address and password stay here until \
                 you use what is found."
            .to_owned(),
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
        return Stage::Missed("Type the whole address, like ada@example.com.".to_owned());
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
        (Err(why), Err(no_jmap)) => Stage::Missed(format!(
            "{why}\n\nNo JMAP server answered at {well_known} either: {no_jmap}\n\nIf the \
             provider gives a JMAP session URL, enter it by hand."
        )),
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
        Stage::ByHand(hand) if hand.auth != auth => Some(Stage::ByHand(Hand {
            auth,
            ..hand.clone()
        })),
        _ => None,
    }
}

/// From looking up to typing a JMAP server in, starting from any JMAP session that was found;
/// and from typing one in back to looking up.
pub(in crate::ui) fn alternate(stage: &Stage) -> Stage {
    let offer = match stage {
        Stage::ByHand(_) => return Stage::Blank,
        Stage::Found(offer) | Stage::Refused(offer, _) => offer,
        _ => return Stage::ByHand(Hand::blank()),
    };
    let found = [Some(offer), offer.other.as_deref()]
        .into_iter()
        .flatten()
        .find_map(|offer| match &offer.setup {
            Setup::Jmap {
                session: Some(session),
                auth,
                ..
            } => Some(Hand {
                session: session.clone(),
                auth: *auth,
            }),
            _ => None,
        });
    Stage::ByHand(found.unwrap_or_else(Hand::blank))
}

/// A JMAP server typed in, as an offer, or why it is not one yet. Only `https://` is taken,
/// since the password or token goes to that address.
pub(in crate::ui) fn by_hand(typed: &str, hand: &Hand) -> Result<Offer, String> {
    let address = typed.trim().to_lowercase();
    if domain_of(&address).is_none() {
        return Err("Type the whole address, like ada@example.com.".to_owned());
    }
    let session = hand.session.trim();
    let usable = url::Url::parse(session)
        .is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some_and(|h| !h.is_empty()));
    if usable {
        let source = "by hand".to_owned();
        return Ok(Offer::jmap(&address, session.to_owned(), hand.auth, source));
    }
    if session.is_empty() {
        return Err("Type the JMAP session URL.".to_owned());
    }
    let secret = match hand.auth {
        HttpAuth::Basic => "password",
        HttpAuth::Bearer => "token",
    };
    Err(format!(
        "mailo takes a session URL that starts with https://, since the {secret} goes to it."
    ))
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
pub(in crate::ui) fn provider(issuer: OAuthIssuer) -> &'static str {
    match issuer {
        OAuthIssuer::Google => "Google",
        OAuthIssuer::Microsoft => "Microsoft",
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
            let what = if offer.token() { "token" } else { "password" };
            return Stage::Refused(offer, format!("Type the {what} first."));
        }
        SignIn::Password => Some(password),
        SignIn::OAuth {
            issuer,
            client: Client::Missing,
        } => {
            let why = missing_client(issuer);
            return Stage::Refused(offer, why);
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
        Err(why) => Stage::Refused(offer, format!("Not added: {why}")),
    }
}

/// Why an OAuth account cannot be signed in from here yet.
pub(in crate::ui) fn missing_client(issuer: OAuthIssuer) -> String {
    let secret = match issuer {
        OAuthIssuer::Google => " and MAILO_OAUTH_CLIENT_SECRET",
        OAuthIssuer::Microsoft => "",
    };
    format!(
        "Signing in with {} needs an OAuth client id, which mailo cannot ship. Start mailo with \
         MAILO_OAUTH_CLIENT_ID{secret} set, or run mailo account add in a terminal, which says \
         where to get one. Nothing has been added.",
        provider(issuer)
    )
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
                format!(
                    "The password is in the system keyring, for the login {}.",
                    login.trim_matches('"')
                )
            } else if line == "token stored in the keyring" {
                "The token is in the system keyring.".to_owned()
            } else if let Some(rest) = line.strip_prefix("signed in; token stored in the keyring") {
                match rest.strip_prefix(", client id in ") {
                    Some(path) => format!(
                        "Signed in. The sign-in is in the system keyring, and the client id is \
                         remembered in {path}."
                    ),
                    None => "Signed in. The sign-in is in the system keyring.".to_owned(),
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
