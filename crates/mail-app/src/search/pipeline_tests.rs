//! A search's two answers: the matches in date order, and the few top results above them.

use std::cell::RefCell;

use chrono::{DateTime, Utc};
use mail_domain::{Filter, PageReq, ThreadSummary};

use super::super::ranking::Affinity;
use super::super::source::{Source, Term, first};
use super::super::support::{at, sqlite_with};
use super::{STRIP, WINDOW, search_list};

fn searched(input: &str, store: &mail_store::SqliteStore) -> super::Searched {
    search_list(
        input,
        store,
        &Affinity::default(),
        &Utc,
        &|_| Vec::new(),
        first(100),
        at(10_000),
    )
    .unwrap_or_else(|why| panic!("{input:?} is not a regex: {why}"))
}

fn subjects(rows: &[ThreadSummary]) -> Vec<&str> {
    rows.iter().map(|row| row.subject.as_str()).collect()
}

#[test]
fn the_list_is_newest_first_and_the_strip_is_best_first() {
    let (store, _dir) = sqlite_with(&[
        ("Invoice for March", "the invoice is attached", 100),
        ("Lunch", "and the invoice, again", 200),
        ("Weekly notes", "nothing about money", 300),
        ("Parking", "an invoice was mentioned once", 400),
    ]);
    let found = searched("invoice", &store);
    assert_eq!(
        subjects(&found.rows),
        vec!["Parking", "Lunch", "Invoice for March"],
        "the matches, newest first"
    );
    let strip = found.strip();
    assert_eq!(
        strip.first().map(|row| row.subject.as_str()),
        Some("Invoice for March"),
        "the subject hit leads the strip although it is the oldest: {:?}",
        subjects(&strip)
    );
    assert!(strip.len() <= STRIP, "{:?}", subjects(&strip));
}

#[test]
fn operators_alone_list_without_a_strip() {
    let (store, _dir) = sqlite_with(&[
        ("Invoice for March", "the invoice is attached", 100),
        ("Lunch", "sandwiches", 200),
    ]);
    let found = searched("from:a@b.c", &store);
    assert_eq!(subjects(&found.rows), vec!["Lunch", "Invoice for March"]);
    assert!(found.top.is_empty(), "nothing to rank by: {:?}", found.top);
    assert!(found.strip().is_empty());
}

#[test]
fn a_strip_that_repeats_the_first_rows_is_not_shown() {
    let (store, _dir) = sqlite_with(&[
        ("Keyset cursors", "not offsets", 100),
        ("Lunch", "sandwiches", 200),
    ]);
    let found = searched("cursors", &store);
    assert_eq!(subjects(&found.rows), vec!["Keyset cursors"]);
    assert_eq!(
        found
            .top
            .iter()
            .map(|(row, _)| row.subject.as_str())
            .collect::<Vec<_>>(),
        vec!["Keyset cursors"],
        "the store did find a top result"
    );
    assert!(
        found.strip().is_empty(),
        "one match is its own top result, and a strip of it says nothing"
    );
}

/// A source that writes down what it was asked.
#[derive(Default)]
struct Asked {
    listed: RefCell<Vec<u32>>,
    top: RefCell<Vec<(usize, usize)>>,
}

impl Source for Asked {
    fn terms_with_prefix(&self, _: &str, _: usize) -> Vec<Term> {
        Vec::new()
    }

    fn listed(&self, _: &Filter, page: PageReq, _: DateTime<Utc>) -> Vec<ThreadSummary> {
        self.listed.borrow_mut().push(page.limit);
        Vec::new()
    }

    fn top(
        &self,
        _: &Filter,
        k: usize,
        window: usize,
        _: DateTime<Utc>,
    ) -> Vec<(ThreadSummary, f64)> {
        self.top.borrow_mut().push((k, window));
        Vec::new()
    }
}

/// What was typed, the page limits `listed` was asked for, and the `(k, window)` of each `top`.
type AskedFor = (&'static str, &'static [u32], &'static [(usize, usize)]);

#[test]
fn a_search_asks_for_one_page_and_a_bounded_window_never_every_match() {
    let cases: &[AskedFor] = &[
        ("the ", &[100], &[(STRIP, WINDOW)]),
        ("invoice from:a@b.c", &[100], &[(STRIP, WINDOW)]),
        ("\"lunch on friday\"", &[100], &[(STRIP, WINDOW)]),
        ("from:a@b.c is:unread", &[100], &[]),
    ];
    for (input, listed, top) in cases {
        let asked = Asked::default();
        search_list(
            input,
            &asked,
            &Affinity::default(),
            &Utc,
            &|_| Vec::new(),
            first(100),
            at(0),
        )
        .unwrap_or_else(|why| panic!("{input:?}: {why}"));
        assert_eq!(*asked.listed.borrow(), *listed, "{input:?}: listed");
        assert_eq!(*asked.top.borrow(), *top, "{input:?}: top");
    }
}
