//! What signing mail in means to mailo, as a porter provider: the conversation the add-account
//! window's sheet runs.
//!
//! porter's sheet machine (`porter_core::sheet`, driven by `porter_service::AccountService`) is
//! the add-account flow: it asks nothing and stores nothing by itself, it says what to draw and
//! asks a provider's sign-in what to do next. This file is that sign-in for mail, over what mailo
//! already has: the address's lookup (`mail_core::discover`), the browser sign-in
//! (`mail_runtime::authorize`) and the account itself (`mail_core::account`, which writes mailo's
//! store and files the password where `mail_runtime` reads it). Every rule the old sheet kept is
//! kept here: look only when asked (the lookup sends the domain and nothing else), add only when
//! told to (`Confirm`), the password goes to the add and nowhere else.
//!
//! When the person's own accounts are porter's (step E6), the provider is `porter-families`'
//! and this file goes; the window and its mapping stay as they are.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use mail_core::account::Setup;
use mail_core::discover::{Failed, Found, Looked as Resolved};
use mail_core::password::Password;
use mail_domain::presets::{self, Preset};
use mail_domain::{AuthPlan, HttpAuth};
use mail_store::SqliteStore;
use porter_core::sheet::{
    Entry, FieldAnswer, FieldKind, FieldSpec, FieldValue, Manual as Typed, Presence, Protocol,
    SignInFault, SignInInput, manual_form, parse_manual,
};
use porter_core::{
    Account, AccountId, AccountLabel, AuthKind, CapabilityKind, Claim, Credential, Offer,
    Provenance, Restriction, Subject, UnixSeconds, WebUrl,
};
use porter_provider::{
    Issuer, Presented, Provider, ProviderError, ProviderSession, ProviderSpec, RevokeOutcome,
    SignIn, SignInMode, SignInStart, SignInStep, Signed,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// The provider files mailo offers to sign mail in with, in the order the list shows them: the
/// ones an address of their own domains signs in with a browser, the ones that sign in with a
/// (app) password, and "Other email account", which takes any other address.
const OFFERED: [&str; 7] = [
    "google",
    "microsoft",
    "fastmail",
    "icloud",
    "yahoo",
    "gmx",
    "generic-imap",
];

/// Search an address's domain for servers. Blocks.
pub type Lookup = dyn Fn(&str) -> Result<Found, Failed> + Send + Sync;
/// Follow a domain's `/.well-known/jmap` to its session URL. Blocks.
pub type FindJmap = dyn Fn(&str) -> Result<String, String> + Send + Sync;
/// Whether an OAuth client id is at hand for an issuer.
pub type HasClient = dyn Fn(Issuer) -> bool + Send + Sync;
/// Where a browser sign-in's address goes as soon as it exists.
pub type UrlSink = Arc<dyn Fn(&str) + Send + Sync>;
/// Sign `Issuer` in in the browser with these scopes, handing the address to the sink before it
/// waits. Dropping the future cancels the sign-in and frees its listener.
pub type Authorize = dyn Fn(Issuer, Vec<String>, UrlSink) -> Waiting + Send + Sync;
/// A sign-in in a browser, under way.
pub type Waiting = Pin<Box<dyn Future<Output = Result<Credential, String>> + Send>>;
/// Add the account to mailo's store, its secret to the keyring. Blocks.
pub type Add = dyn Fn(Request) -> Result<String, String> + Send + Sync;

/// What the add is handed. The password, or the credential a browser sign-in made, is moved in
/// and dropped with it.
#[derive(Debug)]
pub struct Request {
    pub address: String,
    pub setup: Setup,
    pub password: Option<Password>,
    pub signed: Option<Credential>,
}

/// The sign-in's reach into the world, handed in so tests reach nothing.
#[derive(Clone)]
pub struct Seams {
    pub lookup: Arc<Lookup>,
    pub jmap: Arc<FindJmap>,
    pub client: Arc<HasClient>,
    pub authorize: Arc<Authorize>,
    pub add: Arc<Add>,
}

impl Seams {
    /// The network, the keyring and the recorded OAuth clients.
    pub(in crate::ui) fn real(store: Arc<SqliteStore>) -> Seams {
        Seams {
            lookup: Arc::new(|address| {
                crate::edge::block_on(mail_core::discover::search(address, Utc::now()))
            }),
            jmap: Arc::new(|domain| {
                crate::edge::block_on(mail_core::discover::find_jmap(domain)).map_err(String::from)
            }),
            client: Arc::new(|issuer| {
                mail_core::account::oauth_client(
                    issuer,
                    &crate::edge::environment(),
                    &mail_core::account::saved_clients(),
                )
                .is_some()
            }),
            authorize: Arc::new(|issuer, scopes, urls| {
                Box::pin(async move {
                    // `sign_in` borrows a callback that is not `Sync`, so it runs on a thread of
                    // its own and the future waits for it. Dropping the future drops `stop`, which
                    // ends the sign-in and frees its listener.
                    let (done, answer) = tokio::sync::oneshot::channel();
                    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
                    std::thread::spawn(move || {
                        let ended = crate::edge::block_on(async {
                            let saved = mail_core::account::saved_clients();
                            let client = mail_core::account::oauth_client(
                                issuer,
                                &crate::edge::environment(),
                                &saved,
                            )
                            .ok_or_else(|| "no client id".to_owned())?;
                            let now = UnixSeconds(Utc::now().timestamp());
                            let on_url = |url: &str| urls(url);
                            tokio::select! {
                                signed = mail_runtime::authorize::sign_in(&client, &scopes, &on_url, now) => {
                                    signed.map_err(|why| why.to_string())
                                }
                                _ = stopped => Err("cancelled".to_owned()),
                            }
                        });
                        let _ = done.send(ended);
                    });
                    let _stop = stop;
                    answer
                        .await
                        .unwrap_or_else(|_| Err("the sign-in stopped".to_owned()))
                })
            }),
            add: Arc::new(move |request| {
                let Request {
                    address,
                    setup,
                    password,
                    signed,
                } = request;
                let environment = crate::edge::environment();
                crate::edge::block_on(mail_core::account::add_with_password(
                    &store,
                    &address,
                    Some(&setup),
                    false,
                    false,
                    Utc::now(),
                    mail_core::account::Credentials {
                        password: password.as_ref(),
                        saved: &mail_core::account::saved_clients(),
                        secrets: crate::edge::secrets().as_ref(),
                        environment: &environment,
                        // The sign-in has been made and its address shown already.
                        on_url: &|_| {},
                        signed: signed.as_ref(),
                    },
                ))
                .map(|added| added.address)
                .map_err(String::from)
            }),
        }
    }
}

/// The accounts the window has added, by address: what a finished sheet leaves for whoever
/// opened it (the Space the account joins, the list that draws it).
#[derive(Debug, Clone, Default)]
pub(in crate::ui) struct Added(Arc<Mutex<Vec<String>>>);

impl Added {
    fn record(&self, address: String) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(address);
    }

    /// The address of the account added last, once.
    pub(in crate::ui) fn take(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop()
    }
}

