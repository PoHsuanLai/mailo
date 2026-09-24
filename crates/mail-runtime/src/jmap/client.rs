//! The HTTP half of JMAP: fetching the session, posting requests, moving blobs.
//!
//! Everything protocol-shaped is `mail_proto::jmap`'s; this module turns its values into HTTP
//! requests and HTTP answers back into its values, and classifies what went wrong the one way
//! the outbox can act on — `Retryable`.

use crate::RuntimeError;
use mail_proto::jmap::{self, Call, Responses, Session};
use mail_proto::{ProtoError, Refusal};
use std::fmt;
use std::time::Duration;

/// How requests prove who is asking. The secret is never printed.
#[derive(Clone)]
pub enum Auth {
    Basic { username: String, password: String },
    Bearer(String),
}

// By hand, like `Credential`'s: a derived Debug would put the password in every error chain.
impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Auth::Basic { username, .. } => f
                .debug_struct("Auth::Basic")
                .field("username", username)
                .field("password", &"<redacted>")
                .finish(),
            Auth::Bearer(_) => f.write_str("Auth::Bearer(<redacted>)"),
        }
    }
}

impl Auth {
    fn apply(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Auth::Basic { username, password } => request.basic_auth(username, Some(password)),
            Auth::Bearer(token) => request.bearer_auth(token),
        }
    }
}

/// A fetched session, and what is needed to use it.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    auth: Auth,
    pub session: Session,
}

/// Whether `url` may carry a credential: HTTPS, or plain HTTP to this machine alone.
///
/// A session resource names four more URLs, and a password sent to any of them in the clear is
/// a password given away — so every one is checked, not only the one the user typed. Loopback
/// is allowed because nothing crosses a network to reach it, and it is how the tests run.
pub fn safe_url(url: &url::Url) -> Result<(), RuntimeError> {
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    if url.scheme() == "https" || (url.scheme() == "http" && loopback) {
        Ok(())
    } else {
        Err(RuntimeError::Tls(format!(
            "JMAP: {url} is not HTTPS, and the account's password would cross the network in \
             the clear"
        )))
    }
}

/// `relative` against `base`, and checked by [`safe_url`].
fn resolve(base: &url::Url, relative: &str) -> Result<String, RuntimeError> {
    // A template's `{…}` survive `join`: they are ordinary characters until expanded.
    let url = base
        .join(relative)
        .map_err(|e| ProtoError::Malformed(format!("JMAP: {relative:?} is not a URL: {e}")))?;
    safe_url(&url)?;
    // `join` percent-encodes braces; the templates need them back to be expanded.
    Ok(url.as_str().replace("%7B", "{").replace("%7D", "}"))
}

