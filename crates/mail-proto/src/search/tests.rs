//! What each protocol is asked for each query, as tables: one row per query, what goes on the
//! wire (or into the JSON, or the URL), and every query that cannot be asked, with what is said
//! instead. Nothing here is widened: a clause a protocol cannot ask makes the whole query
//! unsaid, and the table names it.

use super::graph::{self, GraphPlace, GraphPlan, GraphQuery};
use super::imap::{self, Hits, ImapCtx, ImapPlan, SearchKey};
use super::jmap::{self, JmapCtx};
use super::{Asked, Unsaid};
use crate::imap::Untagged;
use crate::jmap::Mailboxes;
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use serde_json::{Value, json};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const OTHER: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"));
const TRAVEL: LabelId = LabelId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-00000000c0de"));

fn contains(s: &str) -> TextMatch {
    TextMatch::Contains(s.to_owned())
}

fn text(s: &str) -> Filter {
    Filter::Text(contains(s))
}

fn and(parts: Vec<Filter>) -> Filter {
    Filter::And(parts)
}

fn not(f: Filter) -> Filter {
    Filter::Not(Box::new(f))
}

/// Midnight of a day in Taipei, UTC+8, as `after:`/`before:` resolve there.
fn taipei_midnight(y: i32, m: u32, d: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap() - chrono::TimeDelta::hours(8)
}

fn between(from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>) -> Filter {
    Filter::Date(DateRange { from, to })
}

fn folder(path: &str, special: Option<SpecialUse>) -> Folder {
    Folder {
        account: ACCOUNT,
        path: path.to_owned(),
        delimiter: Some('/'),
        special,
        subscription: Subscription::Subscribed,
        holds: Holds::Mail,
    }
}

fn roles(pairs: &[(&str, MailboxRole)]) -> FolderRoles {
    FolderRoles(pairs.iter().map(|(p, r)| ((*p).to_owned(), *r)).collect())
}

/// One query: its name, the filter, and what is asked or what is said instead.
type Row<T> = (&'static str, Filter, Result<T, Vec<&'static str>>);

fn unsaid(parts: &[&str]) -> Unsaid {
    Unsaid(parts.iter().map(|p| (*p).to_owned()).collect())
}

// ---------------------------------------------------------------------------------------------
// IMAP

/// A plan as it goes out: mailboxes, and the command with every part joined and `|` where the
/// client waits for the server's `+`.
fn wire(plan: &ImapPlan, caps: &[&str]) -> (Vec<String>, String) {
    let caps: Vec<String> = caps.iter().map(|c| format!("Atom(\"{c}\")")).collect();
    let parts = imap::command("a1", &plan.keys, &caps).unwrap();
    let joined = parts
        .iter()
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect::<Vec<_>>()
        .join("|");
    (plan.mailboxes.clone(), joined)
}

type ImapRow = (
    &'static str,
    Filter,
    Result<(Vec<&'static str>, &'static str), Vec<&'static str>>,
);

