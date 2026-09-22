//! Table-driven coverage of [`Filter::fit`], one case per semantic decision.
//!
//! `ThreadSummary` values are built as struct literals rather than through
//! `ThreadSummary::derive`, which belongs to another brief: these cases are about the
//! predicate, not about the derivation, and they must not fail for someone else's reason.

use chrono::{DateTime, Utc};
use mail_domain::{
    AccountId, Address, Attachments, DateRange, Filter, LabelId, MailboxRole, MailboxSet, MatchCtx,
    Pin, ReadState, Snooze, Star, TextMatch, ThreadId, ThreadSummary,
};
use uuid::Uuid;

const ACCOUNT_A: AccountId = AccountId::from_uuid(Uuid::from_u128(0xA));
const ACCOUNT_B: AccountId = AccountId::from_uuid(Uuid::from_u128(0xB));
const LABEL_WORK: LabelId = LabelId::from_uuid(Uuid::from_u128(0x10));
const LABEL_HOME: LabelId = LabelId::from_uuid(Uuid::from_u128(0x11));

/// A fixed instant. No `Utc::now()` anywhere, so a failure reproduces.
fn ts(rfc3339: &str) -> DateTime<Utc> {
    rfc3339.parse().expect("fixture timestamp is well-formed")
}

fn now() -> DateTime<Utc> {
    ts("2024-01-15T00:00:00Z")
}

fn addr(name: Option<&str>, email: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        email: email.to_owned(),
    }
}

/// The one summary builder: a plausible base thread, with `tweak` applied to it.
///
/// Base: account A, subject `"Lunch on Friday"`, snippet `"Sounds good, see you at the cafe"`,
/// from `Ada Lovelace <ada@example.com>`, last date 2024-01-10, unread, unstarred, in
/// `Inbox`+`Sent`, labelled `work`, no attachments, not snoozed, not pinned.
fn summary(tweak: impl FnOnce(&mut ThreadSummary)) -> ThreadSummary {
    let mut s = ThreadSummary {
        id: ThreadId::from_uuid(Uuid::from_u128(0x100)),
        account: ACCOUNT_A,
        subject: "Lunch on Friday".to_owned(),
        snippet: "Sounds good, see you at the cafe".to_owned(),
        from: addr(Some("Ada Lovelace"), "ada@example.com"),
        participants: vec![
            addr(Some("Ada Lovelace"), "ada@example.com"),
            addr(Some("Bob Stone"), "bob@example.org"),
        ],
        recipients: Vec::new(),
        last_date: ts("2024-01-10T12:00:00Z"),
        message_count: 2,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailboxes: MailboxSet::only(MailboxRole::Inbox).with(MailboxRole::Sent),
        labels: vec![LABEL_WORK],
        attachments: Attachments::None,
        snooze: Snooze::Inactive,
        pin: Pin::Unpinned,
    };
    tweak(&mut s);
    s
}

#[derive(Debug)]
struct Case {
    name: &'static str,
    filter: Filter,
    summary: ThreadSummary,
    body: Option<&'static str>,
    now: DateTime<Utc>,
    want: bool,
}

impl Case {
    /// A case against the base summary, with no body text, at the fixed `now`.
    fn new(name: &'static str, filter: Filter, want: bool) -> Self {
        Case {
            name,
            filter,
            summary: summary(|_| {}),
            body: None,
            now: now(),
            want,
        }
    }

    fn on(mut self, tweak: impl FnOnce(&mut ThreadSummary)) -> Self {
        tweak(&mut self.summary);
        self
    }

    fn body(mut self, text: &'static str) -> Self {
        self.body = Some(text);
        self
    }

    fn at(mut self, now: DateTime<Utc>) -> Self {
        self.now = now;
        self
    }
}

fn contains(s: &str) -> TextMatch {
    TextMatch::Contains(s.to_owned())
}

fn exact(s: &str) -> TextMatch {
    TextMatch::Exact(s.to_owned())
}

fn not(f: Filter) -> Filter {
    Filter::Not(Box::new(f))
}

