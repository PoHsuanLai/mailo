//! Server-side rules: a ManageSieve session driven on a socket, and the push that compiles an
//! account's rules and vacation reply and installs them.
//!
//! The protocol is `mail_proto::sieve`; this module only connects, drives, and joins the two
//! connections a push needs — one to learn what the server's Sieve can do, one to install a
//! script written for exactly that.

use crate::{Cancel, RuntimeError, Transport, drive};
use chrono::{DateTime, Utc};
use mail_domain::{Rule, Vacation};
use mail_proto::sieve::{
    Compiled, Endpoint, Places, SieveAuthentication, SieveJob, SieveLogin, SieveOutcome,
    SieveSession, Takeover, compile,
};
use porter_core::Credential;

/// Who to sign in as.
///
/// `Debug` is derived: [`Credential`]'s own redacts the secret.
#[derive(Debug, Clone)]
pub struct SieveAuth {
    pub username: String,
    pub credential: Credential,
    /// The name this client's script goes by on the server, and who the script says wrote it.
    pub script_name: String,
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
            auth: SieveAuthentication::SignIn {
                username: auth.username.clone(),
                credential: auth.credential.clone(),
            },
            script_name: auth.script_name.clone(),
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

/// What an account is to run: its rules, its vacation reply, and where its folders are.
#[derive(Debug, Clone, Copy)]
pub struct Wanted<'a> {
    pub rules: &'a [Rule],
    pub vacation: Option<&'a Vacation>,
    pub places: &'a Places,
}

/// Compile `rules` and `vacation` for this server and make the result its active script — or,
/// when nothing in them can run there, take this client's script down.
///
/// Two connections: the first reads the server's Sieve extensions, so the script is written for
/// what it runs rather than for what a server usually runs. `now` places a dated vacation reply
/// on a server that cannot test dates itself.
pub async fn push(
    endpoint: &Endpoint,
    auth: &SieveAuth,
    wanted: Wanted<'_>,
    takeover: Takeover,
    now: DateTime<Utc>,
    cancel: &mut Cancel,
) -> Result<Pushed, RuntimeError> {
    let Wanted {
        rules,
        vacation,
        places,
    } = wanted;
    let caps = match manage(endpoint, auth, SieveJob::Status, cancel).await? {
        SieveOutcome::Status { caps, .. } => caps,
        other => {
            return Err(RuntimeError::UnsupportedIo(format!(
                "a status request answered as {other:?}"
            )));
        }
    };
    let compiled = compile(rules, vacation, &caps.sieve, places, &auth.script_name, now);
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
