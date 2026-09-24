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
//!
//! ## Past 4 MB
//!
//! `sendMail` refuses a request over 4 MB, so a larger message goes the long way Graph
//! documents for large attachments:
//!
//! 1. `POST /me/messages` creates a draft from the message's parts ([`mail_mime::graph_draft`]),
//!    with as many of the small attachments as fit in that one request;
//! 2. every other attachment of 3 MB or less is `POST`ed to the draft's `attachments`, and every
//!    larger one goes through an upload session: `createUploadSession`, then `PUT`s of
//!    [`CHUNK`] bytes to the pre-authenticated `uploadUrl`, which is never shown the token;
//! 3. `POST /me/messages/{id}/send`.
//!
//! A failure anywhere after the draft exists deletes it, so the user's Drafts folder does not
//! collect half-built copies of a message the outbox will try again. Creating a draft needs
//! `Mail.ReadWrite` as well as `Mail.Send`; a sign-in from before this asked for it gets a `403`
//! that says so.
//!
//! The draft carries what Graph's message resource can hold, which is less than MIME: `Date` is
//! not sent (Graph stamps its own `sentDateTime` as it sends, the moment [`mail_mime::restamp`]
//! would have written anyway), only `X-` fields survive as `internetMessageHeaders`, and
//! `In-Reply-To`/`References` — which Graph has no property for — are set as the MAPI
//! properties Exchange writes those headers from (`PidTagInReplyToId`, 0x1042, and
//! `PidTagInternetReferences`, 0x1039) through `singleValueExtendedProperties`.

use crate::RuntimeError;
use base64::Engine as _;
use mail_domain::{Address, Inline, Retry};
use mail_mime::{GraphBody, GraphDraft, GraphImportance, ParsedPart};
use serde_json::{Value, json};
use std::time::Duration;

/// Graph's `sendMail` for the signed-in user.
pub const SEND_MAIL: &str = "https://graph.microsoft.com/v1.0/me/sendMail";

/// Graph refuses a request body over 4 MB, and base64 makes the message a third larger.
///
/// A message whose encoding fits goes in one `sendMail`; a larger one is built as a draft.
pub const LIMIT: usize = 4 * 1024 * 1024;

/// The largest attachment Graph takes in one `POST`; a larger one needs an upload session.
pub const SMALL_ATTACHMENT: usize = 3 * 1024 * 1024;

/// The most an upload session takes, and so the largest message this can send.
pub const MAX_MESSAGE: u64 = 150 * 1024 * 1024;

/// One `PUT` of an upload session.
///
/// Graph wants byte ranges in multiples of 320 KiB, and Outlook's upload sessions at most 4 MB
/// each: twelve of them is the largest range that is both.
pub const CHUNK: usize = 12 * 320 * 1024;

/// How long one chunk may take. The client's own timeout is for small requests, and a slow
/// uplink needs more than that for [`CHUNK`] bytes.
const CHUNK_TIMEOUT: Duration = Duration::from_secs(300);

/// The sizes that decide how a message is sent. [`Limits::GRAPH`] is Graph's own; a test
/// passes smaller ones, so that crossing them does not take a hundred megabytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The largest request body: `sendMail`'s encoded MIME, or the draft's JSON.
    pub request: usize,
    /// The largest attachment posted whole; anything larger goes through an upload session.
    pub small_attachment: usize,
    /// The largest message, body and attachments together.
    pub message: u64,
    /// One upload `PUT`.
    pub chunk: usize,
}

impl Limits {
    pub const GRAPH: Limits = Limits {
        request: LIMIT,
        small_attachment: SMALL_ATTACHMENT,
        message: MAX_MESSAGE,
        chunk: CHUNK,
    };
}

/// Submit `mime` for delivery to `rcpt_to`, through `url` (normally [`SEND_MAIL`]).
pub async fn send_mime(
    http: &reqwest::Client,
    url: &str,
    access_token: &str,
    mime: &[u8],
    rcpt_to: &[String],
) -> Result<(), RuntimeError> {
    send_mime_within(http, url, access_token, mime, rcpt_to, &Limits::GRAPH).await
}

