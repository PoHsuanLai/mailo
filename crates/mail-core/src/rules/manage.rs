//! Managing an account's rules from a sheet rather than a command: listed, written, moved,
//! switched, deleted, and run over the mail already here.
//!
//! What `mailo rules` does, asked a function at a time. Every refusal is a value ([`SaveError`],
//! [`ConditionError`]) for a front end to word; nothing here prints or phrases.

use super::{ListedRule, condition};
use crate::error::CoreError;
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{AfterMatch, Filter, LabelId, Rule, RuleAction, RuleId, RuleState};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

/// How many conversations "Run on existing mail" takes at a time: `mailo rules run`'s batch.
pub const BATCH: u32 = 200;

/// A rule being written: what an editor's fields hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// The rule being edited, or `None` for a new one.
    pub id: Option<RuleId>,
    pub name: String,
    /// The condition as typed.
    pub query: String,
    pub actions: Vec<RuleAction>,
    pub after: AfterMatch,
}

impl Draft {
    /// A new rule, with nothing in it yet.
    pub fn blank() -> Self {
        Self {
            id: None,
            name: String::new(),
            query: String::new(),
            actions: Vec::new(),
            after: AfterMatch::Continue,
        }
    }

    /// `listed`, opened for editing: its condition as the list shows it.
    pub fn of(listed: &ListedRule) -> Self {
        Self {
            id: Some(listed.rule.id),
            name: listed.rule.name.clone(),
            query: listed.condition.clone(),
            actions: listed.rule.actions.clone(),
            after: listed.rule.after,
        }
    }
}

/// Which way a rule moves in the order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Up,
    Down,
}

/// Why a condition cannot be a rule's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionError {
    /// Nothing was typed.
    Blank,
    /// `field:` with nothing after the colon.
    NoValue { field: String },
    /// `before:`/`after:` with something that is not a date.
    NotADate { word: String, field: String },
    /// `label:` naming a label the account does not have.
    NoLabel { value: String },
    /// `is:` with a word it does not take.
    BadIs,
    /// `in:` with a word it does not take.
    BadIn,
    /// `has:` with a word it does not take.
    BadHas,
    /// A `word:` the search language has no term for.
    UnknownField { field: String },
}

/// Why a draft was not kept.
#[derive(Debug)]
pub enum SaveError {
    NoName,
    /// Another rule of the account has this name.
    Taken {
        name: String,
    },
    Condition(ConditionError),
    /// It does nothing: no action, and no stop.
    NoAction,
    Store(CoreError),
}

impl From<CoreError> for SaveError {
    fn from(error: CoreError) -> Self {
        SaveError::Store(error)
    }
}

/// The account's labels by name, which `label:` in a condition resolves against.
pub fn labels(store: &SqliteStore, account: AccountId) -> Vec<(String, LabelId)> {
    store
        .labels(account)
        .unwrap_or_default()
        .into_iter()
        .map(|label| (label.name, label.id))
        .collect()
}

/// A label's name, for writing a condition back; one since deleted says so.
fn name_of(index: &[(String, LabelId)]) -> impl Fn(LabelId) -> String + '_ {
    move |id| {
        index
            .iter()
            .find(|(_, known)| *known == id)
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| "(deleted)".to_owned())
    }
}

/// The account's rules in the order they run: by position, then by name.
pub fn listed(store: &SqliteStore, account: AccountId) -> Result<Vec<ListedRule>, CoreError> {
    let index = labels(store, account.clone());
    let name = name_of(&index);
    let mut rules = store.rules(account)?;
    rules.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(rules
        .into_iter()
        .map(|rule| ListedRule {
            condition: condition(&rule.filter, &name),
            rule,
        })
        .collect())
}

/// The condition `text` means, or why a rule cannot use it.
///
/// A search box reads a word it does not know as text, so as not to refuse what is being
/// typed. A rule is different: it runs unseen on every message that arrives, and `frm:bank` read
/// as the text "frm:bank" is a rule that silently never fires. So every `word:` term is read on
/// its own through the same parser, and one the parser did not take as that term is refused here
/// before it is kept.
pub fn read_condition<Tz: TimeZone>(
    text: &str,
    labels: &[(String, LabelId)],
    zone: &Tz,
) -> Result<Filter, ConditionError> {
    if text.trim().is_empty() {
        return Err(ConditionError::Blank);
    }
    let named = crate::query::named(labels);
    for word in text.split_whitespace() {
        let bare = word.strip_prefix('-').unwrap_or(word);
        if bare.starts_with('"') {
            continue;
        }
        let Some((field, value)) = bare.split_once(':') else {
            continue;
        };
        let alone = crate::query::parse_with(bare, zone, &named);
        if !matches!(alone, Filter::Text(_)) {
            continue;
        }
        return Err(refused(bare, field.to_ascii_lowercase(), value));
    }
    Ok(crate::query::parse_with(text, zone, &named))
}

