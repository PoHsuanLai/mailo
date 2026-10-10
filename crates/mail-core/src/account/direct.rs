//! Signing in straight in the browser: no form and no review, as a sign-in button on the web does.
//!
//! A provider whose accounts sign in with an OAuth redirect needs nothing typed: the browser page
//! is the whole conversation, and the issuer says afterwards whose account it was. So a front end
//! that knows the issuer ("Google") can open the page, wait for the loopback redirect (RFC 8252)
//! and add the account with the services the review step keeps (mail), with nothing of its own
//! on screen but a way to cancel. [`sign_in_with`] is that, from the pieces the add-account sheet
//! uses: [`super::oauth_client`], the runtime's browser sign-in, and [`super::add_receiving`] with
//! the credential already obtained.

use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, Utc};
use mail_domain::AuthPlan;
use mail_domain::presets::{self, Preset};
use mail_runtime::authorize::{SignInFault, identity_scopes, sign_in_told, whoami};

/// Where the issuer is asked whose account signed in; the real endpoints by default.
pub use mail_runtime::authorize::Profiles;
use mail_runtime::{AccountSecrets, ClientRegistry};
use mail_store::SqliteStore;
use porter_provider::Issuer;

use super::{Added, Credentials, Receive, Setup, add_receiving, oauth_client};
use crate::environment::Environment;

/// How long the browser has to come back before the sign-in gives up. Long enough to find a
/// password or a phone for a second factor; short enough that a tab closed without a word does
/// not leave a sign-in waiting all day.
pub const WAIT: Duration = Duration::from_secs(10 * 60);

/// Whose sign-in it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// A new account: whichever the person signs in as.
    Add,
    /// An account already here whose sign-in has to be made again. The person must sign in as
    /// this address; its servers and how it reads and sends stay as they are.
    Again { address: String },
}

/// Why a direct sign-in ended without an account. Each says itself in plain words
/// (`Display`); what to offer next is the front end's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignInError {
    /// This installation has no OAuth client for the issuer, so its page cannot be opened.
    #[error("Mail has no sign-in set up for {}.", named(.0))]
    NoClient(Issuer),
    /// The issuer hosts no mail Mail reads, or the account does not sign in with it.
    #[error("Mail can't sign in to this account with {}.", named(.0))]
    NotMail(Issuer),
    /// The person closed or denied the sign-in on the issuer's page.
    #[error("Sign-in cancelled")]
    Declined,
    /// The sign-in was cancelled here, before the browser came back.
    #[error("Sign-in cancelled")]
    Cancelled,
    /// No browser came back in time.
    #[error("The sign-in took too long. Try again when you're ready.")]
    TimedOut,
    /// The person signed in again as someone other than the account that asked.
    #[error("You signed in as {signed}, not {asked}.")]
    OtherAccount { asked: String, signed: String },
    /// No browser would open the page.
    #[error("Couldn't open your browser: {0}")]
    Browser(String),
    /// The issuer could not be reached, or answered with something that was not a sign-in.
    #[error("Couldn't finish signing in: {0}")]
    Network(String),
    /// The account or its sign-in could not be kept.
    #[error("Couldn't save the account: {0}")]
    Store(String),
}

/// An issuer as the person knows it.
pub fn name_of(issuer: Issuer) -> &'static str {
    match issuer {
        Issuer::Google => "Google",
        Issuer::Microsoft => "Microsoft",
        Issuer::Fastmail => "Fastmail",
        Issuer::Dropbox => "Dropbox",
        Issuer::Box => "Box",
        _ => "this provider",
    }
}

/// [`name_of`], for the words of an error, which hold the issuer by reference.
fn named(issuer: &Issuer) -> &'static str {
    name_of(*issuer)
}

/// Whether accounts of `issuer` are mail Mail can add from a sign-in alone: the issuer names its
/// servers (`presets::preset_for_issuer`), so nothing is left to ask once the browser is done.
pub fn signs_in_directly(issuer: Issuer) -> bool {
    // Whether there is a preset at all: the time it would be stamped with does not matter.
    presets::preset_for_issuer(issuer, "", DateTime::<Utc>::default()).is_some()
}

