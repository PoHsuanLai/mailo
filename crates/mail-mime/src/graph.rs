//! A frozen message, taken apart into what Microsoft Graph's message resource can hold.
//!
//! Graph's `sendMail` takes MIME, but only up to 4 MB in one request. Past that, a message has
//! to be created as a draft from its parts, its attachments uploaded one by one, and the draft
//! sent. The parts come from the bytes frozen when the user pressed Send — the same bytes SMTP
//! would carry — so what goes out through either path is one message. This is the pure half: it
//! reads bytes and says what the draft should be. `mail-runtime` turns that into requests.
//!
//! What does not survive: `Date` (Graph stamps its own `sentDateTime`), any header that is
//! neither one of the properties below nor an `X-` field (Graph refuses other names in
//! `internetMessageHeaders`), and the MIME structure itself — a `text/calendar` reply, say,
//! becomes an ordinary `.ics` attachment.

use crate::{MimeError, ParsedPart};
use mail_domain::Address;
use mail_parser::MessageParser;

/// The body Graph shows. One of the two: a message resource has a single `body`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphBody {
    Html(String),
    Text(String),
}

/// Graph's `importance`, from `Importance:` or, failing that, `X-Priority:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphImportance {
    Low,
    Normal,
    High,
}

/// Everything Graph's message resource is told about one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphDraft {
    pub subject: String,
    pub body: GraphBody,
    pub from: Option<Address>,
    pub reply_to: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    /// The message's own `Bcc:` and every envelope recipient `To`/`Cc` do not name. Graph reads
    /// no envelope, so a blind copy it is not told about is never delivered.
    pub bcc: Vec<Address>,
    /// `Message-ID`, angle brackets included, as the bytes spell it.
    pub message_id: Option<String>,
    /// `In-Reply-To`, as the bytes spell it. Graph has no property for it; see the runtime.
    pub in_reply_to: Option<String>,
    /// `References`, as the bytes spell it, unfolded.
    pub references: Option<String>,
    pub importance: Option<GraphImportance>,
    /// `Disposition-Notification-To` was present: Graph's `isReadReceiptRequested`.
    pub read_receipt: bool,
    /// `X-` fields, name and unfolded value, in the order they appear.
    pub custom_headers: Vec<(String, String)>,
    /// Every attachment, and every image the HTML references by `cid:`.
    pub attachments: Vec<ParsedPart>,
}

impl GraphDraft {
    /// Bytes of body and attachments together: what Graph counts against its size limit.
    pub fn size(&self) -> u64 {
        let body = match &self.body {
            GraphBody::Html(s) | GraphBody::Text(s) => s.len(),
        };
        self.attachments
            .iter()
            .fold(body as u64, |sum, part| sum + part.bytes.len() as u64)
    }
}

/// Take `message` apart for Graph, with every one of `rcpt_to` still a recipient.
pub fn graph_draft(message: &[u8], rcpt_to: &[String]) -> Result<GraphDraft, MimeError> {
    let parsed = crate::parse(message)?;
    let raw = MessageParser::default()
        .parse(message)
        .ok_or_else(|| MimeError::Unparseable("no RFC 5322 header was found".to_owned()))?;
    let field = |name: &str| {
        raw.headers_raw()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| unfold(v))
            .filter(|v| !v.is_empty())
    };

    let named: Vec<String> = parsed
        .to
        .iter()
        .chain(&parsed.cc)
        .chain(&parsed.bcc)
        .map(|a| a.email.to_lowercase())
        .collect();
    let mut bcc = parsed.bcc.clone();
    for rcpt in rcpt_to {
        let lower = rcpt.to_lowercase();
        if !named.contains(&lower) && !bcc.iter().any(|a| a.email.to_lowercase() == lower) {
            bcc.push(Address {
                name: None,
                email: rcpt.clone(),
            });
        }
    }

    let importance = field("Importance")
        .and_then(|v| match v.to_ascii_lowercase().as_str() {
            "high" => Some(GraphImportance::High),
            "low" => Some(GraphImportance::Low),
            "normal" => Some(GraphImportance::Normal),
            _ => None,
        })
        .or_else(|| {
            // `X-Priority: 1 (Highest)` — only the leading digit means anything.
            match field("X-Priority")?.trim_start().chars().next()? {
                '1' | '2' => Some(GraphImportance::High),
                '3' => Some(GraphImportance::Normal),
                '4' | '5' => Some(GraphImportance::Low),
                _ => None,
            }
        });

    let custom_headers = raw
        .headers_raw()
        .filter(|(name, _)| name.len() > 2 && name[..2].eq_ignore_ascii_case("x-"))
        .map(|(name, value)| (name.to_owned(), unfold(value)))
        // Graph takes header values as ASCII text; an 8-bit one would be refused whole.
        .filter(|(_, value)| value.is_ascii() && !value.is_empty())
        .collect();

    let body = match (parsed.html, parsed.text) {
        (Some(html), _) => GraphBody::Html(html),
        (None, text) => GraphBody::Text(text.unwrap_or_default()),
    };

    Ok(GraphDraft {
        subject: parsed.subject,
        body,
        from: parsed.from,
        reply_to: parsed.reply_to,
        to: parsed.to,
        cc: parsed.cc,
        bcc,
        message_id: field("Message-ID"),
        in_reply_to: field("In-Reply-To"),
        references: field("References"),
        importance,
        read_receipt: field("Disposition-Notification-To").is_some(),
        custom_headers,
        attachments: parsed.attachments,
    })
}

