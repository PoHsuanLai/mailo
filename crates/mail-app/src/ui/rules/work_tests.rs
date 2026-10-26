//! The Rules sheet's functions against a store: a condition the rules cannot read refused in a
//! sentence of its own, and a rule run over the mail already here.

use std::sync::Mutex;

use chrono::Utc;
use mail_core::Store;
use mail_domain::*;

use super::work::{self, Draft};
use crate::ui::fixtures::{acct_account, realistic, seeded};

fn draft(name: &str, query: &str, actions: Vec<RuleAction>) -> Draft {
    Draft {
        name: name.to_owned(),
        query: query.to_owned(),
        actions,
        ..Draft::blank()
    }
}

#[test]
fn a_condition_the_rules_cannot_read_is_refused_and_not_kept() {
    let (store, _dir) = seeded();
    // Each kind of refusal core's `rules::manage` tells apart, said in a sentence of its own.
    const CASES: &[&str] = &[
        "frm:bank.example",
        "from:bank.example before:yesterday",
        "label:nowhere",
        "is:important",
        "in:elsewhere",
        "subject:",
        "   ",
    ];
    let mut said = std::collections::HashSet::new();
    for typed in CASES {
        let refused = work::read_condition(
            typed,
            &mail_core::rules::manage::labels(&store, acct_account()),
            &Utc,
        );
        match refused {
            Err(why) => assert!(said.insert(why.clone()), "{typed:?}: {why} said twice"),
            Ok(filter) => panic!("{typed:?} was read as {filter:?}"),
        }
        let saved = work::save(
            &crate::ui::fixtures::client(&store),
            acct_account(),
            &draft("Bad", typed, vec![RuleAction::Archive]),
            &Utc,
        );
        assert!(saved.is_err(), "{typed:?} was kept");
    }
    assert!(
        store.rules(acct_account()).unwrap().is_empty(),
        "a refused rule was kept"
    );
}

#[test]
fn running_a_rule_acts_on_what_matches_and_counts_to_the_end() {
    let (store, _dir) = realistic();
    let receipt = crate::ui::fixtures::thread_like(&store, "Your receipt from");
    let others: Vec<ThreadId> = store
        .threads(&crate::ui::fixtures::inbox_query(), Utc::now())
        .unwrap()
        .items
        .into_iter()
        .map(|t| t.id)
        .filter(|id| *id != receipt)
        .collect();
    let starred = |id: ThreadId| store.thread(id).unwrap().summary.star;
    assert_eq!(starred(receipt), Star::Unstarred);
    assert!(others.iter().all(|id| starred(*id) == Star::Unstarred));

    let rule = work::save(
        &crate::ui::fixtures::client(&store),
        acct_account(),
        &draft(
            "Receipts",
            "from:receipts@stripe.test",
            vec![RuleAction::Star],
        ),
        &Utc,
    )
    .unwrap();
    let total = mail_core::rules::manage::conversations(&store, acct_account(), Utc::now());
    assert_eq!(total, others.len() + 1);
    let seen = Mutex::new(Vec::new());
    let said = work::run(
        &crate::ui::fixtures::client(&store),
        &rule,
        Utc::now(),
        &|done| seen.lock().unwrap().push(done),
    )
    .unwrap();

    assert_eq!(starred(receipt), Star::Starred, "the match was not starred");
    assert!(
        others.iter().all(|id| starred(*id) == Star::Unstarred),
        "a conversation that does not match was starred"
    );
    let seen = seen.into_inner().unwrap();
    assert_eq!(
        seen.last(),
        Some(&total),
        "the progress stopped short: {seen:?}"
    );
    assert!(
        said.starts_with("“Receipts” looked at 7 messages and acted on 1."),
        "{said}"
    );
}
