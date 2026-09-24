//! Server-side rules: Sieve scripts (RFC 5228) and the ManageSieve protocol (RFC 5804) that
//! installs them.
//!
//! [`compile`] turns an account's rules and vacation reply into one script, and says which rules
//! it could not express; [`SieveSession`] is the sans-I/O client that puts that script on the
//! server. Neither decides *whether* to push — that is the caller's, which knows whether the
//! account's server offers ManageSieve at all (Gmail and Microsoft do not).

mod script;
mod session;
mod wire;

pub use script::{Compiled, Places, Unmappable, VacationPlaced, compile, quoted};
pub use session::{
    Active, Deleted, Offered, ScriptEntry, SieveCaps, SieveJob, SieveLogin, SieveOutcome,
    SieveSession, Takeover,
};

use mail_domain::{AccountPlan, AuthPlan, Incoming, OAuthIssuer, Tls};

/// Where an account's ManageSieve server is expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub tls: Tls,
}

/// Why an account has no ManageSieve server to talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoSieve {
    /// Its mail is kept on this computer.
    Local,
    /// Its provider runs its own filters and offers no ManageSieve: Google and Microsoft. Their
    /// own filter and vacation settings are reached through their own APIs, which this client
    /// does not use; rules on these accounts run here.
    Provider(OAuthIssuer),
}

impl std::fmt::Display for NoSieve {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoSieve::Local => f.write_str("this account keeps its mail on this computer"),
            NoSieve::Provider(issuer) => write!(
                f,
                "{issuer:?} offers no ManageSieve, so this account's rules run in this client \
                 and it has no server-side vacation reply here"
            ),
        }
    }
}

/// The standard ManageSieve port (RFC 5804 §1.8).
pub const PORT: u16 = 4190;

/// The ManageSieve server an account's plan implies: its incoming host, on [`PORT`], with
/// `STARTTLS` required.
///
/// Required and never optional, because ManageSieve has no implicit-TLS port to prefer and the
/// rule that holds for IMAP and SMTP holds here: an upgrade that may silently not happen is one
/// an attacker chooses for you. A server that does not offer it is not signed in to.
pub fn endpoint(plan: &AccountPlan) -> Result<Endpoint, NoSieve> {
    if let AuthPlan::OAuth { issuer, .. } = &plan.auth {
        return Err(NoSieve::Provider(*issuer));
    }
    let host = match &plan.incoming {
        Incoming::Imap { host, .. } | Incoming::Pop3 { host, .. } => host.clone(),
        Incoming::Local => return Err(NoSieve::Local),
    };
    // The same providers signed in to with a password, which a manually added account can be.
    if let Some(issuer) = provider_of(&host) {
        return Err(NoSieve::Provider(issuer));
    }
    Ok(Endpoint {
        host,
        port: PORT,
        tls: Tls::StartTlsRequired,
    })
}

/// The provider a mail host belongs to, compared by whole domain labels so that a host merely
/// ending in the same letters is not taken for it.
fn provider_of(host: &str) -> Option<OAuthIssuer> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let under = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    if under("gmail.com") || under("googlemail.com") {
        Some(OAuthIssuer::Google)
    } else if under("office365.com") || under("outlook.com") {
        Some(OAuthIssuer::Microsoft)
    } else {
        None
    }
}

/// The name this client's script goes by on the server.
///
/// One name, owned by this client: a script with any other name was written by someone else,
/// and is never replaced, deactivated or deleted without being told to.
pub const SCRIPT_NAME: &str = "mailo";
