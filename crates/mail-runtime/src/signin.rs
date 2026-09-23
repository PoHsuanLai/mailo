//! Which OAuth client this build authenticates as, and keeping an account signed in.
//!
//! An access token lasts about an hour. Nothing renewed one. `account add` stored what the
//! browser handed back, and every sync afterwards read that value out of the keyring and used
//! it verbatim — so an OAuth account worked until the token expired and then failed on every
//! pass, permanently, with an authentication error and no way out but adding the account again.
//! The refresh token sitting beside it in the same keyring entry was never spent.
//!
//! Renewing needs a client id, and a client id is not a property of the user's account: it is
//! deployment configuration, one per issuer per build channel, which is why `AuthPlan::OAuth`
//! deliberately does not carry one. This module is the configuration that comment promised —
//! a file the setup command writes and every later sync reads.
//!
//! The client id is not a secret. An installed application cannot keep one, which is the entire
//! reason for PKCE, so this is an ordinary config file and not a keyring entry.

use crate::RuntimeError;
use crate::oauth::{self, Endpoints, Freshness};
use crate::secrets::Secrets;
use chrono::{DateTime, Utc};
use mail_domain::{AccountId, Credential, OAuthIssuer, SecretKey, SecretPurpose};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A token endpoint that never answers must not hang a setup command or a sync pass forever.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// The OAuth client this build presents to one issuer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    pub issuer: OAuthIssuer,
    pub client_id: String,
    /// Google issues one with every "Desktop app" client and requires it in both the code
    /// exchange and every later refresh. Absent for Microsoft's public clients.
    ///
    /// Stored beside the client id rather than in the keyring, and that is a deliberate
    /// distinction rather than an exception to "credentials live in the keyring". This is not
    /// the user's credential: it is the *application's*, identical for every user of this build,
    /// and Google documents it as not confidential for installed clients precisely because such
    /// an application has to ship it. What protects the exchange is PKCE and the loopback
    /// redirect. The user's tokens remain in the keyring, where they belong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// Where to send the exchange when it is not where the issuer publishes it: a sovereign
    /// cloud, an inspecting proxy, or a test that must not reach the internet.
    #[serde(rename = "endpoints", default, skip_serializing_if = "Option::is_none")]
    elsewhere: Option<Endpoints>,
}

impl Registration {
    pub fn new(issuer: OAuthIssuer, client_id: impl Into<String>) -> Self {
        Self {
            issuer,
            client_id: client_id.into(),
            client_secret: None,
            elsewhere: None,
        }
    }

    /// The application secret the issuer requires, where it requires one.
    pub fn with_secret(mut self, secret: Option<impl Into<String>>) -> Self {
        self.client_secret = secret.map(Into::into);
        self
    }

    /// Send this registration's exchanges somewhere other than the published host.
    pub fn at(mut self, endpoints: Endpoints) -> Self {
        self.elsewhere = Some(endpoints);
        self
    }

    /// Where to ask, which is the override when there is one and the published host otherwise.
    pub fn endpoints(&self) -> Endpoints {
        self.elsewhere
            .clone()
            .unwrap_or_else(|| oauth::endpoints_for(self.issuer))
    }
}

/// Every issuer this installation is registered with.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthRegistry {
    /// A list rather than a map keyed by issuer: an issuer is a closed enum today and a map
    /// would make the file's shape depend on how serde spells its variants.
    #[serde(default)]
    registrations: Vec<Registration>,
}

impl OAuthRegistry {
    /// The registration for an issuer, if this installation has one.
    pub fn get(&self, issuer: OAuthIssuer) -> Option<&Registration> {
        self.registrations.iter().find(|r| r.issuer == issuer)
    }

    /// Record a registration, replacing any earlier one for the same issuer.
    ///
    /// Replacing rather than appending, because two rows for one issuer is a file where which
    /// client id is used depends on the order they were written in.
    pub fn set(&mut self, registration: Registration) {
        match self
            .registrations
            .iter_mut()
            .find(|r| r.issuer == registration.issuer)
        {
            Some(existing) => *existing = registration,
            None => self.registrations.push(registration),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.registrations.is_empty()
    }

    /// Read the file, or an empty registry if there is none.
    ///
    /// A missing file is the ordinary state of a fresh installation, not an error. Unreadable
    /// contents *are* one: silently continuing with no client id would report the account as
    /// unconfigured when it is configured and the file is simply damaged.
    pub fn load(path: &Path) -> Result<Self, RuntimeError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(RuntimeError::Io(format!("{}: {e}", path.display()))),
        };
        serde_json::from_str(&text)
            .map_err(|e| RuntimeError::Secrets(format!("{}: unreadable: {e}", path.display())))
    }

    /// Write the file, creating its directory.
    pub fn save(&self, path: &Path) -> Result<(), RuntimeError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| RuntimeError::Io(format!("{}: {e}", parent.display())))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| RuntimeError::Secrets(format!("cannot encode the registry: {e}")))?;
        std::fs::write(path, text)
            .map_err(|e| RuntimeError::Io(format!("{}: {e}", path.display())))?;
        // Owner-only. The application secret in here is not the user's credential and Google
        // publishes it to every installed client, but a config file nobody else can read costs
        // one syscall and removes the question.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Read the registry from where this installation keeps it.
    pub fn load_default() -> Result<Self, RuntimeError> {
        match default_path() {
            Some(path) => Self::load(&path),
            None => Ok(Self::default()),
        }
    }
}