/// One way of signing mail in, as a provider file declares it.
#[derive(Clone)]
pub(in crate::ui) struct MailProvider {
    spec: ProviderSpec,
    seams: Seams,
    added: Added,
    prefill: Option<String>,
}

/// Every provider mailo offers, for `seams`; the address the person is signing in again with, if
/// any, is already in the form.
pub(in crate::ui) fn offered(
    seams: &Seams,
    added: &Added,
    prefill: Option<String>,
) -> Vec<MailProvider> {
    let known = mail_core::discover::providers::set();
    OFFERED
        .iter()
        .filter_map(|id| porter_core::ProviderId::parse(id).ok())
        .filter_map(|id| known.get(&id))
        .map(|spec| MailProvider {
            spec: spec.clone(),
            seams: seams.clone(),
            added: added.clone(),
            prefill: prefill.clone(),
        })
        .collect()
}

/// The session of a mail account in porter's sense: there is none, mailo opens its own.
#[derive(Debug)]
pub(in crate::ui) enum NoSession {}

impl ProviderSession for NoSession {
    async fn access_token(
        &self,
        _audience: &porter_core::Audience,
    ) -> Result<porter_core::IssuedToken, ProviderError> {
        match *self {}
    }

    fn renewed(&self) -> Option<Credential> {
        match *self {}
    }
}