/// Why `word`, whose field is `field`, is not a term.
fn refused(word: &str, field: String, value: &str) -> ConditionError {
    if value.is_empty() {
        return ConditionError::NoValue { field };
    }
    match field.as_str() {
        "before" | "after" => ConditionError::NotADate {
            word: word.to_owned(),
            field,
        },
        "label" => ConditionError::NoLabel {
            value: value.to_owned(),
        },
        "is" => ConditionError::BadIs,
        "in" => ConditionError::BadIn,
        "has" => ConditionError::BadHas,
        _ => ConditionError::UnknownField { field },
    }
}

/// How many conversations on the account match `filter` now.
pub fn matching(
    store: &SqliteStore,
    account: AccountId,
    filter: Filter,
    now: DateTime<Utc>,
) -> Result<u64, CoreError> {
    Ok(store.count(&Filter::And(vec![Filter::Account(account), filter]), now)?)
}

/// Keep `draft` as a rule on `account`, or say why not. A new rule goes last and starts on; an
/// edited one keeps its place and its switch.
pub fn save<Tz: TimeZone>(
    store: &SqliteStore,
    account: AccountId,
    draft: &Draft,
    zone: &Tz,
) -> Result<Rule, SaveError> {
    let name = draft.name.trim();
    if name.is_empty() {
        return Err(SaveError::NoName);
    }
    let existing = store.rules(account.clone()).map_err(CoreError::from)?;
    if existing
        .iter()
        .any(|rule| rule.name == name && Some(rule.id) != draft.id)
    {
        return Err(SaveError::Taken {
            name: name.to_owned(),
        });
    }
    let filter = read_condition(&draft.query, &labels(store, account.clone()), zone)
        .map_err(SaveError::Condition)?;
    if draft.actions.is_empty() && draft.after == AfterMatch::Continue {
        return Err(SaveError::NoAction);
    }
    let kept = draft
        .id
        .and_then(|id| existing.iter().find(|rule| rule.id == id));
    let (id, position, state) = match kept {
        Some(rule) => (rule.id, rule.position, rule.state),
        None => (
            RuleId::generate(),
            existing.iter().map(|r| r.position + 1).max().unwrap_or(1),
            RuleState::Enabled,
        ),
    };
    let rule = Rule {
        id,
        account,
        name: name.to_owned(),
        position,
        state,
        filter,
        actions: draft.actions.clone(),
        after: draft.after,
    };
    store.put_rule(&rule).map_err(CoreError::from)?;
    Ok(rule)
}

/// Move the rule `id` one place up or down, and number the account's rules 1, 2, 3… again.
pub fn reorder(
    store: &SqliteStore,
    account: AccountId,
    id: RuleId,
    step: Step,
) -> Result<(), CoreError> {
    let mut rules: Vec<Rule> = listed(store, account)?
        .into_iter()
        .map(|l| l.rule)
        .collect();
    let Some(at) = rules.iter().position(|rule| rule.id == id) else {
        return Ok(());
    };
    let to = match step {
        Step::Up => at.checked_sub(1),
        Step::Down => Some(at + 1).filter(|to| *to < rules.len()),
    };
    let Some(to) = to else {
        return Ok(());
    };
    rules.swap(at, to);
    for (index, rule) in rules.into_iter().enumerate() {
        let position = u32::try_from(index + 1).unwrap_or(u32::MAX);
        if rule.position != position {
            store.put_rule(&Rule { position, ..rule })?;
        }
    }
    Ok(())
}

/// Turn a rule on or off. Off, it is kept and skipped, here and in the server's script.
pub fn switch(store: &SqliteStore, rule: &Rule, state: RuleState) -> Result<(), CoreError> {
    Ok(store.put_rule(&Rule {
        state,
        ..rule.clone()
    })?)
}

/// Forget a rule.
pub fn delete(store: &SqliteStore, rule: &Rule) -> Result<(), CoreError> {
    Ok(store.delete_rule(rule.id)?)
}

/// The conversations "Run on existing mail" goes through: every one on the account.
pub fn conversations(store: &SqliteStore, account: AccountId, now: DateTime<Utc>) -> usize {
    store
        .count(&Filter::Account(account), now)
        .ok()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0)
}

