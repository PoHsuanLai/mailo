//! Server-side rules: a ManageSieve session driven on a socket, and the push that compiles an
//! account's rules and vacation reply and installs them.
//!
//! The protocol is `mail_proto::sieve`; this module only connects, drives, and joins the two
//! connections a push needs — one to learn what the server's Sieve can do, one to install a
//! script written for exactly that.

use crate::{Cancel, RuntimeError, Transport, drive};
use chrono::{DateTime, Utc};
use mail_domain::{Credential, Rule, Vacation};
use mail_proto::sieve::{
    Compiled, Endpoint, Places, SieveJob, SieveLogin, SieveOutcome, SieveSession, Takeover, compile,
};

/// Who to sign in as.
///
/// `Debug` is derived: [`Credential`]'s own redacts the secret.
#[derive(Debug, Clone)]
pub struct SieveAuth {
    pub username: String,
    pub credential: Credential,
}

/// Do one job on the account's ManageSieve server.
pub async fn manage(
    endpoint: &Endpoint,
    auth: &SieveAuth,
    job: SieveJob,
    cancel: &mut Cancel,
) -> Result<SieveOutcome, RuntimeError> {
    let mut transport = Transport::connect(&endpoint.host, endpoint.port, endpoint.tls).await?;
    let mut session = SieveSession::new(
        SieveLogin {
            host: endpoint.host.clone(),
            port: endpoint.port,
            tls: endpoint.tls,
            username: auth.username.clone(),
            credential: auth.credential.clone(),
        },
        job,
    );
    drive(&mut session, &mut transport, cancel).await
}

/// What a push wrote, and what the server said to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed {
    pub compiled: Compiled,
    /// [`SieveOutcome::Installed`], [`SieveOutcome::Refused`] when another script is active and
    /// `takeover` did not allow replacing it, or [`SieveOutcome::Removed`] when there was nothing
    /// left to install.
    pub outcome: SieveOutcome,
}

/// Compile `rules` and `vacation` for this server and make the result its active script — or,
/// when nothing in them can run there, take this client's script down.
///
/// Two connections: the first reads the server's Sieve extensions, so the script is written for
/// what it runs rather than for what a server usually runs. `now` places a dated vacation reply
/// on a server that cannot test dates itself.
#[allow(clippy::too_many_arguments)]
pub async fn push(
    endpoint: &Endpoint,
    auth: &SieveAuth,
    rules: &[Rule],
    vacation: Option<&Vacation>,
    places: &Places,
    takeover: Takeover,
    now: DateTime<Utc>,
    cancel: &mut Cancel,
) -> Result<Pushed, RuntimeError> {
    let caps = match manage(endpoint, auth, SieveJob::Status, cancel).await? {
        SieveOutcome::Status { caps, .. } => caps,
        other => {
            return Err(RuntimeError::UnsupportedIo(format!(
                "a status request answered as {other:?}"
            )));
        }
    };
    let compiled = compile(rules, vacation, &caps.sieve, places, now);
    let job = if compiled.is_empty() {
        SieveJob::Remove
    } else {
        SieveJob::Install {
            script: compiled.script.clone(),
            takeover,
        }
    };
    let outcome = manage(endpoint, auth, job, cancel).await?;
    Ok(Pushed { compiled, outcome })
}
