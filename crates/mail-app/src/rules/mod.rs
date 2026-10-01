//! `mailo rules`: what to do with mail as it arrives.
//!
//! A rule's condition is written in the search language — `from:bank.example subject:statement`
//! means here what it means in `mailo search` — and its actions are flags. Rules run on each
//! sync over mail that arrived in the inbox (`mail_store::rules::at_arrival`), and on demand
//! with `rules run`. Where the account's server takes Sieve, `mailo sieve push` puts the ones
//! it can run there too; see [`server`].

pub mod block;
pub mod server;

use chrono::{DateTime, Utc};
use mail_domain::{
    AccountCaps, AccountId, AfterMatch, DateRange, Filter, LabelId, MailboxRole, ReadState, Rule,
    RuleAction, RuleId, RuleState, Star, TextMatch,
};
use mail_store::{SqliteStore, Store};
use std::fmt::Write as _;

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

/// How many conversations `rules run` takes at a time.
const BATCH: u32 = 200;

/// Parse what follows `rules`.
pub fn parse(args: &[String]) -> Result<RulesCmd, String> {
    let verb = args.first().map(String::as_str);
    let rest = args.get(1..).unwrap_or_default();
    match verb {
        None | Some("list") => {
            let mut account = None;
            let mut words = rest.iter();
            while let Some(word) = words.next() {
                match word.as_str() {
                    "--account" => account = Some(value(&mut words, "--account", "an address")?),
                    other => {
                        return Err(format!("unexpected {other:?}\n\n{}", crate::cli::usage()));
                    }
                }
            }
            Ok(RulesCmd::List { account })
        }
        Some("add") => parse_add(rest),
        Some(verb @ ("remove" | "enable" | "disable" | "run")) => {
            let mut name = None;
            let mut account = None;
            let mut batch = BATCH;
            let mut words = rest.iter();
            while let Some(word) = words.next() {
                match word.as_str() {
                    "--account" => account = Some(value(&mut words, "--account", "an address")?),
                    "--batch" if verb == "run" => {
                        batch = value(&mut words, "--batch", "a number")?
                            .parse()
                            .ok()
                            .filter(|n| *n > 0)
                            .ok_or("--batch needs a number above nought")?;
                    }
                    flag if flag.starts_with("--") => {
                        return Err(format!(
                            "unknown option {flag:?}\n\n{}",
                            crate::cli::usage()
                        ));
                    }
                    _ if name.is_none() => name = Some(word.clone()),
                    other => return Err(format!("unexpected {other:?}: one rule at a time")),
                }
            }
            let name = name.ok_or_else(|| format!("rules {verb} needs a rule's name"))?;
            Ok(match verb {
                "remove" => RulesCmd::Remove { name, account },
                "enable" => RulesCmd::Set {
                    name,
                    account,
                    state: RuleState::Enabled,
                },
                "disable" => RulesCmd::Set {
                    name,
                    account,
                    state: RuleState::Disabled,
                },
                _ => RulesCmd::Run {
                    name,
                    account,
                    batch,
                },
            })
        }
        Some(other) => Err(format!(
            "rules does not know {other:?}\n\n{}",
            crate::cli::usage()
        )),
    }
}

/// `rules add NAME <search words…> [actions]`: every word that is not a flag or a flag's value
/// is part of the condition, in the order typed.
fn parse_add(args: &[String]) -> Result<RulesCmd, String> {
    let name = args
        .first()
        .filter(|n| !n.starts_with("--"))
        .ok_or_else(|| format!("rules add needs a name first\n\n{}", crate::cli::usage()))?
        .clone();
    let mut account = None;
    let mut actions = Vec::new();
    let mut after = AfterMatch::Continue;
    let mut query: Vec<String> = Vec::new();
    let mut words = args[1..].iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--account" => account = Some(value(&mut words, "--account", "an address")?),
            "--label" => actions.push(RuleAction::Label(value(&mut words, "--label", "a name")?)),
            "--folder" => actions.push(RuleAction::File(value(&mut words, "--folder", "a path")?)),
            "--archive" => actions.push(RuleAction::Archive),
            "--trash" => actions.push(RuleAction::Trash),
            "--spam" => actions.push(RuleAction::Spam),
            "--read" => actions.push(RuleAction::MarkRead),
            "--star" => actions.push(RuleAction::Star),
            "--stop" => after = AfterMatch::Stop,
            flag if flag.starts_with("--") => {
                return Err(format!(
                    "unknown option {flag:?}\n\n{}",
                    crate::cli::usage()
                ));
            }
            _ => query.push(word.clone()),
        }
    }
    let query = query.join(" ");
    if query.trim().is_empty() {
        // A rule with no condition files every message the account receives. Asked for
        // explicitly or not at all.
        return Err(format!(
            "rules add needs a condition, in the words search takes: \
             mailo rules add {name} from:news@example.com --archive"
        ));
    }
    if actions.is_empty() && after == AfterMatch::Continue {
        return Err(format!(
            "rules add needs something to do: --label NAME, --folder PATH, --archive, --trash, \
             --spam, --read, --star or --stop\n\n{}",
            crate::cli::usage()
        ));
    }
    Ok(RulesCmd::Add {
        name,
        account,
        query,
        actions,
        after,
    })
}

fn value(
    words: &mut std::slice::Iter<'_, String>,
    flag: &str,
    what: &str,
) -> Result<String, String> {
    words
        .next()
        .filter(|v| !v.starts_with("--"))
        .cloned()
        .ok_or_else(|| format!("{flag} needs {what}"))
}

