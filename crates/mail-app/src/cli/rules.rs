//! `mailo rules`, `mailo sieve` and `mailo vacation`: the words typed after them, as the commands
//! [`mail_core::rules`] runs.

use crate::said::rules::said;
use chrono::{DateTime, Utc};
use mail_core::SqliteStore;
use mail_core::error::CoreError;
use mail_core::rules::server::{
    KeptVacation, PushDone, ScriptState, SieveCmd, SieveDone, SieveStatus, VacationCmd,
    VacationDone,
};
use mail_core::rules::{AccountRules, RulesCmd, RulesDone, RunProgress};
use mail_core::{AccountSecrets, ClientRegistry};
use mail_domain::{AfterMatch, Rule, RuleAction, RuleState, Vacation};
use mail_proto::sieve::Takeover;
use std::fmt::Write as _;
use std::path::PathBuf;

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
                        return Err(format!("unexpected {other:?}\n\n{}", super::usage()));
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
                        return Err(format!("unknown option {flag:?}\n\n{}", super::usage()));
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
            super::usage()
        )),
    }
}

/// `rules add NAME <search words…> [actions]`: every word that is not a flag or a flag's value
/// is part of the condition, in the order typed.
fn parse_add(args: &[String]) -> Result<RulesCmd, String> {
    let name = args
        .first()
        .filter(|n| !n.starts_with("--"))
        .ok_or_else(|| format!("rules add needs a name first\n\n{}", super::usage()))?
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
                return Err(format!("unknown option {flag:?}\n\n{}", super::usage()));
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
            super::usage()
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

pub fn parse_sieve(args: &[String]) -> Result<SieveCmd, String> {
    let mut account = None;
    let mut takeover = Takeover::Refuse;
    let mut words = args.get(1..).unwrap_or_default().iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--account" => {
                account = Some(words.next().ok_or("--account needs an address")?.clone());
            }
            "--replace-active" => takeover = Takeover::Replace,
            other => return Err(format!("unexpected {other:?}\n\n{}", super::usage())),
        }
    }
    match args.first().map(String::as_str) {
        None | Some("status") if takeover == Takeover::Refuse => Ok(SieveCmd::Status { account }),
        Some("push") => Ok(SieveCmd::Push { account, takeover }),
        _ => Err(super::usage()),
    }
}

pub fn parse_vacation(args: &[String]) -> Result<VacationCmd, String> {
    let mut account = None;
    let mut subject = None;
    let mut body_file = None;
    let mut days = Vacation::DEFAULT_DAYS;
    let (mut from, mut until) = (None, None);
    let mut words = args.get(1..).unwrap_or_default().iter();
    let value = |flag: &str, words: &mut std::slice::Iter<'_, String>| {
        words
            .next()
            .cloned()
            .ok_or_else(|| format!("{flag} needs a value"))
    };
    while let Some(word) = words.next() {
        match word.as_str() {
            "--account" => account = Some(value("--account", &mut words)?),
            "--subject" => subject = Some(value("--subject", &mut words)?),
            "--body-file" => body_file = Some(PathBuf::from(value("--body-file", &mut words)?)),
            "--days" => {
                days = value("--days", &mut words)?
                    .parse()
                    .ok()
                    .filter(|d| *d > 0)
                    .ok_or("--days needs a whole number of days, at least one")?;
            }
            "--from" => from = Some(value("--from", &mut words)?),
            "--until" => until = Some(value("--until", &mut words)?),
            other => return Err(format!("unexpected {other:?}\n\n{}", super::usage())),
        }
    }
    match args.first().map(String::as_str) {
        None | Some("show") => Ok(VacationCmd::Show { account }),
        Some("off") => Ok(VacationCmd::Off { account }),
        Some("on") => Ok(VacationCmd::On {
            account,
            subject: subject.ok_or("vacation on needs --subject")?,
            body_file: body_file.ok_or("vacation on needs --body-file with the reply's text")?,
            days,
            from,
            until,
        }),
        Some(other) => Err(format!(
            "vacation does not know {other:?}\n\n{}",
            super::usage()
        )),
    }
}