/// [`send_mime`], with `limits` in place of Graph's.
///
/// The draft path's endpoints are found beside `url`: `…/me/sendMail` becomes `…/me/messages`.
pub async fn send_mime_within(
    http: &reqwest::Client,
    url: &str,
    access_token: &str,
    mime: &[u8],
    rcpt_to: &[String],
    limits: &Limits,
) -> Result<(), RuntimeError> {
    let blind = with_blind_copies(mime, rcpt_to)?;
    let body = base64::engine::general_purpose::STANDARD.encode(&blind);
    if body.len() > limits.request {
        let me = url.strip_suffix("/sendMail").unwrap_or(url);
        return send_as_draft(http, me, access_token, mime, rcpt_to, limits).await;
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
    let after = retry_after(&response);
    let text = response.text().await.unwrap_or_default();
    Err(refusal(status.as_u16(), &text, after, Asked::SendMail))
}

/// The long way: a draft, its attachments, and a send. See the module's documentation.
async fn send_as_draft(
    http: &reqwest::Client,
    me: &str,
    token: &str,
    mime: &[u8],
    rcpt_to: &[String],
    limits: &Limits,
) -> Result<(), RuntimeError> {
    let draft = mail_mime::graph_draft(mime, rcpt_to)?;
    let size = draft.size();
    if size > limits.message {
        return Err(too_large(format!(
            "the message is {} MB, and Microsoft Graph sends at most {} MB",
            size / (1024 * 1024),
            limits.message / (1024 * 1024)
        )));
    }

    // As many small attachments as fit go in the request that creates the draft; the rest of
    // the small ones are posted one by one, and the large ones uploaded.
    let mut message = message_json(&draft);
    let mut used = json_len(&message);
    if used > limits.request {
        return Err(too_large(format!(
            "its text alone is {} MB, and Microsoft Graph takes at most {} MB in one request",
            used / (1024 * 1024),
            limits.request / (1024 * 1024)
        )));
    }
    let mut packed = Vec::new();
    let mut posted = Vec::new();
    let mut uploaded = Vec::new();
    for part in &draft.attachments {
        if part.bytes.len() > limits.small_attachment {
            uploaded.push(part);
            continue;
        }
        let value = attachment_json(part);
        let len = json_len(&value) + 1;
        if used + len <= limits.request {
            used += len;
            packed.push(value);
        } else {
            posted.push(value);
        }
    }
    if !packed.is_empty() {
        message["attachments"] = Value::Array(packed);
    }

    let messages = format!("{me}/messages");
    let created = call(
        http,
        reqwest::Method::POST,
        &messages,
        token,
        Some(&message),
    )
    .await?;
    let Some(id) = created.get("id").and_then(Value::as_str).map(segment) else {
        return Err(RuntimeError::Graph {
            why: "Microsoft Graph created the draft and did not say what it is called".to_owned(),
            retry: Retry::After(Duration::from_secs(60)),
        });
    };
    let draft_url = format!("{messages}/{id}");

    let finished = async {
        let attachments = format!("{draft_url}/attachments");
        for value in &posted {
            call(
                http,
                reqwest::Method::POST,
                &attachments,
                token,
                Some(value),
            )
            .await?;
        }
        for part in &uploaded {
            upload(http, &draft_url, token, part, limits.chunk).await?;
        }
        let send = format!("{draft_url}/send");
        call(http, reqwest::Method::POST, &send, token, None).await?;
        Ok::<_, RuntimeError>(())
    }
    .await;
    if let Err(failed) = finished {
        // Best effort, and its own failure is not the one worth reporting. After a `send` that
        // failed only on the way back, the draft has moved to Sent Items under another id, and
        // this deletes nothing.
        let _ = call(http, reqwest::Method::DELETE, &draft_url, token, None).await;
        return Err(failed);
    }
    Ok(())
}

/// One authenticated Graph request; its JSON answer, or `Null` when it has none.
async fn call(
    http: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    token: &str,
    body: Option<&Value>,
) -> Result<Value, RuntimeError> {
    let request = http.request(method, url).bearer_auth(token);
    let request = match body {
        Some(body) => request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(body).unwrap_or_default()),
        // `send` has no body, and Graph refuses a POST without a length (`411`).
        None => request.body(Vec::new()),
    };
    let response = request
        .send()
        .await
        .map_err(|e| RuntimeError::Connect(format!("Microsoft Graph: {e}")))?;
    let status = response.status();
    if status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Ok(serde_json::from_str(&text).unwrap_or(Value::Null));
    }
    let after = retry_after(&response);
    let text = response.text().await.unwrap_or_default();
    Err(refusal(status.as_u16(), &text, after, Asked::Draft))
}