fn generic_rows() -> Vec<ImapRow> {
    let anywhere =
        "OR SUBJECT \"lunch\" OR FROM \"lunch\" OR TO \"lunch\" OR CC \"lunch\" BODY \"lunch\"";
    let both = Box::leak(
        format!(
            "a1 UID SEARCH ({anywhere} {})\r\n",
            anywhere.replace("lunch", "friday")
        )
        .into_boxed_str(),
    );
    let one = Box::leak(format!("a1 UID SEARCH {anywhere}\r\n").into_boxed_str());
    let inbox_archive = vec!["INBOX", "Archive"];
    vec![
        (
            "from:ada",
            Filter::From(contains("ada")),
            Ok((inbox_archive.clone(), "a1 UID SEARCH FROM \"ada\"\r\n")),
        ),
        (
            "a word, in every field the index holds",
            text("lunch"),
            Ok((inbox_archive.clone(), one)),
        ),
        (
            "two words, each found",
            text("lunch friday"),
            Ok((inbox_archive.clone(), both)),
        ),
        (
            "a phrase, found whole",
            Filter::Text(TextMatch::Exact("on friday".to_owned())),
            Ok((
                inbox_archive.clone(),
                "a1 UID SEARCH OR SUBJECT \"on friday\" OR FROM \"on friday\" OR TO \"on friday\" OR CC \"on friday\" BODY \"on friday\"\r\n",
            )),
        ),
        (
            "is:unread to:bob — to: is To or Cc, as the store's is",
            and(vec![
                Filter::Read(ReadState::Unread),
                Filter::To(contains("bob")),
            ]),
            Ok((
                inbox_archive.clone(),
                "a1 UID SEARCH UNSEEN OR TO \"bob\" CC \"bob\"\r\n",
            )),
        ),
        (
            "-from:news is:starred",
            and(vec![
                not(Filter::From(contains("news"))),
                Filter::Starred(Star::Starred),
            ]),
            Ok((
                inbox_archive.clone(),
                "a1 UID SEARCH NOT FROM \"news\" FLAGGED\r\n",
            )),
        ),
        (
            "in:sent chooses the mailbox",
            and(vec![
                Filter::InMailbox(MailboxRole::Sent),
                Filter::Subject(contains("x")),
            ]),
            Ok((vec!["Sent"], "a1 UID SEARCH SUBJECT \"x\"\r\n")),
        ),
        (
            "a folder chooses the mailbox",
            Filter::InFolder(MailboxRef {
                account: ACCOUNT,
                path: "Projects/2026".to_owned(),
            }),
            Ok((vec!["Projects/2026"], "a1 UID SEARCH ALL\r\n")),
        ),
        (
            "after: and before: typed in Taipei are the days typed",
            between(
                Some(taipei_midnight(2026, 1, 10)),
                Some(taipei_midnight(2026, 1, 12)),
            ),
            Ok((
                inbox_archive.clone(),
                "a1 UID SEARCH (SENTSINCE 10-Jan-2026 SENTBEFORE 12-Jan-2026)\r\n",
            )),
        ),
        (
            "one day typed in Taipei is still that day",
            between(
                Some(taipei_midnight(2026, 1, 10)),
                Some(taipei_midnight(2026, 1, 11)),
            ),
            Ok((
                inbox_archive.clone(),
                "a1 UID SEARCH (SENTSINCE 10-Jan-2026 SENTBEFORE 11-Jan-2026)\r\n",
            )),
        ),
        (
            "a quotation mark in a string is escaped",
            Filter::Subject(contains("say \"hi\"")),
            Ok((
                inbox_archive.clone(),
                "a1 UID SEARCH SUBJECT \"say \\\"hi\\\"\"\r\n",
            )),
        ),
        (
            "an account clause for this account is dropped",
            and(vec![
                Filter::Account(ACCOUNT),
                Filter::From(contains("ada")),
            ]),
            Ok((inbox_archive.clone(), "a1 UID SEARCH FROM \"ada\"\r\n")),
        ),
        (
            "is:pinned is this client's own",
            and(vec![Filter::Pinned, text("budget")]),
            Err(vec!["is:pinned (kept on this computer)"]),
        ),
        (
            "is:snoozed is this client's own",
            Filter::Snoozed,
            Err(vec!["is:snoozed (kept on this computer)"]),
        ),
        (
            "has:attachment has no IMAP key, and is not dropped",
            and(vec![Filter::HasAttachment, Filter::From(contains("ada"))]),
            Err(vec!["has:attachment (IMAP has no search for it)"]),
        ),
        (
            "every clause that cannot be asked is named",
            and(vec![Filter::Pinned, Filter::HasAttachment]),
            Err(vec![
                "is:pinned (kept on this computer)",
                "has:attachment (IMAP has no search for it)",
            ]),
        ),
        (
            "label: where the server keeps no labels",
            Filter::HasLabel(TRAVEL),
            Err(vec!["label: (the server keeps no labels to search)"]),
        ),
        (
            "a place under OR cannot choose a mailbox",
            Filter::Or(vec![Filter::InMailbox(MailboxRole::Sent), text("lunch")]),
            Err(vec!["in:sent inside another clause"]),
        ),
        (
            "a place under NOT cannot either",
            and(vec![
                not(Filter::InMailbox(MailboxRole::Inbox)),
                text("lunch"),
            ]),
            Err(vec!["in:inbox inside another clause"]),
        ),
        (
            "two places at once",
            and(vec![
                Filter::InMailbox(MailboxRole::Sent),
                Filter::InMailbox(MailboxRole::Inbox),
            ]),
            Err(vec!["in:inbox and in:sent at once"]),
        ),
        (
            "in:trash where the server has no Trash",
            Filter::InMailbox(MailboxRole::Trash),
            Err(vec!["in:trash (the server names no such folder)"]),
        ),
    ]
}

