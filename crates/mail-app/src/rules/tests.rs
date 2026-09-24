use super::server::{SieveCmd, VacationCmd, instant, parse_sieve, parse_vacation};
use super::*;
use mail_proto::sieve::Takeover;

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

#[test]
fn a_date_is_the_start_of_that_day_in_the_readers_zone() {
    let taipei = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    let cases = [
        ("2026-10-08", "2026-10-07T16:00:00+00:00"),
        ("2026-10-08T09:00", "2026-10-08T01:00:00+00:00"),
        ("2026-10-08 09:30", "2026-10-08T01:30:00+00:00"),
    ];
    for (text, want) in cases {
        assert_eq!(instant(text, &taipei).unwrap().to_rfc3339(), want, "{text}");
    }
    assert!(instant("next week", &taipei).is_err());
}

#[test]
fn a_rule_is_listed_in_the_words_it_was_written_in() {
    let index: Vec<(String, LabelId)> = vec![("Money".into(), LabelId::generate())];
    let label_id = index[0].1;
    let filter = crate::query::parse_with(
        "from:bank.example -is:read label:money \"due date\"",
        &chrono::Utc,
        &crate::query::named(&index),
    );
    let name = |id: LabelId| {
        if id == label_id {
            "Money".to_owned()
        } else {
            "?".to_owned()
        }
    };
    assert_eq!(
        condition(&filter, &name),
        "from:bank.example -is:read label:Money \"due date\""
    );
}