/// What [`sign_in_with`] reaches the world with.
pub struct Direct<'a> {
    pub store: &'a SqliteStore,
    /// Where the sign-in's token is kept.
    pub secrets: &'a dyn AccountSecrets,
    /// The OAuth clients earlier sign-ins recorded.
    pub saved: &'a ClientRegistry,
    /// What the program was started with: an OAuth client typed there wins.
    pub environment: &'a Environment,
    /// Open the sign-in page in the person's browser.
    pub browse: &'a dyn Fn(&str) -> std::io::Result<()>,
    /// Where the issuer is asked whose account signed in.
    pub profiles: &'a Profiles,
    /// How long the browser has to come back ([`WAIT`]).
    pub wait: Duration,
    pub now: DateTime<Utc>,
}

// By hand: the browser is a closure.
impl std::fmt::Debug for Direct<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Direct")
            .field("wait", &self.wait)
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// Sign in with `issuer` in the browser and add the account, or sign `Mode::Again`'s account in
/// again: open the page, wait for the browser to come back, ask the issuer whose account it was,
/// and keep it.
///
/// Ends early when `cancel` resolves (a front end's Cancel, or its handle dropped), and dropping
/// the future cancels it too; either way the loopback listener is closed with it. Not `Send`:
/// the browser's callback is borrowed across the wait, so a front end runs it on a thread of its
/// own, as the add-account sheet runs its sign-in.
pub async fn sign_in_with(
    issuer: Issuer,
    mode: Mode,
    direct: Direct<'_>,
    cancel: impl Future<Output = ()>,
) -> Result<Added, SignInError> {
    let Direct {
        store,
        secrets,
        saved,
        environment,
        browse,
        profiles,
        wait,
        now,
    } = direct;
    let client = oauth_client(issuer, environment, saved).ok_or(SignInError::NoClient(issuer))?;
    // What the account will be: the issuer's own servers for a new one, the stored plan for one
    // signed in again (a Microsoft account that reads through Graph keeps doing so).
    let kept = match &mode {
        Mode::Add => None,
        Mode::Again { address } => Some(stored(store, address, issuer, now)?),
    };
    let scopes = match &kept {
        Some(preset) => scopes_of(&preset.plan.auth),
        None => presets::preset_for_issuer(issuer, "", now)
            .map(|preset| scopes_of(&preset.plan.auth))
            .ok_or(SignInError::NotMail(issuer))?,
    };
    let scopes = with_identity(scopes, issuer);

    // The page is opened as soon as the listener has its port; a browser that would not open
    // ends the wait, since nobody will come back to it.
    let (unopened, mut refused) = tokio::sync::mpsc::unbounded_channel::<String>();
    let on_url = move |url: &str| {
        if let Err(why) = browse(url) {
            let _ = unopened.send(why.to_string());
        }
    };
    let at = porter_core::UnixSeconds(now.timestamp());
    let signing = sign_in_told(&client, &scopes, &on_url, at, wait);
    let credential = tokio::select! {
        biased;
        () = cancel => return Err(SignInError::Cancelled),
        Some(why) = refused.recv() => return Err(SignInError::Browser(why)),
        signed = signing => signed.map_err(fault)?,
    };

    let signed_as = whoami(&client, &credential, profiles, now)
        .await
        .map_err(|why| SignInError::Network(why.to_string()))?;
    let preset = match (mode, kept) {
        (Mode::Again { address }, Some(preset)) => {
            if !address.eq_ignore_ascii_case(&signed_as) {
                return Err(SignInError::OtherAccount {
                    asked: address.to_lowercase(),
                    signed: signed_as,
                });
            }
            preset
        }
        _ => presets::preset_for_issuer(issuer, &signed_as, now)
            .ok_or(SignInError::NotMail(issuer))?,
    };
    let setup = Setup::Discovered(Box::new(preset));
    add_receiving(
        store,
        &signed_as,
        Some(&setup),
        false,
        false,
        Receive::Imap,
        now,
        Credentials {
            password: None,
            saved,
            secrets,
            environment,
            // The page was opened above, and is not opened again.
            on_url: &|_| {},
            signed: Some(&credential),
        },
    )
    .await
    .map_err(|why| SignInError::Store(why.to_string()))
}

