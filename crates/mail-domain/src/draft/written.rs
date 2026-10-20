//! What the person put in a draft, apart from what mailo put there for them.
//!
//! A new draft is not empty: its body is the identity's signature, below the `"-- "` line RFC
//! 3676 §4.3 names. So "nothing written" is read above that line.

use super::Draft;

impl Draft {
    /// The body above the signature, trimmed: what was written. Empty for a new draft that
    /// nobody typed in.
    pub fn written(&self) -> &str {
        let body = self.text.as_str();
        let mut at = 0;
        for line in body.split_inclusive('\n') {
            if line.trim_end_matches(['\r', '\n']) == "-- " {
                return body[..at].trim();
            }
            at += line.len();
        }
        body.trim()
    }

    /// Whether nothing was put in the draft: no recipient, no subject, no words above the
    /// signature and no attachment, and it answers or forwards nothing. Closing a composer on
    /// such a draft throws it away rather than keeping it.
    pub fn untouched(&self) -> bool {
        self.to.is_empty()
            && self.cc.is_empty()
            && self.bcc.is_empty()
            && self.subject.trim().is_empty()
            && self.written().is_empty()
            && self.attachments.is_empty()
            && self.in_reply_to.is_none()
            && self.forward_of.is_none()
    }
}

#[cfg(test)]
mod tests {
    use crate::account::Identity;
    use crate::content::Address;
    use crate::draft::Draft;
    use crate::id::{IdentityId, MessageId, new_account_id};
    use crate::state::IsDefault;
    use chrono::{TimeZone, Utc};

    fn new_draft() -> Draft {
        let identity = Identity {
            id: IdentityId::generate(),
            account: new_account_id(),
            from: Address {
                name: None,
                email: "me@example.test".to_owned(),
            },
            reply_to: None,
            signature: None,
            default: IsDefault::Default,
        };
        Draft::blank(&identity, Utc.timestamp_opt(1_700_000_000, 0).unwrap())
    }

    #[test]
    fn what_was_written_stops_at_the_signature() {
        const CASES: &[(&str, &str)] = &[
            ("", ""),
            ("  \r\n", ""),
            ("\r\n\r\n-- \r\nAda\r\nAnalytical Engines", ""),
            ("Lunch at noon?\r\n\r\n-- \r\nAda", "Lunch at noon?"),
            ("Lunch at noon?\n-- \nAda", "Lunch at noon?"),
            // Two hyphens without the space are words, not the delimiter.
            ("--\r\nAda", "--\r\nAda"),
            ("No signature here", "No signature here"),
        ];
        for (text, want) in CASES {
            let draft = Draft {
                text: (*text).to_owned(),
                ..new_draft()
            };
            assert_eq!(draft.written(), *want, "{text:?}");
        }
    }

    #[test]
    fn a_draft_is_untouched_until_anything_is_put_in_it() {
        let ada = Address {
            name: None,
            email: "ada@example.test".to_owned(),
        };
        type Case = (&'static str, fn(&mut Draft, &Address), bool);
        let cases: [Case; 9] = [
            ("as made", |_, _| {}, true),
            (
                "signature alone",
                |d, _| d.text = "\r\n-- \r\nAda".to_owned(),
                true,
            ),
            ("blank subject", |d, _| d.subject = "  ".to_owned(), true),
            ("a recipient", |d, a| d.to.push(a.clone()), false),
            ("a Bcc", |d, a| d.bcc.push(a.clone()), false),
            ("a subject", |d, _| d.subject = "Plans".to_owned(), false),
            (
                "words",
                |d, _| d.text = "Hi\r\n-- \r\nAda".to_owned(),
                false,
            ),
            (
                "a reply",
                |d, _| d.in_reply_to = Some(MessageId::generate()),
                false,
            ),
            (
                "a forward",
                |d, _| d.forward_of = Some(MessageId::generate()),
                false,
            ),
        ];
        for (case, change, want) in cases {
            let mut draft = new_draft();
            change(&mut draft, &ada);
            assert_eq!(draft.untouched(), want, "{case}");
        }
    }
}
