//! Sieve scripts written from rules (goldens), and ManageSieve sessions (transcripts).

mod common;

use chrono::{DateTime, TimeZone, Utc};
use common::replay;
use mail_domain::{
    AccountId, AfterMatch, Credential, DateRange, Filter, LabelId, MailboxRole, Rule, RuleAction,
    RuleId, RuleState, TextMatch, Tls, Vacation,
};
use mail_proto::ProtoError;
use mail_proto::sieve::{
    Active, Compiled, Deleted, Places, ScriptEntry, SieveJob, SieveLogin, SieveOutcome,
    SieveSession, Takeover, Unmappable, VacationPlaced, compile, quoted,
};

const ACCOUNT: AccountId = AccountId::from_uuid(uuid::Uuid::from_u128(1));

fn at(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, day, 0, 0, 0).unwrap()
}

fn rule(position: u32, name: &str, filter: Filter, actions: Vec<RuleAction>) -> Rule {
    Rule {
        id: RuleId::from_uuid(uuid::Uuid::from_u128(u128::from(position) + 100)),
        account: ACCOUNT,
        name: name.to_owned(),
        position,
        state: RuleState::Enabled,
        filter,
        actions,
        after: AfterMatch::Continue,
    }
}

fn contains(s: &str) -> TextMatch {
    TextMatch::Contains(s.to_owned())
}

