//! What a person can ask for, and whether the store answers the same question.
//!
//! `mail-domain` has had a complete query algebra since phase 1, proptested against the SQL that
//! answers it, and every search this application could make was `Text(Contains(the whole line))`.
//! These tests are in two halves: what the parser builds, and — because a filter that parses and
//! does not select is worse than no filter — what the store returns for it.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::{query, view};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn taipei() -> chrono::FixedOffset {
    chrono::FixedOffset::east_opt(8 * 3600).unwrap()
}

fn parse(input: &str) -> Filter {
    query::parse_with(input, &taipei(), &|_| Vec::new())
}

mod what_it_builds {
    use super::*;

    #[test]
    fn a_bare_word_is_still_full_text() {
        assert_eq!(
            parse("lunch"),
            Filter::Text(TextMatch::Contains("lunch".to_owned()))
        );
        // Several words are one query, not several — the FTS index answers co-occurrence.
        assert_eq!(
            parse("lunch friday"),
            Filter::Text(TextMatch::Contains("lunch friday".to_owned()))
        );
    }

    #[test]
    fn fields_become_their_clauses() {
        assert_eq!(
            parse("from:ada"),
            Filter::From(TextMatch::Contains("ada".to_owned()))
        );
        assert_eq!(
            parse("to:bob"),
            Filter::To(TextMatch::Contains("bob".to_owned()))
        );
        assert_eq!(
            parse("subject:lunch"),
            Filter::Subject(TextMatch::Contains("lunch".to_owned()))
        );
        assert_eq!(parse("has:attachment"), Filter::HasAttachment);
        assert_eq!(parse("is:unread"), Filter::Read(ReadState::Unread));
        assert_eq!(parse("is:starred"), Filter::Starred(Star::Starred));
        assert_eq!(parse("is:pinned"), Filter::Pinned);
        assert_eq!(parse("is:snoozed"), Filter::Snoozed);
        assert_eq!(parse("in:archive"), Filter::InMailbox(MailboxRole::Archive));
    }

    #[test]
    fn terms_narrow_rather_than_widen() {
        // Every client works this way and everyone expects it: each word you add finds less.
        assert_eq!(
            parse("from:ada is:unread"),
            Filter::And(vec![
                Filter::From(TextMatch::Contains("ada".to_owned())),
                Filter::Read(ReadState::Unread),
            ])
        );
        // Words and fields together: the words become one text clause at the end.
        assert_eq!(
            parse("from:ada lunch friday"),
            Filter::And(vec![
                Filter::From(TextMatch::Contains("ada".to_owned())),
                Filter::Text(TextMatch::Contains("lunch friday".to_owned())),
            ])
        );
    }

    #[test]
    fn a_minus_negates_the_term_it_is_attached_to() {
        assert_eq!(
            parse("-from:newsletter"),
            Filter::Not(Box::new(Filter::From(TextMatch::Contains(
                "newsletter".to_owned()
            ))))
        );
        // And a bare `-` is a word, not a negation of nothing.
        assert_eq!(
            parse("-"),
            Filter::Text(TextMatch::Contains("-".to_owned()))
        );
    }

    #[test]
    fn a_quoted_run_is_a_phrase_and_a_quoted_value_stays_together() {
        assert_eq!(
            parse("\"lunch on friday\""),
            Filter::Text(TextMatch::Exact("lunch on friday".to_owned()))
        );
        assert_eq!(
            parse("subject:\"lunch on friday\""),
            Filter::Subject(TextMatch::Contains("lunch on friday".to_owned()))
        );
    }

    #[test]
    fn dates_are_read_in_the_readers_zone() {
        // Midnight in Taipei is 16:00 the previous day in UTC. Read as UTC, "after 2026-09-22"
        // would include eight hours of the 21st — someone else's day.
        let Filter::Date(range) = parse("after:2026-09-22") else {
            panic!("not a date filter");
        };
        assert_eq!(
            range.from.unwrap().format("%Y-%m-%d %H:%M").to_string(),
            "2026-09-21 16:00"
        );
        assert!(range.to.is_none());

        // `before` is exclusive and `after` inclusive, which is `DateRange`'s own `>= from` and
        // `< to`: a day named is a whole day, and "before the 25th" must not include it.
        let Filter::Date(range) = parse("before:2026-09-25") else {
            panic!("not a date filter");
        };
        assert!(range.from.is_none());
        assert_eq!(
            range.to.unwrap().format("%Y-%m-%d %H:%M").to_string(),
            "2026-09-24 16:00"
        );
    }

