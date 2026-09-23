//! RFC 8058 one-click unsubscribe: one HTTPS `POST`, and nothing else.
//!
//! The request is the whole protocol. Its body is `List-Unsubscribe=One-Click`, form-encoded;
//! it carries no cookies (reqwest's `cookies` feature is off, so there is no jar to carry) and
//! no credentials ([`HttpsUrl`] cannot hold any). A `2xx` means the list took it. Anything else
//! is a [`UnsubscribeFailure`] saying which way it went wrong.
//!
//! Redirects are followed only where they keep the request what it was: `307` and `308` to
//! another `https:` URL. A `301`, `302` or `303` would turn the `POST` into a `GET` with no
//! body, which is a page load rather than an unsubscribe, so it is reported and not followed.
//! A redirect to `http:` is refused outright: the request would leave in the clear.

use crate::RuntimeError;
use mail_domain::{Retry, Retryable};
use mail_mime::{HttpsUrl, ONE_CLICK};
use reqwest::redirect::{Attempt, Policy};
use std::time::Duration;

/// How long the whole request may take, redirects included.
///
/// Short, because someone pressed a button and is waiting on it, and a list server that has not
/// answered in ten seconds is not going to.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// The most redirects to follow before giving up.
const REDIRECTS: usize = 3;

/// Why a one-click unsubscribe did not happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnsubscribeFailure {
    /// The server answered, and not with `2xx`. A `3xx` here is a redirect that would have
    /// dropped the `POST`, which is not an unsubscribe either.
    #[error("the list's server answered {status}; nothing was unsubscribed")]
    Refused { status: u16 },
    /// A redirect pointed somewhere other than `https:`.
    #[error(
        "the list's server redirected to {to}, which is not https; the request was not sent there"
    )]
    InsecureRedirect { to: String },
    /// No answer: the name did not resolve, the connection failed, TLS failed, or it timed out.
    #[error("could not reach the list's server: {0}")]
    Unreachable(String),
}

impl Retryable for UnsubscribeFailure {
    fn retry(&self) -> Retry {
        match self {
            // The server is busy or broken this minute; the same request may work later.
            UnsubscribeFailure::Refused {
                status: 429 | 500..=599,
            } => Retry::After(Duration::from_secs(60)),
            UnsubscribeFailure::Refused { status } => {
                Retry::Fatal(format!("the list's server answered {status}"))
            }
            UnsubscribeFailure::InsecureRedirect { to } => {
                Retry::Fatal(format!("redirected to {to}, which is not https"))
            }
            UnsubscribeFailure::Unreachable(_) => Retry::After(Duration::from_secs(5)),
        }
    }
}

/// A client configured for one-click `POST`s: https only, [`TIMEOUT`], the redirect rule above.
///
/// A builder rather than a client so a test can add a root it trusts and point a name at a
/// local listener; everything that makes the request safe is already set.
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .connect_timeout(TIMEOUT)
        .https_only(true)
        .redirect(Policy::custom(redirect))
}

/// [`client_builder`], built.
pub fn client() -> Result<reqwest::Client, RuntimeError> {
    client_builder()
        .build()
        .map_err(|e| RuntimeError::Connect(format!("cannot build an HTTP client: {e}")))
}

/// Which redirects keep the request a one-click unsubscribe.
fn redirect(attempt: Attempt<'_>) -> reqwest::redirect::Action {
    if attempt.url().scheme() != "https" {
        let to = attempt.url().to_string();
        return attempt.error(UnsubscribeFailure::InsecureRedirect { to });
    }
    let keeps_method = matches!(attempt.status().as_u16(), 307 | 308);
    if !keeps_method || attempt.previous().len() > REDIRECTS {
        // Stopping hands the redirect back as the answer, which is then a `Refused`.
        return attempt.stop();
    }
    attempt.follow()
}

/// Unsubscribe with one `POST` to `url`, through a client from [`client_builder`].
pub async fn one_click(http: &reqwest::Client, url: &HttpsUrl) -> Result<(), UnsubscribeFailure> {
    let response = http
        .post(url.as_str())
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(ONE_CLICK)
        .send()
        .await
        .map_err(failure)?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    Err(UnsubscribeFailure::Refused {
        status: status.as_u16(),
    })
}

/// A transport error, named. The redirect rule's own refusal travels inside reqwest's error.
fn failure(err: reqwest::Error) -> UnsubscribeFailure {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&err);
    while let Some(cause) = source {
        if let Some(ours) = cause.downcast_ref::<UnsubscribeFailure>() {
            return ours.clone();
        }
        source = cause.source();
    }
    UnsubscribeFailure::Unreachable(chain(&err))
}

/// An error and its causes on one line: reqwest's own message is only "error sending request".
fn chain(err: &dyn std::error::Error) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}

impl From<UnsubscribeFailure> for RuntimeError {
    fn from(failure: UnsubscribeFailure) -> Self {
        RuntimeError::Unsubscribe(failure)
    }
}