/// `$XDG_CONFIG_HOME/mailo/oauth.json`, following the same base-directory spec as the database.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("mailo").join("oauth.json"))
}

/// The HTTP client a token endpoint is reached with.
///
/// One place, because a token exchange with no timeout is a setup command that hangs forever
/// and a sync pass that never returns.
pub fn http_client() -> Result<reqwest::Client, RuntimeError> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| RuntimeError::Connect(format!("cannot build an HTTP client: {e}")))
}

/// Bring a credential up to date before it is used, storing whatever comes back.
///
/// Returns the credential to authenticate with. A password is returned untouched: this is the
/// one place that knows expiry is an OAuth concept, and making the caller ask first would put
/// that knowledge in every caller.
pub async fn renew(
    account: AccountId,
    registration: &Registration,
    credential: Credential,
    secrets: &dyn Secrets,
    http: &reqwest::Client,
    now: DateTime<Utc>,
) -> Result<Credential, RuntimeError> {
    let refresh_token = match oauth::assess(&credential, now) {
        Freshness::Ready => return Ok(credential),
        // Owned before the await: the borrow is of the credential this function then replaces.
        Freshness::Expired { refresh_token } => refresh_token.to_owned(),
    };

    let renewed = oauth::refresh_at(
        &registration.endpoints(),
        &registration.client_id,
        registration.client_secret.as_deref(),
        &refresh_token,
        http,
        now,
    )
    .await?;

    // Both entries, because `account add` wrote both. The sync path reads `IncomingPassword`
    // and the browser flow is what writes `OAuthRefresh`; leaving either stale is the same
    // account failing an hour later, just via a different key.
    for purpose in [SecretPurpose::IncomingPassword, SecretPurpose::OAuthRefresh] {
        secrets.put(&SecretKey { account, purpose }, &renewed)?;
    }
    Ok(renewed)
}

/// A Graph access token for an account that sends through [`mail_domain::Outgoing::Graph`].
///
/// Kept as the account's *outgoing* credential, which `AccountEngine` already prefers for
/// submission, beside the incoming one for Exchange's IMAP: Microsoft's tokens are each for one
/// resource, so an account that reads over IMAP and sends through Graph holds two. Minted from
/// the sign-in's refresh token the first time, and from its own after that; a still-valid one
/// is returned without touching the network.
pub async fn graph_token(
    account: AccountId,
    registration: &Registration,
    secrets: &dyn Secrets,
    http: &reqwest::Client,
    now: DateTime<Utc>,
) -> Result<Credential, RuntimeError> {
    let key = |purpose| SecretKey { account, purpose };
    let held = secrets.get(&key(SecretPurpose::OutgoingPassword)).ok();
    let refresh_token = match held.as_ref().map(|c| oauth::assess(c, now)) {
        Some(Freshness::Ready) => {
            if let Some(held @ Credential::OAuth { .. }) = held {
                return Ok(held);
            }
            // A password here is left over from an SMTP setup; the sign-in's token replaces it.
            sign_in_refresh(secrets, account)?
        }
        Some(Freshness::Expired { refresh_token }) => refresh_token.to_owned(),
        None => sign_in_refresh(secrets, account)?,
    };
    let minted = oauth::refresh_for(
        &registration.endpoints(),
        &registration.client_id,
        registration.client_secret.as_deref(),
        &refresh_token,
        &[
            mail_domain::presets::GRAPH_SEND_SCOPE.to_owned(),
            "offline_access".to_owned(),
        ],
        http,
        now,
    )
    .await?;
    secrets.put(&key(SecretPurpose::OutgoingPassword), &minted)?;
    Ok(minted)
}

