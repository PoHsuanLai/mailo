//! A Microsoft Graph mailbox's automatic replies, set: one `PATCH` of `/me/mailboxSettings`
//! with the body [`mail_proto::automatic_replies`] builds. Graph answers `200` with the settings
//! as they stand.
//!
//! It needs `MailboxSettings.ReadWrite`, which a sign-in asks for since mailo sets the reply
//! (`mail_domain::presets::GRAPH_SCOPES`). One made before lacks it, and Graph's `403` is said
//! as that, with the way out: signing in again.

use crate::RuntimeError;
use crate::error::Failure;
use mail_domain::Retry;
use mail_proto::automatic_replies::MAILBOX_SETTINGS;
use serde_json::Value;
use std::time::Duration;

/// Make the mailbox's automatic replies what `body` says.
pub async fn set_automatic_replies(
    http: &reqwest::Client,
    access_token: &str,
    body: &Value,
) -> Result<(), RuntimeError> {
    let response = http
        .patch(MAILBOX_SETTINGS)
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
}