fn cases() -> Vec<Case> {
    vec![
        // ---------------------------------------------------------------- constants
        Case::new("All matches", Filter::All, true),
        // `All` has no non-matching thread by construction; its negation is the counterpart.
        Case::new("Not(All) matches nothing", not(Filter::All), false),
        Case::new("Nothing matches nothing", Filter::Nothing, false),
        Case::new("Not(Nothing) matches", not(Filter::Nothing), true),
        // ---------------------------------------------------------------- connectives
        Case::new(
            "And of two truths",
            Filter::And(vec![Filter::All, Filter::All]),
            true,
        ),
        Case::new(
            "And with one falsehood",
            Filter::And(vec![Filter::All, Filter::Nothing]),
            false,
        ),
        // Identity of `all`: an empty conjunction is true.
        Case::new("empty And is true", Filter::And(vec![]), true),
        Case::new(
            "Or with one truth",
            Filter::Or(vec![Filter::Nothing, Filter::All]),
            true,
        ),
        Case::new(
            "Or of two falsehoods",
            Filter::Or(vec![Filter::Nothing, Filter::Nothing]),
            false,
        ),
        // Identity of `any`: an empty disjunction is false.
        Case::new("empty Or is false", Filter::Or(vec![]), false),
        Case::new("Not inverts a match", not(Filter::All), false),
        Case::new(
            "Not inverts a non-match",
            not(Filter::Account(ACCOUNT_B)),
            true,
        ),
        Case::new("Not(Not(All)) is All", not(not(Filter::All)), true),
        Case::new(
            "Not(Not(Nothing)) is Nothing",
            not(not(Filter::Nothing)),
            false,
        ),
        Case::new(
            "Not(Not(Not(All))) is false",
            not(not(not(Filter::All))),
            false,
        ),
        Case::new(
            "nested Not under And",
            Filter::And(vec![not(Filter::Nothing), not(not(Filter::All))]),
            true,
        ),
        // ---------------------------------------------------------------- account
        Case::new("Account matches", Filter::Account(ACCOUNT_A), true),
        Case::new(
            "Account of another account",
            Filter::Account(ACCOUNT_B),
            false,
        ),
        // ---------------------------------------------------------------- mailbox
        Case::new(
            "InMailbox hits a role in the union",
            Filter::InMailbox(MailboxRole::Inbox),
            true,
        ),
        Case::new(
            "InMailbox hits the second role in the union",
            Filter::InMailbox(MailboxRole::Sent),
            true,
        ),
        Case::new(
            "InMailbox misses a role not in the union",
            Filter::InMailbox(MailboxRole::Spam),
            false,
        ),
        Case::new(
            "InMailbox on an empty set",
            Filter::InMailbox(MailboxRole::Inbox),
            false,
        )
        .on(|s| s.mailboxes = MailboxSet::empty()),
        // ---------------------------------------------------------------- flags
        Case::new(
            "Read(Unread) on an unread thread",
            Filter::Read(ReadState::Unread),
            true,
        ),
        Case::new(
            "Read(Read) on an unread thread",
            Filter::Read(ReadState::Read),
            false,
        ),
        Case::new(
            "Read(Read) on a read thread",
            Filter::Read(ReadState::Read),
            true,
        )
        .on(|s| s.read = ReadState::Read),
        Case::new(
            "Starred(Unstarred) on an unstarred thread",
            Filter::Starred(Star::Unstarred),
            true,
        ),
        Case::new(
            "Starred(Starred) on an unstarred thread",
            Filter::Starred(Star::Starred),
            false,
        ),
        Case::new(
            "Starred(Starred) on a starred thread",
            Filter::Starred(Star::Starred),
            true,
        )
        .on(|s| s.star = Star::Starred),
        // ---------------------------------------------------------------- labels
        Case::new("HasLabel present", Filter::HasLabel(LABEL_WORK), true),
        Case::new("HasLabel absent", Filter::HasLabel(LABEL_HOME), false),
        Case::new(
            "HasLabel on an unlabelled thread",
            Filter::HasLabel(LABEL_WORK),
            false,
        )
        .on(|s| s.labels.clear()),
        // ---------------------------------------------------------------- from
        Case::new(
            "From contains part of the email",
            Filter::From(contains("ada@")),
            true,
        ),
        Case::new(
            "From contains part of the display name",
            Filter::From(contains("lovelace")),
            true,
        ),
        Case::new(
            "From exact on the whole email",
            Filter::From(exact("ADA@example.com")),
            true,
        ),
        Case::new(
            "From exact on the whole display name",
            Filter::From(exact("ada lovelace")),
            true,
        ),
        Case::new(
            "From exact on a prefix of the email is not a match",
            Filter::From(exact("ada")),
            false,
        ),
        Case::new(
            "From misses a different sender",
            Filter::From(contains("bob@example.org")),
            false,
        ),
        // `from` is the newest sender only; the older participant does not satisfy `From`.
        Case::new(
            "From does not consult participants",
            Filter::From(contains("bob")),
            false,
        ),
        Case::new(
            "From against a nameless sender",
            Filter::From(contains("lovelace")),
            false,
        )
        .on(|s| s.from = addr(None, "ada@example.com")),
        // ---------------------------------------------------------------- to
        // `To` is unsatisfiable from a summary alone; see `to_fits` in filter.rs. Both of
        // these would match if `To` consulted participants or fell back to "always true".
        Case::new(
            "To never matches, even a real participant",
            Filter::To(exact("bob@example.org")),
            false,
        ),
        Case::new(
            "To never matches, even the empty substring",
            Filter::To(contains("")),
            false,
        ),
        Case::new(
            "Not(To) is therefore always true",
            not(Filter::To(contains(""))),
            true,
        ),
        // ---------------------------------------------------------------- subject
        Case::new(
            "Subject contains, case-insensitively",
            Filter::Subject(contains("LUNCH")),
            true,
        ),
        Case::new(
            "Subject exact, case-insensitively",
            Filter::Subject(exact("lunch on friday")),
            true,
        ),
        Case::new(
            "Subject exact on a substring is not a match",
            Filter::Subject(exact("Lunch")),
            false,
        ),
        Case::new("Subject misses", Filter::Subject(contains("dinner")), false),
        // ---------------------------------------------------------------- text: scope
        Case::new(
            "Text hits the subject",
            Filter::Text(contains("friday")),
            true,
        ),
        Case::new(
            "Text hits the sender's name",
            Filter::Text(contains("Lovelace")),
            true,
        ),
        Case::new(
            "Text hits the sender's email",
            Filter::Text(contains("example.com")),
            true,
        ),
        Case::new(
            "Text hits the snippet",
            Filter::Text(contains("cafe")),
            true,
        ),
        // `Text` searches every participant; `From` is the newest sender only. Adjacent, so
        // the difference in scope between the two clauses stays pinned.
        Case::new(
            "Text consults participants, not just the newest sender",
            Filter::Text(contains("bob")),
            true,
        ),
        Case::new(
            "From consults only the newest sender",
            Filter::From(contains("bob")),
            false,
        ),
        // The same needle, present only in the body: absent with `None`, found with `Some`.
        Case::new(
            "Text with body_text None does not see the body",
            Filter::Text(contains("quarterly")),
            false,
        ),
        Case::new(
            "Text with body_text Some sees the body",
            Filter::Text(contains("quarterly")),
            true,
        )
        .body("The quarterly report is attached."),
        Case::new(
            "Text misses everywhere, body absent",
            Filter::Text(contains("zeppelin")),
            false,
        ),
        Case::new(
            "Text misses everywhere, body present",
            Filter::Text(contains("zeppelin")),
            false,
        )
        .body("The quarterly report is attached."),
        // ---------------------------------------------------------------- text: whole words
        // The case this semantics exists for: a partial word is not a full-text hit, because
        // an FTS5 index cannot answer one.
        Case::new(
            "Text Contains does NOT match a partial word",
            Filter::Text(contains("xamp")),
            false,
        ),
        Case::new(
            "Text Contains matches the whole word",
            Filter::Text(contains("example")),
            true,
        ),
        // ... while the LIKE-backed clauses still do match a partial word. Adjacent, on
        // purpose: this is the asymmetry a reader will otherwise call a bug.
        Case::new(
            "From Contains still matches a partial word",
            Filter::From(contains("xamp")),
            true,
        ),
        Case::new(
            "Subject Contains still matches across a word boundary",
            Filter::Subject(contains("unch on fri")),
            true,
        ),
        Case::new(
            "Text Contains does not match across a word boundary",
            Filter::Text(contains("unch on fri")),
            false,
        ),
        Case::new(
            "Text Contains does not match a word prefix",
            Filter::Text(contains("frida")),
            false,
        ),
        Case::new(
            "Text Contains does not match a word with a suffix added",
            Filter::Text(contains("fridays")),
            false,
        ),
        Case::new(
            "Text folds case like every other clause",
            Filter::Text(contains("LUNCH")),
            true,
        ),
        Case::new(
            "Text splits on punctuation, so a bare word in an address matches",
            Filter::Text(contains("ada")),
            true,
        ),
        // ---------------------------------------------------------------- text: multi-token
        // Multi-token `Contains` is CO-OCCURRENCE: order, adjacency and field are all free.
        Case::new(
            "Text Contains matches two words out of order",
            Filter::Text(contains("friday lunch")),
            true,
        ),
        Case::new(
            "Text Contains matches two words from different fields",
            Filter::Text(contains("lovelace friday")),
            true,
        ),
        Case::new(
            "Text Contains needs every word, not any",
            Filter::Text(contains("lunch dinner")),
            false,
        ),
        Case::new(
            "Text Contains tokenizes the needle on punctuation too",
            Filter::Text(contains("ada@example.com")),
            true,
        ),
        // Multi-token `Exact` is a PHRASE: adjacent, in order, within one field.
        Case::new(
            "Text Exact matches an adjacent in-order phrase",
            Filter::Text(exact("lunch on friday")),
            true,
        ),
        Case::new(
            "Text Exact matches a phrase inside a longer field",
            Filter::Text(exact("on friday")),
            true,
        ),
        Case::new(
            "Text Exact rejects the same words out of order",
            Filter::Text(exact("friday on lunch")),
            false,
        ),
        Case::new(
            "Text Exact rejects non-adjacent words",
            Filter::Text(exact("lunch friday")),
            false,
        ),
        Case::new(
            "Text Exact does not span two fields",
            Filter::Text(exact("friday sounds")),
            false,
        ),
        Case::new(
            "Text Exact of one word is a whole-word match, not whole-value",
            Filter::Text(exact("lunch")),
            true,
        ),
        Case::new(
            "Text Exact of one word still needs the whole word",
            Filter::Text(exact("lunc")),
            false,
        ),
        Case::new(
            "Text Exact matches a phrase in a display name",
            Filter::Text(exact("ada lovelace")),
            true,
        ),
        // `Exact` implies `Contains` for the same needle.
        Case::new(
            "Text Exact implies Text Contains",
            Filter::And(vec![
                Filter::Text(exact("lunch on friday")),
                Filter::Text(contains("lunch on friday")),
            ]),
            true,
        ),
        // ---------------------------------------------------------------- text: no tokens
        // A needle with no tokens matches nothing, for both — FTS5 has no empty MATCH. This is
        // the one place `Contains("")` is not vacuously true, so both readings sit adjacent.
        Case::new(
            "Text Contains with an empty needle matches nothing",
            Filter::Text(contains("")),
            false,
        ),
        Case::new(
            "Subject Contains with an empty needle still matches everything",
            Filter::Subject(contains("")),
            true,
        ),
        Case::new(
            "Text Exact with an empty needle matches nothing",
            Filter::Text(exact("")),
            false,
        ),
        Case::new(
            "Text with a punctuation-only needle matches nothing",
            Filter::Text(contains("!!! --- ???")),
            false,
        ),
        Case::new(
            "Not(Text(empty)) is therefore true",
            not(Filter::Text(contains(""))),
            true,
        ),
        // ---------------------------------------------------------------- date
        Case::new(
            "Date inside the range",
            Filter::Date(DateRange {
                from: Some(ts("2024-01-01T00:00:00Z")),
                to: Some(ts("2024-02-01T00:00:00Z")),
            }),
            true,
        ),
        // Half-open `[from, to)`: the lower bound is included.
        Case::new(
            "Date exactly at `from` is included",
            Filter::Date(DateRange {
                from: Some(ts("2024-01-10T12:00:00Z")),
                to: Some(ts("2024-02-01T00:00:00Z")),
            }),
            true,
        ),
        // ... and the upper bound is excluded.
        Case::new(
            "Date exactly at `to` is excluded",
            Filter::Date(DateRange {
                from: Some(ts("2024-01-01T00:00:00Z")),
                to: Some(ts("2024-01-10T12:00:00Z")),
            }),
            false,
        ),
        Case::new(
            "Date one second before `to` is included",
            Filter::Date(DateRange {
                from: None,
                to: Some(ts("2024-01-10T12:00:01Z")),
            }),
            true,
        ),
        Case::new(
            "Date one second after `from` is excluded",
            Filter::Date(DateRange {
                from: Some(ts("2024-01-10T12:00:01Z")),
                to: None,
            }),
            false,
        ),
        Case::new(
            "Date with an open upper bound",
            Filter::Date(DateRange {
                from: Some(ts("2024-01-01T00:00:00Z")),
                to: None,
            }),
            true,
        ),
        Case::new(
            "Date with an open lower bound",
            Filter::Date(DateRange {
                from: None,
                to: Some(ts("2024-02-01T00:00:00Z")),
            }),
            true,
        ),
        Case::new(
            "Date fully open matches everything",
            Filter::Date(DateRange::default()),
            true,
        ),
        Case::new(
            "Date entirely before the thread",
            Filter::Date(DateRange {
                from: Some(ts("2023-01-01T00:00:00Z")),
                to: Some(ts("2023-02-01T00:00:00Z")),
            }),
            false,
        ),
        Case::new(
            "Date is measured against last_date, not now",
            Filter::Date(DateRange {
                from: Some(ts("2024-01-14T00:00:00Z")),
                to: None,
            }),
            false,
        ),
        // ---------------------------------------------------------------- attachments
        Case::new(
            "HasAttachment with attachments",
            Filter::HasAttachment,
            true,
        )
        .on(|s| s.attachments = Attachments::Present { count: 2 }),
        Case::new(
            "HasAttachment with a single attachment",
            Filter::HasAttachment,
            true,
        )
        .on(|s| s.attachments = Attachments::Present { count: 1 }),
        Case::new("HasAttachment with none", Filter::HasAttachment, false),
        // ---------------------------------------------------------------- snooze
        Case::new("Snoozed into the future", Filter::Snoozed, true)
            .on(|s| s.snooze = Snooze::Until(ts("2024-02-01T00:00:00Z"))),
        Case::new(
            "Snoozed into the past is still snoozed",
            Filter::Snoozed,
            true,
        )
        .on(|s| s.snooze = Snooze::Until(ts("2024-01-01T00:00:00Z"))),
        Case::new("Snoozed when inactive", Filter::Snoozed, false),
        Case::new(
            "SnoozeDue when the instant has passed",
            Filter::SnoozeDue,
            true,
        )
        .on(|s| s.snooze = Snooze::Until(ts("2024-01-14T23:59:59Z"))),
        // Boundary: `t <= now` is due. Exactly at `now` counts.
        Case::new("SnoozeDue exactly at now", Filter::SnoozeDue, true)
            .on(|s| s.snooze = Snooze::Until(now())),
        Case::new("SnoozeDue one second after now", Filter::SnoozeDue, false)
            .on(|s| s.snooze = Snooze::Until(ts("2024-01-15T00:00:01Z"))),
        Case::new("SnoozeDue when inactive", Filter::SnoozeDue, false),
        // `SnoozeDue` implies `Snoozed`: the same thread satisfies both.
        Case::new(
            "SnoozeDue implies Snoozed",
            Filter::And(vec![Filter::SnoozeDue, Filter::Snoozed]),
            true,
        )
        .on(|s| s.snooze = Snooze::Until(now())),
        Case::new(
            "due-ness moves with now, not with the thread",
            Filter::SnoozeDue,
            true,
        )
        .on(|s| s.snooze = Snooze::Until(ts("2024-01-15T00:00:01Z")))
        .at(ts("2024-03-01T00:00:00Z")),
        // ---------------------------------------------------------------- pin
        Case::new("Pinned with a rank", Filter::Pinned, true).on(|s| s.pin = Pin::Rank(1_000)),
        Case::new("Pinned with rank zero", Filter::Pinned, true).on(|s| s.pin = Pin::Rank(0)),
        Case::new("Pinned when unpinned", Filter::Pinned, false),
        // ---------------------------------------------------------------- case folding
        // ASCII-only folding, asserted rather than assumed. See `text_fits` in filter.rs.
        Case::new(
            "an accented needle folds only its ASCII half: É does not reach é",
            Filter::Subject(contains("ÉCOLE DE PARIS")),
            false,
        )
        .on(|s| s.subject = "école de paris".to_owned()),
        Case::new(
            "the ASCII part of a non-ASCII subject still folds",
            Filter::Subject(contains("COLE DE PARIS")),
            true,
        )
        .on(|s| s.subject = "École de Paris".to_owned()),
        Case::new(
            "non-ASCII characters match themselves",
            Filter::Subject(contains("École")),
            true,
        )
        .on(|s| s.subject = "École de Paris".to_owned()),
        Case::new(
            "non-ASCII case is NOT folded: É does not match é",
            Filter::Subject(exact("École")),
            false,
        )
        .on(|s| s.subject = "école".to_owned()),
        Case::new(
            "non-ASCII exact matches when the case already agrees",
            Filter::Subject(exact("école")),
            true,
        )
        .on(|s| s.subject = "école".to_owned()),
        Case::new(
            "ß is not folded to ss",
            Filter::Subject(contains("STRASSE")),
            false,
        )
        .on(|s| s.subject = "Straße".to_owned()),
        Case::new(
            "Turkish dotless i is not folded to ASCII i",
            Filter::Subject(exact("ISTANBUL")),
            false,
        )
        .on(|s| s.subject = "İstanbul".to_owned()),
        Case::new(
            "an empty Contains matches any field",
            Filter::Subject(contains("")),
            true,
        ),
        Case::new(
            "an empty Exact matches only an empty field",
            Filter::Subject(exact("")),
            false,
        ),
        Case::new(
            "an empty Exact matches an empty subject",
            Filter::Subject(exact("")),
            true,
        )
        .on(|s| s.subject.clear()),
        // ------------------------------------------------- folding: Text vs the rest
        // The intended asymmetry, pinned in adjacent pairs: `Text` folds diacritics because
        // FTS5's `remove_diacritics 2` does; `Subject`/`From`/`To` do not, because SQL `LIKE`
        // does not. Neither is a bug; both are load-bearing for the parity proptest.
        Case::new(
            "Text folds diacritics: resume finds Résumé",
            Filter::Text(contains("resume")),
            true,
        )
        .on(|s| s.subject = "Résumé for Ada".to_owned()),
        Case::new(
            "Subject does NOT fold diacritics: resume misses Résumé",
            Filter::Subject(contains("resume")),
            false,
        )
        .on(|s| s.subject = "Résumé for Ada".to_owned()),
        Case::new(
            "Subject matches Résumé written out in full",
            Filter::Subject(contains("Résumé")),
            true,
        )
        .on(|s| s.subject = "Résumé for Ada".to_owned()),
        Case::new(
            "Text folds the needle as well as the field",
            Filter::Text(contains("RÉSUMÉ")),
            true,
        )
        .on(|s| s.subject = "resume for ada".to_owned()),
        Case::new(
            "Text phrases fold too",
            Filter::Text(exact("resume for ada")),
            true,
        )
        .on(|s| s.subject = "Résumé for Ada".to_owned()),
        // Decomposed input: `e` + U+0301 is one token, not two.
        Case::new(
            "Text folds decomposed accents",
            Filter::Text(contains("resume")),
            true,
        )
        .on(|s| s.subject = "Re\u{301}sume\u{301} for Ada".to_owned()),
        Case::new(
            "Subject sees the combining mark and misses",
            Filter::Subject(contains("resume")),
            false,
        )
        .on(|s| s.subject = "Re\u{301}sume\u{301} for Ada".to_owned()),
        // A letter with no ASCII base is not "an accented s": neither clause folds ß, and
        // SQLite does not either. The documented limit, asserted.
        Case::new(
            "Text does not fold ß to ss either",
            Filter::Text(contains("strasse")),
            false,
        )
        .on(|s| s.subject = "Straße 7".to_owned()),
        Case::new(
            "Text matches ß written as itself",
            Filter::Text(contains("straße")),
            true,
        )
        .on(|s| s.subject = "Straße 7".to_owned()),
        // Non-Latin scripts tokenize and case-fold; they are simply not transliterated.
        Case::new(
            "Text tokenizes a non-Latin subject",
            Filter::Text(contains("Москва")),
            true,
        )
        .on(|s| s.subject = "письмо из москвы: москва".to_owned()),
        Case::new(
            "Text still needs the whole non-Latin word",
            Filter::Text(contains("моск")),
            false,
        )
        .on(|s| s.subject = "письмо из москвы: москва".to_owned()),
    ]
}