/// Upload one attachment to the draft at `draft_url`, `chunk` bytes at a time.
async fn upload(
    http: &reqwest::Client,
    draft_url: &str,
    token: &str,
    part: &ParsedPart,
    chunk: usize,
) -> Result<(), RuntimeError> {
    let total = part.bytes.len();
    let mut item = json!({
        "attachmentType": "file",
        "name": part.name,
        "size": total,
        "contentType": part.mime,
        "isInline": matches!(part.inline, Inline::Embedded { .. }),
    });
    if let Inline::Embedded { cid } = &part.inline {
        item["contentId"] = json!(cid);
    }
    let session = call(
        http,
        reqwest::Method::POST,
        &format!("{draft_url}/attachments/createUploadSession"),
        token,
        Some(&json!({ "AttachmentItem": item })),
    )
    .await?;
    let Some(upload_url) = session.get("uploadUrl").and_then(Value::as_str) else {
        return Err(RuntimeError::Graph {
            why: "Microsoft Graph opened an upload session and gave no address for it".to_owned(),
            retry: Retry::After(Duration::from_secs(60)),
        });
    };
    // The address carries its own authorization; a plain-HTTP one handed out by an HTTPS Graph
    // would send the attachment, and that authorization, in the clear.
    if draft_url.starts_with("https://") && !upload_url.starts_with("https://") {
        return Err(RuntimeError::Graph {
            why: format!("Microsoft Graph gave an upload address that is not HTTPS: {upload_url}"),
            retry: Retry::Fatal("the upload address is not HTTPS".to_owned()),
        });
    }

    // Not the caller's client: nothing here may carry the token, and a redirect away from the
    // pre-authenticated address is not somewhere the attachment should follow.
    let put = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(CHUNK_TIMEOUT)
        .build()
        .map_err(|e| RuntimeError::Connect(format!("cannot build an HTTP client: {e}")))?;
    let chunk = chunk.max(1);
    let mut offset = next_offset(&session).unwrap_or(0);
    // Each answer names the next range; a server that never moves on must not keep this here.
    let mut rounds = 2 * total.div_ceil(chunk) + 4;
    loop {
        if offset >= total || rounds == 0 {
            return Err(RuntimeError::Graph {
                why: format!(
                    "Microsoft Graph kept asking for more of {} than there is ({offset} of \
                     {total} bytes)",
                    part.name
                ),
                retry: Retry::After(Duration::from_secs(60)),
            });
        }
        rounds -= 1;
        let end = (offset + chunk).min(total);
        let response = put
            .put(upload_url)
            .header(
                reqwest::header::CONTENT_RANGE,
                format!("bytes {offset}-{}/{total}", end - 1),
            )
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(part.bytes[offset..end].to_vec())
            .send()
            .await
            .map_err(|e| RuntimeError::Connect(format!("Microsoft Graph upload: {e}")))?;
        match response.status().as_u16() {
            200 | 201 => return Ok(()),
            202 => {
                let text = response.text().await.unwrap_or_default();
                let answer = serde_json::from_str(&text).unwrap_or(Value::Null);
                offset = next_offset(&answer).unwrap_or(end);
            }
            status => {
                let after = retry_after(&response);
                let text = response.text().await.unwrap_or_default();
                return Err(refusal(status, &text, after, Asked::Upload));
            }
        }
    }
}

/// Where an upload session wants the next bytes: the start of its first expected range.
///
/// `{"nextExpectedRanges": ["2097152-"]}`, or `["2097152-4194303"]`.
fn next_offset(answer: &Value) -> Option<usize> {
    let first = answer
        .get("nextExpectedRanges")?
        .as_array()?
        .first()?
        .as_str()?;
    first.split('-').next()?.trim().parse().ok()
}

/// Graph's message resource for `draft`, attachments aside.
fn message_json(draft: &GraphDraft) -> Value {
    let (kind, content) = match &draft.body {
        GraphBody::Html(html) => ("html", html),
        GraphBody::Text(text) => ("text", text),
    };
    let mut message = json!({
        "subject": draft.subject,
        "body": { "contentType": kind, "content": content },
        "toRecipients": recipients(&draft.to),
        "ccRecipients": recipients(&draft.cc),
        "bccRecipients": recipients(&draft.bcc),
    });
    if let Some(from) = &draft.from {
        message["from"] = recipient(from);
    }
    if !draft.reply_to.is_empty() {
        message["replyTo"] = recipients(&draft.reply_to);
    }
    if let Some(importance) = draft.importance {
        message["importance"] = json!(match importance {
            GraphImportance::Low => "low",
            GraphImportance::Normal => "normal",
            GraphImportance::High => "high",
        });
    }
    if draft.read_receipt {
        message["isReadReceiptRequested"] = json!(true);
    }
    // Writable while the message is a draft, and the id the local copy is threaded by.
    if let Some(id) = &draft.message_id {
        message["internetMessageId"] = json!(id);
    }
    if !draft.custom_headers.is_empty() {
        message["internetMessageHeaders"] = draft
            .custom_headers
            .iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect();
    }
    let threading: Vec<Value> = [
        ("String 0x1042", &draft.in_reply_to),
        ("String 0x1039", &draft.references),
    ]
    .into_iter()
    .filter_map(|(id, value)| Some(json!({ "id": id, "value": value.as_ref()? })))
    .collect();
    if !threading.is_empty() {
        message["singleValueExtendedProperties"] = Value::Array(threading);
    }
    message
}