impl Provider for MailProvider {
    type Session = NoSession;
    type SignIn = MailSignIn;

    fn spec(&self) -> &ProviderSpec {
        &self.spec
    }

    // The accounts mailo signs in are mailo's, not porter's: nothing asks the service for what
    // an account can do, a session or a revoke.
    async fn discover(
        &self,
        _account: &Account,
        _presented: &Presented,
    ) -> Result<Vec<Claim>, ProviderError> {
        Err(ProviderError::Unreadable)
    }

    async fn open(
        &self,
        _account: &AccountId,
        _presented: Presented,
    ) -> Result<NoSession, ProviderError> {
        Err(ProviderError::Unreadable)
    }

    fn sign_in(&self, start: SignInStart) -> Result<MailSignIn, ProviderError> {
        match start.mode {
            SignInMode::Add => Ok(MailSignIn {
                spec: self.spec.clone(),
                seams: self.seams.clone(),
                added: self.added.clone(),
                prefill: self.prefill.clone(),
                state: State::Fresh,
            }),
            SignInMode::Reauthenticate { .. } => Err(ProviderError::Unreadable),
        }
    }

    async fn revoke(
        &self,
        _account: &Account,
        _presented: &Presented,
    ) -> Result<RevokeOutcome, ProviderError> {
        Ok(RevokeOutcome::Unsupported)
    }
}

/// How an offer signs in.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Auth {
    /// A password, typed into the form.
    Password,
    /// In a browser, with `issuer`, asking for `scopes`.
    OAuth { issuer: Issuer, scopes: Vec<String> },
}

/// Settings found for an address.
#[derive(Debug, Clone, PartialEq)]
struct Proposal {
    address: String,
    setup: Setup,
    auth: Auth,
}

impl Proposal {
    /// Servers discovery or the built-in table found.
    fn discovered(address: &str, preset: Preset) -> Proposal {
        let auth = match &preset.plan.auth {
            // Discovery finds servers to sign in to with mailo's own sign-in; an account of the
            // desktop's accountd is read from it (`mail_core::account::reconcile`), never found.
            AuthPlan::Password { .. } | AuthPlan::Granted { .. } => Auth::Password,
            AuthPlan::OAuth { issuer, scopes } => Auth::OAuth {
                issuer: *issuer,
                scopes: scopes.clone(),
            },
        };
        Proposal {
            address: address.to_owned(),
            setup: Setup::Discovered(Box::new(preset)),
            auth,
        }
    }

    /// A JMAP session at `session`, its secret sent as a password.
    fn jmap(address: &str, session: String) -> Proposal {
        Proposal {
            address: address.to_owned(),
            setup: Setup::Jmap {
                session: Some(session),
                login: None,
                auth: HttpAuth::Basic,
            },
            auth: Auth::Password,
        }
    }

    /// The servers the person typed ([`mail_core::discover::typed`] decides what they come to).
    fn typed(
        address: &str,
        typed: Typed,
        password: Option<Password>,
        now: DateTime<Utc>,
    ) -> (Proposal, Option<Password>) {
        let (setup, password) = mail_core::discover::typed(address, typed, password, now);
        let proposal = Proposal {
            address: address.to_owned(),
            setup,
            auth: Auth::Password,
        };
        (proposal, password)
    }
}

/// What looking an address up came to.
#[derive(Debug, Clone, PartialEq)]
enum Looked {
    Found(Proposal),
    /// Nothing usable is published: ask for the server.
    Ask,
    /// It ended without an offer.
    Failed(SignInFault),
}

