//! `mailo rules`: what to do with mail as it arrives.
//!
//! A rule's condition is written in the search language — `from:bank.example subject:statement`
//! means here what it means in `mailo search` — and its actions are flags. Rules run on each
//! sync over mail that arrived in the inbox (`mail_store::rules::at_arrival`), and on demand
//! with `rules run`. Where the account's server takes Sieve, `mailo sieve push` puts the ones
//! it can run there too; see [`server`].

pub mod block;
pub mod manage;
pub mod server;
pub mod speak;

use crate::error::CoreError;
use chrono::{DateTime, Utc};
use mail_domain::{AfterMatch, Filter, LabelId, Rule, RuleAction, RuleId, RuleState, Words};
use mail_store::{SqliteStore, Store};

/// What `mailo rules …` asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulesCmd {
    /// Every rule, on one account or on all of them.
    List { account: Option<String> },
    Add {
        name: String,
        account: Option<String>,
        /// The condition, in the search language, as typed.
        query: String,
        actions: Vec<RuleAction>,
        after: AfterMatch,
    },
    Remove {
        name: String,
        account: Option<String>,
    },
    Set {
        name: String,
        account: Option<String>,
        state: RuleState,
    },
    /// Run one rule over the mail already here, or in one folder.
    Run {
        name: String,
        account: Option<String>,
        /// The folder's path, to run over that folder alone.
        folder: Option<String>,
        batch: u32,
    },
}

/// The one account a command means: the one named, or the only one there is.
pub(crate) fn pick(
    store: &SqliteStore,
    named: Option<&str>,
) -> Result<crate::sync::Configured, CoreError> {
    let mut accounts = crate::sync::configured(store)?;
    match named {
        Some(address) => {
            let at = accounts
                .iter()
                .position(|a| a.address.eq_ignore_ascii_case(address))
                .ok_or_else(|| CoreError::UnknownAccount(address.to_owned()))?;
            Ok(accounts.swap_remove(at))
        }
        None => match accounts.len() {
            1 => Ok(accounts.remove(0)),
            0 => Err(CoreError::AddAnAccountFirst),
            _ => Err(CoreError::NameTheAccount),
        },
    }
}

/// One rule of a listing, with its condition written back in the search language.
#[derive(Debug, Clone, PartialEq)]
pub struct ListedRule {
    pub rule: Rule,
    pub condition: String,
}

/// An account's rules, in the order they run.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountRules {
    pub address: String,
    pub rules: Vec<ListedRule>,
}

/// How far a `Run` has got: one batch of conversations looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunProgress {
    pub examined: usize,
    pub matched: usize,
}

/// What a rules command did.
#[derive(Debug, Clone, PartialEq)]
pub enum RulesDone {
    /// The accounts that have rules; none at all when no account has any.
    Listed(Vec<AccountRules>),
    Added {
        name: String,
        address: String,
        /// Whether the account's server takes Sieve, so the rule could also run there.
        server_can_run: bool,
    },
    Removed {
        name: String,
        address: String,
    },
    Set {
        name: String,
        state: RuleState,
    },
    Ran {
        name: String,
        examined: usize,
        matched: usize,
        /// Changes queued for the server.
        queued: usize,
    },
}

/// Run a rules command. `progress` hears of each batch a `Run` looks at, as it goes.
pub fn run(
    store: &SqliteStore,
    command: &RulesCmd,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(RunProgress),
) -> Result<RulesDone, CoreError> {
    match command {
        RulesCmd::List { account } => {
            let accounts = match account {
                Some(_) => vec![pick(store, account.as_deref())?],
                None => crate::sync::configured(store)?,
            };
            let mut listed = Vec::new();
            for account in accounts {
                let rules = store.rules(account.id.clone())?;
                if rules.is_empty() {
                    continue;
                }
                let labels = store.labels(account.id)?;
                let label = |id: LabelId| {
                    labels
                        .iter()
                        .find(|l| l.id == id)
                        .map(|l| l.name.clone())
                        .unwrap_or_else(|| "(deleted)".to_owned())
                };
                listed.push(AccountRules {
                    address: account.address,
                    rules: rules
                        .into_iter()
                        .map(|rule| ListedRule {
                            condition: condition(&rule.filter, &label),
                            rule,
                        })
                        .collect(),
                });
            }
            Ok(RulesDone::Listed(listed))
        }
        RulesCmd::Add {
            name,
            account,
            query,
            actions,
            after,
        } => {
            let account = pick(store, account.as_deref())?;
            let index: Vec<(String, LabelId)> = store
                .labels(account.id.clone())?
                .into_iter()
                .map(|l| (l.name, l.id))
                .collect();
            let filter =
                crate::query::parse_with(query, &chrono::Local, &crate::query::named(&index));
            let position = store
                .rules(account.id.clone())?
                .iter()
                .map(|r| r.position + 1)
                .max()
                .unwrap_or(1);
            let rule = Rule {
                id: RuleId::generate(),
                account: account.id,
                name: name.clone(),
                position,
                state: RuleState::Enabled,
                filter,
                actions: actions.clone(),
                after: *after,
            };
            store.put_rule(&rule)?;
            Ok(RulesDone::Added {
                name: name.clone(),
                server_can_run: mail_proto::sieve::endpoint(&account.plan).is_ok(),
                address: account.address,
            })
        }
        RulesCmd::Remove { name, account } => {
            let (account, rule) = named_rule(store, name, account.as_deref())?;
            store.delete_rule(rule.id)?;
            Ok(RulesDone::Removed {
                name: name.clone(),
                address: account.address,
            })
        }
        RulesCmd::Set {
            name,
            account,
            state,
        } => {
            let (_, rule) = named_rule(store, name, account.as_deref())?;
            store.put_rule(&Rule {
                state: *state,
                ..rule
            })?;
            Ok(RulesDone::Set {
                name: name.clone(),
                state: *state,
            })
        }
        RulesCmd::Run {
            name,
            account,
            folder,
            batch,
        } => {
            let (account, rule) = named_rule(store, name, account.as_deref())?;
            let within = folder.as_ref().map_or(Filter::All, |path| {
                Filter::InFolder(mail_domain::MailboxRef {
                    account: account.id.clone(),
                    path: path.clone(),
                })
            });
            let caps = crate::act::caps_here(store, account.id, now);
            let ran =
                mail_store::rules::run_now(store, &caps, &rule, &within, *batch, now, &mut |b| {
                    progress(RunProgress {
                        examined: b.examined,
                        matched: b.acted.len(),
                    });
                })?;
            Ok(RulesDone::Ran {
                name: name.clone(),
                examined: ran.examined,
                matched: ran.acted.len(),
                queued: ran.queued,
            })
        }
    }
}

fn named_rule(
    store: &SqliteStore,
    name: &str,
    account: Option<&str>,
) -> Result<(crate::sync::Configured, Rule), CoreError> {
    let account = pick(store, account)?;
    let rule = store
        .rules(account.id.clone())?
        .into_iter()
        .find(|r| r.name == name)
        .ok_or_else(|| CoreError::NoRuleNamed {
            name: name.to_owned(),
            address: account.address.clone(),
        })?;
    Ok((account, rule))
}

/// A filter written back in the search language, as near as it goes.
pub fn condition(filter: &Filter, label: &dyn Fn(LabelId) -> String) -> String {
    filter.spelled(&Words {
        label,
        // The search language has no word for a server folder; said as the path.
        folder: &|path| format!("in folder {path:?}"),
        account: "on this account",
        snooze_due: "is:snoozed",
        any_date: "",
    })
}

#[cfg(test)]
mod tests;
