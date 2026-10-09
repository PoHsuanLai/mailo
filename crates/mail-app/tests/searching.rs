//! What a person can ask for, and whether the store answers the same question.
//!
//! `mail-domain` has had a complete query algebra since phase 1, proptested against the SQL that
//! answers it, and every search this application could make was `Text(Contains(the whole line))`.
//! These tests are in two halves: what the parser builds, and — because a filter that parses and
//! does not select is worse than no filter — what the store returns for it.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::ui::view;
use mail_core::query;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

fn taipei() -> chrono::FixedOffset {
    chrono::FixedOffset::east_opt(8 * 3600).unwrap()
}

fn parse(input: &str) -> Filter {
    query::parse_with(input, &taipei(), &|_| Vec::new())
}

mod what_it_builds {
    use super::*;

    fn text(word: &str) -> Filter {
        Filter::Text(TextMatch::Contains(word.to_owned()))
    }

    fn from(word: &str) -> Filter {
        Filter::From(TextMatch::Contains(word.to_owned()))
    }

    /// Each line typed into the box, and the filter it builds. Dates are apart, below, because
    /// what they build depends on the reader's zone.
    #[test]
    fn what_each_term_builds() {
        let cases: Vec<(&str, &str, Filter)> = vec![
            ("a bare word is still full text", "lunch", text("lunch")),
            // Several words are one query, not several: the FTS index answers co-occurrence.
            (
                "several words are one query",
                "lunch friday",
                text("lunch friday"),
            ),
            ("from:", "from:ada", from("ada")),
            (
                "to:",
                "to:bob",
                Filter::To(TextMatch::Contains("bob".to_owned())),
            ),
            (
                "subject:",
                "subject:lunch",
                Filter::Subject(TextMatch::Contains("lunch".to_owned())),
            ),
            ("has:attachment", "has:attachment", Filter::HasAttachment),
            ("is:unread", "is:unread", Filter::Read(ReadState::Unread)),
            ("is:starred", "is:starred", Filter::Starred(Star::Starred)),
            ("is:pinned", "is:pinned", Filter::Pinned),
            ("is:snoozed", "is:snoozed", Filter::Snoozed),
            (
                "in:archive",
                "in:archive",
                Filter::InMailbox(MailboxRole::Archive),
            ),
            // Every client works this way and everyone expects it: each word you add finds less.
            (
                "terms narrow rather than widen",
                "from:ada is:unread",
                Filter::And(vec![from("ada"), Filter::Read(ReadState::Unread)]),
            ),
            // Words and fields together: the words become one text clause at the end.
            (
                "words after a field are one text clause",
                "from:ada lunch friday",
                Filter::And(vec![from("ada"), text("lunch friday")]),
            ),
            (
                "a minus negates the term it is attached to",
                "-from:newsletter",
                Filter::Not(Box::new(from("newsletter"))),
            ),
            // A bare `-` is a word, not a negation of nothing.
            ("a bare minus is a word", "-", text("-")),
            (
                "a quoted run is a phrase",
                "\"lunch on friday\"",
                Filter::Text(TextMatch::Exact("lunch on friday".to_owned())),
            ),
            (
                "a quoted value stays together",
                "subject:\"lunch on friday\"",
                Filter::Subject(TextMatch::Contains("lunch on friday".to_owned())),
            ),
            // A box that rejects what is typed while it is being typed is unusable. `frm:` is a
            // typo, not an error, and the result is a search that finds nothing, which is the
            // feedback.
            ("an unknown field is text", "frm:ada", text("frm:ada")),
            ("an unknown is: is text", "is:sideways", text("is:sideways")),
            (
                "a date that is not one is text",
                "before:not-a-date",
                text("before:not-a-date"),
            ),
            // A field with nothing after it is the word so far, mid-typing.
            (
                "a field with nothing after it is text",
                "from:",
                text("from:"),
            ),
            // `Filter::All`, so the caller can decide what an empty search means, which for the
            // shell is "the place you were already in".
            ("an empty box", "", Filter::All),
            ("a box of spaces", "   ", Filter::All),
            ("case in a field name: FROM", "FROM:ada", from("ada")),
            (
                "case in a value of is:",
                "is:UNREAD",
                Filter::Read(ReadState::Unread),
            ),
            (
                "case in a field name and value: In:Archive",
                "In:Archive",
                Filter::InMailbox(MailboxRole::Archive),
            ),
        ];
        for (name, typed, want) in cases {
            assert_eq!(parse(typed), want, "{name}: {typed:?}");
        }
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
                [acct_account().to_string()],
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
                acct_account(),
                MailboxRef {
                    account: acct_account(),
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

    /// One store, every question. The rows are in the order the store lists them: newest first.
    #[test]
    fn the_store_answers_what_was_parsed() {
        const LUNCH: &str = "lunch on friday";
        const INVOICE: &str = "the invoice";
        const DIGEST: &str = "weekly digest";
        const CASES: &[(&str, &str, &[&str])] = &[
            ("a sender narrows to that sender", "from:ada", &[LUNCH]),
            ("another sender", "from:bob", &[INVOICE]),
            (
                "a negated sender removes only that one",
                "-from:newsletter",
                &[LUNCH, INVOICE],
            ),
            // Both true: one thread. One true, one not: nothing.
            ("two terms, both true", "from:ada subject:lunch", &[LUNCH]),
            ("two terms, one true", "from:ada subject:invoice", &[]),
            // The digest is from last August; the other two are from this September.
            ("after a date", "after:2026-01-01", &[LUNCH, INVOICE]),
            ("before a date", "before:2026-01-01", &[DIGEST]),
            // Exclusive on `before`: the 22nd's mail is not "before the 22nd". Inclusive on
            // `after`, read in Taipei: the 21st's mail is not "after the 22nd".
            (
                "before is exclusive",
                "before:2026-09-22",
                &[INVOICE, DIGEST],
            ),
            ("after is inclusive", "after:2026-09-22", &[LUNCH]),
            (
                "the thing that was the only search still works",
                "invoice",
                &[INVOICE],
            ),
            // The query this whole parser exists for: "from Bob, about the invoice".
            (
                "a field and a word together",
                "from:bob invoice",
                &[INVOICE],
            ),
            (
                "a field and a word, the wrong sender",
                "from:ada invoice",
                &[],
            ),
            // The cost of not refusing an unknown field: it becomes text. What it must not do is
            // become `All` and show the whole mailbox as if it had matched.
            (
                "a typo finds nothing rather than everything",
                "frm:ada",
                &[],
            ),
        ];
        let (store, _dir) = seeded();
        for (name, search, want) in CASES {
            assert_eq!(found(&store, search), *want, "{name}: {search:?}");
        }
    }
}

/// `label:`, which is the one term that needs the world: the same search, typed into the
/// terminal or into the window.
///
/// `label:` arrives as a resolver so the parser can stay pure. `mailo search` passes one. The
/// window called `query::parse`, which passes a resolver that knows no names at all, so every
/// `label:` typed there resolved to nothing, became text by the unknown-term rule, and full-text
/// searched for the literal string "label:travel". No results, no error, and the same query
/// working in the terminal.
mod labels {
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
    fn a_label_term_resolves_through_the_window() {
        let (travel, work, home) = (label(7), label(1), label(2));
        type Row = (
            &'static str,
            &'static str,
            Vec<(&'static str, LabelId)>,
            Filter,
        );
        let cases: Vec<Row> = vec![
            (
                "one label with that name is one clause",
                "label:travel",
                vec![("travel", travel)],
                Filter::HasLabel(travel),
            ),
            // `UNIQUE (account, name)`, so "travel" on the Gmail account and "travel" on the
            // work one are two labels. Someone typing `label:travel` means the word: taking the
            // first silently searched one mailbox, which is a wrong answer that looks like an
            // empty one.
            (
                "the same name on two accounts means either",
                "label:travel",
                vec![("travel", work), ("travel", home), ("receipts", label(3))],
                Filter::Or(vec![Filter::HasLabel(work), Filter::HasLabel(home)]),
            ),
            // The same rule as any other unrecognised term. Not `Filter::Nothing`, which would
            // find nothing and look identical to a label that exists and has no mail.
            (
                "a name nothing bears is text",
                "label:nosuch",
                vec![],
                Filter::Text(TextMatch::Contains("label:nosuch".to_owned())),
            ),
            // The rule that must survive the window's fix: a search box has to keep working
            // while a word is half-typed.
            (
                "a half-typed name is still text",
                "label:trav",
                vec![("travel", travel)],
                Filter::Text(TextMatch::Contains("label:trav".to_owned())),
            ),
            // And the rest of the vocabulary keeps working beside it.
            (
                "a label term composes with the others",
                "label:travel is:unread",
                vec![("travel", travel)],
                Filter::And(vec![
                    Filter::HasLabel(travel),
                    Filter::Read(ReadState::Unread),
                ]),
            ),
        ];
        for (name, typed, known, want) in cases {
            let known: Vec<(String, LabelId)> = known
                .into_iter()
                .map(|(word, id)| (word.to_owned(), id))
                .collect();
            assert_eq!(
                query::parse_with(typed, &taipei(), &query::named(&known)),
                want,
                "{name}, in the terminal"
            );
            assert_eq!(shell_searching(typed, known), want, "{name}, in the window");
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
                [acct_account().to_string()],
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
                acct_account(),
                MailboxRef {
                    account: acct_account(),
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
                acct_account(),
                Ingest {
                    mailbox: MailboxRef {
                        account: acct_account(),
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

    /// The index has to survive the sync that creates it, which is the whole point of rebuilding
    /// it on each revision rather than once at startup.
    #[test]
    fn a_label_that_did_not_exist_at_startup_is_found_once_it_does() {
        let (store, _dir) = seeded();
        let at_startup = query::known_labels(&store);
        assert!(!at_startup.iter().any(|(name, _)| name == "receipts"));

        store
            .ingest(
                acct_account(),
                Ingest {
                    mailbox: MailboxRef {
                        account: acct_account(),
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
