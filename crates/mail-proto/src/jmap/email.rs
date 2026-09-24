//! Emails (RFC 8621 §4), as a summary: enough to list one, file it, and fetch its bytes later.

use super::Mailboxes;
use super::field::{date, malformed, opt_string, string, strings, true_keys, unsigned};
use crate::ProtoError;
use chrono::{DateTime, Utc};
use mail_domain::{Address, MailboxRole, ReadState, Star};
use serde_json::Value;

/// The properties a summary asks for.
///
/// `headers` is the one that matters most. It is every header field exactly as the message
/// carries it (RFC 8621 §4.1.2, the raw form), so the header block handed to `mail-mime` is the
/// message's own and not a rebuild from parsed values — which is how search, threading, receipts
/// and invitations see the same fields on a JMAP account as on any other. The parsed properties
/// beside it are what the server says, kept for a caller that wants them without parsing.
pub const SUMMARY: &[&str] = &[
    "id",
    "blobId",
    "threadId",
    "mailboxIds",
    "keywords",
    "size",
    "receivedAt",
    "messageId",
    "inReplyTo",
    "references",
    "from",
    "to",
    "cc",
    "bcc",
    "replyTo",
    "subject",
    "sentAt",
    "hasAttachment",
    "preview",
    "headers",
];

/// Whether the server thinks an email has attachments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HasAttachment {
    Yes,
    No,
}

/// One email as `Email/get` described it with [`SUMMARY`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailSummary {
    pub id: String,
    /// The raw RFC 5322 message, to download.
    pub blob_id: String,
    pub thread_id: Option<String>,
    pub mailbox_ids: Vec<String>,
    pub keywords: Vec<String>,
    pub size: u64,
    pub received_at: Option<DateTime<Utc>>,
    pub message_id: Vec<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub from: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub subject: Option<String>,
    pub sent_at: Option<DateTime<Utc>>,
    pub has_attachment: HasAttachment,
    pub preview: String,
    /// Every header field, name and raw value, in the order the message has them.
    pub headers: Vec<(String, String)>,
}

impl EmailSummary {
    /// Parse the `list` of an `Email/get` answer.
    pub fn parse_list(args: &Value) -> Result<Vec<EmailSummary>, ProtoError> {
        args.get("list")
            .and_then(Value::as_array)
            .ok_or_else(|| malformed("Email/get has no list"))?
            .iter()
            .map(EmailSummary::parse)
            .collect()
    }

    /// Parse one email. Only `id` is required: a server may omit what it was not asked for,
    /// and a caller asking for keywords alone gets a summary with everything else empty.
    pub fn parse(email: &Value) -> Result<EmailSummary, ProtoError> {
        Ok(EmailSummary {
            id: string(email, "id")?.to_owned(),
            blob_id: opt_string(email, "blobId")?.unwrap_or("").to_owned(),
            thread_id: opt_string(email, "threadId")?.map(str::to_owned),
            mailbox_ids: true_keys(email, "mailboxIds")?,
            keywords: true_keys(email, "keywords")?,
            size: unsigned(email, "size", 0)?,
            received_at: date(email, "receivedAt"),
            message_id: strings(email, "messageId")?,
            in_reply_to: strings(email, "inReplyTo")?,
            references: strings(email, "references")?,
            from: addresses(email, "from")?,
            to: addresses(email, "to")?,
            cc: addresses(email, "cc")?,
            bcc: addresses(email, "bcc")?,
            reply_to: addresses(email, "replyTo")?,
            subject: opt_string(email, "subject")?.map(str::to_owned),
            sent_at: date(email, "sentAt"),
            has_attachment: match email.get("hasAttachment").and_then(Value::as_bool) {
                Some(true) => HasAttachment::Yes,
                _ => HasAttachment::No,
            },
            preview: opt_string(email, "preview")?.unwrap_or("").to_owned(),
            headers: headers(email)?,
        })
    }

    /// Read or unread, from `$seen` (RFC 8621 §4.1.1). Keywords compare case-insensitively.
    pub fn read(&self) -> ReadState {
        if self.has_keyword("$seen") {
            ReadState::Read
        } else {
            ReadState::Unread
        }
    }

    /// Starred or not, from `$flagged`.
    pub fn star(&self) -> Star {
        if self.has_keyword("$flagged") {
            Star::Starred
        } else {
            Star::Unstarred
        }
    }

    fn has_keyword(&self, keyword: &str) -> bool {
        self.keywords
            .iter()
            .any(|k| k.eq_ignore_ascii_case(keyword))
    }

    /// The message's header block, as it arrived: each field's name, a colon, its raw value,
    /// and the blank line that ends the header section.
    ///
    /// The raw form starts after the colon and keeps its folding, so this is byte for byte the
    /// header section the message was stored with, for any field whose name and value are
    /// themselves exact — which RFC 8621 promises for the raw form.
    pub fn raw_headers(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, value) in &self.headers {
            out.extend_from_slice(name.as_bytes());
            out.push(b':');
            out.extend_from_slice(value.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        out
    }
}

fn addresses(email: &Value, key: &str) -> Result<Vec<Address>, ProtoError> {
    match email.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|a| {
                Ok(Address {
                    name: opt_string(a, "name")?.map(str::to_owned),
                    email: opt_string(a, "email")?.unwrap_or("").to_owned(),
                })
            })
            .collect(),
        Some(_) => Err(malformed(format!("`{key}` is not a list of addresses"))),
    }
}

fn headers(email: &Value) -> Result<Vec<(String, String)>, ProtoError> {
    match email.get("headers") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|h| {
                Ok((
                    string(h, "name")?.to_owned(),
                    string(h, "value")?.to_owned(),
                ))
            })
            .collect(),
        Some(_) => Err(malformed("`headers` is not a list")),
    }
}

/// Where an email is filed here, given the mailboxes it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filing {
    /// Synced: filed under `role`, carrying `labels`.
    Filed {
        role: MailboxRole,
        labels: Vec<String>,
    },
    /// In no mailbox this client follows — only Drafts or Junk — so not synced, and if held,
    /// gone.
    Unfollowed,
}

/// File an email in `ids` against the account's `mailboxes`.
///
/// One role, because a message here has one: the first of Trash, Inbox, Archive and Sent that
/// it is in, in that order. Trash first because a client that trashes keeps the other mailboxes
/// on some servers, and a trashed email listed in the inbox would come back from the bin. Then
/// Inbox, for mail sent to oneself. Archive over Sent because archiving a sent message leaves it
/// in Sent too ([`super::filing_patch`]), and the archive is what the user asked for. An email
/// only in mailboxes without a role is filed as Archive — kept, and out of the inbox — which is
/// what an IMAP folder files as.
///
/// Drafts and Junk file nothing: an email only there is [`Filing::Unfollowed`].
pub fn filing(ids: &[String], mailboxes: &Mailboxes) -> Filing {
    let roles = mailboxes.filing_roles(ids);
    let role = [
        MailboxRole::Trash,
        MailboxRole::Inbox,
        MailboxRole::Archive,
        MailboxRole::Sent,
    ]
    .into_iter()
    .find(|r| roles.contains(r))
    .or_else(|| mailboxes.any_label(ids).then_some(MailboxRole::Archive));
    match role {
        Some(role) => Filing::Filed {
            role,
            labels: mailboxes.labels(ids),
        },
        None => Filing::Unfollowed,
    }
}