/// Look `typed` up: the built-in table first, which needs no lookup, then the autoconfig search
/// and the JMAP search side by side. Each is handed only what it needs: the search the address,
/// which it reduces to the domain, and the JMAP search the domain's well-known URL. When both
/// answer, what the autoconfig named is the offer, as it was the first of the two the old sheet
/// showed.
fn look(typed: &str, seams: &Seams, now: DateTime<Utc>) -> Looked {
    let address = typed.trim().to_lowercase();
    let (Some(_), Some(well_known)) = (
        mail_core::discover::typed_domain(&address),
        presets::well_known(&address),
    ) else {
        return Looked::Failed(SignInFault::Refused);
    };
    if let Some(preset) = mail_core::discover::known(&address, now) {
        return Looked::Found(Proposal::discovered(&address, preset));
    }
    let (found, jmap) = std::thread::scope(|scope| {
        let jmap = scope.spawn(|| (seams.jmap)(&well_known));
        let found = (seams.lookup)(&address);
        // A search that panicked found nothing.
        (found, jmap.join().ok().and_then(Result::ok))
    });
    match mail_core::discover::settle(found, jmap) {
        Resolved::Found(found) => Looked::Found(Proposal::discovered(&address, found.preset)),
        Resolved::Jmap(session) => Looked::Found(Proposal::jmap(&address, session)),
        // A personal mailbox no longer takes a password, and the form cannot say what does.
        Resolved::PersonalMicrosoft(_) => Looked::Failed(SignInFault::Forbidden),
        Resolved::Ask(_) => Looked::Ask,
        Resolved::Unreachable(_) => Looked::Failed(SignInFault::Unreachable),
    }
}

/// The fields the first form asks for: the address, then a password where one is wanted.
fn form(wants: Wants, prefill: Option<&str>) -> Vec<FieldSpec> {
    let address = FieldSpec {
        kind: FieldKind::Address,
        entry: Entry::Plain,
        presence: Presence::Required,
        prefill: prefill.map(str::to_owned),
    };
    let password = FieldSpec {
        kind: FieldKind::Password,
        entry: Entry::Secret,
        presence: Presence::Required,
        prefill: None,
    };
    let mut fields = vec![address];
    if wants == Wants::Password {
        fields.push(password);
    }
    fields
}

/// Whether a form asks for a password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wants {
    Password,
    NoPassword,
}

/// The plain text typed for `kind`, trimmed, when it is not empty.
fn text_of(answers: &[FieldAnswer], kind: FieldKind) -> Option<String> {
    answers
        .iter()
        .find(|answer| answer.kind == kind)
        .map(|answer| match &answer.value {
            FieldValue::Plain(text) => text.trim().to_owned(),
            FieldValue::Secret(secret) => secret.expose().trim().to_owned(),
        })
        .filter(|text| !text.is_empty())
}

/// The secret typed for `kind`, exactly as typed (a password may start or end with a space).
fn secret_of(answers: &[FieldAnswer], kind: FieldKind) -> Option<Password> {
    answers
        .iter()
        .find(|answer| answer.kind == kind)
        .map(|answer| match &answer.value {
            FieldValue::Secret(secret) => secret.expose().to_owned(),
            FieldValue::Plain(text) => text.clone(),
        })
        .filter(|text| !text.is_empty())
        .map(Password::new)
}

/// A browser sign-in under way. Dropping it cancels the sign-in.
struct Signing {
    task: JoinHandle<Result<Credential, String>>,
}

impl Drop for Signing {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// What a form's answer has to wait for.
#[derive(Debug)]
struct Held {
    address: String,
    password: Option<Password>,
}

enum State {
    Fresh,
    /// The first form is on screen.
    Asked,
    /// No servers were found: the server is asked for.
    AskedServer(Held),
    /// A browser sign-in is waiting for the person.
    Signing {
        signing: Signing,
        offer: Proposal,
    },
    /// What was found is on screen, waiting for the person's confirmation.
    Reviewing {
        offer: Proposal,
        password: Option<Password>,
        signed: Option<Credential>,
    },
    Ended,
}

// By hand: the state holds a password and a credential, and a task that is not `Debug`.
impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            State::Fresh => "Fresh",
            State::Asked => "Asked",
            State::AskedServer(_) => "AskedServer",
            State::Signing { .. } => "Signing",
            State::Reviewing { .. } => "Reviewing",
            State::Ended => "Ended",
        })
    }
}

/// The sign-in conversation of one provider.
pub(in crate::ui) struct MailSignIn {
    spec: ProviderSpec,
    seams: Seams,
    added: Added,
    prefill: Option<String>,
    state: State,
}

