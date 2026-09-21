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
    /// The composer, when one is open.
    ///
    /// `Option` rather than a `mode` enum on `Shell`: composing does not replace reading, it
    /// sits beside it. A user who opens a reply and then clicks another thread should still
    /// have their half-written reply when they come back, which a mode would have thrown away.
    pub composing: Option<Composing>,
}

/// A message being edited, as the widgets hold it.
///
/// Recipients are `String`s, not `Vec<Address>`, because that is what a text box contains. The
/// user is mid-typing for most of this struct's life, and a half-typed address is not an
/// `Address` — forcing it to be one means either rejecting every keystroke or inventing a
/// parse that silently discards what was typed. Parsing happens once, on save, in
/// [`parse_addresses`].
///
/// No `Default`: a composer with no draft behind it is not a state this can be in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composing {
    /// The draft this edits. It already exists in the store before the composer opens.
    pub draft: DraftId,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
    /// What went wrong with the last save or send, shown inline.
    pub notice: Option<String>,
}

impl Composing {
    /// Open the composer on an existing draft.
    pub fn of(draft: &Draft) -> Self {
        Self {
            draft: draft.id,
            to: join_addresses(&draft.to),
            cc: join_addresses(&draft.cc),
            subject: draft.subject.clone(),
            body: draft.text.clone(),
            notice: None,
        }
    }