fn every_extension() -> Vec<String> {
    ["fileinto", "imap4flags", "vacation", "date", "relational"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn places() -> Places {
    Places {
        archive: Some("Archive".to_owned()),
        trash: Some("Trash".to_owned()),
        spam: Some("Junk".to_owned()),
    }
}

/// A golden is written with LF for legibility; scripts use CRLF.
fn crlf(text: &str) -> String {
    text.replace('\n', "\r\n")
}

fn compiled(rules: &[Rule], vacation: Option<&Vacation>) -> Compiled {
    compile(rules, vacation, &every_extension(), &places(), at(1))
}

#[test]
fn rules_become_one_script_in_their_order_with_only_what_they_need() {
    let mut bills = rule(
        2,
        "Bills",
        Filter::And(vec![
            Filter::From(contains("bank.example")),
            Filter::Subject(contains("statement")),
        ]),
        vec![
            RuleAction::MarkRead,
            RuleAction::File("Money/Bills".to_owned()),
        ],
    );
    bills.after = AfterMatch::Stop;
    let lists = rule(
        1,
        "Lists",
        Filter::Or(vec![
            Filter::To(contains("dev@lists.example")),
            Filter::Not(Box::new(Filter::From(TextMatch::Exact(
                "boss@example.test".to_owned(),
            )))),
        ]),
        vec![RuleAction::Star, RuleAction::Archive],
    );
    let out = compiled(&[bills, lists], None);
    let want = crlf(
        r#"# Written by mailo, and replaced whenever its rules change: edit them there.
require ["fileinto", "imap4flags"];

# Lists
if anyof (header :contains ["to", "cc"] "dev@lists.example", not address :all :is "from" "boss@example.test") {
    addflag "\\Flagged";
    fileinto "Archive";
}

# Bills
if allof (header :contains "from" "bank.example", header :contains "subject" "statement") {
    addflag "\\Seen";
    fileinto "Money/Bills";
    stop;
}
"#,
    );
    assert_eq!(out.script, want);
    assert_eq!(out.mapped, vec!["Lists".to_owned(), "Bills".to_owned()]);
    assert!(out.local_only.is_empty());
    assert_eq!(out.vacation, VacationPlaced::Absent);
}

#[test]
fn quotes_backslashes_and_line_breaks_are_escaped_in_every_string() {
    const CASES: &[(&str, &str)] = &[
        ("plain", "\"plain\""),
        ("say \"hi\"", "\"say \\\"hi\\\"\""),
        ("C:\\mail\\", "\"C:\\\\mail\\\\\""),
        ("\\\"", "\"\\\\\\\"\""),
        ("two\nlines", "\"two\r\nlines\""),
        ("already\r\ncrlf", "\"already\r\ncrlf\""),
        ("nul\0gone", "\"nulgone\""),
        ("日本語", "\"日本語\""),
    ];
    for (text, want) in CASES {
        assert_eq!(quoted(text), *want, "quoting {text:?}");
    }

    // And in place: a needle, a folder and a rule name that try to end their string or line.
    let hostile = rule(
        1,
        "evil\n} stop; if true {",
        Filter::Subject(contains("a\"b\\c")),
        vec![RuleAction::File("Odd \"Folder\"\\x".to_owned())],
    );
    let out = compiled(&[hostile], None);
    assert!(
        out.script
            .contains("header :contains \"subject\" \"a\\\"b\\\\c\""),
        "{}",
        out.script
    );
    assert!(
        out.script.contains("fileinto \"Odd \\\"Folder\\\"\\\\x\";"),
        "{}",
        out.script
    );
    assert!(
        out.script.contains("# evil } stop; if true {\r\n"),
        "a name stays inside its comment: {}",
        out.script
    );
}

#[test]
fn rules_sieve_cannot_express_stay_local_and_say_why() {
    let cases: Vec<(Rule, Unmappable)> = vec![
        (
            rule(
                1,
                "words",
                Filter::Text(contains("invoice")),
                vec![RuleAction::Archive],
            ),
            Unmappable::Clause("full text"),
        ),
        (
            rule(
                2,
                "labelled",
                Filter::HasLabel(LabelId::from_uuid(uuid::Uuid::from_u128(9))),
                vec![RuleAction::Archive],
            ),
            Unmappable::Clause("label:"),
        ),
        (
            rule(
                3,
                "unread",
                Filter::Read(mail_domain::ReadState::Unread),
                vec![RuleAction::Star],
            ),
            Unmappable::Clause("is:read / is:unread"),
        ),
        (
            rule(
                4,
                "label action",
                Filter::All,
                vec![RuleAction::Label("Work".to_owned())],
            ),
            Unmappable::Label,
        ),
        (
            rule(
                5,
                "a name, exactly",
                Filter::From(TextMatch::Exact("Ada Lovelace".to_owned())),
                vec![RuleAction::Archive],
            ),
            Unmappable::Clause("from:"),
        ),
        (
            rule(
                6,
                "spanning",
                Filter::From(contains("Ada <ada")),
                vec![RuleAction::Archive],
            ),
            Unmappable::Clause("from:"),
        ),
        (
            rule(
                7,
                "attached",
                Filter::HasAttachment,
                vec![RuleAction::Archive],
            ),
            Unmappable::Clause("has:attachment"),
        ),
    ];
    for (r, why) in &cases {
        let out = compiled(std::slice::from_ref(r), None);
        assert_eq!(out.local_only, vec![(r.name.clone(), *why)], "{}", r.name);
        assert!(out.mapped.is_empty(), "{}", r.name);
        assert!(out.is_empty(), "{}: nothing to install", r.name);
    }

    // No spam folder named, no `fileinto` offered: the rule stays here rather than guessing.
    let spam = rule(1, "spam", Filter::All, vec![RuleAction::Spam]);
    let out = compile(
        std::slice::from_ref(&spam),
        None,
        &every_extension(),
        &Places::default(),
        at(1),
    );
    assert_eq!(
        out.local_only,
        vec![("spam".to_owned(), Unmappable::NoFolder(MailboxRole::Spam))]
    );
    let out = compile(&[spam], None, &["vacation".to_owned()], &places(), at(1));
    assert_eq!(
        out.local_only,
        vec![("spam".to_owned(), Unmappable::Missing("fileinto"))]
    );
}

#[test]
fn a_disabled_rule_is_left_out_and_not_called_local() {
    let mut off = rule(1, "off", Filter::All, vec![RuleAction::Archive]);
    off.state = RuleState::Disabled;
    let out = compiled(&[off], None);
    assert!(out.mapped.is_empty() && out.local_only.is_empty());
    assert!(out.is_empty());
}

fn vacation() -> Vacation {
    Vacation {
        account: ACCOUNT,
        subject: "Away until the 8th".to_owned(),
        body: "I am away.\nFor \"urgent\" things, call the desk.".to_owned(),
        days: 7,
        addresses: vec![
            "me@example.test".to_owned(),
            "alias@example.test".to_owned(),
        ],
        from: Some("me@example.test".to_owned()),
        during: DateRange {
            from: Some(at(1)),
            to: Some(at(8)),
        },
    }
}

#[test]
fn a_vacation_reply_with_dates_is_tested_by_the_server() {
    let out = compiled(&[], Some(&vacation()));
    let want = crlf(
        r#"# Written by mailo, and replaced whenever its rules change: edit them there.
require ["date", "relational", "vacation"];

# Vacation reply
if allof (currentdate :zone "+0000" :value "ge" "iso8601" "2026-10-01T00:00:00", currentdate :zone "+0000" :value "lt" "iso8601" "2026-10-08T00:00:00") {
    vacation :days 7 :subject "Away until the 8th" :from "me@example.test" :addresses ["me@example.test", "alias@example.test"] "I am away.
For \"urgent\" things, call the desk.";
}
"#,
    );
    assert_eq!(out.script, want);
    assert_eq!(out.vacation, VacationPlaced::Dated);
    assert!(!out.is_empty());
}

#[test]
fn without_the_date_extension_the_reply_is_in_only_while_it_applies() {
    let no_dates = vec!["vacation".to_owned(), "fileinto".to_owned()];
    let open = Vacation {
        during: DateRange::default(),
        from: None,
        addresses: Vec::new(),
        ..vacation()
    };
    let out = compile(&[], Some(&open), &no_dates, &places(), at(20));
    let want = crlf(
        r#"# Written by mailo, and replaced whenever its rules change: edit them there.
require ["vacation"];

# Vacation reply
vacation :days 7 :subject "Away until the 8th" "I am away.
For \"urgent\" things, call the desk.";
"#,
    );
    assert_eq!(out.script, want);
    assert_eq!(out.vacation, VacationPlaced::Undated);

    // Dated, and the server cannot test the dates: in during them, out either side.
    let cases = [
        (at(1), VacationPlaced::Undated),
        (at(7), VacationPlaced::Undated),
        (at(8), VacationPlaced::Outside),
        (
            Utc.with_ymd_and_hms(2026, 9, 30, 23, 59, 59).unwrap(),
            VacationPlaced::Outside,
        ),
    ];
    for (now, want) in cases {
        let out = compile(&[], Some(&vacation()), &no_dates, &places(), now);
        assert_eq!(out.vacation, want, "at {now}");
        assert_eq!(
            out.script.contains("vacation :days"),
            want == VacationPlaced::Undated
        );
    }

    let out = compile(
        &[],
        Some(&vacation()),
        &["fileinto".to_owned()],
        &places(),
        at(2),
    );
    assert_eq!(out.vacation, VacationPlaced::Unsupported);
    assert!(out.is_empty());
}

// ---------------------------------------------------------------------------------------
// ManageSieve transcripts.
// ---------------------------------------------------------------------------------------

fn login(credential: Credential) -> SieveLogin {
    SieveLogin {
        host: "sieve.example.test".to_owned(),
        port: 4190,
        tls: Tls::StartTlsRequired,
        username: "me@example.test".to_owned(),
        credential,
    }
}

fn password() -> Credential {
    Credential::Password("s3cret".to_owned())
}

fn session(job: SieveJob) -> SieveSession {
    SieveSession::new(login(password()), job)
}

fn install(takeover: Takeover) -> SieveJob {
    SieveJob::Install {
        script: String::from_utf8(include_bytes!("traces/sieve/install.script").to_vec()).unwrap(),
        takeover,
    }
}

#[test]
fn status_upgrades_first_then_lists_and_fetches_our_script() {
    let out = replay(
        &mut session(SieveJob::Status),
        include_str!("traces/sieve/status.trace"),
    )
    .unwrap();
    let SieveOutcome::Status {
        caps,
        scripts,
        ours,
    } = out
    else {
        panic!("{out:?}");
    };
    // The capabilities from inside TLS, not the plaintext ones, which offered no mechanism.
    assert_eq!(caps.sasl, vec!["PLAIN".to_owned(), "LOGIN".to_owned()]);
    assert!(caps.has_extension("vacation") && caps.has_extension("DATE"));
    assert!(!caps.has_extension("date relational"));
    assert_eq!(
        scripts,
        vec![
            ScriptEntry {
                name: "webmail".to_owned(),
                active: Active::No
            },
            ScriptEntry {
                name: "mailo".to_owned(),
                active: Active::Yes
            },
        ]
    );
    assert_eq!(ours.as_deref(), Some("keep;\r\n# done\r\n"));
}

#[test]
fn without_starttls_nothing_is_signed_in_and_the_session_ends() {
    for trace in [
        include_str!("traces/sieve/starttls_absent.trace"),
        include_str!("traces/sieve/starttls_refused.trace"),
    ] {
        let err = replay(&mut session(SieveJob::Status), trace).unwrap_err();
        assert!(matches!(err, ProtoError::Unsupported(_)), "{err}");
    }
}

#[test]
fn bytes_injected_before_the_handshake_are_refused() {
    let err = replay(
        &mut session(SieveJob::Status),
        include_str!("traces/sieve/starttls_injected.trace"),
    )
    .unwrap_err();
    assert!(matches!(err, ProtoError::Malformed(_)), "{err}");
}

#[test]
fn a_script_goes_up_as_a_literal_and_is_made_active() {
    let out = replay(
        &mut session(install(Takeover::Refuse)),
        include_str!("traces/sieve/install.trace"),
    )
    .unwrap();
    assert!(
        matches!(
            out,
            SieveOutcome::Installed {
                displaced: None,
                ..
            }
        ),
        "{out:?}"
    );
}

#[test]
fn another_active_script_is_never_replaced_unasked() {
    let out = replay(
        &mut session(install(Takeover::Refuse)),
        include_str!("traces/sieve/install_other_active.trace"),
    )
    .unwrap();
    let SieveOutcome::Refused { active, .. } = out else {
        panic!("{out:?}");
    };
    assert_eq!(active, "webmail");
}

#[test]
fn told_to_replace_it_ours_is_activated_and_theirs_is_named() {
    let out = replay(
        &mut session(install(Takeover::Replace)),
        include_str!("traces/sieve/install_replace.trace"),
    )
    .unwrap();
    let SieveOutcome::Installed { displaced, .. } = out else {
        panic!("{out:?}");
    };
    assert_eq!(displaced.as_deref(), Some("webmail"));
}

#[test]
fn a_script_the_server_rejects_is_a_permanent_refusal() {
    let err = replay(
        &mut session(install(Takeover::Refuse)),
        include_str!("traces/sieve/install_rejected.trace"),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            ProtoError::Refused {
                kind: mail_proto::Refusal::Permanent,
                ..
            }
        ),
        "{err}"
    );
}

#[test]
fn removing_an_active_script_switches_it_off_first() {
    let out = replay(
        &mut session(SieveJob::Remove),
        include_str!("traces/sieve/remove_active.trace"),
    )
    .unwrap();
    assert!(
        matches!(
            out,
            SieveOutcome::Removed {
                deleted: Deleted::Ours,
                ..
            }
        ),
        "{out:?}"
    );
}

#[test]
fn a_refused_sign_in_asks_for_reauthentication() {
    let err = replay(
        &mut session(SieveJob::Status),
        include_str!("traces/sieve/auth_rejected.trace"),
    )
    .unwrap_err();
    assert!(matches!(err, ProtoError::AuthRejected(_)), "{err}");

    let bearer = Credential::OAuth {
        access: "ya29.token".to_owned(),
        refresh: "refresh".to_owned(),
        expires_at: at(9),
    };
    let err = replay(
        &mut SieveSession::new(login(bearer), SieveJob::Status),
        include_str!("traces/sieve/xoauth2_challenge.trace"),
    )
    .unwrap_err();
    assert!(matches!(err, ProtoError::AuthRejected(_)), "{err}");
}

// ---------------------------------------------------------------------------------------
// Where the server is.
// ---------------------------------------------------------------------------------------

#[test]
fn the_server_is_the_incoming_host_on_4190_with_starttls_required() {
    use mail_domain::{AccountPlan, AuthPlan, Incoming, OAuthIssuer, Outgoing, SaslMech, Username};
    use mail_proto::sieve::{Endpoint, NoSieve, endpoint};
    let plan = |host: &str, auth: AuthPlan| AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Imap {
            host: host.to_owned(),
            port: 993,
            tls: Tls::Implicit,
        },
        outgoing: Outgoing::Nowhere,
        auth,
        identities: vec![],
    };
    let password = AuthPlan::Password {
        username: Username::SameAsAddress,
        sasl: vec![SaslMech::Plain],
    };
    let cases: Vec<(AccountPlan, Result<Endpoint, NoSieve>)> = vec![
        (
            plan("mail.example.test", password.clone()),
            Ok(Endpoint {
                host: "mail.example.test".to_owned(),
                port: 4190,
                tls: Tls::StartTlsRequired,
            }),
        ),
        (
            plan(
                "imap.gmail.com",
                AuthPlan::OAuth {
                    issuer: OAuthIssuer::Google,
                    scopes: vec![],
                },
            ),
            Err(NoSieve::Provider(OAuthIssuer::Google)),
        ),
        // Signed in with a password, and still a provider with no ManageSieve.
        (
            plan("outlook.office365.com", password.clone()),
            Err(NoSieve::Provider(OAuthIssuer::Microsoft)),
        ),
        // Ends in the same letters, is not the same domain.
        (
            plan("mail.notgmail.com", password.clone()),
            Ok(Endpoint {
                host: "mail.notgmail.com".to_owned(),
                port: 4190,
                tls: Tls::StartTlsRequired,
            }),
        ),
        (
            AccountPlan {
                incoming: Incoming::Local,
                ..plan("x", password)
            },
            Err(NoSieve::Local),
        ),
    ];
    for (plan, want) in cases {
        assert_eq!(endpoint(&plan), want, "{:?}", plan.incoming);
    }
}