    #[test]
    fn what_it_does_not_recognise_is_searched_for_rather_than_refused() {
        // A box that rejects what is typed while it is being typed is unusable. `frm:` is a
        // typo, not an error — and the result is a search that finds nothing, which is the
        // feedback.
        assert_eq!(
            parse("frm:ada"),
            Filter::Text(TextMatch::Contains("frm:ada".to_owned()))
        );
        assert_eq!(
            parse("is:sideways"),
            Filter::Text(TextMatch::Contains("is:sideways".to_owned()))
        );
        assert_eq!(
            parse("before:not-a-date"),
            Filter::Text(TextMatch::Contains("before:not-a-date".to_owned()))
        );
        // A field with nothing after it is the word so far, mid-typing.
        assert_eq!(
            parse("from:"),
            Filter::Text(TextMatch::Contains("from:".to_owned()))
        );
    }

    #[test]
    fn an_empty_box_is_not_a_filter_that_matches_nothing() {
        // `Filter::All`, so the caller can decide what an empty search means — which for the
        // shell is "the place you were already in".
        assert_eq!(parse(""), Filter::All);
        assert_eq!(parse("   "), Filter::All);
    }

    #[test]
    fn case_in_a_field_name_is_not_part_of_what_was_meant() {
        assert_eq!(parse("FROM:ada"), parse("from:ada"));
        assert_eq!(parse("is:UNREAD"), parse("is:unread"));
        assert_eq!(parse("In:Archive"), parse("in:archive"));
    }
}

/// The half that matters: the store must answer the question the parser asked.
mod what_the_store_returns {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 6, 0, 0).unwrap()
    }

    fn seeded() -> (SqliteStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, 'me@example.test', '{}', datetime('now'))",
                [ACCOUNT.to_string()],
            )
            .unwrap();

        let mail: [(&str, &str, &str, &str); 3] = [
            (
                "ada@example.test",
                "me@example.test",
                "lunch on friday",
                "Tue, 22 Sep 2026 09:00:00 +0800",
            ),
            (
                "bob@example.test",
                "me@example.test",
                "the invoice",
                "Mon, 21 Sep 2026 09:00:00 +0800",
            ),
            (
                "newsletter@example.test",
                "list@example.test",
                "weekly digest",
                "Fri, 01 Aug 2025 09:00:00 +0800",
            ),
        ];
        for (n, (from, to, subject, date)) in mail.iter().enumerate() {
            let raw = format!(
                "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\n\
                 Date: {date}\r\nMessage-ID: <m{n}@example.test>\r\n\r\n\
                 body of {subject}\r\n"
            );
            absorb(
                &store,
                ACCOUNT,
                MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                Some(SyncCursor::Pop),
                vec![Arrival {
                    remote: RemoteRef::Pop {
                        uidl: format!("u{n}"),
                    },
                    raw: raw.into_bytes(),
                }],
                false,
                now(),
            )
            .unwrap();
        }
        (store, dir)
    }

    fn found(store: &SqliteStore, search: &str) -> Vec<String> {
        store
            .threads(
                &Query {
                    filter: parse(search),
                    sort: Sort {
                        property: Property::Date,
                        dir: SortDir::Desc,
                    },
                    page: PageReq {
                        after: None,
                        limit: 50,
                    },
                },
                now(),
            )
            .unwrap()
            .items
            .into_iter()
            .map(|t| t.subject)
            .collect()
    }

    #[test]
    fn a_sender_narrows_to_that_sender() {
        let (store, _dir) = seeded();
        assert_eq!(found(&store, "from:ada"), vec!["lunch on friday"]);
        assert_eq!(found(&store, "from:bob"), vec!["the invoice"]);
    }

    #[test]
    fn a_negated_sender_removes_only_that_one() {
        let (store, _dir) = seeded();
        let rest = found(&store, "-from:newsletter");
        assert_eq!(rest.len(), 2, "{rest:?}");
        assert!(!rest.iter().any(|s| s == "weekly digest"), "{rest:?}");
    }

    #[test]
    fn terms_compose_the_way_the_reading_suggests() {
        let (store, _dir) = seeded();
        // Both true: one thread.
        assert_eq!(
            found(&store, "from:ada subject:lunch"),
            vec!["lunch on friday"]
        );
        // One true, one not: nothing.
        assert!(found(&store, "from:ada subject:invoice").is_empty());
    }

    #[test]
    fn a_date_bound_cuts_where_it_says() {
        let (store, _dir) = seeded();
        // The digest is from last August; the other two are from this September.
        assert_eq!(found(&store, "after:2026-01-01").len(), 2);
        assert_eq!(found(&store, "before:2026-01-01"), vec!["weekly digest"]);
        // Exclusive on `before`: the 22nd's mail is not "before the 22nd".
        assert!(
            !found(&store, "before:2026-09-22")
                .iter()
                .any(|s| s == "lunch on friday")
        );
        assert!(
            found(&store, "after:2026-09-22")
                .iter()
                .any(|s| s == "lunch on friday")
        );
    }

    #[test]
    fn the_thing_that_was_the_only_search_still_works() {
        let (store, _dir) = seeded();
        assert_eq!(found(&store, "invoice"), vec!["the invoice"]);
    }

    #[test]
    fn a_field_and_a_word_together() {
        // The query this whole parser exists for: "from Bob, about the invoice".
        let (store, _dir) = seeded();
        assert_eq!(found(&store, "from:bob invoice"), vec!["the invoice"]);
        assert!(found(&store, "from:ada invoice").is_empty());
    }

    #[test]
    fn a_typo_finds_nothing_rather_than_everything() {
        // The cost of not refusing an unknown field: it becomes text. What it must not do is
        // become `All` and show the whole mailbox as if it had matched.
        let (store, _dir) = seeded();
        assert!(found(&store, "frm:ada").is_empty());
    }
}