    /// The edited draft, or what is wrong with it.
    ///
    /// Takes the stored draft rather than building one, so everything the composer does not
    /// show — the identity, the message being replied to, the attachments — survives an edit
    /// instead of being reset to a default the user never chose.
    pub fn apply_to(
        &self,
        base: &Draft,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Draft, String> {
        let to = parse_addresses(&self.to).map_err(|e| format!("To: {e}"))?;
        let cc = parse_addresses(&self.cc).map_err(|e| format!("Cc: {e}"))?;
        Ok(Draft {
            to,
            cc,
            subject: self.subject.clone(),
            text: self.body.clone(),
            updated: now,
            ..base.clone()
        })
    }
}

/// Render addresses back into something a text box can hold.
pub fn join_addresses(list: &[Address]) -> String {
    list.iter()
        .map(|a| match &a.name {
            Some(name) if !name.is_empty() => format!("{name} <{}>", a.email),
            _ => a.email.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parse a recipient box into addresses.
///
/// Accepts `a@b.test`, `Name <a@b.test>` and `"Name" <a@b.test>`, separated by commas or
/// semicolons. Empty entries are skipped, because a trailing comma is what typing looks like.
///
/// An entry with no `@` is an error rather than a guess. The alternative — dropping it, or
/// appending a default domain — means the user sees their recipient vanish, or sends to
/// someone they did not name. Both are silent; an error is not.
pub fn parse_addresses(input: &str) -> Result<Vec<Address>, String> {
    let mut out: Vec<Address> = Vec::new();
    for entry in split_entries(input) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let address = parse_one(entry)?;
        // First spelling wins, matching `mail_mime::posting`. Two RCPT TO lines for one mailbox
        // are two deliveries.
        if !out
            .iter()
            .any(|kept| kept.email.eq_ignore_ascii_case(&address.email))
        {
            out.push(address);
        }
    }
    Ok(out)
}

/// Split a recipient box on separators that are not inside a display name or an address.
///
/// Not `split([',', ';'])`. `"Lovelace, Ada" <ada@example.test>` is one recipient, and every
/// other mail client writes a display name containing a comma exactly that way — so a naive
/// split turns a pasted recipient into two, one of which is not an address at all. The comma
/// inside quotes is the single most common character this has to *not* split on.
fn split_entries(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut angled) = (0usize, false, false);
    for (i, ch) in input.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            // An unbalanced '<' would otherwise swallow the rest of the box; '>' only closes
            // what a '<' opened, so a stray '>' cannot re-enable splitting that was never off.
            '<' if !quoted => angled = true,
            '>' if !quoted => angled = false,
            ',' | ';' if !quoted && !angled => {
                out.push(&input[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&input[start..]);
    out
}

fn parse_one(entry: &str) -> Result<Address, String> {
    let (name, email) = match (entry.rfind('<'), entry.rfind('>')) {
        (Some(open), Some(close)) if close > open => {
            let name = entry[..open].trim().trim_matches('"').trim();
            (
                if name.is_empty() {
                    None
                } else {
                    Some(name.to_owned())
                },
                entry[open + 1..close].trim(),
            )
        }
        _ => (None, entry),
    };
    if email.is_empty() {
        return Err(format!("{entry:?} has no address"));
    }
    // One `@`, with something either side. Not a full RFC 5322 validation: that accepts things
    // no mail server does, and rejecting what a user's own server accepts is worse than letting
    // the server answer. This catches the mistake people actually make, which is a missing `@`.
    let mut halves = email.split('@');
    let (local, domain) = (halves.next().unwrap_or(""), halves.next().unwrap_or(""));
    if local.is_empty() || domain.is_empty() || halves.next().is_some() {
        return Err(format!("{email:?} is not an email address"));
    }
    if email.contains(char::is_whitespace) {
        return Err(format!("{email:?} contains a space"));
    }
    Ok(Address {
        name,
        email: email.to_owned(),
    })
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            places: default_places(),
            selected: 0,
            search: String::new(),
            open: None,
            show_remote_images: false,
            composing: None,
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

    /// Open the composer on `draft`.
    pub fn compose(&mut self, draft: &Draft) {
        self.composing = Some(Composing::of(draft));
    }

    /// Close the composer, discarding whatever is in it.
    ///
    /// Discarding is safe because every edit the composer makes is saved to the store before it
    /// can be lost — the draft row is the document, and this struct is only the widgets.
    pub fn close_composer(&mut self) {
        self.composing = None;
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

/// The message a reply to this thread should answer.
///
/// The newest, which is what "reply" means to everyone except the person who wrote the
/// threading code. Replying to the thread's *root* would quote a conversation's opening line
/// back at someone who has since sent four more, and would set `In-Reply-To` to a message the
/// recipient's client threads above everything they last read.
///
/// Ties break on id so a thread with two messages at the same timestamp — which a bulk import
/// produces easily — picks the same one every time rather than alternating between renders.
pub fn reply_target(messages: &[Message]) -> Option<&Message> {
    messages.iter().max_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| a.id.to_string().cmp(&b.id.to_string()))
    })
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
        let mut shell = Shell {
            show_remote_images: true,
            ..Shell::default()
        };
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

#[cfg(test)]
mod composer_tests {
    use super::*;

    fn addr(name: Option<&str>, email: &str) -> Address {
        Address {
            name: name.map(str::to_owned),
            email: email.to_owned(),
        }
    }

    #[test]
    fn a_recipient_box_accepts_the_shapes_people_type() {
        assert_eq!(
            parse_addresses("ada@example.test").unwrap(),
            vec![addr(None, "ada@example.test")]
        );
        assert_eq!(
            parse_addresses("Ada Lovelace <ada@example.test>").unwrap(),
            vec![addr(Some("Ada Lovelace"), "ada@example.test")]
        );
        // Quoted display names are what other clients paste in.
        assert_eq!(
            parse_addresses("\"Lovelace, Ada\" <ada@example.test>").unwrap(),
            vec![addr(Some("Lovelace, Ada"), "ada@example.test")]
        );
    }

    #[test]
    fn a_comma_inside_a_display_name_does_not_split_the_recipient() {
        // Every other client writes "Last, First" this way, so this arrives by paste constantly.
        // Splitting here turns one recipient into two, one of which is not an address at all.
        let parsed = parse_addresses("\"Lovelace, Ada\" <ada@x.test>, bob@x.test").unwrap();
        assert_eq!(parsed.len(), 2, "{parsed:?}");
        assert_eq!(parsed[0].name.as_deref(), Some("Lovelace, Ada"));
        assert_eq!(parsed[0].email, "ada@x.test");
        assert_eq!(parsed[1].email, "bob@x.test");
    }

    #[test]
    fn a_separator_inside_the_address_itself_does_not_split_either() {
        let parsed = parse_addresses("<a;b@x.test>").unwrap();
        assert_eq!(parsed.len(), 1, "{parsed:?}");
    }

    #[test]
    fn commas_and_semicolons_both_separate() {
        // Outlook produces semicolons. A user pasting from it should not have to know.
        let by_comma = parse_addresses("a@x.test, b@x.test").unwrap();
        let by_semi = parse_addresses("a@x.test; b@x.test").unwrap();
        assert_eq!(by_comma, by_semi);
        assert_eq!(by_comma.len(), 2);
    }

    #[test]
    fn a_trailing_comma_is_what_typing_looks_like() {
        // The box is parsed on every keystroke to show errors. If "a@x.test," were an error,
        // the composer would shout at the user in the middle of typing the second recipient.
        assert_eq!(parse_addresses("a@x.test,").unwrap().len(), 1);
        assert_eq!(parse_addresses("  ,, a@x.test , ,").unwrap().len(), 1);
        assert!(parse_addresses("").unwrap().is_empty());
        assert!(parse_addresses("   ").unwrap().is_empty());
    }

    #[test]
    fn something_that_is_not_an_address_is_an_error_not_a_guess() {
        // Dropping it makes the recipient vanish; appending a default domain sends to someone
        // the user did not name. Both are silent, which is what makes them worse than an error.
        for bad in ["ada", "ada@", "@example.test", "a@b@c", "ada example.test"] {
            assert!(
                parse_addresses(bad).is_err(),
                "{bad:?} was accepted as an address"
            );
        }
    }

    #[test]
    fn a_bad_entry_fails_the_whole_box_rather_than_half_of_it() {
        // Sending to "everyone I could parse" is the failure mode this prevents.
        let err = parse_addresses("good@x.test, nonsense, other@x.test").unwrap_err();
        assert!(err.contains("nonsense"), "{err}");
    }

    #[test]
    fn one_mailbox_twice_is_one_recipient() {
        let parsed = parse_addresses("Ada <ada@x.test>, ADA@X.TEST").unwrap();
        assert_eq!(parsed.len(), 1, "{parsed:?}");
        // The first spelling wins, so the display name the user typed survives.
        assert_eq!(parsed[0].name.as_deref(), Some("Ada"));
    }

    #[test]
    fn what_the_box_shows_parses_back_to_what_it_held() {
        let original = vec![
            addr(Some("Ada Lovelace"), "ada@example.test"),
            addr(None, "bob@example.test"),
        ];
        assert_eq!(
            parse_addresses(&join_addresses(&original)).unwrap(),
            original
        );
    }

    fn draft_of(to: Vec<Address>) -> Draft {
        Draft {
            id: DraftId::generate(),
            account: AccountId::generate(),
            identity: IdentityId::generate(),
            to,
            cc: Vec::new(),
            bcc: vec![addr(None, "blind@example.test")],
            subject: "Re: lunch".to_owned(),
            in_reply_to: Some(MessageId::generate()),
            forward_of: None,
            text: "body".to_owned(),
            html: None,
            attachments: Vec::new(),
            state: SendState::Editing,
            updated: chrono::Utc::now(),
        }
    }

    #[test]
    fn editing_changes_only_what_the_composer_shows() {
        // The composer has no Bcc box, no identity picker and no attachment list. If it built a
        // Draft from scratch it would silently drop all three — and the blind recipient would
        // stop receiving the message because the user fixed a typo in the subject.
        let base = draft_of(vec![addr(None, "ada@example.test")]);
        let mut editing = Composing::of(&base);
        editing.subject = "Re: lunch, moved".to_owned();
        let now = chrono::Utc::now();

        let edited = editing.apply_to(&base, now).unwrap();
        assert_eq!(edited.subject, "Re: lunch, moved");
        assert_eq!(edited.bcc, base.bcc, "the blind recipient was dropped");
        assert_eq!(edited.identity, base.identity);
        assert_eq!(edited.in_reply_to, base.in_reply_to, "threading was lost");
        assert_eq!(edited.id, base.id, "editing must not mint a second draft");
        assert_eq!(edited.updated, now);
    }

    #[test]
    fn a_bad_recipient_names_which_box_it_is_in() {
        let base = draft_of(Vec::new());
        let mut editing = Composing::of(&base);
        editing.to = "fine@example.test".to_owned();
        editing.cc = "not-an-address".to_owned();

        let err = editing
            .apply_to(&base, chrono::Utc::now())
            .expect_err("the Cc box is wrong");
        assert!(err.starts_with("Cc:"), "{err}");
    }

    #[test]
    fn opening_the_composer_shows_what_the_draft_holds() {
        let base = draft_of(vec![addr(Some("Ada"), "ada@example.test")]);
        let editing = Composing::of(&base);
        assert_eq!(editing.to, "Ada <ada@example.test>");
        assert_eq!(editing.subject, "Re: lunch");
        assert_eq!(editing.body, "body");
        assert_eq!(editing.draft, base.id);
        assert_eq!(editing.notice, None);
    }

    #[test]
    fn a_half_written_reply_survives_reading_another_thread() {
        // Composing sits beside reading rather than replacing it. A mode enum here would throw
        // the draft's widgets away the moment the user clicked another conversation.
        let base = draft_of(vec![addr(None, "ada@example.test")]);
        let mut shell = Shell::default();
        shell.compose(&base);
        shell.composing.as_mut().unwrap().body = "half a sentence".to_owned();

        shell.open(ThreadId::generate());
        assert_eq!(
            shell.composing.as_ref().map(|c| c.body.as_str()),
            Some("half a sentence"),
            "opening a thread discarded the composer"
        );

        shell.close_composer();
        assert!(shell.composing.is_none());
    }
}

#[cfg(test)]
mod reply_target_tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn message(n: i64) -> Message {
        Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account: AccountId::generate(),
            key: MessageKey::Rfc(format!("m{n}@example.test")),
            date: Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap(),
            from: Address {
                name: None,
                email: format!("s{n}@example.test"),
            },
            reply_to: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: format!("subject {n}"),
            in_reply_to: None,
            references: Vec::new(),
            rfc_message_id: Some(format!("m{n}@example.test")),
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: Vec::new(),
            body: Body::Absent,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn a_reply_answers_the_newest_message_not_the_first() {
        // Replying to the root quotes a conversation's opening line back at someone who has
        // since sent four more, and threads the reply above everything they last read.
        let messages = vec![message(0), message(300), message(100)];
        let target = reply_target(&messages).expect("a thread has messages");
        assert_eq!(target.subject, "subject 300");
    }

    #[test]
    fn the_choice_is_stable_when_two_messages_share_a_timestamp() {
        // A bulk import produces these easily. An unstable pick would reply to a different
        // message on each render, which the user would see as the quote changing under them.
        let messages = vec![message(5), message(5), message(5)];
        let first = reply_target(&messages).unwrap().id;
        for _ in 0..8 {
            assert_eq!(reply_target(&messages).unwrap().id, first);
        }
    }

    #[test]
    fn an_empty_thread_has_nothing_to_reply_to() {
        // Reachable: the messages are loaded one by one and any of them can fail to read.
        assert!(reply_target(&[]).is_none());
    }
}
