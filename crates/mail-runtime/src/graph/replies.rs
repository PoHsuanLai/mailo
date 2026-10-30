//! A Microsoft Graph mailbox's automatic replies, set: one `PATCH` of `/me/mailboxSettings`
//! with the body [`mail_proto::automatic_replies`] builds. Graph answers `200` with the settings
//! as they stand.
//!
//! It needs `MailboxSettings.ReadWrite`, which a sign-in asks for since mailo sets the reply
//! (`mail_domain::presets::GRAPH_SCOPES`). One made before lacks it, and Graph's `403` is said
//! as that, with the way out: signing in again.

use crate::{AccountSecrets, ClientRegistry, RuntimeError};
use chrono::{DateTime, Utc};
use mail_domain::{AccountPlan, AuthPlan, Retry, Vacation};
use mail_net::Failure;
use mail_proto::automatic_replies::MAILBOX_SETTINGS;
use porter_core::{AccountId, Credential};
use serde_json::Value;
use std::time::Duration;

/// Make the mailbox's automatic replies the reply `vacation` is, or switch them off for `None`,
/// signed in as the account's mail is: its Graph token fetched or refreshed through `secrets`,
/// with the OAuth client `saved` holds for its issuer.
pub async fn set(
    account: AccountId,
    plan: &AccountPlan,
    vacation: Option<&Vacation>,
    saved: &ClientRegistry,
    secrets: &dyn AccountSecrets,
    now: DateTime<Utc>,
) -> Result<(), RuntimeError> {
    let AuthPlan::OAuth { issuer, .. } = &plan.auth else {
        return Err(RuntimeError::Secrets(Failure::said(
            "an account that reads through Microsoft Graph signs in with Microsoft",
        )));
    };
    let client = crate::clients::client(saved, *issuer).ok_or_else(|| {
        RuntimeError::Secrets(Failure::said(
            "no OAuth client id is configured, so Microsoft Graph cannot be signed in to",
        ))
    })?;
    let reach = crate::tokens::GraphReach::of(plan);
    let token = crate::tokens::graph_token(account, client, reach, secrets, now).await?;
    let Credential::OAuth { access, .. } = token else {
        return Err(RuntimeError::Secrets(Failure::said(
            "the Microsoft Graph credential kept for this account is not a token",
        )));
    };
    let http = mail_net::http::http_client()?;
    let body = mail_proto::automatic_replies::automatic_replies(vacation, now);
    set_automatic_replies(&http, access.expose(), &body).await
}

/// Make the mailbox's automatic replies what `body` says.
pub async fn set_automatic_replies(
    http: &reqwest::Client,
    access_token: &str,
    body: &Value,
) -> Result<(), RuntimeError> {
    patch_settings(http, MAILBOX_SETTINGS, access_token, body).await
}

/// The `PATCH` of the settings at `url`.
async fn patch_settings(
    http: &reqwest::Client,
    url: &str,
    access_token: &str,
    body: &Value,
) -> Result<(), RuntimeError> {
    let response = http
        .patch(url)
        .bearer_auth(access_token)
        .json(body)
        .send()
        .await
        .map_err(|e| RuntimeError::Connect(Failure::new("Microsoft Graph", e)))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let after = super::retry_after(&response);
    let text = super::refusal_text(response).await;
    Err(refused(status.as_u16(), &text, after))
}

/// What a refusal of the settings means, from its status and Graph's error body.
fn refused(status: u16, body: &str, after: Option<Duration>) -> RuntimeError {
    let why = format!(
        "Microsoft Graph did not change the automatic replies ({status}): {}",
        super::detail(body)
    );
    let retry = match status {
        401 => Retry::NeedsReauth,
        403 => Retry::Fatal(
            "the sign-in does not carry Graph's MailboxSettings.ReadWrite permission, which \
             setting the vacation reply needs. Sign-ins made before mailo asked for it lack it: \
             sign in to the account again and accept it, or ask an administrator to consent to it"
                .to_owned(),
        ),
        429 | 503 | 504 => Retry::After(after.unwrap_or(Duration::from_secs(60))),
        500..=599 => Retry::After(Duration::from_secs(60)),
        _ => Retry::Fatal(format!("Graph answered {status}")),
    };
    RuntimeError::Graph { why, retry }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::Retryable;

    #[test]
    fn a_refusal_says_what_to_do_about_it() {
        let denied = r#"{"error":{"code":"ErrorAccessDenied","message":"Access is denied."}}"#;
        let cases = [
            (401, Some(Retry::NeedsReauth)),
            (429, Some(Retry::After(Duration::from_secs(60)))),
            (503, Some(Retry::After(Duration::from_secs(60)))),
        ];
        for (status, retry) in cases {
            assert_eq!(
                Some(refused(status, denied, None).retry()),
                retry,
                "{status}"
            );
        }
        let forbidden = refused(403, denied, None);
        assert!(
            matches!(forbidden.retry(), Retry::Fatal(why) if why.contains("MailboxSettings.ReadWrite")),
            "{forbidden:?}"
        );
        assert!(
            forbidden.to_string().contains("ErrorAccessDenied"),
            "{forbidden}"
        );
    }

    /// Answer one request on a local listener with `status`, and hand back the request as the
    /// server read it: its head, then its body.
    async fn served(
        status: &'static str,
        body: Value,
    ) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/me/mailboxSettings",
            listener.local_addr().unwrap()
        );
        let task = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            let wanted = loop {
                let n = sock.read(&mut buf).await.unwrap();
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen).into_owned();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|n| n.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if seen.len() >= end + 4 + length || n == 0 {
                        break text;
                    }
                }
                assert!(n > 0, "the request ended before its head did");
            };
            let reply = body.to_string();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            sock.write_all(response.as_bytes()).await.unwrap();
            wanted
        });
        (url, task)
    }

    #[tokio::test]
    async fn the_settings_are_patched_with_the_token_and_the_body() {
        let (url, request) = served("200 OK", serde_json::json!({})).await;
        let body = serde_json::json!({ "automaticRepliesSetting": { "status": "disabled" } });
        patch_settings(
            &mail_net::http::http_client().unwrap(),
            &url,
            "token-1",
            &body,
        )
        .await
        .unwrap();
        let request = request.await.unwrap();
        assert!(
            request.starts_with("PATCH /me/mailboxSettings "),
            "{request}"
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer token-1"),
            "{request}"
        );
        assert!(
            request.ends_with(r#"{"automaticRepliesSetting":{"status":"disabled"}}"#),
            "{request}"
        );
    }

    #[tokio::test]
    async fn a_forbidden_patch_says_the_permission_is_missing() {
        let denied = serde_json::json!({ "error": { "code": "ErrorAccessDenied", "message": "Access is denied." } });
        let (url, _request) = served("403 Forbidden", denied).await;
        let err = patch_settings(
            &mail_net::http::http_client().unwrap(),
            &url,
            "token-1",
            &serde_json::json!({}),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err.retry(), Retry::Fatal(why) if why.contains("MailboxSettings.ReadWrite")),
            "{err:?}"
        );
    }
}
