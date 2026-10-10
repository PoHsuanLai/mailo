//! `mailo rules`: what to do with mail as it arrives.
//!
//! A rule's condition is written in the search language — `from:bank.example subject:statement`
//! means here what it means in `mailo search` — and its actions are flags. Rules run on each
//! sync over mail that arrived in the inbox (`mail_store::rules::at_arrival`), and on demand
//! with `rules run`. Where the account's server takes Sieve, `mailo sieve push` puts the ones
//! it can run there too; see [`server`].

pub mod block;
pub mod server;

use crate::error::CoreError;
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountCaps, AfterMatch, DateRange, Filter, LabelId, MailboxRole, ReadState, Rule, RuleAction,
    RuleId, RuleState, Star, TextMatch,
};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

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
    /// Run one rule over the mail already here.
    Run {
        name: String,
        account: Option<String>,
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
            batch,
        } => {
            let (account, rule) = named_rule(store, name, account.as_deref())?;
            let caps = caps_or_local(store, account.id, now);
            let ran = mail_store::rules::run_now(store, &caps, &rule, *batch, now, &mut |b| {
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

/// What the server was last seen to support, or nothing at all before the first sync — in which
/// case a rule acts here alone and the server hears nothing, rather than hearing a guess.
fn caps_or_local(store: &SqliteStore, account: AccountId, now: DateTime<Utc>) -> AccountCaps {
    crate::sync::caps_of(store, account).unwrap_or(AccountCaps {
        labels: mail_domain::ServerLabels::LocalOnly,
        threads: mail_domain::ServerThreads::Jwz,
        watch: mail_domain::WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: mail_domain::ArchiveMeans::LocalOnly,
        folders: mail_domain::FolderRoles::default(),
        condstore: mail_domain::Condstore::Absent,
        move_ext: mail_domain::MoveExt::Absent,
        expunge: mail_domain::ExpungeMeans::Forbidden,
        top: mail_domain::Supported::Absent,
        pipelining: mail_domain::Supported::Absent,
        connections: mail_domain::ConnectionBudget::default(),
        observed_at: now,
    })
}

/// A filter written back in the search language, as near as it goes.
pub fn condition(filter: &Filter, label: &dyn Fn(LabelId) -> String) -> String {
    let text = |m: &TextMatch| match m {
        TextMatch::Contains(s) if s.contains(char::is_whitespace) => format!("\"{s}\""),
        TextMatch::Contains(s) => s.clone(),
        TextMatch::Exact(s) => format!("\"{s}\""),
    };
    match filter {
        Filter::All => "everything".to_owned(),
        Filter::Nothing => "nothing".to_owned(),
        Filter::And(fs) => fs
            .iter()
            .map(|f| condition(f, label))
            .collect::<Vec<_>>()
            .join(" "),
        Filter::Or(fs) => format!(
            "({})",
            fs.iter()
                .map(|f| condition(f, label))
                .collect::<Vec<_>>()
                .join(" or ")
        ),
        Filter::Not(f) => format!("-{}", condition(f, label)),
        Filter::Account(_) => "on this account".to_owned(),
        Filter::InMailbox(role) => format!("in:{}", role_word(*role)),
        // The search language has no word for a server folder; said as the path.
        Filter::InFolder(mailbox) => format!("in folder {:?}", mailbox.path),
        Filter::Read(ReadState::Read) => "is:read".to_owned(),
        Filter::Read(ReadState::Unread) => "is:unread".to_owned(),
        Filter::Starred(Star::Starred) => "is:starred".to_owned(),
        Filter::Starred(Star::Unstarred) => "is:unstarred".to_owned(),
        Filter::HasLabel(id) => format!("label:{}", label(*id)),
        Filter::From(m) => format!("from:{}", text(m)),
        Filter::To(m) => format!("to:{}", text(m)),
        Filter::Subject(m) => format!("subject:{}", text(m)),
        Filter::Text(m) => text(m),
        Filter::Date(DateRange { from, to }) => {
            let mut parts = Vec::new();
            if let Some(from) = from {
                parts.push(format!("after:{}", from.format("%Y-%m-%d")));
            }
            if let Some(to) = to {
                parts.push(format!("before:{}", to.format("%Y-%m-%d")));
            }
            parts.join(" ")
        }
        Filter::HasAttachment => "has:attachment".to_owned(),
        Filter::Snoozed | Filter::SnoozeDue => "is:snoozed".to_owned(),
        Filter::Pinned => "is:pinned".to_owned(),
    }
}

fn role_word(role: MailboxRole) -> &'static str {
    match role {
        MailboxRole::Inbox => "inbox",
        MailboxRole::Archive => "archive",
        MailboxRole::Sent => "sent",
        MailboxRole::Drafts => "drafts",
        MailboxRole::Trash => "trash",
        MailboxRole::Spam => "spam",
    }
}

#[cfg(test)]
mod tests;
