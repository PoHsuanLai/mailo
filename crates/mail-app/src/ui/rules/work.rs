//! Every question the Rules sheet asks and every rule it writes, as functions of a store.
//!
//! What `mailo rules` does, reached from the window: the same `Rule` rows, the condition in the
//! same search language (`crate::query`), and the same `mail_store::rules::run_now`. The sheet
//! only draws what these answer, so the tests drive these and not the markup.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{AccountId, AfterMatch, Filter, LabelId, Rule, RuleAction, RuleId, RuleState};
use mail_store::{SqliteStore, Store};

use super::super::files::work::{grouped, messages};

/// How many conversations "Run on existing mail" takes at a time: `mailo rules run`'s batch.
const BATCH: u32 = 200;

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

/// A rule being written: what the editor's fields hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Draft {
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
    pub(in crate::ui) fn blank() -> Self {
        Self {
            id: None,
            name: String::new(),
            query: String::new(),
            actions: Vec::new(),
            after: AfterMatch::Continue,
        }
    }

    /// `listed`, opened for editing: its condition as the list shows it.
    pub(in crate::ui) fn of(listed: &Listed) -> Self {
        Self {
            id: Some(listed.rule.id),
            name: listed.rule.name.clone(),
            query: listed.when.clone(),
            actions: listed.rule.actions.clone(),
            after: listed.rule.after,
        }
    }
}

/// Which way a rule moves in the order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Step {
    Up,
    Down,
}

fn failed(error: mail_store::StoreError) -> String {
    error.to_string()
}

/// The account's labels by name, which `label:` in a condition resolves against.
pub(in crate::ui) fn labels(store: &SqliteStore, account: AccountId) -> Vec<(String, LabelId)> {
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
pub(in crate::ui) fn listed(
    store: &SqliteStore,
    account: AccountId,
) -> Result<Vec<Listed>, String> {
    let index = labels(store, account);
    let name = name_of(&index);
    let mut rules = store.rules(account).map_err(failed)?;
    rules.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(rules
        .into_iter()
        .map(|rule| Listed {
            when: crate::rules::condition(&rule.filter, &name),
            does: does(&rule),
            rule,
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

/// The condition `text` means, or why a rule cannot use it.
///
/// A search box reads a word it does not know as text, so as not to refuse what is being
/// typed. A rule is different: it runs unseen on every message that arrives, and `frm:bank` read
/// as the text "frm:bank" is a rule that silently never fires. So every `word:` term is read on
/// its own through the same parser, and one the parser did not take as that term is refused here,
/// in words, before it is kept.
pub(in crate::ui) fn read_condition<Tz: TimeZone>(
    text: &str,
    labels: &[(String, LabelId)],
    zone: &Tz,
) -> Result<Filter, String> {
    if text.trim().is_empty() {
        return Err(
            "A rule needs a condition, in the words a search takes: from:news@example.com"
                .to_owned(),
        );
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
        return Err(refusal(bare, &field.to_ascii_lowercase(), value));
    }
    Ok(crate::query::parse_with(text, zone, &named))
}

/// Why `word`, whose field is `field`, is not a term.
fn refusal(word: &str, field: &str, value: &str) -> String {
    if value.is_empty() {
        return format!("“{field}:” needs something after the colon.");
    }
    match field {
        "before" | "after" => format!("“{word}” needs a date, like {field}:2026-10-08."),
        "label" => format!("There is no label called “{value}” on this account."),
        "is" => "is: takes unread, read, starred, unstarred, pinned or snoozed.".to_owned(),
        "in" => "in: takes inbox, archive, sent, drafts, spam or trash.".to_owned(),
        "has" => "has: takes attachment.".to_owned(),
        _ => format!("“{field}:” is not a word a rule knows. It knows {KNOWN}."),
    }
}

/// How many conversations on the account match `filter` now.
pub(in crate::ui) fn matching(
    store: &SqliteStore,
    account: AccountId,
    filter: Filter,
    now: DateTime<Utc>,
) -> Result<u64, String> {
    store
        .count(&Filter::And(vec![Filter::Account(account), filter]), now)
        .map_err(failed)
}

/// What the live count under the condition says.
pub(in crate::ui) fn matching_words(count: u64) -> String {
    match count {
        0 => "Nothing here matches it now; new mail still can.".to_owned(),
        1 => "1 conversation here matches it now.".to_owned(),
        n => format!(
            "{} conversations here match it now.",
            grouped(usize::try_from(n).unwrap_or(usize::MAX))
        ),
    }
}

/// Keep `draft` as a rule on `account`, or say why not. A new rule goes last and starts on; an
/// edited one keeps its place and its switch.
pub(in crate::ui) fn save<Tz: TimeZone>(
    store: &SqliteStore,
    account: AccountId,
    draft: &Draft,
    zone: &Tz,
) -> Result<Rule, String> {
    let name = draft.name.trim();
    if name.is_empty() {
        return Err("A rule needs a name.".to_owned());
    }
    let existing = store.rules(account).map_err(failed)?;
    if existing
        .iter()
        .any(|rule| rule.name == name && Some(rule.id) != draft.id)
    {
        return Err(format!(
            "There is already a rule called “{name}” on this account."
        ));
    }
    let filter = read_condition(&draft.query, &labels(store, account), zone)?;
    if draft.actions.is_empty() && draft.after == AfterMatch::Continue {
        return Err(
            "A rule needs something to do: add an action, or stop the rules after it.".to_owned(),
        );
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
    store.put_rule(&rule).map_err(failed)?;
    Ok(rule)
}

/// Move the rule `id` one place up or down, and number the account's rules 1, 2, 3… again.
pub(in crate::ui) fn reorder(
    store: &SqliteStore,
    account: AccountId,
    id: RuleId,
    step: Step,
) -> Result<(), String> {
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
            store.put_rule(&Rule { position, ..rule }).map_err(failed)?;
        }
    }
    Ok(())
}

/// Turn a rule on or off. Off, it is kept and skipped, here and in the server's script.
pub(in crate::ui) fn switch(
    store: &SqliteStore,
    rule: &Rule,
    state: RuleState,
) -> Result<(), String> {
    store
        .put_rule(&Rule {
            state,
            ..rule.clone()
        })
        .map_err(failed)
}

/// Forget a rule, returning what the toast says.
pub(in crate::ui) fn delete(store: &SqliteStore, rule: &Rule) -> Result<String, String> {
    store.delete_rule(rule.id).map_err(failed)?;
    Ok(format!("Deleted the rule “{}”", rule.name))
}

/// The conversations "Run on existing mail" goes through: every one on the account.
pub(in crate::ui) fn conversations(
    store: &SqliteStore,
    account: AccountId,
    now: DateTime<Utc>,
) -> usize {
    store
        .count(&Filter::Account(account), now)
        .ok()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0)
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
    let caps = super::super::ops::caps_here(store, rule.account, now);
    let total = conversations(store, rule.account, now);
    let mut pages = 0usize;
    let ran = mail_store::rules::run_now(store, &caps, rule, BATCH, now, &mut |_| {
        pages += 1;
        report((pages * BATCH as usize).min(total));
    })
    .map_err(failed)?;
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