#[test]
fn fit_decides_every_case() {
    for case in cases() {
        let ctx = MatchCtx {
            summary: &case.summary,
            corpus: case.body,
            now: case.now,
        };
        assert_eq!(
            case.filter.fit(&ctx),
            case.want,
            "case {:?}: expected fit == {}, filter = {:?}",
            case.name,
            case.want,
            case.filter,
        );
    }
}

/// `Filter::To` was always-false until `ThreadSummary` gained `recipients` (FINDINGS F1).
/// It now answers honestly, against To and Cc only — never Bcc.
#[test]
fn to_matches_thread_recipients() {
    let s = summary(|s| {
        s.recipients = vec![
            Address {
                name: Some("Ada Lovelace".into()),
                email: "ada@example.test".into(),
            },
            Address {
                name: None,
                email: "team@example.test".into(),
            },
        ];
    });
    fn ctx(sum: &ThreadSummary) -> MatchCtx<'_> {
        MatchCtx {
            summary: sum,
            corpus: None,
            now: now(),
        }
    }

    assert!(Filter::To(TextMatch::Contains("team@".into())).fit(&ctx(&s)));
    assert!(
        Filter::To(TextMatch::Contains("lovelace".into())).fit(&ctx(&s)),
        "a display name is addressable too"
    );
    assert!(!Filter::To(TextMatch::Contains("nobody@".into())).fit(&ctx(&s)));
    assert!(Filter::To(TextMatch::Exact("ada@example.test".into())).fit(&ctx(&s)));
    // Substring, not word-match: To goes through SQL LIKE, unlike Filter::Text.
    assert!(Filter::To(TextMatch::Contains("eam@exa".into())).fit(&ctx(&s)));

    let empty = summary(|s| s.recipients = Vec::new());
    assert!(!Filter::To(TextMatch::Contains("ada".into())).fit(&ctx(&empty)));
}

/// Free text finds a thread by who it was addressed to, not only by who wrote in it.
#[test]
fn text_matches_thread_recipients() {
    let s = summary(|s| {
        s.recipients = vec![Address {
            name: Some("Grace Hopper".into()),
            email: "grace@navy.test".into(),
        }];
    });
    let ctx = MatchCtx {
        summary: &s,
        corpus: None,
        now: now(),
    };
    assert!(Filter::Text(TextMatch::Contains("hopper".into())).fit(&ctx));
    assert!(Filter::Text(TextMatch::Contains("navy".into())).fit(&ctx));
    assert!(!Filter::Text(TextMatch::Contains("babbage".into())).fit(&ctx));
}
