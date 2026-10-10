//! Every question the Rules sheet asks and every rule it writes, as functions of a store.
//!
//! What `mailo rules` does, reached from the window: the same `Rule` rows, the condition in the
//! same search language (`mail_core::query`), and the same `mail_store::rules::run_now`. The
//! doing is `mail_core::rules::manage`; this is the words the sheet says about it. The sheet only
//! draws what these answer, so the tests drive these and not the markup.

use chrono::{DateTime, TimeZone, Utc};
use mail_core::SqliteStore;
use mail_core::rules::manage::{self, ConditionError, SaveError};
use mail_domain::{AfterMatch, Filter, LabelId, Rule, RuleAction, RuleId, RuleState};
use porter_core::AccountId;

use super::super::files::work::{grouped, messages};

pub(in crate::ui) use mail_core::rules::manage::{Draft, Step, conversations, labels};

/// The search words a rule's condition may use, as the refusal lists them.
const KNOWN: &str = "from:, to:, subject:, is:, in:, has:, label:, before: and after:";

/// One rule as the sheet lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Listed {
    pub rule: Rule,
    /// Its condition, written back in the search language.
    pub when: String,
    /// What it does, in words.
    pub does: String,
}

/// The account's rules in the order they run: by position, then by name.
pub(in crate::ui) fn listed(
    store: &SqliteStore,
    account: AccountId,
) -> Result<Vec<Listed>, String> {
    let rules = manage::listed(store, account).map_err(|e| e.to_string())?;
    Ok(rules
        .into_iter()
        .map(|listed| Listed {
            when: listed.condition,
            does: does(&listed.rule),
            rule: listed.rule,
        })
        .collect())
}

/// One action, as the list and the editor say it.
pub(in crate::ui) fn action_words(action: &RuleAction) -> String {
    match action {
        RuleAction::Label(name) => format!("Label “{name}”"),
        RuleAction::File(path) => format!("Move to “{path}”"),
        RuleAction::Archive => "Archive".to_owned(),
        RuleAction::Trash => "Move to Trash".to_owned(),
        RuleAction::Spam => "Mark as spam".to_owned(),
        RuleAction::MarkRead => "Mark read".to_owned(),
        RuleAction::Star => "Star".to_owned(),
    }
}

/// Everything a rule does, in one line.
pub(in crate::ui) fn does(rule: &Rule) -> String {
    let mut parts: Vec<String> = rule.actions.iter().map(action_words).collect();
    if rule.after == AfterMatch::Stop {
        parts.push("then no later rule".to_owned());
    }
    if parts.is_empty() {
        "Nothing".to_owned()
    } else {
        parts.join(" · ")
    }
}

/// The condition `text` means, or why a rule cannot use it, in words.
pub(in crate::ui) fn read_condition<Tz: TimeZone>(
    text: &str,
    labels: &[(String, LabelId)],
    zone: &Tz,
) -> Result<Filter, String> {
    manage::read_condition(text, labels, zone).map_err(|why| refusal(&why))
}

/// Why a condition is refused, as the sheet says it.
fn refusal(why: &ConditionError) -> String {
    match why {
        ConditionError::Blank => "Add a condition, like from:news@example.com".to_owned(),
        ConditionError::NoValue { field } => format!("“{field}:” needs something after the colon."),
        ConditionError::NotADate { word, field } => {
            format!("“{word}” needs a date, like {field}:2026-10-08.")
        }
        ConditionError::NoLabel { value } => format!("No label “{value}” on this account."),
        ConditionError::BadIs => {
            "is: takes unread, read, starred, unstarred, pinned or snoozed.".to_owned()
        }
        ConditionError::BadIn => {
            "in: takes inbox, archive, sent, drafts, spam or trash.".to_owned()
        }
        ConditionError::BadHas => "has: takes attachment.".to_owned(),
        ConditionError::UnknownField { field } => {
            format!("“{field}:” is not a word a rule knows. It knows {KNOWN}.")
        }
    }
}

/// How many conversations on the account match `filter` now.
pub(in crate::ui) fn matching(
    store: &SqliteStore,
    account: AccountId,
    filter: Filter,
    now: DateTime<Utc>,
) -> Result<u64, String> {
    manage::matching(store, account, filter, now).map_err(|e| e.to_string())
}

/// What the live count under the condition says.
pub(in crate::ui) fn matching_words(count: u64) -> String {
    match count {
        0 => "No matches yet".to_owned(),
        1 => "1 conversation matches".to_owned(),
        n => format!(
            "{} conversations match",
            grouped(usize::try_from(n).unwrap_or(usize::MAX))
        ),
    }
}

/// Keep `draft` as a rule on `account`, or say why not.
pub(in crate::ui) fn save<Tz: TimeZone>(
    store: &SqliteStore,
    account: AccountId,
    draft: &Draft,
    zone: &Tz,
) -> Result<Rule, String> {
    manage::save(store, account, draft, zone).map_err(|why| match why {
        SaveError::NoName => "A rule needs a name.".to_owned(),
        SaveError::Taken { name } => format!("A rule called “{name}” already exists."),
        SaveError::Condition(why) => refusal(&why),
        SaveError::NoAction => "Add an action.".to_owned(),
        SaveError::Store(why) => why.to_string(),
    })
}

/// Move the rule `id` one place up or down, and number the account's rules 1, 2, 3… again.
pub(in crate::ui) fn reorder(
    store: &SqliteStore,
    account: AccountId,
    id: RuleId,
    step: Step,
) -> Result<(), String> {
    manage::reorder(store, account, id, step).map_err(|e| e.to_string())
}

/// Turn a rule on or off. Off, it is kept and skipped, here and in the server's script.
pub(in crate::ui) fn switch(
    store: &SqliteStore,
    rule: &Rule,
    state: RuleState,
) -> Result<(), String> {
    manage::switch(store, rule, state).map_err(|e| e.to_string())
}

/// Forget a rule, returning what the toast says.
pub(in crate::ui) fn delete(store: &SqliteStore, rule: &Rule) -> Result<String, String> {
    manage::delete(store, rule).map_err(|e| e.to_string())?;
    Ok(format!("Deleted the rule “{}”", rule.name))
}

/// Run `rule` over the mail already here, as `mailo rules run` does, reporting how many
/// conversations it has been through, and say what it did.
///
/// A switched-off rule runs too: asking is the switch. Blocking; the sheet calls it off the
/// thread that draws.
pub(in crate::ui) fn run(
    store: &SqliteStore,
    rule: &Rule,
    now: DateTime<Utc>,
    report: &(dyn Fn(usize) + Sync),
) -> Result<String, String> {
    let ran = manage::run(store, rule, now, report).map_err(|e| e.to_string())?;
    let mut said = format!(
        "“{}” looked at {} and acted on {}.",
        rule.name,
        messages(ran.examined),
        grouped(ran.acted.len())
    );
    if ran.queued > 0 {
        said.push_str(&format!(
            " {} for the server go with the next sync.",
            if ran.queued == 1 {
                "1 change".to_owned()
            } else {
                format!("{} changes", grouped(ran.queued))
            }
        ));
    }
    Ok(said)
}
