//! Google Takeout's mbox: every message once, with its labels in an `X-Gmail-Labels` header.
//!
//! The header is a comma-separated list; a label containing a comma is double-quoted. A few of
//! the names are not labels the user made but Gmail's own state — which folder, whether read,
//! whether starred — and those become the message's role and flags. Everything else, the
//! user's labels and Gmail's categories alike, is kept as a label.

use super::{Placement, header};
use mail_domain::{MailboxRole, SystemFlag};

/// The labels in `raw`'s `X-Gmail-Labels` header, or `None` when it has none.
pub fn labels(raw: &[u8]) -> Option<Vec<String>> {
    header(raw, "X-Gmail-Labels").map(|value| split(&value))
}

/// Split a label list on commas outside double quotes.
pub fn split(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ',' if !quoted => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    out.push(current);
    out.into_iter()
        .map(|label| label.trim().to_owned())
        .filter(|label| !label.is_empty())
        .collect()
}

/// A Takeout label list as a placement.
///
/// The role is the most specific place named: trash and spam over everything, a draft next,
/// then the inbox, then sent; a message in none of them is archived, which is what it is in
/// Gmail. Read unless `Unread` is listed, starred when `Starred` is.
pub fn placement(labels: &[String]) -> Placement {
    let has = |name: &str| labels.iter().any(|l| l.eq_ignore_ascii_case(name));
    let role = if has("Trash") {
        MailboxRole::Trash
    } else if has("Spam") {
        MailboxRole::Spam
    } else if has("Drafts") || has("Draft") {
        MailboxRole::Drafts
    } else if has("Inbox") {
        MailboxRole::Inbox
    } else if has("Sent") {
        MailboxRole::Sent
    } else {
        MailboxRole::Archive
    };
    let mut flags = Vec::new();
    if !has("Unread") {
        flags.push(SystemFlag::Seen);
    }
    if has("Starred") {
        flags.push(SystemFlag::Flagged);
    }
    if role == MailboxRole::Drafts {
        flags.push(SystemFlag::Draft);
    }
    let kept = labels.iter().filter(|l| !is_state(l)).cloned().collect();
    Placement::new(role, flags, kept)
}

/// Whether a Takeout label is Gmail's own state rather than a label.
fn is_state(label: &str) -> bool {
    const STATE: [&str; 10] = [
        "Inbox", "Sent", "Drafts", "Draft", "Trash", "Spam", "Starred", "Unread", "Opened",
        "Archived",
    ];
    STATE.iter().any(|s| s.eq_ignore_ascii_case(label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::{ReadState, Star};

    #[test]
    fn a_quoted_label_keeps_its_comma() {
        assert_eq!(
            split(r#"Inbox,"Travel, 2019",Receipts"#),
            vec!["Inbox", "Travel, 2019", "Receipts"]
        );
    }

    #[test]
    fn gmail_state_becomes_role_and_flags_and_the_rest_stay_labels() {
        let labels = split("Archived,Starred,Category Updates,Receipts,Opened");
        let placed = placement(&labels);
        assert_eq!(placed.role, MailboxRole::Archive);
        assert_eq!(placed.read(), ReadState::Read);
        assert_eq!(placed.star(), Star::Starred);
        assert_eq!(placed.labels, vec!["Category Updates", "Receipts"]);
    }

    #[test]
    fn unread_inbox_mail_stays_unread() {
        let placed = placement(&split("Inbox,Unread,Important"));
        assert_eq!(placed.role, MailboxRole::Inbox);
        assert_eq!(placed.read(), ReadState::Unread);
        assert_eq!(placed.labels, vec!["Important"]);
    }

    #[test]
    fn trash_wins_over_the_inbox() {
        assert_eq!(placement(&split("Inbox,Trash")).role, MailboxRole::Trash);
    }
}