/// Run `rule` over the mail already here, as `mailo rules run` does, reporting how many
/// conversations it has been through.
///
/// A switched-off rule runs too: asking is the switch. Blocking; a sheet calls it off the thread
/// that draws.
pub fn run(
    store: &SqliteStore,
    rule: &Rule,
    now: DateTime<Utc>,
    report: &(dyn Fn(usize) + Sync),
) -> Result<mail_store::rules::Ran, CoreError> {
    let caps = crate::act::caps_here(store, rule.account.clone(), now);
    let total = conversations(store, rule.account.clone(), now);
    let mut pages = 0usize;
    Ok(mail_store::rules::run_now(
        store,
        &caps,
        rule,
        BATCH,
        now,
        &mut |_| {
            pages += 1;
            report((pages * BATCH as usize).min(total));
        },
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn account() -> AccountId {
        mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(0xa1))
    }

    /// A store with the one account the rules belong to.
    fn seeded() -> (SqliteStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        mail_store::testing::seed_account(&store, account(), "me@example.test");
        (store, dir)
    }

    fn draft(name: &str, query: &str, actions: Vec<RuleAction>) -> Draft {
        Draft {
            name: name.to_owned(),
            query: query.to_owned(),
            actions,
            ..Draft::blank()
        }
    }

    #[test]
    fn a_condition_the_rules_cannot_read_is_refused_by_kind() {
        use ConditionError::*;
        let cases = [
            (
                "frm:bank.example",
                UnknownField {
                    field: "frm".to_owned(),
                },
            ),
            (
                "from:bank.example before:yesterday",
                NotADate {
                    word: "before:yesterday".to_owned(),
                    field: "before".to_owned(),
                },
            ),
            (
                "label:nowhere",
                NoLabel {
                    value: "nowhere".to_owned(),
                },
            ),
            ("is:important", BadIs),
            ("in:elsewhere", BadIn),
            ("has:wings", BadHas),
            (
                "subject:",
                NoValue {
                    field: "subject".to_owned(),
                },
            ),
            ("   ", Blank),
        ];
        for (typed, expect) in cases {
            assert_eq!(read_condition(typed, &[], &Utc), Err(expect), "{typed:?}");
        }
        // What a search reads as text but a rule reads fine: a phrase, a negation, a plain word.
        for fine in [
            "\"weekly report\"",
            "-from:boss@example.com invoice",
            "subject:\"re: lunch\"",
        ] {
            assert!(
                read_condition(fine, &[], &Utc).is_ok(),
                "{fine:?} was refused"
            );
        }
    }

    #[test]
    fn a_draft_is_refused_for_its_name_its_condition_or_its_idleness_and_nothing_is_kept() {
        let (store, _dir) = seeded();
        let account = account();
        let refused = |draft: &Draft| save(&store, account.clone(), draft, &Utc).unwrap_err();
        assert!(matches!(
            refused(&draft(" ", "from:a", vec![RuleAction::Star])),
            SaveError::NoName
        ));
        assert!(matches!(
            refused(&draft("Idle", "from:a", Vec::new())),
            SaveError::NoAction
        ));
        assert!(matches!(
            refused(&draft("Bad", "frm:a", vec![RuleAction::Star])),
            SaveError::Condition(ConditionError::UnknownField { .. })
        ));
        assert!(store.rules(account.clone()).unwrap().is_empty());

        save(
            &store,
            account.clone(),
            &draft("Twice", "from:a", vec![RuleAction::Star]),
            &Utc,
        )
        .unwrap();
        assert!(matches!(
            refused(&draft("Twice", "from:b", vec![RuleAction::Star])),
            SaveError::Taken { name } if name == "Twice"
        ));
        assert_eq!(store.rules(account).unwrap().len(), 1);
    }

    #[test]
    fn rules_are_listed_in_order_moved_edited_switched_and_forgotten() {
        let (store, _dir) = seeded();
        let account = account();
        let make = |name: &str, query: &str| {
            save(
                &store,
                account.clone(),
                &draft(name, query, vec![RuleAction::Archive]),
                &Utc,
            )
            .unwrap()
        };
        let names = || -> Vec<String> {
            listed(&store, account.clone())
                .unwrap()
                .into_iter()
                .map(|l| l.rule.name)
                .collect()
        };
        let bills = make(" Bills ", "  from:bank.example   subject:statement ");
        let news = make("News", "from:news@example.com");
        assert_eq!(names(), ["Bills", "News"]);
        assert!(
            bills.position < news.position,
            "a new rule does not go last"
        );

        reorder(&store, account.clone(), news.id, Step::Up).unwrap();
        assert_eq!(names(), ["News", "Bills"]);
        reorder(&store, account.clone(), news.id, Step::Up).unwrap();
        assert_eq!(names(), ["News", "Bills"], "at the top already");

        // Edited, it keeps its place and its id, and reads back as the search language writes it.
        let listed_now = listed(&store, account.clone()).unwrap();
        let mut edit = Draft::of(&listed_now[1]);
        assert_eq!(edit.query, "from:bank.example subject:statement");
        edit.query = "from:bank.example".to_owned();
        let edited = save(&store, account.clone(), &edit, &Utc).unwrap();
        assert_eq!((edited.id, edited.position), (bills.id, 2));

        switch(&store, &edited, RuleState::Disabled).unwrap();
        let off = store
            .rules(account.clone())
            .unwrap()
            .into_iter()
            .find(|r| r.id == bills.id)
            .unwrap();
        assert_eq!(off.state, RuleState::Disabled);

        delete(&store, &news).unwrap();
        assert_eq!(names(), ["Bills"]);
    }
}
