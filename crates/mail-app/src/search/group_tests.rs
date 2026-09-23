use mail_domain::ThreadId;
use mail_store::MemoryStore;

use super::super::ranking::{Affinity, SenderStats};
use super::super::support::{self, at};
use super::{Command, Top};
use crate::search::run;

fn thread_of(n: u128) -> ThreadId {
    ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n))
}

fn shown(top: &Option<Top>, mail: &[super::MailHit]) -> Vec<ThreadId> {
    let mut ids = Vec::new();
    if let Some(Top::Mail(hit)) = top {
        ids.push(hit.summary.id);
    }
    ids.extend(mail.iter().map(|hit| hit.summary.id));
    ids
}

#[test]
fn empty_query_is_actions_and_five_recent_threads() {
    let store = MemoryStore::new();
    for n in 0..7 {
        support::remember(&store, n, &format!("note {n}"), "body", n as i64 * 100);
    }
    let commands = vec![
        Command {
            label: "compose".to_owned(),
        },
        Command {
            label: "archive".to_owned(),
        },
    ];
    let results = run("", &store, &Affinity::default(), &commands, at(10_000));
    assert_eq!(results.actions.len(), commands.len());
    assert!(results.people.is_empty());
    let ids = shown(&results.top, &results.mail);
    assert_eq!(
        ids,
        (2..7).rev().map(thread_of).collect::<Vec<_>>(),
        "the five newest, newest first, top not repeated"
    );
    let top = match &results.top {
        Some(Top::Mail(hit)) => hit.summary.id,
        other => panic!("top should be the newest thread, got {other:?}"),
    };
    assert!(results.mail.iter().all(|hit| hit.summary.id != top));
}

#[test]
fn caps_hold() {
    let store = MemoryStore::new();
    for n in 0..10 {
        support::remember(&store, n, &format!("note {n}"), "alpha is here", n as i64);
    }
    let mut affinity = Affinity::default();
    for n in 0..5 {
        affinity.insert(
            format!("alpha{n}@b.c"),
            SenderStats {
                threads: 1,
                replied: false,
            },
        );
    }
    let commands = ["alpha", "alpha-two", "alpha-three", "compose"]
        .into_iter()
        .map(|label| Command {
            label: label.to_owned(),
        })
        .collect::<Vec<_>>();
    let results = run("alpha", &store, &affinity, &commands, at(100));
    assert_eq!(results.mail.len(), 6, "mail cap");
    assert_eq!(results.people.len(), 3, "people cap");
}

#[test]
fn top_hit_is_not_duplicated_in_mail() {
    let store = MemoryStore::new();
    support::remember(&store, 1, "invoice", "invoice details", 0);
    support::remember(&store, 2, "other", "see the invoice", 0);
    support::remember(&store, 3, "notes", "invoice attached", 0);
    let commands = vec![Command {
        label: "compose".to_owned(),
    }];
    let results = run("invoice", &store, &Affinity::default(), &commands, at(10));
    let Some(Top::Mail(top)) = &results.top else {
        panic!("the subject hit should be the top, got {:?}", results.top);
    };
    assert_eq!(top.summary.id, thread_of(1));
    assert!(
        results
            .mail
            .iter()
            .all(|hit| hit.summary.id != top.summary.id),
        "mail repeated the top hit: {:?}",
        results
            .mail
            .iter()
            .map(|hit| hit.summary.id)
            .collect::<Vec<_>>()
    );
    assert!(
        !results.mail.is_empty(),
        "the other invoice threads should remain"
    );
}

#[test]
fn cli_search_order_matches_run_mail() {
    let (store, _dir) = support::sqlite_with(&[
        ("alpha", "please compose this", 10),
        ("bravo", "compose the note", 20),
        ("charlie", "do compose it", 30),
    ]);
    let now = at(10_000);
    let out = crate::cli::run(
        &store,
        &crate::cli::Command::Search {
            needle: "compose".to_owned(),
            limit: 20,
        },
        now,
    )
    .expect("search");
    let cli_ids: Vec<String> = out
        .lines()
        .map(|line| line.split_whitespace().next_back().unwrap_or("").to_owned())
        .collect();
    let commands = vec![Command {
        label: "compose".to_owned(),
    }];
    let results = run("compose", &store, &Affinity::default(), &commands, now);
    assert!(
        matches!(results.top, Some(Top::Action(_))),
        "compose should open the command, got {:?}",
        results.top
    );
    let mail_ids: Vec<String> = results
        .mail
        .iter()
        .map(|hit| hit.summary.id.to_string())
        .collect();
    assert_eq!(cli_ids, mail_ids, "cli:\n{out}");
}