/// `rules …`: run it, and say what it did. A `rules run` reports each batch on stderr as it goes.
pub fn run(
    store: &SqliteStore,
    command: &RulesCmd,
    now: DateTime<Utc>,
) -> Result<String, CoreError> {
    let done = mail_core::rules::run(store, command, now, &mut |batch: RunProgress| {
        eprintln!("  {} looked at, {} matched", batch.examined, batch.matched);
    })?;
    Ok(rules_text(&done))
}

/// What a rules command did, for the terminal.
pub fn rules_text(done: &RulesDone) -> String {
    match done {
        RulesDone::Listed(accounts) => listing(accounts),
        RulesDone::Added {
            name,
            address,
            server_can_run,
        } => {
            let mut out = format!("added rule {name:?} on {address}\n");
            if *server_can_run {
                out.push_str(
                    "  it runs here on each sync; `mailo sieve push` also puts it on the server\n",
                );
            }
            out
        }
        RulesDone::Removed { name, address } => {
            format!("removed rule {name:?} from {address}\n")
        }
        RulesDone::Set { name, state } => format!(
            "rule {name:?} {}\n",
            match state {
                RuleState::Enabled => "enabled",
                RuleState::Disabled => "disabled",
            }
        ),
        RulesDone::Ran {
            name,
            examined,
            matched,
            queued,
        } => {
            let mut out = format!(
                "rule {name:?}: {examined} message(s) looked at, {matched} matched, {queued} change(s) queued for the server\n"
            );
            if *queued > 0 {
                out.push_str("  they reach the server on the next `mailo sync`\n");
            }
            out
        }
    }
}

fn listing(accounts: &[AccountRules]) -> String {
    if accounts.is_empty() {
        return "no rules. Add one with: mailo rules add NAME <search> --archive\n".to_owned();
    }
    let mut out = String::new();
    for account in accounts {
        let _ = writeln!(out, "{}:", account.address);
        for listed in &account.rules {
            let _ = writeln!(out, "{}", line(&listed.rule, &listed.condition));
        }
    }
    out
}