/// The account at `address` as stored, as the preset it was added from: its plan, and what its
/// server was last seen to support (or the issuer's expectation, before it ever connected).
fn stored(
    store: &SqliteStore,
    address: &str,
    issuer: Issuer,
    now: DateTime<Utc>,
) -> Result<Preset, SignInError> {
    let address = address.to_lowercase();
    let account = store
        .account_by_address(&address)
        .map_err(|why| SignInError::Store(why.to_string()))?
        .ok_or(SignInError::NotMail(issuer))?;
    let plan = account
        .plan
        .map_err(|why| SignInError::Store(why.to_string()))?;
    if !matches!(&plan.auth, AuthPlan::OAuth { issuer: theirs, .. } if *theirs == issuer) {
        return Err(SignInError::NotMail(issuer));
    }
    let expected = presets::preset_for_issuer(issuer, &address, now)
        .ok_or(SignInError::NotMail(issuer))?
        .expected_caps;
    let expected_caps = account.caps.and_then(Result::ok).unwrap_or(expected);
    Ok(Preset {
        plan,
        expected_caps,
    })
}

/// The scopes an OAuth plan signs in with; none for any other.
fn scopes_of(auth: &AuthPlan) -> Vec<String> {
    match auth {
        AuthPlan::OAuth { scopes, .. } => scopes.clone(),
        AuthPlan::Password { .. } | AuthPlan::Granted { .. } => Vec::new(),
    }
}

/// `scopes` and what the issuer needs to say whose account signed in, each once. The identity's
/// go last, so a Microsoft code is still redeemed for the mail resource named first.
pub(crate) fn with_identity(mut scopes: Vec<String>, issuer: Issuer) -> Vec<String> {
    for scope in identity_scopes(issuer) {
        if !scopes.iter().any(|have| have == scope) {
            scopes.push((*scope).to_owned());
        }
    }
    scopes
}

/// The runtime's fault as the person's reason.
fn fault(fault: SignInFault) -> SignInError {
    match fault {
        SignInFault::Declined(_) => SignInError::Declined,
        SignInFault::TimedOut => SignInError::TimedOut,
        other => SignInError::Network(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_scope_is_added_once_and_last() {
        let google = vec!["https://mail.google.com/".to_owned(), "email".to_owned()];
        assert_eq!(with_identity(google.clone(), Issuer::Google), google);
        let microsoft = with_identity(
            vec![
                "https://outlook.office.com/IMAP.AccessAsUser.All".to_owned(),
                "offline_access".to_owned(),
            ],
            Issuer::Microsoft,
        );
        assert_eq!(
            microsoft.last().map(String::as_str),
            Some(mail_runtime::authorize::MICROSOFT_PROFILE_SCOPE)
        );
        // The resource a code is redeemed for is the first one named: still the mail one.
        assert_eq!(
            microsoft[0],
            "https://outlook.office.com/IMAP.AccessAsUser.All"
        );
    }

    #[test]
    fn google_and_microsoft_sign_in_directly_and_no_other_issuer_does() {
        let cases = [
            (Issuer::Google, true),
            (Issuer::Microsoft, true),
            (Issuer::Fastmail, false),
            (Issuer::Dropbox, false),
            (Issuer::Box, false),
        ];
        for (issuer, direct) in cases {
            assert_eq!(signs_in_directly(issuer), direct, "{issuer:?}");
        }
    }

    #[test]
    fn closing_the_page_and_cancelling_here_say_the_same_plain_words() {
        assert_eq!(SignInError::Declined.to_string(), "Sign-in cancelled");
        assert_eq!(SignInError::Cancelled.to_string(), "Sign-in cancelled");
        assert_eq!(
            SignInError::NoClient(Issuer::Google).to_string(),
            "Mail has no sign-in set up for Google."
        );
    }
}
