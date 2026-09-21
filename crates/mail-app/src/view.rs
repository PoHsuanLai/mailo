//! What the shell shows, as data.
//!
//! Deliberately free of Dioxus. Which view is selected, what query that means, which thread is
//! open and what the reader should do with a body are all decisions that can be wrong, and none
//! of them needs a window to be wrong in. The rendering layer reads this and draws it.

use mail_domain::*;
use mail_mime::{RemoteImages, SanitizePolicy};

/// An entry in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub filter: Filter,
    /// Shown as a badge. `None` until counted, which is not the same as zero.
    pub unread: Option<u64>,
}

/// The default sidebar.
///
/// Inbox is `Filter::InMailbox(Inbox)` rather than anything special, which is the plan's claim
/// that a place is just a saved filter — made true here rather than asserted.
pub fn default_places() -> Vec<Place> {
    [
        ("Inbox", MailboxRole::Inbox),
        ("Archive", MailboxRole::Archive),
        ("Sent", MailboxRole::Sent),
        ("Drafts", MailboxRole::Drafts),
        ("Spam", MailboxRole::Spam),
        ("Trash", MailboxRole::Trash),
    ]
    .into_iter()
    .map(|(name, role)| Place {
        name: name.to_owned(),
        filter: Filter::InMailbox(role),
        unread: None,
    })
    .collect()
}

/// Everything the shell is currently showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    pub places: Vec<Place>,
    pub selected: usize,
    /// What the user typed in the search box.
    pub search: String,
    pub open: Option<ThreadId>,
    /// Whether the reader may fetch remote images for the thread currently open.
    ///
    /// Per thread and not persisted: consenting to load one sender's images is not consent for
    /// the next message, and a remote image is a read receipt the sender never asked for.
    pub show_remote_images: bool,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            places: default_places(),
            selected: 0,
            search: String::new(),
            open: None,
            show_remote_images: false,
        }
    }
}

impl Shell {
    /// The query the list pane should run.
    ///
    /// A non-empty search box replaces the place rather than narrowing it, which is what every
    /// mail client does and what users expect: searching while in Archive should not hide
    /// results that live in the Inbox.
    pub fn query(&self, limit: u32) -> Query {
        let needle = self.search.trim();
        let filter = if needle.is_empty() {
            self.places
                .get(self.selected)
                .map(|place| place.filter.clone())
                .unwrap_or(Filter::All)
        } else {
            Filter::Text(TextMatch::Contains(needle.to_owned()))
        };
        Query {
            filter,
            sort: Sort {
                property: Property::Date,
                dir: SortDir::Desc,
            },
            page: PageReq { after: None, limit },
        }
    }

    /// Select a place, and drop any open thread that no longer belongs to the new list.
    pub fn select(&mut self, index: usize) {
        if index < self.places.len() {
            self.selected = index;
            self.open = None;
            // Consent is per thread, so changing what is shown revokes it.
            self.show_remote_images = false;
        }
    }

    /// Open a thread.
    pub fn open(&mut self, thread: ThreadId) {
        self.open = Some(thread);
        self.show_remote_images = false;
    }

    /// The sanitizer policy for the thread currently open.
    pub fn policy(&self) -> SanitizePolicy {
        SanitizePolicy {
            remote_images: if self.show_remote_images {
                RemoteImages::Allowed
            } else {
                RemoteImages::Blocked
            },
            version: SanitizePolicy::CURRENT.version,
        }
    }
}

/// What the hover strip offers for a thread.
///
/// Derived from where the thread is, so Archive does not offer "archive" and Trash offers
/// "restore". Returns [`OpKind`] rather than [`Op`] because a button cannot carry a payload
/// that does not exist yet.
pub fn hover_actions(summary: &ThreadSummary) -> Vec<OpKind> {
    let mut out = Vec::new();
    if summary.mailboxes.contains(MailboxRole::Inbox) {
        out.push(OpKind::Archive);
    }
    if summary.mailboxes.contains(MailboxRole::Trash)
        || summary.mailboxes.contains(MailboxRole::Archive)
        || summary.mailboxes.contains(MailboxRole::Spam)
    {
        out.push(OpKind::Restore);
    }
    if !summary.mailboxes.contains(MailboxRole::Trash) {
        out.push(OpKind::Trash);
    }
    out.push(match summary.read {
        ReadState::Unread => OpKind::MarkRead,
        ReadState::Read => OpKind::MarkUnread,
    });
    out.push(match summary.star {
        Star::Unstarred => OpKind::Star,
        Star::Starred => OpKind::Unstar,
    });
    out
}