/// `label:`, which is the one term that needs the world.
mod labels {
    use super::*;

    const A: LabelId = LabelId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e1"));
    const B: LabelId = LabelId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e2"));

    fn with(found: Vec<LabelId>) -> impl Fn(&str) -> Vec<LabelId> {
        move |_| found.clone()
    }

    #[test]
    fn one_label_with_that_name_is_one_clause() {
        assert_eq!(
            query::parse_with("label:travel", &taipei(), &with(vec![A])),
            Filter::HasLabel(A)
        );
    }

    #[test]
    fn the_same_name_on_two_accounts_means_either() {
        // `UNIQUE (account, name)`, so "travel" on the Gmail account and "travel" on the work one
        // are two labels. Someone typing `label:travel` means the word — taking the first
        // silently searched one mailbox, which is a wrong answer that looks like an empty one.
        assert_eq!(
            query::parse_with("label:travel", &taipei(), &with(vec![A, B])),
            Filter::Or(vec![Filter::HasLabel(A), Filter::HasLabel(B)])
        );
    }

    #[test]
    fn a_name_nothing_bears_is_searched_for_as_text() {
        // The same rule as any other unrecognised term. Not `Filter::Nothing`, which would find
        // nothing and look identical to a label that exists and has no mail.
        assert_eq!(
            query::parse_with("label:nosuch", &taipei(), &with(vec![])),
            Filter::Text(TextMatch::Contains("label:nosuch".to_owned()))
        );
    }

    #[test]
    fn it_still_composes_with_everything_else() {
        assert_eq!(
            query::parse_with("label:travel is:unread", &taipei(), &with(vec![A])),
            Filter::And(vec![Filter::HasLabel(A), Filter::Read(ReadState::Unread)])
        );
    }
}

/// The same search, typed into the window instead of the terminal.
///
/// `label:` is the one term that needs the store, and it arrives as a resolver so the parser can
/// stay pure. `mailo search` passes one. The window called `query::parse`, which passes a
/// resolver that knows no names at all — so every `label:` typed there resolved to nothing,
/// became text by the unknown-term rule, and full-text searched for the literal string
/// "label:travel". No results, no error, and the same query working in the terminal.
mod typed_into_the_window {
    use super::*;

    fn label(n: u8) -> LabelId {
        LabelId::from_uuid(uuid::Uuid::from_bytes([n; 16]))
    }

    fn shell_searching(needle: &str, known: Vec<(String, LabelId)>) -> Filter {
        let mut shell = view::Shell {
            labels: known,
            ..Default::default()
        };
        shell.search = needle.to_owned();
        shell.query(50).filter
    }

    #[test]
    fn a_label_the_window_knows_about_selects_by_it() {
        let travel = label(7);
        let filter = shell_searching("label:travel", vec![("travel".to_owned(), travel)]);
        assert_eq!(
            filter,
            Filter::HasLabel(travel),
            "the window searched for the words instead of the label"
        );
    }

    /// The word can name a label on each account, and someone typing it means the word.
    #[test]
    fn the_same_word_on_two_accounts_matches_both() {
        let (work, home) = (label(1), label(2));
        let filter = shell_searching(
            "label:travel",
            vec![
                ("travel".to_owned(), work),
                ("travel".to_owned(), home),
                ("receipts".to_owned(), label(3)),
            ],
        );
        match filter {
            Filter::Or(any) => assert_eq!(
                any,
                vec![Filter::HasLabel(work), Filter::HasLabel(home)],
                "one account's label was dropped"
            ),
            other => panic!("{other:?}"),
        }
    }

