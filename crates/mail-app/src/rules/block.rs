//! "Block sender": a rule that sends one address's mail to Spam, and stops the rules after it.
//!
//! An ordinary [`Rule`], written through the same rows the Rules sheet and `mailo rules` use,
//! so it is listed, edited, switched off and put on a Sieve server like any other. Its condition
//! is the whole address (`TextMatch::Exact`), which the Sieve compiler writes as an `address
//! :all :is "from"` test; a substring would also block every address that contains this one.
//!
//! It runs where rules run: on mail as it arrives. Mail already here stays where it is.

use mail_domain::{AccountId, AfterMatch, Filter, Rule, RuleAction, RuleId, RuleState, TextMatch};
use mail_store::{SqliteStore, Store};

/// What blocking did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocked {
    /// A rule was made.
    Made(Rule),
    /// The address was already blocked by this rule; nothing was written.
    Already(Rule),
}

/// The condition a block of `email` is: its From is exactly that address.
pub fn condition(email: &str) -> Filter {
    Filter::From(TextMatch::Exact(email.trim().to_owned()))
}

/// Whether `rule` is a block of `email`: its condition, and Spam among its actions.
fn blocks(rule: &Rule, email: &str) -> bool {
    let same = match &rule.filter {
        Filter::From(TextMatch::Exact(had)) => had.eq_ignore_ascii_case(email.trim()),
        _ => false,
    };
    same && rule.actions.contains(&RuleAction::Spam)
}

/// Block `email` on `account`: one rule, first in the order so no earlier rule files the mail
/// somewhere else or stops the rules before it is reached. Blocking an address already blocked
/// writes nothing.
pub fn block(store: &SqliteStore, account: AccountId, email: &str) -> Result<Blocked, String> {
    let email = email.trim();
    if email.is_empty() || !email.contains('@') {
        return Err(format!("{email:?} is not an address to block"));
    }
    let existing = store.rules(account).map_err(|e| e.to_string())?;
    if let Some(rule) = existing.iter().find(|rule| blocks(rule, email)) {
        return Ok(Blocked::Already(rule.clone()));
    }
    // Names are unique on an account; a rule the user already called this keeps its name.
    let base = format!("Block {email}");
    let name = std::iter::once(base.clone())
        .chain((2..).map(|n| format!("{base} ({n})")))
        .find(|name| existing.iter().all(|rule| &rule.name != name))
        .unwrap_or(base); // `find` over an endless chain only ends by finding one.
    // Before every other rule. Positions start at 1 from the sheet and the command line, so
    // this is 0; blocks share it and run in name order, which between blocks does not matter.
    let position = existing
        .iter()
        .map(|rule| rule.position)
        .min()
        .map_or(1, |first| first.saturating_sub(1));
    let rule = Rule {
        id: RuleId::generate(),
        account,
        name,
        position,
        state: RuleState::Enabled,
        filter: condition(email),
        actions: vec![RuleAction::Spam],
        after: AfterMatch::Stop,
    };
    store.put_rule(&rule).map_err(|e| e.to_string())?;
    Ok(Blocked::Made(rule))
}

/// Take a block back: forget the rule it made.
pub fn unblock(store: &SqliteStore, rule: RuleId) -> Result<(), String> {
    store.delete_rule(rule).map_err(|e| e.to_string())
}