#[test]
fn imap_asks_each_clause_of_its_own_field_and_names_what_it_cannot() {
    let folders = [
        folder("INBOX", Some(SpecialUse::Inbox)),
        folder("Archive", Some(SpecialUse::Archive)),
    ];
    let roles = roles(&[
        ("Archive", MailboxRole::Archive),
        ("Sent", MailboxRole::Sent),
    ]);
    let named = [(TRAVEL, "travel".to_owned())];
    let ctx = ImapCtx {
        account: ACCOUNT,
        folders: &folders,
        roles: &roles,
        labels: ServerLabels::LocalOnly,
        labels_named: &named,
    };
    for (name, filter, expected) in generic_rows() {
        let got = imap::translate(&filter, &ctx);
        match (got, expected) {
            (Ok(Asked::Ask(plan)), Ok((mailboxes, line))) => {
                let (got_boxes, got_line) = wire(&plan, &[]);
                assert_eq!(got_boxes, mailboxes, "{name}: mailboxes");
                assert_eq!(got_line, line, "{name}: the command");
            }
            (Err(said), Err(parts)) => assert_eq!(said, unsaid(&parts), "{name}"),
            (got, expected) => panic!("{name}: got {got:?}, expected {expected:?}"),
        }
    }
}

#[test]
fn a_query_that_can_match_nothing_here_asks_nothing() {
    let roles = roles(&[]);
    let ctx = ImapCtx {
        account: ACCOUNT,
        folders: &[],
        roles: &roles,
        labels: ServerLabels::LocalOnly,
        labels_named: &[],
    };
    const CASES: &[&str] = &["another account", "punctuation only", "and with nothing"];
    let filters = [
        Filter::Account(OTHER),
        text("…!?"),
        and(vec![text("lunch"), Filter::Account(OTHER)]),
    ];
    for (name, filter) in CASES.iter().zip(filters) {
        assert_eq!(imap::translate(&filter, &ctx), Ok(Asked::Nothing), "{name}");
        assert_eq!(
            graph::translate(&filter, ACCOUNT),
            Ok(Asked::Nothing),
            "{name}"
        );
    }
}

#[test]
fn on_gmail_all_mail_is_searched_and_labels_are_x_gm_labels() {
    let folders = [
        folder("INBOX", Some(SpecialUse::Inbox)),
        folder("[Gmail]/All Mail", Some(SpecialUse::All)),
        folder("[Gmail]/Sent Mail", Some(SpecialUse::Sent)),
    ];
    let roles = roles(&[
        ("[Gmail]/All Mail", MailboxRole::Archive),
        ("[Gmail]/Sent Mail", MailboxRole::Sent),
    ]);
    let named = [(TRAVEL, "travel".to_owned())];
    let ctx = ImapCtx {
        account: ACCOUNT,
        folders: &folders,
        roles: &roles,
        labels: ServerLabels::Supported,
        labels_named: &named,
    };
    let cases: Vec<(&str, Filter, Vec<&str>, &str)> = vec![
        (
            "no place: All Mail alone",
            Filter::From(contains("ada")),
            vec!["[Gmail]/All Mail"],
            "a1 UID SEARCH FROM \"ada\"\r\n",
        ),
        (
            "label:",
            Filter::HasLabel(TRAVEL),
            vec!["[Gmail]/All Mail"],
            "a1 UID SEARCH X-GM-LABELS \"travel\"\r\n",
        ),
        (
            "in:inbox is the inbox",
            Filter::InMailbox(MailboxRole::Inbox),
            vec!["INBOX"],
            "a1 UID SEARCH ALL\r\n",
        ),
        (
            "in:archive is All Mail less the inbox",
            Filter::InMailbox(MailboxRole::Archive),
            vec!["[Gmail]/All Mail"],
            "a1 UID SEARCH NOT X-GM-LABELS \"\\\\Inbox\"\r\n",
        ),
    ];
    for (name, filter, mailboxes, line) in cases {
        let Ok(Asked::Ask(plan)) = imap::translate(&filter, &ctx) else {
            panic!("{name}: not asked");
        };
        assert_eq!(
            wire(&plan, &[]),
            (
                mailboxes.iter().map(|m| (*m).to_owned()).collect(),
                line.to_owned()
            ),
            "{name}"
        );
    }
}