/// The `Op` a hover button performs, where it needs no payload.
///
/// `None` for the ones that open a composer instead — a reply is a draft, not an operation.
pub fn op_for(kind: OpKind) -> Option<Op> {
    match kind {
        OpKind::Archive => Some(Op::Archive),
        OpKind::Trash => Some(Op::Trash),
        OpKind::Restore => Some(Op::Restore),
        OpKind::Spam => Some(Op::Spam),
        OpKind::MarkRead => Some(Op::SetRead(ReadState::Read)),
        OpKind::MarkUnread => Some(Op::SetRead(ReadState::Unread)),
        OpKind::Star => Some(Op::SetStar(Star::Starred)),
        OpKind::Unstar => Some(Op::SetStar(Star::Unstarred)),
        // These need a label picked, a draft created, or a date chosen.
        OpKind::AddLabel
        | OpKind::RemoveLabel
        | OpKind::Snooze
        | OpKind::Pin
        | OpKind::Reply
        | OpKind::ReplyAll
        | OpKind::Forward => None,
    }
}

/// What the reader should display for one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// Headers only so far. Normal mid-sync, and not an empty message.
    NotFetched,
    /// Plain text, to render as text.
    Text(String),
    /// Sanitized markup, for a sandboxed frame.
    Html(String),
}

