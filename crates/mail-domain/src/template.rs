//! Templates: a message kept so that new ones can be started from it.
//!
//! A type of its own rather than a kind of [`Draft`]. A draft is one message on its way
//! somewhere — it has a [`SendState`], and it may answer or forward a particular message. A
//! template is none of those things: it is never sent, and a template that remembered the message
//! it was first written in reply to would make every message started from it a reply to that
//! same message. With a kind on `Draft`, every place that sends, lists or discards drafts would
//! have to remember to ask which kind it holds, and the one that forgot would send a template;
//! as its own type, sending one is a type error.
//!
//! Local only. A template is not uploaded to the server's Drafts folder and does not arrive from
//! it: the server has no notion of one, and a template stored as an IMAP draft would be one that
//! another client offers to send.

use crate::content::Address;
use crate::draft::{Draft, PendingAttachment, SendState};
use crate::id::{AccountId, DraftId, IdentityId, TemplateId};
use crate::pgp::OpenPgp;
use crate::receipt::ReceiptRequest;
use crate::smime::Smime;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What a template is called when neither its maker nor its subject says.
const UNTITLED: &str = "Untitled template";

/// A message kept to start new ones from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    pub id: TemplateId,
    pub account: AccountId,
    /// The address a message started from it leaves from.
    pub identity: IdentityId,
    /// What the user calls it. Never empty: see [`Template::from_draft`].
    pub name: String,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
    /// Carried by reference. Blobs are content-addressed and shared, so a template and every
    /// draft started from it point at the same bytes rather than copies of them.
    pub attachments: Vec<PendingAttachment>,
    /// Whether messages started from it ask for a read receipt, as the draft it was kept from
    /// did.
    pub receipt: ReceiptRequest,
    /// Whether messages started from it are signed or encrypted, as the draft it was kept from
    /// asked. Defaulted so templates kept before OpenPGP existed load as plain; carried so a
    /// template kept from an encrypted draft never starts a plain one.
    #[serde(default)]
    pub openpgp: OpenPgp,
    /// The same for S/MIME, carried for the same reason.
    #[serde(default)]
    pub smime: Smime,
    pub updated: DateTime<Utc>,
}

impl Template {
    /// Keep `draft` as a template called `name`.
    ///
    /// A blank `name` falls back to the subject, and a blank subject to a fixed word, so a
    /// template listing never has a row with nothing in it to recognise it by.
    ///
    /// `in_reply_to` and `forward_of` are left behind: the template is the message, not the
    /// conversation it was first written in.
    pub fn from_draft(draft: &Draft, name: &str, now: DateTime<Utc>) -> Template {
        let name = [name, draft.subject.as_str()]
            .into_iter()
            .map(str::trim)
            .find(|candidate| !candidate.is_empty())
            .unwrap_or(UNTITLED)
            .to_owned();
        Template {
            id: TemplateId::generate(),
            account: draft.account,
            identity: draft.identity,
            name,
            to: draft.to.clone(),
            cc: draft.cc.clone(),
            bcc: draft.bcc.clone(),
            subject: draft.subject.clone(),
            text: draft.text.clone(),
            html: draft.html.clone(),
            attachments: draft.attachments.clone(),
            receipt: draft.receipt,
            openpgp: draft.openpgp,
            smime: draft.smime,
            updated: now,
        }
    }

    /// A new draft, started from this template. The template itself is left as it was.
    pub fn draft(&self, now: DateTime<Utc>) -> Draft {
        Draft {
            id: DraftId::generate(),
            account: self.account,
            identity: self.identity,
            to: self.to.clone(),
            cc: self.cc.clone(),
            bcc: self.bcc.clone(),
            subject: self.subject.clone(),
            in_reply_to: None,
            forward_of: None,
            text: self.text.clone(),
            html: self.html.clone(),
            attachments: self.attachments.clone(),
            receipt: self.receipt,
            openpgp: self.openpgp,
            smime: self.smime,
            state: SendState::Editing,
            updated: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{BlobId, MessageId};

    fn at(hour: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2026-03-01T{hour:02}:00:00Z"))
            .expect("literal is valid RFC 3339")
            .with_timezone(&Utc)
    }

    fn addr(email: &str) -> Address {
        Address {
            name: None,
            email: email.to_owned(),
        }
    }

    fn draft(subject: &str) -> Draft {
        Draft {
            id: DraftId::generate(),
            account: AccountId::generate(),
            identity: IdentityId::generate(),
            to: vec![addr("team@example.test")],
            cc: vec![addr("lead@example.test")],
            bcc: vec![addr("archive@example.test")],
            subject: subject.to_owned(),
            in_reply_to: Some(MessageId::generate()),
            forward_of: Some(MessageId::generate()),
            text: "This week:\r\n".to_owned(),
            html: Some("<p>This week:</p>".to_owned()),
            attachments: vec![PendingAttachment {
                name: "plan.pdf".to_owned(),
                mime: "application/pdf".to_owned(),
                blob: BlobId::generate(),
            }],
            receipt: ReceiptRequest::Requested,
            openpgp: OpenPgp::SignAndEncrypt,
            smime: Smime::Sign,
            state: SendState::Sent {
                at: at(8),
                message: None,
            },
            updated: at(8),
        }
    }

    #[test]
    fn a_template_keeps_the_message_and_not_the_conversation() {
        let original = draft("Weekly report");
        let kept = Template::from_draft(&original, "weekly", at(9));
        assert_eq!(kept.name, "weekly");
        assert_eq!(
            (&kept.to, &kept.cc, &kept.bcc),
            (&original.to, &original.cc, &original.bcc)
        );
        assert_eq!(kept.subject, original.subject);
        assert_eq!(kept.text, original.text);
        assert_eq!(kept.html, original.html);
        assert_eq!(kept.attachments, original.attachments);
        assert_eq!(kept.receipt, ReceiptRequest::Requested);
        assert_eq!(kept.openpgp, OpenPgp::SignAndEncrypt);
        assert_eq!(kept.smime, Smime::Sign);
        assert_eq!(kept.identity, original.identity);
        assert_eq!(kept.account, original.account);
        assert_eq!(kept.updated, at(9));

        let started = kept.draft(at(10));
        assert_ne!(started.id, original.id, "a new draft, not the old one back");
        assert_eq!(started.in_reply_to, None);
        assert_eq!(started.forward_of, None);
        assert_eq!(started.state, SendState::Editing);
        assert_eq!(started.updated, at(10));
        assert_eq!(started.to, original.to);
        assert_eq!(started.attachments, original.attachments);
        assert_eq!(started.receipt, ReceiptRequest::Requested);
        assert_eq!(
            started.openpgp,
            OpenPgp::SignAndEncrypt,
            "never silently plain"
        );
        assert_eq!(started.smime, Smime::Sign, "never silently plain");
    }

    #[test]
    fn two_drafts_from_one_template_are_two_drafts() {
        let kept = Template::from_draft(&draft("Weekly report"), "", at(9));
        assert_ne!(kept.draft(at(10)).id, kept.draft(at(10)).id);
    }

    #[test]
    fn a_template_is_always_called_something() {
        const CASES: &[(&str, &str, &str)] = &[
            ("weekly", "Weekly report", "weekly"),
            ("  ", "Weekly report", "Weekly report"),
            ("", "  Weekly report ", "Weekly report"),
            ("", "", UNTITLED),
            (" ", "   ", UNTITLED),
        ];
        for (name, subject, expected) in CASES {
            let kept = Template::from_draft(&draft(subject), name, at(9));
            assert_eq!(kept.name, *expected, "name {name:?}, subject {subject:?}");
        }
    }
}