/// One rule as `rules list` prints it.
fn line(rule: &Rule, condition: &str) -> String {
    let state = match rule.state {
        RuleState::Enabled => "",
        RuleState::Disabled => " (disabled)",
    };
    let mut actions: Vec<String> = rule.actions.iter().map(action).collect();
    if rule.after == AfterMatch::Stop {
        actions.push("stop".to_owned());
    }
    format!(
        "  {}. {}{state}: {condition} → {}",
        rule.position,
        rule.name,
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

/// `vacation …`: run it, and say what it did.
pub async fn run_vacation(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    command: &VacationCmd,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<String, CoreError> {
    mail_core::rules::server::run_vacation(store, secrets, command, saved, now)
        .await
        .map(|done| vacation_text(&done))
}

/// `sieve …`: run it, and say what it did.
pub async fn run_sieve(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    command: &SieveCmd,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<String, CoreError> {
    mail_core::rules::server::run_sieve(store, secrets, command, saved, now)
        .await
        .map(|done| sieve_text(&done))
}

/// What a vacation command did, for the terminal.
pub fn vacation_text(done: &VacationDone) -> String {
    match done {
        VacationDone::Show { address, vacation } => match vacation {
            None => format!("{address}: no vacation reply\n"),
            Some(kept) => shown(kept),
        },
        VacationDone::On { kept, pushed } => {
            let mut out = shown(kept);
            out.push_str(&push_text(pushed));
            out
        }
        VacationDone::Off { address, pushed } => {
            let mut out = format!("{address}: vacation reply off\n");
            if let Some(pushed) = pushed {
                out.push_str(&push_text(pushed));
            }
            out
        }
    }
}

fn shown(kept: &KeptVacation) -> String {
    let v = &kept.vacation;
    let mut out = format!("{}: vacation reply {:?}", kept.address, v.subject);
    let when = |t: DateTime<Utc>| {
        t.with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string()
    };
    match (v.during.from, v.during.to) {
        (None, None) => out.push_str(", from now until turned off"),
        (Some(a), None) => out.push_str(&format!(", from {}", when(a))),
        (None, Some(b)) => out.push_str(&format!(", until {}", when(b))),
        (Some(a), Some(b)) => out.push_str(&format!(", {} to {}", when(a), when(b))),
    }
    let _ = write!(out, ", once every {} day(s) per sender", v.days);
    if !kept.in_effect {
        out.push_str(" (not in effect now)");
    }
    out.push('\n');
    out
}

/// A push after a change, said either way: kept here is not the same as running there.
fn push_text(pushed: &PushDone) -> String {
    match pushed {
        PushDone::Pushed { address, pushed } => said(address, pushed),
        PushDone::NotPushed(why) => format!(
            "  kept here, but not on the server yet: {why}\n  `mailo sieve push` tries again\n"
        ),
    }
}

/// What a sieve command did, for the terminal.
pub fn sieve_text(done: &SieveDone) -> String {
    match done {
        SieveDone::Pushed { address, pushed } => said(address, pushed),
        SieveDone::Status(status) => status_text(status),
    }
}

fn status_text(status: &SieveStatus) -> String {
    let mut out = format!(
        "{}: ManageSieve at {}:{}{}\n  extensions: {}\n",
        status.address,
        status.host,
        status.port,
        status
            .implementation
            .as_ref()
            .map(|i| format!(" ({i})"))
            .unwrap_or_default(),
        status.extensions.join(" ")
    );
    if status.scripts.is_empty() {
        out.push_str("  no scripts\n");
    }
    for script in &status.scripts {
        let _ = writeln!(
            out,
            "  script {:?}{}",
            script.name,
            if script.active { " (active)" } else { "" }
        );
    }
    let current = match status.script {
        ScriptState::UpToDate => "up to date",
        ScriptState::NothingToRun => "up to date: nothing to run there",
        ScriptState::OutOfDate => {
            "out of date: `mailo sieve push` installs the rules as they are now"
        }
    };
    let _ = writeln!(out, "  this client's script: {current}");
    for (name, why) in &status.local_only {
        let _ = writeln!(out, "  {name:?} runs in this client only: {why}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::{AfterMatch, RuleAction, RuleState};

    #[test]
    fn what_a_rules_command_did_is_said_in_the_words_it_always_was() {
        assert_eq!(
            rules_text(&RulesDone::Listed(Vec::new())),
            "no rules. Add one with: mailo rules add NAME <search> --archive\n"
        );
        assert_eq!(
            rules_text(&RulesDone::Added {
                name: "Bills".into(),
                address: "me@example.test".into(),
                server_can_run: true,
            }),
            "added rule \"Bills\" on me@example.test\n  it runs here on each sync; \
             `mailo sieve push` also puts it on the server\n"
        );
        assert_eq!(
            rules_text(&RulesDone::Ran {
                name: "News".into(),
                examined: 2,
                matched: 1,
                queued: 2,
            }),
            "rule \"News\": 2 message(s) looked at, 1 matched, 2 change(s) queued for the server\n  \
             they reach the server on the next `mailo sync`\n"
        );
        assert_eq!(
            status_text(&SieveStatus {
                address: "me@example.test".into(),
                host: "sieve.example.test".into(),
                port: 4190,
                implementation: Some("Dovecot".into()),
                extensions: vec!["fileinto".into(), "vacation".into()],
                scripts: Vec::new(),
                script: ScriptState::NothingToRun,
                local_only: Vec::new(),
            }),
            "me@example.test: ManageSieve at sieve.example.test:4190 (Dovecot)\n  \
             extensions: fileinto vacation\n  no scripts\n  \
             this client's script: up to date: nothing to run there\n"
        );
    }

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn every_rules_form_parses_to_what_it_says() {
        let cases: Vec<(&str, RulesCmd)> = vec![
            ("", RulesCmd::List { account: None }),
            (
                "list --account me@example.test",
                RulesCmd::List {
                    account: Some("me@example.test".into()),
                },
            ),
            (
                "add Bills from:bank.example subject:statement --read --label Money --folder Money/Bills --stop",
                RulesCmd::Add {
                    name: "Bills".into(),
                    account: None,
                    query: "from:bank.example subject:statement".into(),
                    actions: vec![
                        RuleAction::MarkRead,
                        RuleAction::Label("Money".into()),
                        RuleAction::File("Money/Bills".into()),
                    ],
                    after: AfterMatch::Stop,
                },
            ),
            (
                // Flags and words may be mixed: every word that is not a flag's is the condition.
                "add News --archive from:lists.example --account me@example.test -is:starred --star",
                RulesCmd::Add {
                    name: "News".into(),
                    account: Some("me@example.test".into()),
                    query: "from:lists.example -is:starred".into(),
                    actions: vec![RuleAction::Archive, RuleAction::Star],
                    after: AfterMatch::Continue,
                },
            ),
            (
                "add Junk in:inbox has:attachment --spam --trash",
                RulesCmd::Add {
                    name: "Junk".into(),
                    account: None,
                    query: "in:inbox has:attachment".into(),
                    actions: vec![RuleAction::Spam, RuleAction::Trash],
                    after: AfterMatch::Continue,
                },
            ),
            (
                "remove Bills",
                RulesCmd::Remove {
                    name: "Bills".into(),
                    account: None,
                },
            ),
            (
                "enable Bills --account me@example.test",
                RulesCmd::Set {
                    name: "Bills".into(),
                    account: Some("me@example.test".into()),
                    state: RuleState::Enabled,
                },
            ),
            (
                "disable Bills",
                RulesCmd::Set {
                    name: "Bills".into(),
                    account: None,
                    state: RuleState::Disabled,
                },
            ),
            (
                "run Bills",
                RulesCmd::Run {
                    name: "Bills".into(),
                    account: None,
                    batch: 200,
                },
            ),
            (
                "run Bills --batch 50",
                RulesCmd::Run {
                    name: "Bills".into(),
                    account: None,
                    batch: 50,
                },
            ),
        ];
        for (line, want) in cases {
            assert_eq!(parse(&args(line)), Ok(want), "rules {line}");
        }
    }

    #[test]
    fn a_rules_command_that_cannot_mean_anything_is_refused() {
        for line in [
            "add",
            "add --archive",
            // No condition: a rule over every message is not written by accident.
            "add Everything --archive",
            // Nothing to do.
            "add Quiet from:x",
            "add Bad from:x --label",
            "add Bad from:x --frobnicate",
            "remove",
            "run Bills --batch 0",
            "enable a b",
            "frobnicate",
        ] {
            assert!(parse(&args(line)).is_err(), "rules {line}");
        }
    }

    #[test]
    fn sieve_and_vacation_parse_to_what_they_say() {
        assert_eq!(
            parse_sieve(&args("")),
            Ok(SieveCmd::Status { account: None })
        );
        assert_eq!(
            parse_sieve(&args("push --replace-active --account me@example.test")),
            Ok(SieveCmd::Push {
                account: Some("me@example.test".into()),
                takeover: Takeover::Replace,
            })
        );
        assert_eq!(
            parse_sieve(&args("push")),
            Ok(SieveCmd::Push {
                account: None,
                takeover: Takeover::Refuse,
            })
        );
        assert!(parse_sieve(&args("status --replace-active")).is_err());
        assert!(parse_sieve(&args("frobnicate")).is_err());

        assert_eq!(
            parse_vacation(&args(
                "on --subject Away --body-file away.txt --days 3 --from 2026-10-01 --until 2026-10-08T09:00"
            )),
            Ok(VacationCmd::On {
                account: None,
                subject: "Away".into(),
                body_file: "away.txt".into(),
                days: 3,
                from: Some("2026-10-01".into()),
                until: Some("2026-10-08T09:00".into()),
            })
        );
        assert_eq!(
            parse_vacation(&args("on --subject Away --body-file away.txt")),
            Ok(VacationCmd::On {
                account: None,
                subject: "Away".into(),
                body_file: "away.txt".into(),
                days: 7,
                from: None,
                until: None,
            })
        );
        assert_eq!(
            parse_vacation(&args("off --account me@example.test")),
            Ok(VacationCmd::Off {
                account: Some("me@example.test".into())
            })
        );
        assert_eq!(
            parse_vacation(&args("")),
            Ok(VacationCmd::Show { account: None })
        );
        for line in [
            "on --body-file away.txt",
            "on --subject Away",
            "on --subject Away --body-file a --days 0",
            "on --subject Away --body-file a --days many",
            "maybe",
        ] {
            assert!(parse_vacation(&args(line)).is_err(), "vacation {line}");
        }
    }
}