/// Decide what to show for a message body.
///
/// HTML is sanitized here rather than at ingest, so a policy change takes effect immediately
/// and an `ammonia` upgrade does not leave old messages sanitized under old rules.
pub fn reading(body: &Body, html: Option<&str>, policy: SanitizePolicy) -> Reading {
    match body {
        Body::Absent => Reading::NotFetched,
        Body::Present { text, .. } => match (html, text) {
            (Some(raw), _) => Reading::Html(mail_mime::sanitize(raw, policy).as_str().to_owned()),
            (None, Some(text)) => Reading::Text(text.clone()),
            (None, None) => Reading::Text(String::new()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn summary(tweak: impl FnOnce(&mut ThreadSummary)) -> ThreadSummary {
        let mut s = ThreadSummary {
            id: ThreadId::generate(),
            account: AccountId::generate(),
            subject: "s".into(),
            snippet: String::new(),
            from: Address {
                name: None,
                email: "a@b.test".into(),
            },
            participants: vec![],
            recipients: vec![],
            last_date: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            message_count: 1,
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailboxes: MailboxSet::only(MailboxRole::Inbox),
            labels: vec![],
            attachments: Attachments::None,
            snooze: Snooze::Inactive,
            pin: Pin::Unpinned,
        };
        tweak(&mut s);
        s
    }

    #[test]
    fn inbox_is_just_a_filter() {
        // The plan claims a place is a saved filter rather than anything special. Assert it.
        let shell = Shell::default();
        assert_eq!(
            shell.query(20).filter,
            Filter::InMailbox(MailboxRole::Inbox)
        );
    }

    #[test]
    fn searching_replaces_the_place_rather_than_narrowing_it() {
        // Searching while in Archive must not hide a result that lives in the Inbox.
        let mut shell = Shell::default();
        shell.select(1);
        shell.search = "  lunch  ".to_owned();
        assert_eq!(
            shell.query(20).filter,
            Filter::Text(TextMatch::Contains("lunch".to_owned())),
            "a search is global, and the box is trimmed"
        );
    }

    #[test]
    fn an_empty_search_box_falls_back_to_the_place() {
        let mut shell = Shell::default();
        shell.select(2);
        shell.search = "   ".to_owned();
        assert_eq!(shell.query(20).filter, Filter::InMailbox(MailboxRole::Sent));
    }

    #[test]
    fn consent_to_remote_images_does_not_survive_changing_what_is_shown() {
        // A remote image is a read receipt. Agreeing to load one sender's is not agreeing to
        // the next message's, so consent is revoked by opening anything else.
        let mut shell = Shell::default();
        shell.show_remote_images = true;
        shell.open(ThreadId::generate());
        assert!(!shell.show_remote_images);

        shell.show_remote_images = true;
        shell.select(1);
        assert!(!shell.show_remote_images);
        assert_eq!(shell.policy().remote_images, RemoteImages::Blocked);
    }

    #[test]
    fn hover_actions_fit_where_the_thread_is() {
        let inbox = hover_actions(&summary(|_| {}));
        assert!(inbox.contains(&OpKind::Archive), "{inbox:?}");
        assert!(!inbox.contains(&OpKind::Restore), "nothing to restore from");

        let archived = hover_actions(&summary(|s| {
            s.mailboxes = MailboxSet::only(MailboxRole::Archive)
        }));
        assert!(archived.contains(&OpKind::Restore), "{archived:?}");
        assert!(
            !archived.contains(&OpKind::Archive),
            "offering to archive what is archived is noise"
        );

        let trashed = hover_actions(&summary(|s| {
            s.mailboxes = MailboxSet::only(MailboxRole::Trash)
        }));
        assert!(!trashed.contains(&OpKind::Trash), "{trashed:?}");
    }

    #[test]
    fn the_read_and_star_buttons_show_the_opposite_of_the_current_state() {
        let unread = hover_actions(&summary(|_| {}));
        assert!(unread.contains(&OpKind::MarkRead));
        let read = hover_actions(&summary(|s| s.read = ReadState::Read));
        assert!(read.contains(&OpKind::MarkUnread));
    }

    #[test]
    fn actions_needing_a_payload_have_no_bare_op() {
        // A reply is a draft, not an operation, so a hover button cannot perform one.
        for kind in [
            OpKind::Reply,
            OpKind::Forward,
            OpKind::AddLabel,
            OpKind::Snooze,
        ] {
            assert!(op_for(kind).is_none(), "{kind:?} should open something");
        }
        assert_eq!(op_for(OpKind::Archive), Some(Op::Archive));
    }

    #[test]
    fn an_unfetched_body_is_not_an_empty_message() {
        assert_eq!(
            reading(&Body::Absent, None, SanitizePolicy::CURRENT),
            Reading::NotFetched
        );
    }

    #[test]
    fn html_is_sanitized_at_render_and_the_policy_is_honoured() {
        let body = Body::Present {
            text: Some("fallback".into()),
            raw: BlobId::generate(),
        };
        let hostile = r#"<p>hi</p><script>alert(1)</script><img src="https://tracker.test/p.gif">"#;

        let blocked = reading(&body, Some(hostile), SanitizePolicy::CURRENT);
        let Reading::Html(rendered) = blocked else {
            panic!("html should render as html");
        };
        assert!(rendered.contains("<p>hi</p>"), "{rendered}");
        assert!(!rendered.contains("alert"), "script survived: {rendered}");
        assert!(
            !rendered.contains("tracker.test"),
            "a remote image survived blocking: {rendered}"
        );

        let allowed = reading(
            &body,
            Some(hostile),
            SanitizePolicy {
                remote_images: RemoteImages::Allowed,
                version: SanitizePolicy::CURRENT.version,
            },
        );
        let Reading::Html(rendered) = allowed else {
            panic!("html should render as html");
        };
        assert!(rendered.contains("tracker.test"), "opting in must work");
        assert!(!rendered.contains("alert"), "images are not scripts");
    }

    #[test]
    fn plain_text_is_used_when_there_is_no_html() {
        let body = Body::Present {
            text: Some("just text".into()),
            raw: BlobId::generate(),
        };
        assert_eq!(
            reading(&body, None, SanitizePolicy::CURRENT),
            Reading::Text("just text".into())
        );
    }
}
