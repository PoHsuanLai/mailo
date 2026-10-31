//! Every question the Rules sheet asks and every rule it writes, as functions of a client.
//!
//! What `mailo rules` does, reached from the window: the same `Rule` rows, the condition in the
//! same search language (`mail_client::query`), and the same `mail_client::rules::run_now`. The
//! doing is `mail_client::rules::manage`; this is the words the sheet says about it. The sheet only
//! draws what these answer, so the tests drive these and not the markup.

use chrono::{DateTime, TimeZone, Utc};
use mail_client::Client;
use mail_client::rules::manage::{self, ConditionError, SaveError};
use mail_domain::{AfterMatch, Filter, LabelId, Rule, RuleId, RuleState};
use porter_core::AccountId;

use super::super::files::work::{grouped, messages};
use crate::ui::remedy::plainly;
use mail_ui_model::copy;

pub(in crate::ui) use mail_client::rules::manage::{Draft, Step};

/// The search words a rule's condition may use, as the refusal lists them.
const KNOWN: &str = "from:, to:, cc:, subject:, is:, in:, has:, label:, before:, after:, \
                     larger:, smaller:, newer_than: and older_than:";

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
pub(in crate::ui) fn listed(client: &Client, account: AccountId) -> Result<Vec<Listed>, String> {
    let rules = client
        .rules_manage()
        .listed(account)
        .map_err(|why| plainly(copy::errors::NOT_READ_HERE, why))?;
    Ok(rules
        .into_iter()
        .map(|listed| Listed {
            when: listed.condition,
            does: does(&listed.rule),
            rule: listed.rule,
        })
        .collect())
}

/// How many conversations on the account match `filter` now: what applying a rule to the mail
/// already here would go through. `None` when they could not be counted.
pub(in crate::ui) fn matching(
    client: &Client,
    account: AccountId,
    filter: Filter,
    now: DateTime<Utc>,
) -> Option<usize> {
    client
        .rules_manage()
        .matching(account, filter, now)
        .ok()
        .and_then(|n| usize::try_from(n).ok())
}

/// Apply the rules to the messages of `threads`, and say what they did.
pub(in crate::ui) fn apply_to(
    client: &Client,
    threads: &[mail_domain::ThreadId],
    sending: mail_client::rules::manage::Sending,
    now: DateTime<Utc>,
) -> String {
    match client.rules_manage().apply_to(threads, sending, now) {
        Ok(acted) => copy::status::rules_applied(acted),
        Err(why) => copy::status::rules_not_applied(&why.to_string()),
    }
}

/// Everything a rule does, in one line.
pub(in crate::ui) fn does(rule: &Rule) -> String {
    let mut parts: Vec<String> = rule.actions.iter().map(copy::menus::rule_action).collect();
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
            format!("Rules don’t know “{field}:”. They know {KNOWN}.")
        }
    }
}

/// Keep `draft` as a rule on `account`, or say why not.
pub(in crate::ui) fn save<Tz: TimeZone>(
    client: &Client,
    account: AccountId,
    draft: &Draft,
    zone: &Tz,
) -> Result<Rule, String> {
    client
        .rules_manage()
        .save(account, draft, zone)
        .map_err(|why| match why {
            SaveError::NoName => "A rule needs a name.".to_owned(),
            SaveError::Taken { name } => format!("A rule called “{name}” already exists."),
            SaveError::Condition(why) => refusal(&why),
            SaveError::NoAction => "Add an action.".to_owned(),
            SaveError::Store(why) => plainly(copy::errors::NOT_KEPT_HERE, why),
        })
}

/// Move the rule `id` one place up or down, and number the account's rules 1, 2, 3… again.
pub(in crate::ui) fn reorder(
    client: &Client,
    account: AccountId,
    id: RuleId,
    step: Step,
) -> Result<(), String> {
    client
        .rules_manage()
        .reorder(account, id, step)
        .map_err(|why| plainly(copy::errors::NOT_KEPT_HERE, why))
}

/// Turn a rule on or off. Off, it is kept and skipped, here and in the server's script.
pub(in crate::ui) fn switch(client: &Client, rule: &Rule, state: RuleState) -> Result<(), String> {
    client
        .rules_manage()
        .switch(rule, state)
        .map_err(|why| plainly(copy::errors::NOT_KEPT_HERE, why))
}

/// Forget a rule, returning what the toast says.
pub(in crate::ui) fn delete(client: &Client, rule: &Rule) -> Result<String, String> {
    client
        .rules_manage()
        .delete(rule)
        .map_err(|why| plainly(copy::errors::NOT_KEPT_HERE, why))?;
    Ok(format!("Deleted the rule “{}”", rule.name))
}

/// Run `rule` over the mail already here, as `mailo rules run` does, reporting how many
/// conversations it has been through, and say what it did.
///
/// A switched-off rule runs too: asking is the switch. Blocking; the sheet calls it off the
/// thread that draws.
pub(in crate::ui) fn run(
    client: &Client,
    rule: &Rule,
    now: DateTime<Utc>,
    report: &(dyn Fn(usize) + Sync),
) -> Result<String, String> {
    let ran = client
        .rules_manage()
        .run(rule, now, report)
        .map_err(|why| plainly(copy::errors::RULE_STOPPED, why))?;
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