/// The refresh token the browser sign-in stored.
fn sign_in_refresh(secrets: &dyn Secrets, account: AccountId) -> Result<String, RuntimeError> {
    match secrets.get(&SecretKey {
        account,
        purpose: SecretPurpose::OAuthRefresh,
    })? {
        Credential::OAuth { refresh, .. } => Ok(refresh),
        Credential::Password(_) => Err(RuntimeError::Secrets(
            "sending through Graph needs a Microsoft sign-in".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn google() -> Registration {
        Registration::new(OAuthIssuer::Google, "client.apps.googleusercontent.com")
    }

    #[test]
    fn a_registration_without_an_override_asks_the_published_host() {
        assert_eq!(
            google().endpoints(),
            oauth::endpoints_for(OAuthIssuer::Google)
        );
    }

    #[test]
    fn an_override_replaces_the_published_host() {
        let ends = Endpoints {
            auth: "http://127.0.0.1:1/authorize".to_owned(),
            token: "http://127.0.0.1:1/token".to_owned(),
        };
        assert_eq!(google().at(ends.clone()).endpoints(), ends);
    }

    #[test]
    fn setting_an_issuer_twice_replaces_rather_than_appends() {
        // Two rows for one issuer is a file where which client id is used depends on the order
        // they happen to be written in.
        let mut registry = OAuthRegistry::default();
        registry.set(Registration::new(OAuthIssuer::Google, "first"));
        registry.set(Registration::new(OAuthIssuer::Google, "second"));
        registry.set(Registration::new(OAuthIssuer::Microsoft, "ms"));
        assert_eq!(registry.registrations.len(), 2);
        assert_eq!(
            registry.get(OAuthIssuer::Google).unwrap().client_id,
            "second"
        );
        assert_eq!(
            registry.get(OAuthIssuer::Microsoft).unwrap().client_id,
            "ms"
        );
    }

    #[test]
    fn a_missing_file_is_an_empty_registry_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let registry = OAuthRegistry::load(&dir.path().join("nothing-here.json")).unwrap();
        assert!(registry.is_empty());
    }

    #[test]
    fn a_damaged_file_is_reported_rather_than_read_as_empty() {
        // Reading it as empty would report a configured account as unconfigured and send the
        // user off to register a second OAuth client they already have.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth.json");
        std::fs::write(&path, "{ not json").unwrap();
        let error = OAuthRegistry::load(&path).unwrap_err().to_string();
        assert!(error.contains("unreadable"), "{error}");
    }

    #[test]
    fn a_saved_registry_reads_back_identical() {
        let dir = tempfile::tempdir().unwrap();
        // A nested path, because the config directory does not exist on a fresh machine.
        let path = dir.path().join("mailo").join("oauth.json");
        let mut registry = OAuthRegistry::default();
        registry.set(google());
        registry.set(
            Registration::new(OAuthIssuer::Microsoft, "ms-client").at(Endpoints {
                auth: "https://login.microsoftonline.us/organizations/oauth2/v2.0/authorize"
                    .to_owned(),
                token: "https://login.microsoftonline.us/organizations/oauth2/v2.0/token"
                    .to_owned(),
            }),
        );
        registry.save(&path).unwrap();
        assert_eq!(OAuthRegistry::load(&path).unwrap(), registry);
    }

    /// Google requires the application secret, and this file is where it has to live.
    ///
    /// This test used to assert the opposite — that no secret field existed, on the reasoning
    /// that an installed application cannot keep one. The reasoning is sound and the conclusion
    /// was wrong for Google, which issues a secret with every "Desktop app" client and answers
    /// the token exchange with `invalid_request: client_secret is missing` without it. The
    /// assertion held until the first real Google client was used. See FINDINGS F135.
    #[test]
    fn an_application_secret_round_trips_and_is_optional() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mailo").join("oauth.json");
        let mut registry = OAuthRegistry::default();
        registry.set(google().with_secret(Some("GOCSPX-example")));
        // Microsoft's public clients have none, and must not grow an empty one.
        registry.set(Registration::new(OAuthIssuer::Microsoft, "ms-client"));
        registry.save(&path).unwrap();

        let back = OAuthRegistry::load(&path).unwrap();
        assert_eq!(back, registry);
        assert_eq!(
            back.get(OAuthIssuer::Google)
                .unwrap()
                .client_secret
                .as_deref(),
            Some("GOCSPX-example")
        );
        assert_eq!(
            back.get(OAuthIssuer::Microsoft).unwrap().client_secret,
            None
        );

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("client_secret") || text.contains("GOCSPX-example"),
            "an absent secret should not be written at all: {text}"
        );
    }

    /// The user's tokens do not belong in it, whatever else does.
    #[test]
    fn the_file_holds_no_user_credential() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mailo").join("oauth.json");
        let mut registry = OAuthRegistry::default();
        registry.set(google().with_secret(Some("GOCSPX-example")));
        registry.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        for forbidden in ["access_token", "refresh_token", "refresh", "access"] {
            assert!(!text.contains(forbidden), "{forbidden} is in {text}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mailo").join("oauth.json");
        let mut registry = OAuthRegistry::default();
        registry.set(google().with_secret(Some("GOCSPX-example")));
        registry.save(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "mode is {mode:o}");
    }
}