#[test]
fn the_command_takes_what_the_server_offers() {
    let ascii = ImapPlan {
        mailboxes: vec!["INBOX".to_owned()],
        keys: vec![SearchKey::From("ada".to_owned())],
    };
    let chinese = ImapPlan {
        mailboxes: vec!["INBOX".to_owned()],
        keys: vec![SearchKey::Subject("午餐".to_owned()), SearchKey::Unseen],
    };
    let cases: Vec<(&str, &ImapPlan, Vec<&str>, &str)> = vec![
        ("plain", &ascii, vec![], "a1 UID SEARCH FROM \"ada\"\r\n"),
        (
            "ESEARCH: a count and a compact set",
            &ascii,
            vec!["ESEARCH"],
            "a1 UID SEARCH RETURN (COUNT ALL) FROM \"ada\"\r\n",
        ),
        (
            "not ASCII: CHARSET UTF-8 and a literal the server asks for",
            &chinese,
            vec![],
            "a1 UID SEARCH CHARSET UTF-8 SUBJECT {6}\r\n|午餐 UNSEEN\r\n",
        ),
        (
            "LITERAL+: the literal follows at once",
            &chinese,
            vec!["LITERAL+", "ESEARCH"],
            "a1 UID SEARCH RETURN (COUNT ALL) CHARSET UTF-8 SUBJECT {6+}\r\n午餐 UNSEEN\r\n",
        ),
        (
            "LITERAL- for a short one",
            &chinese,
            vec!["LITERAL-"],
            "a1 UID SEARCH CHARSET UTF-8 SUBJECT {6+}\r\n午餐 UNSEEN\r\n",
        ),
    ];
    for (name, plan, caps, line) in cases {
        assert_eq!(wire(plan, &caps).1, line, "{name}");
    }
}

#[test]
fn a_line_break_in_a_search_string_is_never_sent() {
    let keys = [SearchKey::Subject("a\r\nb003 DELETE INBOX".to_owned())];
    assert!(matches!(
        imap::command("a1", &keys, &[]),
        Err(crate::ProtoError::Malformed(_))
    ));
}

fn untagged(during: usize, text: &str) -> Untagged {
    Untagged {
        during,
        text: text.to_owned(),
        raw: text.as_bytes().to_vec(),
    }
}

#[test]
fn both_answers_are_read_and_a_hostile_one_is_refused() {
    type Row = (&'static str, &'static str, Result<Hits, ()>);
    let cases: &[Row] = &[
        (
            "SEARCH",
            "* SEARCH 12 4 9",
            Ok(Hits {
                uids: vec![4, 9, 12],
                count: 3,
            }),
        ),
        (
            "SEARCH, nothing",
            "* SEARCH",
            Ok(Hits {
                uids: vec![],
                count: 0,
            }),
        ),
        (
            "SEARCH with a CONDSTORE modseq",
            "* SEARCH 2 5 (MODSEQ 917)",
            Ok(Hits {
                uids: vec![2, 5],
                count: 2,
            }),
        ),
        (
            "ESEARCH",
            "* ESEARCH (TAG \"a4\") UID COUNT 5 ALL 4:6,9,11",
            Ok(Hits {
                uids: vec![4, 5, 6, 9, 11],
                count: 5,
            }),
        ),
        (
            "ESEARCH, nothing",
            "* ESEARCH (TAG \"a4\") UID COUNT 0",
            Ok(Hits {
                uids: vec![],
                count: 0,
            }),
        ),
        (
            "ESEARCH, lower case",
            "* esearch (tag \"a4\") uid count 1 all 7",
            Ok(Hits {
                uids: vec![7],
                count: 1,
            }),
        ),
        ("not a UID", "* SEARCH 4 x", Err(())),
        ("UID zero", "* SEARCH 0", Err(())),
        (
            "a range to four billion",
            "* ESEARCH (TAG \"a4\") UID ALL 1:4294967295",
            Err(()),
        ),
    ];
    for (name, line, expected) in cases {
        let got = imap::hits(&[untagged(3, line), untagged(2, "* SEARCH 99")], 3).map_err(|_| ());
        assert_eq!(&got, expected, "{name}");
    }
}

