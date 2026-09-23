//! Sending through Microsoft Graph, for a Microsoft 365 tenant that refuses SMTP AUTH.
//!
//! One call: `POST /me/sendMail` with the finished message as base64 MIME. Graph files the copy
//! in Sent Items itself and answers `202 Accepted` with no body. Nothing about the message is
//! rebuilt here — the bytes are the ones frozen when the user pressed send, as for SMTP.
//!
//! What SMTP carries in the envelope, MIME has to carry in the message: Graph takes the
//! recipients from the headers. A blind-copied recipient is in the envelope and, by design, not
//! in the headers, so one is added as a `Bcc:` header on the copy handed to Graph, which Exchange
//! removes before delivery exactly as it does for any message submitted with one.

use crate::RuntimeError;
use base64::Engine as _;
use mail_domain::Retry;
use std::time::Duration;

/// Graph's `sendMail` for the signed-in user.
pub const SEND_MAIL: &str = "https://graph.microsoft.com/v1.0/me/sendMail";

/// Graph refuses a request body over 4 MB, and base64 makes the message a third larger.
///
/// Larger messages need an upload session against a draft, which this does not do; saying so
/// beats a `413` the user cannot act on.
pub const LIMIT: usize = 4 * 1024 * 1024;

/// Submit `mime` for delivery to `rcpt_to`, through `url` (normally [`SEND_MAIL`]).
pub async fn send_mime(
    http: &reqwest::Client,
    url: &str,
    access_token: &str,
    mime: &[u8],
    rcpt_to: &[String],
) -> Result<(), RuntimeError> {
    let mime = with_blind_copies(mime, rcpt_to)?;
    let body = base64::engine::general_purpose::STANDARD.encode(&mime);
    if body.len() > LIMIT {
        return Err(RuntimeError::Graph {
            why: format!(
                "the message is {} MB once encoded, and Microsoft Graph accepts at most 4 MB \
                 in one request. Send it with a smaller attachment, or share the file by link.",
                body.len() / (1024 * 1024)
            ),
            retry: Retry::Fatal("too large for Graph".to_owned()),
        });
    }

    let response = http
        .post(url)
        .bearer_auth(access_token)
        .header(reqwest::header::CONTENT_TYPE, "text/plain")
        .body(body)
        .send()
        .await
        .map_err(|e| RuntimeError::Connect(format!("Microsoft Graph: {e}")))?;

    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let text = response.text().await.unwrap_or_default();
    Err(refusal(status.as_u16(), &text, after))
}

/// What a refusal means, from its status and Graph's error body.
fn refusal(status: u16, body: &str, after: Option<Duration>) -> RuntimeError {
    // `{"error":{"code":"ErrorAccessDenied","message":"Access is denied. …"}}`
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            let error = v.get("error")?;
            let code = error.get("code")?.as_str()?.to_owned();
            let message = error.get("message").and_then(|m| m.as_str()).unwrap_or("");
            Some(format!("{code}: {message}"))
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    let why = format!("Microsoft Graph refused the message ({status}): {detail}");
    let retry = match status {
        // The token was not accepted: expired between the check and the call, or revoked.
        401 => Retry::NeedsReauth,
        // Signed in, and not allowed: most often `Mail.Send` was never consented to.
        403 => Retry::Fatal(
            "the sign-in does not carry Graph's Mail.Send permission; re-run \
             `mailo account add … --microsoft --send graph` and accept it, or ask an \
             administrator to consent to it"
                .to_owned(),
        ),
        429 | 503 | 504 => Retry::After(after.unwrap_or(Duration::from_secs(60))),
        500..=599 => Retry::After(Duration::from_secs(60)),
        _ => Retry::Fatal(format!("Graph answered {status}")),
    };
    RuntimeError::Graph { why, retry }
}

/// `mime` with a `Bcc:` header naming every envelope recipient its `To` and `Cc` do not.
fn with_blind_copies(mime: &[u8], rcpt_to: &[String]) -> Result<Vec<u8>, RuntimeError> {
    let parsed = mail_mime::parse(mime)?;
    let named: Vec<String> = parsed
        .to
        .iter()
        .chain(&parsed.cc)
        .chain(&parsed.bcc)
        .map(|a| a.email.to_lowercase())
        .collect();
    let blind: Vec<&str> = rcpt_to
        .iter()
        .filter(|r| !named.contains(&r.to_lowercase()))
        .map(String::as_str)
        .collect();
    if blind.is_empty() {
        return Ok(mime.to_vec());
    }
    let mut out = format!("Bcc: {}\r\n", blind.join(", ")).into_bytes();
    out.extend_from_slice(mime);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIME: &[u8] = b"From: me@contoso.example\r\nTo: Ada <ada@example.test>\r\n\
        Cc: bob@example.test\r\nSubject: s\r\n\r\nhello\r\n";

    #[test]
    fn a_blind_copy_is_carried_as_a_header_because_graph_reads_no_envelope() {
        let out = with_blind_copies(
            MIME,
            &[
                "ada@example.test".to_owned(),
                "BOB@example.test".to_owned(),
                "eve@example.test".to_owned(),
            ],
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("Bcc: eve@example.test\r\nFrom:"), "{text}");
    }

    #[test]
    fn a_message_with_no_blind_copies_goes_as_it_is() {
        let out = with_blind_copies(
            MIME,
            &["ada@example.test".to_owned(), "bob@example.test".to_owned()],
        )
        .unwrap();
        assert_eq!(out, MIME);
    }

    #[test]
    fn refusals_say_what_to_do() {
        let body = r#"{"error":{"code":"ErrorAccessDenied","message":"Access is denied."}}"#;
        let RuntimeError::Graph { why, retry } = refusal(403, body, None) else {
            panic!()
        };
        assert!(
            why.contains("ErrorAccessDenied: Access is denied."),
            "{why}"
        );
        assert!(matches!(retry, Retry::Fatal(ref w) if w.contains("Mail.Send")));
        let RuntimeError::Graph { retry, .. } = refusal(401, "", None) else {
            panic!()
        };
        assert_eq!(retry, Retry::NeedsReauth);
        let RuntimeError::Graph { retry, .. } = refusal(429, "", Some(Duration::from_secs(7)))
        else {
            panic!()
        };
        assert_eq!(retry, Retry::After(Duration::from_secs(7)));
    }
}