impl Client {
    /// Fetch the session resource at `session_url` and resolve every URL it names against it.
    pub async fn connect(
        http: &reqwest::Client,
        session_url: &str,
        auth: Auth,
    ) -> Result<Client, RuntimeError> {
        let url = url::Url::parse(session_url)
            .map_err(|e| RuntimeError::Connect(format!("JMAP: {session_url:?}: {e}")))?;
        safe_url(&url)?;
        let response = auth
            .apply(http.get(url))
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| RuntimeError::Connect(format!("JMAP session: {e}")))?;
        // Relative URLs in the session are relative to where it was finally found.
        let base = response.url().clone();
        let body = checked(response, "the session").await?;
        let mut session = Session::parse(&body)?;
        session.api_url = resolve(&base, &session.api_url)?;
        session.download_url = resolve(&base, &session.download_url)?;
        session.upload_url = resolve(&base, &session.upload_url)?;
        session.event_source_url = session
            .event_source_url
            .as_deref()
            .map(|u| resolve(&base, u))
            .transpose()?;
        Ok(Client {
            http: http.clone(),
            auth,
            session,
        })
    }

    /// Post `calls` in one request.
    pub async fn call(&self, using: &[&str], calls: &[Call]) -> Result<Responses, RuntimeError> {
        let body = jmap::request(using, calls);
        let response = self
            .auth
            .apply(self.http.post(&self.session.api_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::Connect(format!("JMAP: {e}")))?;
        let bytes = checked(response, "a request").await?;
        Ok(Responses::parse(&bytes)?)
    }

    /// Download a blob: a message's raw RFC 5322 bytes.
    pub async fn download(&self, blob: &str) -> Result<Vec<u8>, RuntimeError> {
        let url = jmap::expand(
            &self.session.download_url,
            &[
                ("accountId", &self.session.account),
                ("blobId", blob),
                ("type", "message/rfc822"),
                ("name", "message.eml"),
            ],
        );
        let response = self
            .auth
            .apply(self.http.get(url))
            .send()
            .await
            .map_err(|e| RuntimeError::Connect(format!("JMAP download: {e}")))?;
        checked(response, "a download").await
    }

    /// Upload `bytes` as a message; returns the blob id to import it by.
    pub async fn upload(&self, bytes: Vec<u8>) -> Result<String, RuntimeError> {
        let limit = self.session.limits.max_size_upload;
        if limit > 0 && bytes.len() as u64 > limit {
            return Err(RuntimeError::Proto(ProtoError::Refused {
                kind: Refusal::Permanent,
                text: format!(
                    "the message is {} MB and the server accepts at most {} MB",
                    bytes.len() / (1024 * 1024),
                    limit / (1024 * 1024)
                ),
            }));
        }
        let url = jmap::expand(
            &self.session.upload_url,
            &[("accountId", &self.session.account)],
        );
        let response = self
            .auth
            .apply(self.http.post(url))
            .header(reqwest::header::CONTENT_TYPE, "message/rfc822")
            .body(bytes)
            .send()
            .await
            .map_err(|e| RuntimeError::Connect(format!("JMAP upload: {e}")))?;
        let body = checked(response, "an upload").await?;
        let value: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| ProtoError::Malformed(format!("JMAP upload answer: {e}")))?;
        value
            .get("blobId")
            .and_then(|b| b.as_str())
            .map(str::to_owned)
            .ok_or_else(|| {
                RuntimeError::Proto(ProtoError::Malformed(
                    "JMAP upload answer has no blobId".to_owned(),
                ))
            })
    }

    /// Open the event source for email and mailbox changes, pinging every `ping`.
    ///
    /// `None` when the server offers none. `http` is the caller's: an event stream is held open
    /// for as long as it lasts, so it needs a client without the per-request timeout.
    pub async fn events(
        &self,
        http: &reqwest::Client,
        ping: Duration,
    ) -> Result<Option<reqwest::Response>, RuntimeError> {
        let Some(template) = &self.session.event_source_url else {
            return Ok(None);
        };
        let url = jmap::expand(
            template,
            &[
                ("types", "Email,Mailbox"),
                ("closeafter", "no"),
                ("ping", &ping.as_secs().to_string()),
            ],
        );
        let response = self
            .auth
            .apply(http.get(url))
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(|e| RuntimeError::Connect(format!("JMAP push: {e}")))?;
        if response.status().is_success() {
            return Ok(Some(response));
        }
        Err(refusal(response, "push").await)
    }
}

/// The body of a successful response, or the refusal an unsuccessful one means.
async fn checked(response: reqwest::Response, what: &str) -> Result<Vec<u8>, RuntimeError> {
    if response.status().is_success() {
        return response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| RuntimeError::Io(format!("JMAP {what}: {e}")));
    }
    Err(refusal(response, what).await)
}

/// What an unsuccessful status means, classified by the status and, for a request-level error,
/// by the problem type RFC 8620 §3.6.1 fixes — never by prose.
async fn refusal(response: reqwest::Response, what: &str) -> RuntimeError {
    let status = response.status().as_u16();
    let after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let body = response.text().await.unwrap_or_default();
    RuntimeError::Proto(classify(status, &body, after, what))
}