// ---------------------------------------------------------------------------------------------
// JMAP

fn mailboxes() -> Mailboxes {
    Mailboxes::parse(&json!({
        "state": "m1",
        "list": [
            { "id": "mbI", "name": "Inbox", "role": "inbox", "sortOrder": 0 },
            { "id": "mbA", "name": "Archive", "role": "archive", "sortOrder": 1 },
            { "id": "mbS", "name": "Sent", "role": "sent", "sortOrder": 2 },
            { "id": "mbT", "name": "travel", "sortOrder": 3 },
        ]
    }))
    .unwrap()
}

#[test]
fn jmap_carries_the_query_whole_as_a_filter_tree() {
    let boxes = mailboxes();
    let named = [(TRAVEL, "travel".to_owned())];
    let ctx = JmapCtx {
        account: ACCOUNT,
        mailboxes: &boxes,
        labels_named: &named,
    };
    let cases: Vec<Row<Value>> = vec![
        (
            "from:ada",
            Filter::From(contains("ada")),
            Ok(json!({ "from": "ada" })),
        ),
        (
            "two words, each found",
            text("lunch friday"),
            Ok(
                json!({ "operator": "AND", "conditions": [{ "text": "lunch" }, { "text": "friday" }] }),
            ),
        ),
        (
            "to: is To or Cc",
            Filter::To(contains("bob")),
            Ok(json!({ "operator": "OR", "conditions": [{ "to": "bob" }, { "cc": "bob" }] })),
        ),
        (
            "state, attachment and a place",
            and(vec![
                Filter::Read(ReadState::Unread),
                Filter::Starred(Star::Starred),
                Filter::HasAttachment,
                Filter::InMailbox(MailboxRole::Archive),
            ]),
            Ok(json!({ "operator": "AND", "conditions": [
                { "notKeyword": "$seen" }, { "hasKeyword": "$flagged" },
                { "hasAttachment": true }, { "inMailbox": "mbA" },
            ] })),
        ),
        (
            "a place under NOT is fine in JMAP",
            not(Filter::InMailbox(MailboxRole::Inbox)),
            Ok(json!({ "operator": "NOT", "conditions": [{ "inMailbox": "mbI" }] })),
        ),
        (
            "label: is its mailbox",
            Filter::HasLabel(TRAVEL),
            Ok(json!({ "inMailbox": "mbT" })),
        ),
        (
            "dates to the second",
            between(
                Some(taipei_midnight(2026, 1, 10)),
                Some(taipei_midnight(2026, 1, 11)),
            ),
            Ok(json!({ "after": "2026-01-09T16:00:00Z", "before": "2026-01-10T16:00:00Z" })),
        ),
        (
            "is:pinned",
            and(vec![Filter::Pinned, text("x")]),
            Err(vec!["is:pinned (kept on this computer)"]),
        ),
        (
            "in:trash with no Trash",
            Filter::InMailbox(MailboxRole::Trash),
            Err(vec!["in:trash (the server names no such mailbox)"]),
        ),
    ];
    for (name, filter, expected) in cases {
        match (jmap::translate(&filter, &ctx), expected) {
            (Ok(Asked::Ask(got)), Ok(want)) => assert_eq!(got, want, "{name}"),
            (Err(said), Err(parts)) => assert_eq!(said, unsaid(&parts), "{name}"),
            (got, expected) => panic!("{name}: got {got:?}, expected {expected:?}"),
        }
    }
    assert_eq!(
        jmap::translate(&Filter::Account(OTHER), &ctx),
        Ok(Asked::Nothing)
    );
}