/// A header value with its folding removed and its ends trimmed.
fn unfold(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for line in value.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(line.trim());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::Inline;

    const RELATED: &[u8] = b"From: Me <me@example.test>\r\n\
To: Ada <ada@example.test>\r\n\
Cc: bob@example.test\r\n\
Subject: the plan\r\n\
Date: Tue, 14 Nov 2023 22:13:20 +0000\r\n\
Message-ID: <Mixed.Case@example.test>\r\n\
In-Reply-To: <Parent@example.test>\r\n\
References: <Root@example.test>\r\n <Parent@example.test>\r\n\
Importance: High\r\n\
Disposition-Notification-To: me@example.test\r\n\
X-Mailer: mailo\r\n\
X-Folded: one\r\n two\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"b\"\r\n\
\r\n\
--b\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>see <img src=\"cid:logo@here\"></p>\r\n\
--b\r\n\
Content-Type: image/png\r\n\
Content-ID: <logo@here>\r\n\
Content-Disposition: inline; filename=\"logo.png\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0K\r\n\
--b--\r\n";

    #[test]
    fn a_message_comes_apart_into_what_graph_can_hold() {
        let draft = graph_draft(
            RELATED,
            &[
                "ada@example.test".to_owned(),
                "BOB@example.test".to_owned(),
                "eve@example.test".to_owned(),
            ],
        )
        .unwrap();
        assert_eq!(draft.subject, "the plan");
        assert!(
            matches!(draft.body, GraphBody::Html(ref h) if h.contains("cid:logo@here")),
            "{:?}",
            draft.body
        );
        assert_eq!(draft.to[0].email, "ada@example.test");
        assert_eq!(draft.cc[0].email, "bob@example.test");
        assert_eq!(
            draft
                .bcc
                .iter()
                .map(|a| a.email.as_str())
                .collect::<Vec<_>>(),
            ["eve@example.test"],
            "the envelope's blind recipient, and nobody the headers already name"
        );
        assert_eq!(
            draft.message_id.as_deref(),
            Some("<Mixed.Case@example.test>")
        );
        assert_eq!(draft.in_reply_to.as_deref(), Some("<Parent@example.test>"));
        assert_eq!(
            draft.references.as_deref(),
            Some("<Root@example.test> <Parent@example.test>")
        );
        assert_eq!(draft.importance, Some(GraphImportance::High));
        assert!(draft.read_receipt);
        assert_eq!(
            draft.custom_headers,
            [
                ("X-Mailer".to_owned(), "mailo".to_owned()),
                ("X-Folded".to_owned(), "one two".to_owned()),
            ]
        );
        assert_eq!(draft.attachments.len(), 1);
        assert_eq!(
            draft.attachments[0].inline,
            Inline::Embedded {
                cid: "logo@here".to_owned()
            }
        );
        assert_eq!(draft.attachments[0].mime, "image/png");
    }

    #[test]
    fn a_plain_message_has_a_text_body_and_no_importance() {
        let draft = graph_draft(
            b"From: me@example.test\r\nTo: ada@example.test\r\nSubject: s\r\n\
              X-Priority: 5 (Lowest)\r\n\r\nhello\r\n",
            &["ada@example.test".to_owned()],
        )
        .unwrap();
        assert!(matches!(draft.body, GraphBody::Text(ref t) if t.starts_with("hello")));
        assert!(draft.bcc.is_empty());
        assert_eq!(draft.importance, Some(GraphImportance::Low));
        assert!(!draft.read_receipt);
        assert_eq!(draft.message_id, None);
    }
}
