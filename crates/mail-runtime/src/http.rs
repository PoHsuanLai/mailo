//! The HTTP client the token endpoints, Graph and JMAP are reached with.
//!
//! One place, because a request with no timeout is a setup command that hangs forever and a sync
//! pass that never returns. (Discovery's client is [`crate::lookup`]'s: https only, and shorter.)

use crate::RuntimeError;
use crate::error::Failure;
use crate::lookup::ReqwestHttp;
use std::time::Duration;

/// A token endpoint that never answers must not hang a setup command or a sync pass forever.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// The shared client.
pub fn http_client() -> Result<reqwest::Client, RuntimeError> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| RuntimeError::Connect(Failure::new("cannot build an HTTP client", e)))
}

/// [`http_client`] as `porter_http`'s seam, which is what porter-oauth sends its exchanges over.
pub fn oauth_http() -> Result<ReqwestHttp, RuntimeError> {
    http_client().map(ReqwestHttp::over)
}