#[test]
fn the_jmap_query_keeps_to_what_a_sync_follows_and_counts() {
    let call = jmap::query("A1", json!({ "from": "ada" }), &["mbD".to_owned()], 50, "q");
    assert_eq!(call.name, "Email/query");
    assert_eq!(
        call.args,
        json!({
            "accountId": "A1",
            "filter": { "operator": "AND", "conditions": [
                { "inMailboxOtherThan": ["mbD"] }, { "from": "ada" }
            ] },
            "sort": [{ "property": "receivedAt", "isAscending": false }],
            "position": 0,
            "limit": 50,
            "calculateTotal": true,
        })
    );
}

// ---------------------------------------------------------------------------------------------
// Graph

#[test]
fn graph_uses_filter_for_state_and_search_for_words_never_both() {
    let everywhere = |query: GraphQuery| GraphPlan {
        place: GraphPlace::Everywhere,
        query,
    };
    let cases: Vec<Row<GraphPlan>> = vec![
        ("from:ada", Filter::From(contains("ada")), Ok(everywhere(GraphQuery::Search("from:\"ada\"".to_owned())))),
        (
            "a word, as itself or a participant",
            text("lunch"),
            Ok(everywhere(GraphQuery::Search("(\"lunch\" OR participants:\"lunch\")".to_owned()))),
        ),
        (
            "words and a subject",
            and(vec![text("lunch"), Filter::Subject(contains("plans"))]),
            Ok(everywhere(GraphQuery::Search(
                "((\"lunch\" OR participants:\"lunch\") AND subject:\"plans\")".to_owned(),
            ))),
        ),
        (
            "has:attachment and dates in KQL are whole days",
            and(vec![
                text("report"),
                Filter::HasAttachment,
                between(Some(taipei_midnight(2026, 1, 10)), None),
            ]),
            Ok(everywhere(GraphQuery::Search(
                "((\"report\" OR participants:\"report\") AND hasAttachments:true AND sent>=2026-01-10)".to_owned(),
            ))),
        ),
        (
            "state alone is $filter",
            and(vec![Filter::Read(ReadState::Unread), Filter::Starred(Star::Starred)]),
            Ok(everywhere(GraphQuery::Filter("(isRead eq false and flag/flagStatus eq 'flagged')".to_owned()))),
        ),
        (
            "dates alone are $filter, to the second",
            between(Some(taipei_midnight(2026, 1, 10)), None),
            Ok(everywhere(GraphQuery::Filter("sentDateTime ge 2026-01-09T16:00:00Z".to_owned()))),
        ),
        (
            "a place chooses the folder",
            and(vec![Filter::InMailbox(MailboxRole::Sent), Filter::From(contains("ada"))]),
            Ok(GraphPlan { place: GraphPlace::Role(MailboxRole::Sent), query: GraphQuery::Search("from:\"ada\"".to_owned()) }),
        ),
        (
            "a place alone lists the folder",
            Filter::InMailbox(MailboxRole::Archive),
            Ok(GraphPlan { place: GraphPlace::Role(MailboxRole::Archive), query: GraphQuery::All }),
        ),
        (
            "is:unread with words cannot be both searched and filtered",
            and(vec![Filter::Read(ReadState::Unread), text("lunch")]),
            Err(vec!["is:unread together with words (Graph cannot filter a search by it)"]),
        ),
        (
            "label:",
            and(vec![Filter::HasLabel(TRAVEL), text("x")]),
            Err(vec!["label: (labels stay on this computer for Microsoft accounts)"]),
        ),
        ("is:pinned", and(vec![Filter::Pinned, text("x")]), Err(vec!["is:pinned (kept on this computer)"])),
    ];
    for (name, filter, expected) in cases {
        match (graph::translate(&filter, ACCOUNT), expected) {
            (Ok(Asked::Ask(got)), Ok(want)) => assert_eq!(got, want, "{name}"),
            (Err(said), Err(parts)) => assert_eq!(said, unsaid(&parts), "{name}"),
            (got, expected) => panic!("{name}: got {got:?}, expected {expected:?}"),
        }
    }
}
