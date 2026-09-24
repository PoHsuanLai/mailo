//! The Ctrl T menu's People, from the contact book.
//!
//! The ranker still decides whether a person is the top hit; who that person is, and who follows
//! them, is the book's answer to the query's free words — the answer the composer's To field
//! gets for the same text — so the two cannot offer different people for "dana". The sender
//! history still supplies the "3 threads" a row shows.

use std::collections::HashMap;

use chrono::Utc;
use mail_store::Store;

use super::super::contacts::book::suggest;
use super::super::history::History;
use crate::search::{PersonHit, Results, Top};

/// People rows under the top hit.
const PEOPLE_CAP: usize = 3;

/// The words of a Ctrl T query a person could be found by: its free words and phrases, without
/// the operators. `from:dana spec` is `spec`.
pub(in crate::ui) fn free_words(query: &str) -> String {
    let parsed = crate::search::parse(query, &Utc, &|_| Vec::new());
    let mut words = parsed.query_words();
    words.extend(parsed.phrases.iter().cloned());
    words.join(" ")
}

/// Put the book's people for `query` in `results`, best first, the top hit first when the
/// ranker made a person the top hit, and their names in `names`.
pub(in crate::ui) fn from_book(
    results: &mut Results,
    names: &mut HashMap<String, String>,
    store: &dyn Store,
    query: &str,
    history: &History,
) {
    let words = free_words(query);
    let found = if words.trim().is_empty() {
        Vec::new()
    } else {
        suggest(store, &words)
    };
    let mut hits: Vec<PersonHit> = found
        .iter()
        .map(|person| {
            let email = person.address.to_ascii_lowercase();
            names.insert(email.clone(), person.name.clone());
            let seen = history.get(&email);
            PersonHit {
                threads: seen.map_or(0, |sender| sender.threads),
                replied: seen.is_some_and(|sender| sender.replied),
                email,
                score: 0,
                indices: Vec::new(),
            }
        })
        .collect();
    if matches!(results.top, Some(Top::Person(_))) {
        results.top = if hits.is_empty() {
            next_best(results)
        } else {
            Some(Top::Person(hits.remove(0)))
        };
    }
    hits.truncate(PEOPLE_CAP);
    results.people = hits;
}

/// The top hit when the person the ranker chose is not in the book: the best mail, else the
/// best action, each taken out of its group so it is not shown twice.
fn next_best(results: &mut Results) -> Option<Top> {
    if !results.mail.is_empty() {
        return Some(Top::Mail(Box::new(results.mail.remove(0))));
    }
    if !results.actions.is_empty() {
        return Some(Top::Action(results.actions.remove(0)));
    }
    None
}