/// The one account a command means: the one named, or the only one there is.
pub(crate) fn pick(
    store: &SqliteStore,
    named: Option<&str>,
) -> Result<crate::sync::Configured, String> {
    let mut accounts = crate::sync::configured(store)?;
    match named {
        Some(address) => {
            let at = accounts
                .iter()
                .position(|a| a.address.eq_ignore_ascii_case(address))
                .ok_or_else(|| format!("no account {address:?}"))?;
            Ok(accounts.swap_remove(at))
        }
        None => match accounts.len() {
            1 => Ok(accounts.remove(0)),
            0 => Err("add an account first: mailo account add <address>".to_owned()),
            _ => Err("name the account: --account you@example.com".to_owned()),
        },
    }
}

/// Run a rules command, returning what to print.
pub fn run(store: &SqliteStore, command: &RulesCmd, now: DateTime<Utc>) -> Result<String, String> {
    let failed = |e: mail_store::StoreError| e.to_string();
    match command {
        RulesCmd::List { account } => {
            let accounts = match account {
                Some(_) => vec![pick(store, account.as_deref())?],
                None => crate::sync::configured(store)?,
            };
            let mut out = String::new();
            for account in accounts {
                let rules = store.rules(account.id).map_err(failed)?;
                if rules.is_empty() {
                    continue;
                }
                let labels = store.labels(account.id).map_err(failed)?;
                let label = |id: LabelId| {
                    labels
                        .iter()
                        .find(|l| l.id == id)
                        .map(|l| l.name.clone())
                        .unwrap_or_else(|| "(deleted)".to_owned())
                };
                let _ = writeln!(out, "{}:", account.address);
                for rule in &rules {
                    let _ = writeln!(out, "{}", line(rule, &label));
                }
            }
            if out.is_empty() {
                out.push_str("no rules. Add one with: mailo rules add NAME <search> --archive\n");
            }
            Ok(out)
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
                .labels(account.id)
                .map_err(failed)?
                .into_iter()
                .map(|l| (l.name, l.id))
                .collect();
            let filter =
                crate::query::parse_with(query, &chrono::Local, &crate::query::named(&index));
            let position = store
                .rules(account.id)
                .map_err(failed)?
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
            store.put_rule(&rule).map_err(failed)?;
            let mut out = format!("added rule {name:?} on {}\n", account.address);
            if mail_proto::sieve::endpoint(&account.plan).is_ok() {
                out.push_str(
                    "  it runs here on each sync; `mailo sieve push` also puts it on the server\n",
                );
            }
            Ok(out)
        }
        RulesCmd::Remove { name, account } => {
            let (account, rule) = named_rule(store, name, account.as_deref())?;
            store.delete_rule(rule.id).map_err(failed)?;
            Ok(format!("removed rule {name:?} from {}\n", account.address))
        }
        RulesCmd::Set {
            name,
            account,
            state,
        } => {
            let (_, rule) = named_rule(store, name, account.as_deref())?;
            store
                .put_rule(&Rule {
                    state: *state,
                    ..rule
                })
                .map_err(failed)?;
            Ok(format!(
                "rule {name:?} {}\n",
                match state {
                    RuleState::Enabled => "enabled",
                    RuleState::Disabled => "disabled",
                }
            ))
        }
        RulesCmd::Run {
            name,
            account,
            batch,
        } => {
            let (account, rule) = named_rule(store, name, account.as_deref())?;
            let caps = caps_or_local(store, account.id, now);
            let ran = mail_store::rules::run_now(store, &caps, &rule, *batch, now, &mut |b| {
                eprintln!("  {} looked at, {} matched", b.examined, b.acted.len());
            })
            .map_err(failed)?;
            let mut out = format!(
                "rule {name:?}: {} message(s) looked at, {} matched, {} change(s) queued for the server\n",
                ran.examined,
                ran.acted.len(),
                ran.queued
            );
            if ran.queued > 0 {
                out.push_str("  they reach the server on the next `mailo sync`\n");
            }
            Ok(out)
        }
    }
}

fn named_rule(
    store: &SqliteStore,
    name: &str,
    account: Option<&str>,
) -> Result<(crate::sync::Configured, Rule), String> {
    let account = pick(store, account)?;
    let rule = store
        .rules(account.id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|r| r.name == name)
        .ok_or_else(|| format!("no rule {name:?} on {}", account.address))?;
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

/// One rule as `rules list` prints it.
fn line(rule: &Rule, label: &dyn Fn(LabelId) -> String) -> String {
    let state = match rule.state {
        RuleState::Enabled => "",
        RuleState::Disabled => " (disabled)",
    };
    let mut actions: Vec<String> = rule.actions.iter().map(action).collect();
    if rule.after == AfterMatch::Stop {
        actions.push("stop".to_owned());
    }
    format!(
        "  {}. {}{state}: {} → {}",
        rule.position,
        rule.name,
        condition(&rule.filter, label),
        actions.join(", ")
    )
}

fn action(action: &RuleAction) -> String {
    match action {
        RuleAction::Label(name) => format!("label {name}"),
        RuleAction::File(path) => format!("move to {path}"),
        RuleAction::Archive => "archive".to_owned(),
        RuleAction::Trash => "trash".to_owned(),
        RuleAction::Spam => "spam".to_owned(),
        RuleAction::MarkRead => "mark read".to_owned(),
        RuleAction::Star => "star".to_owned(),
    }
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
