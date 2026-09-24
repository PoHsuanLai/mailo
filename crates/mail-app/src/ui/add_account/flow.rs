//! What the Add account sheet decides, as functions of what it is handed.
//!
//! The network, the keyring and the browser arrive as [`Seams`], so every rule here — look only when asked,
//! add only when told to, a password to the add and nowhere else — is tested without either.

use std::sync::Arc;

use mail_domain::presets::Preset;
use mail_domain::{AccountId, AuthPlan, OAuthIssuer};
use mail_proto::discover::Found;
use mail_store::SqliteStore;

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
    /// Where they came from: "from the built-in table", "via MX → google.com (…)".
    pub source: String,
    /// Incoming, outgoing and sign-in, each as `(what, how)`.
    pub rows: Vec<(String, String)>,
    pub preset: Preset,
    pub sign_in: SignIn,
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

/// What the add is handed. The password is moved in and dropped with it.
#[derive(Debug)]
pub(in crate::ui) struct Request {
    pub address: String,
    pub preset: Preset,
    pub password: Option<Password>,
}

/// Look an address up. Blocks: run it off the thread that draws.
pub(in crate::ui) type Lookup = dyn Fn(&str) -> Result<Found, String> + Send + Sync;
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
                add: Arc::new(|_, _, _| Err("no accounts are added in tests".to_owned())),
                client: Arc::new(|_| false),
                browse: Arc::new(|_| Err("no browser is opened in tests".to_owned())),
            };
        }
        Seams {
            lookup: Arc::new(|address| crate::discover::lookup(address, chrono::Utc::now())),
            add: Arc::new(|store, request, on_url| {
                let Request {
                    address,
                    preset,
                    password,
                } = request;
                crate::account::add_with_password(
                    store,
                    &address,
                    Some(&crate::cli::Setup::Discovered(Box::new(preset))),
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
            "Looking up sends only the domain, {domain}, to DNS and to its autoconfig servers \
             over HTTPS. Your address and password stay here until you use what is found."
        ),
        None => "Looking up sends only the domain, to DNS and to its autoconfig servers over \
                 HTTPS. Your address and password stay here until you use what is found."
            .to_owned(),
    }
}

/// Look `typed` up: the built-in table first, which needs no lookup, then `seams.lookup`.
pub(in crate::ui) fn look(typed: &str, seams: &Seams, now: chrono::DateTime<chrono::Utc>) -> Stage {
    let address = typed.trim().to_lowercase();
    if domain_of(&address).is_none() {
        return Stage::Missed("Type the whole address, like ada@example.com.".to_owned());
    }
    let (source, preset) = match mail_domain::presets::preset_for(&address, now) {
        Some(preset) => ("from the built-in table".to_owned(), preset),
        None => match (seams.lookup)(&address) {
            Ok(found) => (found.source.to_string(), found.preset),
            Err(why) => return Stage::Missed(why),
        },
    };
    let shown = crate::discover::describe(&address, &source, &preset);
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
    Stage::Found(Offer {
        rows: rows(&shown),
        address,
        source,
        preset,
        sign_in,
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
            return Stage::Refused(offer, "Type the password first.".to_owned());
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
        preset: offer.preset.clone(),
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
