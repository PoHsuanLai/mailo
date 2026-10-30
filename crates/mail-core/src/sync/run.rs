//! What every operation on one account's server starts from, and the sign-in they all begin with.

use super::report::Failure;
use super::{Configured, Mode, clock_for, renewal_for, renewed_typed, stored_credential};
use mail_secrets::{AccountSecrets, ClientRegistry, Held, TokenSource};
use mail_store::SqliteStore;
use std::sync::Arc;

/// The five things every operation on an account's server is handed: where its state is kept,
/// where its credentials are, which client ids sign it in, what time it is, and the account.
#[derive(Clone)]
pub(crate) struct Run<'a> {
    pub store: &'a Arc<SqliteStore>,
    pub secrets: Arc<dyn AccountSecrets>,
    pub registry: &'a ClientRegistry,
    pub now: chrono::DateTime<chrono::Utc>,
    pub account: &'a Configured,
}

/// A credential good for the operation that asked for it, and what keeps it good for as long
/// as the engine built from it runs.
pub(crate) struct Signed {
    /// The credential, which a renewal replaces as it goes.
    pub held: Held,
    /// `None` for a password account and for one this installation has no client id for
    /// ([`renewal_for`]).
    pub renewal: Option<Arc<dyn TokenSource>>,
}

/// Read the account's stored credential, renew it if it is an OAuth one that has expired, and
/// make the source that keeps it renewed.
///
/// `mode` is where the renewal reads the clock ([`clock_for`]).
pub(crate) async fn signed_in(run: &Run<'_>, mode: Mode) -> Result<Signed, Failure> {
    let stored = stored_credential(run.account, run.secrets.as_ref()).await?;
    let credential = renewed_typed(
        run.account,
        stored,
        run.secrets.as_ref(),
        run.registry,
        run.now,
    )
    .await?;
    let held = Held::new(credential);
    let renewal = renewal_for(
        run.account,
        &held,
        run.secrets.clone(),
        run.registry,
        clock_for(mode, run.now),
    );
    Ok(Signed { held, renewal })
}