/// [`refusal`] without the network, so it can be tested.
pub(crate) fn classify(status: u16, body: &str, after: Option<Duration>, what: &str) -> ProtoError {
    let problem = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned));
    let text = format!(
        "JMAP {what}: HTTP {status}{}",
        problem
            .as_deref()
            .map(|p| format!(" ({p})"))
            .unwrap_or_default()
    );
    match status {
        401 => ProtoError::AuthRejected(text),
        429 => ProtoError::Throttled {
            reason: text,
            retry_after: after,
        },
        503 if after.is_some() => ProtoError::Throttled {
            reason: text,
            retry_after: after,
        },
        500..=599 => ProtoError::Refused {
            kind: Refusal::Transient,
            text,
        },
        _ => ProtoError::Refused {
            kind: Refusal::Permanent,
            text,
        },
    }
}

/// Where the JMAP session at `url` really is, found without sending any credential.
///
/// For `account add --jmap` with no URL: `https://<domain>/.well-known/jmap` usually redirects
/// to the provider's own host, and the user is shown that final URL before any password goes
/// there. A session resource refuses a stranger, so `401` is the answer that says one exists;
/// `200` says so too. Anything else — `404`, a redirect to plain HTTP (refused by `http`), no
/// server at all — is "not found".
pub async fn find_session(http: &reqwest::Client, url: &str) -> Result<String, RuntimeError> {
    let response = http
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| RuntimeError::Connect(format!("no JMAP session at {url}: {e}")))?;
    let found = response.url().clone();
    match response.status().as_u16() {
        200 | 401 => {
            safe_url(&found)?;
            Ok(found.to_string())
        }
        status => Err(RuntimeError::Connect(format!(
            "no JMAP session at {url}: the server answered HTTP {status}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::{Retry, Retryable};

    #[test]
    fn a_refusal_is_classified_by_its_status() {
        const CASES: &[(u16, &str)] = &[
            (401, "reauth"),
            (429, "after"),
            (500, "after"),
            (400, "fatal"),
            (404, "fatal"),
        ];
        for (status, want) in CASES {
            let got = match classify(
                *status,
                r#"{"type":"urn:ietf:params:jmap:error:limit"}"#,
                None,
                "x",
            )
            .retry()
            {
                Retry::NeedsReauth => "reauth",
                Retry::After(_) => "after",
                Retry::Fatal(_) => "fatal",
                Retry::Now => "now",
            };
            assert_eq!(got, *want, "{status}");
        }
    }

    #[test]
    fn a_password_never_goes_anywhere_in_the_clear() {
        for (url, ok) in [
            ("https://jmap.example.com/api/", true),
            ("http://127.0.0.1:8080/api/", true),
            ("http://localhost/api/", true),
            ("http://jmap.example.com/api/", false),
            ("ftp://jmap.example.com/", false),
        ] {
            assert_eq!(
                safe_url(&url::Url::parse(url).unwrap()).is_ok(),
                ok,
                "{url}"
            );
        }
    }

    #[test]
    fn templates_survive_resolution() {
        let base = url::Url::parse("https://jmap.example.com/.well-known/jmap").unwrap();
        assert_eq!(
            resolve(&base, "/download/{accountId}/{blobId}/{name}?accept={type}").unwrap(),
            "https://jmap.example.com/download/{accountId}/{blobId}/{name}?accept={type}"
        );
    }

    #[test]
    fn the_secret_is_not_in_debug_output() {
        let basic = format!(
            "{:?}",
            Auth::Basic {
                username: "me".to_owned(),
                password: "hunter2".to_owned()
            }
        );
        assert!(!basic.contains("hunter2"), "{basic}");
        assert!(!format!("{:?}", Auth::Bearer("tok".to_owned())).contains("tok"));
    }
}