fn attachment_json(part: &ParsedPart) -> Value {
    let mut value = json!({
        "@odata.type": "#microsoft.graph.fileAttachment",
        "name": part.name,
        "contentType": part.mime,
        "contentBytes": base64::engine::general_purpose::STANDARD.encode(&part.bytes),
        "isInline": matches!(part.inline, Inline::Embedded { .. }),
    });
    if let Inline::Embedded { cid } = &part.inline {
        value["contentId"] = json!(cid);
    }
    value
}

fn recipients(addresses: &[Address]) -> Value {
    addresses.iter().map(recipient).collect()
}

fn recipient(address: &Address) -> Value {
    match &address.name {
        Some(name) => json!({ "emailAddress": { "name": name, "address": address.email } }),
        None => json!({ "emailAddress": { "address": address.email } }),
    }
}

fn json_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

/// A Graph id as one path segment. Ids are base64, so `/` and `+` can occur in them.
fn segment(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~=".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn too_large(why: String) -> RuntimeError {
    RuntimeError::Graph {
        why: format!("{why}. Send it with a smaller attachment, or share the file by link."),
        retry: Retry::Fatal("too large for Graph".to_owned()),
    }
}

fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// Which request was refused, which decides what a `401` or `403` means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    /// `sendMail`, which needs `Mail.Send`.
    SendMail,
    /// A draft, its attachments, or its send, which need `Mail.ReadWrite` as well.
    Draft,
    /// A `PUT` to an upload session's own address, which is not shown the token.
    Upload,
}

/// What a refusal means, from its status and Graph's error body.
fn refusal(status: u16, body: &str, after: Option<Duration>, asked: Asked) -> RuntimeError {
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
        // The upload address authorizes itself; refused, the session has lapsed, and the next
        // attempt opens another.
        401 | 403 if asked == Asked::Upload => Retry::After(Duration::from_secs(60)),
        // The token was not accepted: expired between the check and the call, or revoked.
        401 => Retry::NeedsReauth,
        // Allowed to send, not to write: a sign-in from before the draft path asked for it.
        403 if asked == Asked::Draft => Retry::Fatal(
            "the sign-in does not carry Graph's Mail.ReadWrite permission, which a message over \
             4 MB needs besides Mail.Send: it is built as a draft before it is sent. Sign-ins \
             made before mailo asked for it lack it; re-run `mailo account add … --microsoft \
             --send graph` and accept it, or ask an administrator to consent to it"
                .to_owned(),
        ),
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
    fn a_reply_keeps_its_threading_where_graph_has_no_property_for_it() {
        let draft = mail_mime::graph_draft(
            b"From: me@contoso.example\r\nTo: ada@example.test\r\nSubject: re: s\r\n\
              In-Reply-To: <Parent@example.test>\r\n\
              References: <Root@example.test> <Parent@example.test>\r\n\
              Importance: low\r\n\r\nyes\r\n",
            &["ada@example.test".to_owned()],
        )
        .unwrap();
        let message = message_json(&draft);
        assert_eq!(
            message["singleValueExtendedProperties"],
            json!([
                { "id": "String 0x1042", "value": "<Parent@example.test>" },
                { "id": "String 0x1039", "value": "<Root@example.test> <Parent@example.test>" },
            ])
        );
        assert_eq!(message["importance"], "low");
        assert_eq!(message["body"]["contentType"], "text");
        assert!(message.get("internetMessageHeaders").is_none());
    }

    #[test]
    fn a_graph_id_is_one_path_segment() {
        assert_eq!(segment("AAMk/a+b=="), "AAMk%2Fa%2Bb==");
    }

    #[test]
    fn an_upload_goes_on_from_the_first_range_graph_expects() {
        assert_eq!(
            next_offset(&json!({ "nextExpectedRanges": ["2097152-", "5000000-6000000"] })),
            Some(2_097_152)
        );
        assert_eq!(next_offset(&json!({})), None);
    }

    #[test]
    fn refusals_say_what_to_do() {
        let body = r#"{"error":{"code":"ErrorAccessDenied","message":"Access is denied."}}"#;
        let RuntimeError::Graph { why, retry } = refusal(403, body, None, Asked::SendMail) else {
            panic!()
        };
        assert!(
            why.contains("ErrorAccessDenied: Access is denied."),
            "{why}"
        );
        assert!(matches!(retry, Retry::Fatal(ref w) if w.contains("Mail.Send")));
        let RuntimeError::Graph { retry, .. } = refusal(401, "", None, Asked::SendMail) else {
            panic!()
        };
        assert_eq!(retry, Retry::NeedsReauth);
        let RuntimeError::Graph { retry, .. } =
            refusal(429, "", Some(Duration::from_secs(7)), Asked::SendMail)
        else {
            panic!()
        };
        assert_eq!(retry, Retry::After(Duration::from_secs(7)));
    }
}
