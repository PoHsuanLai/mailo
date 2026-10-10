//! `account add <address> --jmap` with no URL: the domain's `/.well-known/jmap` (RFC 8620 §2.2),
//! found without a credential. Confirmed before one is sent by the command line
//! (`mail_app::cli::discover`).

use crate::error::CoreError;

/// Follow `url` over the network, without a credential.
pub async fn find(url: &str) -> Result<String, CoreError> {
    let http = mail_runtime::lookup::client_builder()
        .build()
        .map_err(|e| CoreError::cannot("build an HTTP client", e))?;
    Ok(mail_runtime::jmap::find_session(&http, url).await?)
}