impl MailSignIn {
    /// Whether the first form asks for a password: a provider whose accounts sign in in a
    /// browser does not, and one that finds out only after the lookup asks for one then.
    fn wants(&self) -> Wants {
        match self.spec.auth.kind {
            AuthKind::OAuthPkce => Wants::NoPassword,
            _ => Wants::Password,
        }
    }

    fn failed(&mut self, fault: SignInFault) -> SignInStep {
        self.state = State::Ended;
        SignInStep::Failed(fault)
    }

    /// The form again, with `prefill` in its address.
    fn ask(&mut self, wants: Wants, prefill: Option<&str>) -> SignInStep {
        self.state = State::Asked;
        SignInStep::AskFields(form(wants, prefill))
    }

    async fn submitted(&mut self, answers: Vec<FieldAnswer>) -> SignInStep {
        let Some(address) = text_of(&answers, FieldKind::Address) else {
            return self.failed(SignInFault::Refused);
        };
        // An app password (iCloud, Fastmail, Yahoo) is the password the server takes.
        let password = secret_of(&answers, FieldKind::Password)
            .or_else(|| secret_of(&answers, FieldKind::AppPassword));
        let seams = self.seams.clone();
        let typed = address.clone();
        let looked = tokio::task::spawn_blocking(move || look(&typed, &seams, Utc::now())).await;
        match looked {
            Ok(Looked::Found(offer)) => self.found(offer, password).await,
            Ok(Looked::Ask) => {
                // The servers are typed by hand, from guesses at the address's own domain. A
                // password already typed is not asked for again.
                let address = address.to_lowercase();
                let guess = mail_core::discover::typed_domain(&address);
                self.state = State::AskedServer(Held { address, password });
                SignInStep::AskFields(manual_form(Protocol::Imap, guess.as_deref()))
            }
            Ok(Looked::Failed(fault)) => self.failed(fault),
            Err(_) => self.failed(SignInFault::Unreadable),
        }
    }

    /// What the form of typed servers answers: IMAP or POP3 and SMTP there, or a JMAP session.
    async fn served(&mut self, held: Held, answers: Vec<FieldAnswer>) -> SignInStep {
        // The sheet checked the answers before it sent them; one that is wrong anyway is a
        // form this provider did not ask.
        let Ok(typed) = parse_manual(&answers) else {
            return self.failed(SignInFault::Unreadable);
        };
        let Held { address, password } = held;
        let (offer, password) = Proposal::typed(&address, typed, password, Utc::now());
        self.found(offer, password).await
    }

    /// An offer: a password account goes to review once it has its password, a browser account
    /// to the browser.
    async fn found(&mut self, offer: Proposal, password: Option<Password>) -> SignInStep {
        match offer.auth.clone() {
            Auth::Password => match password {
                Some(password) => self.review(offer, Some(password), None),
                None => {
                    // The form that was asked for no password: ask for it, with the address as
                    // it was typed.
                    let address = offer.address.clone();
                    self.ask(Wants::Password, Some(&address))
                }
            },
            Auth::OAuth { issuer, scopes } => {
                if !(self.seams.client)(issuer) {
                    return self.failed(SignInFault::NeedsClientId);
                }
                self.sign_in_browser(offer, issuer, scopes).await
            }
        }
    }

    /// Start the browser sign-in and say where the person goes. It runs on, and is cancelled by
    /// dropping the state it lives in.
    async fn sign_in_browser(
        &mut self,
        offer: Proposal,
        issuer: Issuer,
        scopes: Vec<String>,
    ) -> SignInStep {
        let (sender, mut urls) = mpsc::unbounded_channel::<String>();
        let sink: UrlSink = Arc::new(move |url| {
            // Nobody is waiting for it once the sign-in is cancelled.
            let _ = sender.send(url.to_owned());
        });
        let waiting = (self.seams.authorize)(issuer, scopes, sink);
        let signing = Signing {
            task: tokio::spawn(waiting),
        };
        // A sign-in that ended before it had an address could not start (no listener to bind).
        let Some(url) = urls.recv().await else {
            return self.failed(SignInFault::Unreachable);
        };
        let Ok(page) = WebUrl::parse(&url) else {
            return self.failed(SignInFault::Unreadable);
        };
        self.state = State::Signing { signing, offer };
        SignInStep::OpenBrowser { url: page }
    }

