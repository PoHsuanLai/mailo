//! The Rules sheet's functions against a store: a rule made, listed, moved, switched off and
//! deleted; a condition the rules cannot read refused in words; and a rule run over the mail
//! already here.

use std::sync::Mutex;

use chrono::Utc;
use mail_domain::*;
use mail_store::Store;

use super::work::{self, Draft, Step};
use crate::ui::fixtures::{ACCOUNT, realistic, seeded};

fn draft(name: &str, query: &str, actions: Vec<RuleAction>) -> Draft {
    Draft {
        name: name.to_owned(),
        query: query.to_owned(),
        actions,
        ..Draft::blank()
    }
}

fn names(store: &mail_store::SqliteStore) -> Vec<String> {
    work::listed(store, ACCOUNT)
        .unwrap()
        .into_iter()
        .map(|l| l.rule.name)
        .collect()
}

#[test]
fn a_rule_made_in_the_sheet_is_listed_moved_switched_and_deleted() {
    let (store, _dir) = seeded();
    assert!(store.rules(ACCOUNT).unwrap().is_empty());

    // Typed with stray spaces: it comes back as the search language writes it.
    let bills = work::save(
        &store,
        ACCOUNT,
        &draft(
            " Bills ",
            "  from:bank.example   subject:statement ",
            vec![RuleAction::Label("Bills".to_owned()), RuleAction::MarkRead],
        ),
        &Utc,
    )
    .unwrap();
    let listed = work::listed(&store, ACCOUNT).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].rule, bills);
    assert_eq!(listed[0].rule.name, "Bills");
    assert_eq!(listed[0].when, "from:bank.example subject:statement");
    assert_eq!(listed[0].does, "Label “Bills” · Mark read");
    assert_eq!(listed[0].rule.state, RuleState::Enabled);
    // What the CLI would make of the same words.
    assert_eq!(
        listed[0].rule.filter,
        crate::query::parse_with("from:bank.example subject:statement", &Utc, &|_| Vec::new())
    );

    let news = work::save(
        &store,
        ACCOUNT,
        &draft("News", "from:news@example.com", vec![RuleAction::Archive]),
        &Utc,
    )
    .unwrap();
    assert_eq!(names(&store), ["Bills", "News"]);
    let (bills_at, news_at) = (bills.position, news.position);
    assert!(bills_at < news_at, "a new rule does not go last");

    work::reorder(&store, ACCOUNT, news.id, Step::Up).unwrap();
    assert_eq!(names(&store), ["News", "Bills"]);
    let now: Vec<(String, u32)> = store
        .rules(ACCOUNT)
        .unwrap()
        .into_iter()
        .map(|r| (r.name, r.position))
        .collect();
    assert!(now.contains(&("News".to_owned(), 1)), "{now:?}");
    assert!(now.contains(&("Bills".to_owned(), 2)), "{now:?}");
    // At the top already: nothing moves.
    work::reorder(&store, ACCOUNT, news.id, Step::Up).unwrap();
    assert_eq!(names(&store), ["News", "Bills"]);

    // Edited, it keeps its place and its id.
    let listed = work::listed(&store, ACCOUNT).unwrap();
    let mut edit = Draft::of(&listed[1]);
    assert_eq!(edit.query, "from:bank.example subject:statement");
    edit.query = "from:bank.example".to_owned();
    let edited = work::save(&store, ACCOUNT, &edit, &Utc).unwrap();
    assert_eq!(edited.id, bills.id);
    assert_eq!(edited.position, 2);
    assert_eq!(store.rules(ACCOUNT).unwrap().len(), 2);

    work::switch(&store, &edited, RuleState::Disabled).unwrap();
    let off = store
        .rules(ACCOUNT)
        .unwrap()
        .into_iter()
        .find(|r| r.id == bills.id)
        .unwrap();
    assert_eq!(off.state, RuleState::Disabled);

    let said = work::delete(&store, &news).unwrap();
    assert_eq!(said, "Deleted the rule “News”");
    assert_eq!(names(&store), ["Bills"]);
}

#[test]
fn a_condition_the_rules_cannot_read_is_refused_in_words_and_not_kept() {
    let (store, _dir) = seeded();
    const CASES: &[(&str, &str)] = &[
        ("frm:bank.example", "“frm:” is not a word a rule knows"),
        (
            "from:bank.example before:yesterday",
            "needs a date, like before:2026-10-08",
        ),
        (
            "label:nowhere",
            "There is no label called “nowhere” on this account.",
        ),
        ("is:important", "is: takes unread"),
        ("in:elsewhere", "in: takes inbox"),
        ("subject:", "“subject:” needs something after the colon."),
        ("   ", "A rule needs a condition"),
    ];
    for (typed, said) in CASES {
        let refused = work::read_condition(typed, &work::labels(&store, ACCOUNT), &Utc);
        match refused {
            Err(why) => assert!(why.contains(said), "{typed:?}: {why}"),
            Ok(filter) => panic!("{typed:?} was read as {filter:?}"),
        }
        let saved = work::save(
            &store,
            ACCOUNT,
            &draft("Bad", typed, vec![RuleAction::Archive]),
            &Utc,
        );
        assert!(saved.is_err(), "{typed:?} was kept");
    }
    assert!(
        store.rules(ACCOUNT).unwrap().is_empty(),
        "a refused rule was kept"
    );

    // What a search reads as text but a rule reads fine: a phrase, a negation, a plain word.
    for fine in [
        "\"weekly report\"",
        "-from:boss@example.com invoice",
        "subject:\"re: lunch\"",
    ] {
        assert!(
            work::read_condition(fine, &[], &Utc).is_ok(),
            "{fine:?} was refused"
        );
    }
}

#[test]
fn a_rule_needs_a_name_one_of_its_own_and_something_to_do() {
    let (store, _dir) = seeded();
    let blank = work::save(
        &store,
        ACCOUNT,
        &draft(" ", "from:a", vec![RuleAction::Star]),
        &Utc,
    );
    assert_eq!(blank.unwrap_err(), "A rule needs a name.");
    let idle = work::save(&store, ACCOUNT, &draft("Idle", "from:a", Vec::new()), &Utc);
    assert!(idle.unwrap_err().contains("something to do"));
    work::save(
        &store,
        ACCOUNT,
        &draft("Twice", "from:a", vec![RuleAction::Star]),
        &Utc,
    )
    .unwrap();
    let again = work::save(
        &store,
        ACCOUNT,
        &draft("Twice", "from:b", vec![RuleAction::Star]),
        &Utc,
    );
    assert!(again.unwrap_err().contains("already a rule called “Twice”"));
    assert_eq!(store.rules(ACCOUNT).unwrap().len(), 1);
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
        &store,
        ACCOUNT,
        &draft(
            "Receipts",
            "from:receipts@stripe.test",
            vec![RuleAction::Star],
        ),
        &Utc,
    )
    .unwrap();
    let total = work::conversations(&store, ACCOUNT, Utc::now());
    assert_eq!(total, others.len() + 1);
    let seen = Mutex::new(Vec::new());
    let said = work::run(&store, &rule, Utc::now(), &|done| {
        seen.lock().unwrap().push(done)
    })
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