    /// The rule that must survive the fix: a name nothing bears is still text, because a search
    /// box has to keep working while a word is half-typed.
    #[test]
    fn a_name_nothing_bears_is_still_text() {
        let filter = shell_searching("label:trav", vec![("travel".to_owned(), label(7))]);
        assert!(
            !matches!(filter, Filter::HasLabel(_)),
            "a half-typed name should not select a label: {filter:?}"
        );
    }

    /// And the rest of the vocabulary must keep working beside it.
    #[test]
    fn a_label_term_composes_with_the_others() {
        let travel = label(7);
        let filter = shell_searching(
            "label:travel is:unread",
            vec![("travel".to_owned(), travel)],
        );
        match filter {
            Filter::And(all) => assert!(
                all.contains(&Filter::HasLabel(travel)),
                "the label was lost once another term joined it: {all:?}"
            ),
            other => panic!("{other:?}"),
        }
    }
}

/// And the same thing with a real store behind it.
///
/// The pure half above would pass just as well with a field nobody ever fills — which is the
/// failure this project keeps finding, a capability modelled and unreachable. So this seeds a
/// label the way a Gmail sync does, builds the index the window builds, and asks for the mail.
mod a_label_typed_into_the_window_finds_the_mail {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 6, 0, 0).unwrap()
    }

    /// Two messages, one of them labelled `travel` by the server.
    fn seeded() -> (SqliteStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, 'me@example.test', '{}', datetime('now'))",
                [ACCOUNT.to_string()],
            )
            .unwrap();

        for (n, subject) in ["flight to taipei", "the invoice"].iter().enumerate() {
            let raw = format!(
                "From: ada@example.test\r\nTo: me@example.test\r\nSubject: {subject}\r\n\
                 Date: Tue, 22 Sep 2026 09:00:00 +0800\r\n\
                 Message-ID: <m{n}@example.test>\r\n\r\nbody\r\n"
            );
            absorb(
                &store,
                ACCOUNT,
                MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                Some(SyncCursor::Pop),
                vec![Arrival {
                    remote: RemoteRef::Pop {
                        uidl: format!("u{n}"),
                    },
                    raw: raw.into_bytes(),
                }],
                false,
                now(),
            )
            .unwrap();
        }
        // What a Gmail sync reports: the complete label set for one message.
        store
            .ingest(
                ACCOUNT,
                Ingest {
                    mailbox: MailboxRef {
                        account: ACCOUNT,
                        path: "INBOX".to_owned(),
                    },
                    validity: UidValidity::Same,
                    cursor: None,
                    messages: vec![],
                    flags: vec![],
                    labels: vec![],
                    label_names: vec![(
                        RemoteRef::Pop {
                            uidl: "u0".to_owned(),
                        },
                        vec!["travel".to_owned()],
                    )],
                    gone: vec![],
                },
            )
            .unwrap();
        (store, dir)
    }

    fn subjects(store: &SqliteStore, typed: &str) -> Vec<String> {
        let shell = view::Shell {
            search: typed.to_owned(),
            labels: query::known_labels(store),
            ..Default::default()
        };
        store
            .threads(&shell.query(50), now())
            .unwrap()
            .items
            .into_iter()
            .map(|t| t.subject)
            .collect()
    }

    #[test]
    fn it_returns_the_labelled_message_and_only_that_one() {
        let (store, _dir) = seeded();
        assert_eq!(subjects(&store, "label:travel"), vec!["flight to taipei"]);
    }

    /// Not the same as "the query was wrong": a label that exists and has no mail must look
    /// different from a label nothing knows. Both show nothing, so this pins the one that must
    /// still find mail.
    #[test]
    fn a_name_nothing_bears_finds_nothing_rather_than_everything() {
        let (store, _dir) = seeded();
        assert!(subjects(&store, "label:nosuch").is_empty());
    }

    /// The index has to survive the sync that creates it, which is the whole point of rebuilding
    /// it on each revision rather than once at startup.
    #[test]
    fn a_label_that_did_not_exist_at_startup_is_found_once_it_does() {
        let (store, _dir) = seeded();
        let at_startup = query::known_labels(&store);
        assert!(!at_startup.iter().any(|(name, _)| name == "receipts"));

        store
            .ingest(
                ACCOUNT,
                Ingest {
                    mailbox: MailboxRef {
                        account: ACCOUNT,
                        path: "INBOX".to_owned(),
                    },
                    validity: UidValidity::Same,
                    cursor: None,
                    messages: vec![],
                    flags: vec![],
                    labels: vec![],
                    label_names: vec![(
                        RemoteRef::Pop {
                            uidl: "u1".to_owned(),
                        },
                        vec!["receipts".to_owned()],
                    )],
                    gone: vec![],
                },
            )
            .unwrap();

        assert_eq!(subjects(&store, "label:receipts"), vec!["the invoice"]);
    }
}