    /// The browser sign-in's answer, once there is one. Cancelled by the person's own input
    /// winning the race: nothing is taken out of the state until the sign-in has ended.
    async fn polled(&mut self) -> SignInStep {
        let State::Signing { signing, .. } = &mut self.state else {
            return self.failed(SignInFault::Unreadable);
        };
        let ended = (&mut signing.task).await;
        let State::Signing { offer, .. } = std::mem::replace(&mut self.state, State::Ended) else {
            return self.failed(SignInFault::Unreadable);
        };
        match ended {
            Ok(Ok(credential)) => self.review(offer, None, Some(credential)),
            Ok(Err(_)) => self.failed(SignInFault::Refused),
            Err(_) => self.failed(SignInFault::Unreadable),
        }
    }

    /// Show what was found: the mail the account will bring, under the address.
    fn review(
        &mut self,
        offer: Proposal,
        password: Option<Password>,
        signed: Option<Credential>,
    ) -> SignInStep {
        let step = SignInStep::Review {
            claims: mail_claims(&self.spec),
            endpoints: Vec::new(),
            restriction: Restriction::none(),
            label: AccountLabel(offer.address.clone()),
        };
        self.state = State::Reviewing {
            offer,
            password,
            signed,
        };
        step
    }

    /// The person said yes: add the account. The one call that sends a credential anywhere.
    async fn confirmed(
        &mut self,
        offer: Proposal,
        password: Option<Password>,
        signed: Option<Credential>,
    ) -> SignInStep {
        let add = self.seams.add.clone();
        let request = Request {
            address: offer.address.clone(),
            setup: offer.setup,
            password,
            signed,
        };
        let address = offer.address;
        match tokio::task::spawn_blocking(move || add(request)).await {
            Ok(Ok(_)) => {
                self.added.record(address.clone());
                self.state = State::Ended;
                SignInStep::Done(Signed {
                    label: AccountLabel(address),
                    credentials: Vec::new(),
                    claims: mail_claims(&self.spec),
                    endpoints: Vec::new(),
                    restriction: Restriction::none(),
                })
            }
            Ok(Err(_)) | Err(_) => self.failed(SignInFault::StoreFailed),
        }
    }
}

/// What a mail account offers: the provider file's mail row, once. mailo reads mail and nothing
/// else of the provider's, so nothing else is shown to switch.
fn mail_claims(spec: &ProviderSpec) -> Vec<Claim> {
    spec.capabilities
        .iter()
        .find(|row| row.capability.kind() == CapabilityKind::Mail)
        .map(|row| Claim {
            subject: Subject::Account,
            offer: Offer::Present(row.capability.clone()),
            provenance: Provenance::Declared,
        })
        .into_iter()
        .collect()
}

impl SignIn for MailSignIn {
    async fn next(&mut self, input: SignInInput) -> SignInStep {
        match input {
            // Dropping the state cancels a browser sign-in in flight.
            SignInInput::Cancel => self.failed(SignInFault::Cancelled),
            // Begin, and begin again after a step back or a failure.
            SignInInput::Start => {
                self.state = State::Ended;
                let prefill = self.prefill.clone();
                let wants = self.wants();
                self.ask(wants, prefill.as_deref())
            }
            SignInInput::Poll => self.polled().await,
            SignInInput::Fields(answers) => {
                match std::mem::replace(&mut self.state, State::Ended) {
                    State::Asked => self.submitted(answers).await,
                    State::AskedServer(held) => self.served(held, answers).await,
                    _ => self.failed(SignInFault::Unreadable),
                }
            }
            SignInInput::Confirm(_) => match std::mem::replace(&mut self.state, State::Ended) {
                State::Reviewing {
                    offer,
                    password,
                    signed,
                } => self.confirmed(offer, password, signed).await,
                _ => self.failed(SignInFault::Unreadable),
            },
        }
    }
}

// Nothing of a typed secret is printed.
impl std::fmt::Debug for MailSignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailSignIn")
            .field("provider", &self.spec.id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

// By hand: the seams are closures, and a `Request` holds a password.
impl std::fmt::Debug for Seams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Seams")
    }
}
