use crate::view::Stamp;
use chrono::Local;
use mail_domain::*;

pub(super) fn draft_state(state: &SendState) -> &'static str {
    match state {
        SendState::Editing => "draft",
        SendState::Queued => "queued",
        SendState::Scheduled { .. } => "scheduled",
        SendState::Sending => "sending",
        SendState::Failed { .. } => "failed",
        SendState::Sent { .. } => "sent",
    }
}

pub(super) fn from_name(message: &Message) -> String {
    message.from.name.clone().unwrap_or_default()
}

/// Where an attachment's bytes are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kept {
    /// Stored in this client's blobs.
    Here,
    /// Still on the server: `Attachment::blob` is `None`.
    OnServer,
    /// Inside a protected message opened for reading: in memory only, never stored.
    Opened,
}

/// One attachment row: its safe name, its size as shown, and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AttachmentRow {
    pub index: usize,
    pub name: String,
    pub size: String,
    pub kept: Kept,
}

/// Each attachment as the reading pane draws it.
///
/// The name is `attach::safe_name`, not the claim: showing `../../escape.pdf` would describe
/// something that does not happen, and the pane and the command must agree.
///
/// While a part is on the server its size is the server's figure before transfer-decoding, and
/// decoding never makes a part larger than its encoded form, so the row says "up to" that
/// figure. That is always true, where an estimate would be a guess.
pub(super) fn attachment_rows(message: &Message) -> Vec<AttachmentRow> {
    message
        .attachments
        .iter()
        .enumerate()
        .map(|(index, a)| {
            let kept = match a.blob() {
                Some(_) => Kept::Here,
                None => Kept::OnServer,
            };
            let size = match kept {
                Kept::Here | Kept::Opened => crate::attach::human_size(a.size),
                Kept::OnServer => format!("up to {}", crate::attach::human_size(a.size)),
            };
            AttachmentRow {
                index,
                name: crate::attach::safe_name(&a.name),
                size,
                kept,
            }
        })
        .collect()
}

pub(super) fn address(message: &Message) -> String {
    message.from.email.clone()
}

pub(super) fn stamp(message: &Message) -> String {
    crate::view::stamp(message.date, &Local, Stamp::Full)
}

pub(super) fn sender(summary: &ThreadSummary) -> String {
    summary
        .from
        .name
        .clone()
        .unwrap_or_else(|| summary.from.email.clone())
}

pub(super) fn label(kind: OpKind) -> &'static str {
    match kind {
        OpKind::Archive => "Archive",
        OpKind::Trash => "Trash",
        OpKind::Restore => "Restore",
        OpKind::Spam => "Spam",
        OpKind::MarkRead => "Read",
        OpKind::MarkUnread => "Unread",
        OpKind::Star => "Star",
        OpKind::Unstar => "Unstar",
        OpKind::AddLabel | OpKind::RemoveLabel => "Label",
        OpKind::Snooze => "Snooze",
        OpKind::Pin => "Pin",
        OpKind::Reply => "Reply",
        OpKind::ReplyAll => "Reply all",
        OpKind::Forward => "Forward",
    }
}

#[cfg(test)]
mod tests {
    use super::{AttachmentRow, Kept, attachment_rows};
    use chrono::{TimeZone, Utc};
    use mail_domain::*;

    /// Where the bytes were put when the message was built.
    ///
    /// Not [`Kept`]: this is the input, and the row's `kept` is what the function decides.
    #[derive(Clone, Copy)]
    enum Stored {
        Held,
        Remote,
    }

    fn carrying(cases: &[(&str, u64, Stored)]) -> Message {
        Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account: AccountId::generate(),
            key: MessageKey::Rfc("rows@example.test".to_owned()),
            date: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            from: Address {
                name: None,
                email: "sender@example.test".to_owned(),
            },
            reply_to: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: "rows".to_owned(),
            in_reply_to: None,
            references: Vec::new(),
            rfc_message_id: Some("rows@example.test".to_owned()),
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: Vec::new(),
            body: Body::Absent,
            attachments: cases
                .iter()
                .map(|(name, size, stored)| Attachment {
                    name: (*name).to_owned(),
                    mime: "application/octet-stream".to_owned(),
                    size: *size,
                    content: match stored {
                        Stored::Held => PartContent::Held(BlobId::generate()),
                        Stored::Remote => PartContent::Remote {
                            section: "2".to_owned(),
                        },
                    },
                    inline: Inline::Attached,
                })
                .collect(),
        }
    }

    #[test]
    fn a_row_names_the_file_says_its_size_and_where_the_bytes_are() {
        // `notes.txt` is already a safe name, so it cannot tell a row that prints the claim
        // from one that prints `safe_name`. `../../etc/passwd` can: the row must show `passwd`.
        const CASES: &[(&str, u64, Stored, &str, &str, Kept)] = &[
            (
                "notes.txt",
                1536,
                Stored::Held,
                "notes.txt",
                "1.5 kB",
                Kept::Here,
            ),
            (
                "report.pdf",
                5 * 1024 * 1024,
                Stored::Remote,
                "report.pdf",
                "up to 5.0 MB",
                Kept::OnServer,
            ),
            (
                "../../etc/passwd",
                512,
                Stored::Held,
                "passwd",
                "512 B",
                Kept::Here,
            ),
        ];

        let inputs: Vec<(&str, u64, Stored)> = CASES
            .iter()
            .map(|(name, size, stored, _, _, _)| (*name, *size, *stored))
            .collect();
        let rows = attachment_rows(&carrying(&inputs));
        assert_eq!(rows.len(), CASES.len(), "one row per attachment");
        for (index, (claim, _, _, expect_name, expect_size, expect_kept)) in
            CASES.iter().enumerate()
        {
            assert_eq!(
                rows[index],
                AttachmentRow {
                    index,
                    name: (*expect_name).to_owned(),
                    size: (*expect_size).to_owned(),
                    kept: *expect_kept,
                },
                "{claim}"
            );
        }
    }
}
