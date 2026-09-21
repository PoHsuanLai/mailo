//! The CLI against a real store, which is `plan.md` phase 4's "a tiny CLI can list and open".
//!
//! Driven through `cli::run` rather than by spawning the binary, so the assertions are about
//! what the user sees rather than about process plumbing.

use chrono::{TimeZone, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

// Both modules are pulled in by path: cli.rs calls into account.rs, so the test crate needs
// the same shape the binary has.
#[path = "../src/account.rs"]
mod account;
#[path = "../src/cli.rs"]
mod cli;
#[path = "../src/compose.rs"]
mod compose;
#[path = "../src/sync.rs"]
mod sync;
#[allow(dead_code)]
#[path = "../src/view.rs"]
mod view;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn seeded() -> (SqliteStore, tempfile::TempDir, ThreadId) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();

    let mut first = None;
    for (i, subject) in ["lunch on friday", "invoice 2024", "server outage"]
        .iter()
        .enumerate()
    {
        let thread = ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + i as u128));
        if first.is_none() {
            first = Some(thread);
        }
        let raw = store
            .blobs()
            .put(&store.connection(), format!("raw {i}").as_bytes())
            .unwrap();
        let message = Message {
            id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + i as u128)),
            thread,
            account: ACCOUNT,
            key: MessageKey::Rfc(format!("m{i}@example.test")),
            date: Utc
                .timestamp_opt(1_700_000_000 + i as i64 * 3600, 0)
                .unwrap(),
            from: Address {
                name: Some(format!("Sender {i}")),
                email: format!("s{i}@example.test"),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: (*subject).to_owned(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some(format!("m{i}@example.test")),
            read: if i == 0 {
                ReadState::Unread
            } else {
                ReadState::Read
            },
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            // The second message has headers only, which is the normal mid-sync state and the
            // one a UI most easily gets wrong by showing an empty body as if it were empty.
            body: if i == 1 {
                Body::Absent
            } else {
                Body::Present {
                    text: Some(format!("the body of {subject}")),
                    raw,
                }
            },
            attachments: vec![],
        };
        store
            .apply(
                ACCOUNT,
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::MessageUpsert(Box::new(message))],
                },
            )
            .unwrap();
    }
    (store, dir, first.unwrap())
}

fn now() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000, 0).unwrap()
}

#[test]
fn list_shows_threads_newest_first_and_marks_unread() {
    let (store, _dir, _) = seeded();
    let out = cli::run(
        &store,
        &cli::Command::List {
            mailbox: MailboxRole::Inbox,
            limit: 20,
        },
        now(),
    )
    .unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("server outage"), "newest first: {out}");
    assert!(
        lines[2].starts_with('*'),
        "the unread one should be marked: {out}"
    );
}

#[test]
fn an_empty_mailbox_says_so_instead_of_printing_nothing() {
    let (store, _dir, _) = seeded();
    let out = cli::run(
        &store,
        &cli::Command::List {
            mailbox: MailboxRole::Trash,
            limit: 20,
        },
        now(),
    )
    .unwrap();
    assert!(out.contains("no threads in trash"), "{out}");
}

#[test]
fn show_prints_the_body_and_says_when_it_is_not_fetched() {
    let (store, _dir, first) = seeded();
    let out = cli::run(&store, &cli::Command::Show { thread: first }, now()).unwrap();
    assert!(out.contains("lunch on friday"));
    assert!(out.contains("the body of lunch on friday"), "{out}");

    // The headers-only thread must say so rather than render as an empty message, which is how
    // a mid-sync state gets mistaken for a blank email.
    let headers_only = ThreadId::from_uuid(uuid::Uuid::from_u128(0x7001));
    let out = cli::run(
        &store,
        &cli::Command::Show {
            thread: headers_only,
        },
        now(),
    )
    .unwrap();
    assert!(out.contains("not fetched yet"), "{out}");
}

#[test]
fn search_goes_through_full_text_and_reports_a_miss() {
    let (store, _dir, _) = seeded();
    let hit = cli::run(
        &store,
        &cli::Command::Search {
            needle: "outage".to_owned(),
            limit: 20,
        },
        now(),
    )
    .unwrap();
    assert!(hit.contains("server outage"), "{hit}");

    let miss = cli::run(
        &store,
        &cli::Command::Search {
            needle: "nothingmatchesthis".to_owned(),
            limit: 20,
        },
        now(),
    )
    .unwrap();
    assert!(miss.contains("nothing matches"), "{miss}");
}

#[test]
fn status_counts_unread_separately_from_total() {
    let (store, _dir, _) = seeded();
    let out = cli::run(&store, &cli::Command::Status, now()).unwrap();
    assert!(out.contains("inbox"), "{out}");
    assert!(out.contains("3 total"), "{out}");
    assert!(out.contains("1 unread"), "{out}");
    // Mailboxes with nothing in them are omitted rather than printed as zeroes.
    assert!(!out.contains("spam"), "{out}");
}

#[test]
fn a_missing_thread_is_an_error_not_a_panic() {
    let (store, _dir, _) = seeded();
    let err = cli::run(
        &store,
        &cli::Command::Show {
            thread: ThreadId::generate(),
        },
        now(),
    )
    .expect_err("no such thread");
    assert!(!err.is_empty());
}

#[test]
fn the_parser_understands_the_composing_verbs() {
    let args = |s: &str| -> Vec<String> { s.split(' ').map(str::to_owned).collect() };
    let id = MessageId::generate().to_string();

    assert!(matches!(
        cli::parse(&args(&format!("reply {id}"))).unwrap(),
        cli::Command::Reply {
            scope: ReplyScope::Sender,
            ..
        }
    ));
    // `--all` rather than a second verb: one operation, a wider audience.
    assert!(matches!(
        cli::parse(&args(&format!("reply {id} --all"))).unwrap(),
        cli::Command::Reply {
            scope: ReplyScope::All,
            ..
        }
    ));
    assert!(matches!(
        cli::parse(&args("drafts")).unwrap(),
        cli::Command::Drafts
    ));
    // Mistyped input is explained, never panicked on.
    assert!(cli::parse(&args("send not-a-uuid")).is_err());
    assert!(cli::parse(&args("reply")).is_err());
    assert!(cli::parse(&args(&format!("reply {id} --everyone"))).is_err());
}

#[test]
fn usage_mentions_every_verb_the_parser_accepts() {
    // A command that works but is undocumented is a command nobody uses.
    let text = cli::usage();
    for verb in [
        "list", "show", "search", "reply", "send", "drafts", "status", "sync",
    ] {
        assert!(
            text.contains(verb),
            "usage does not mention {verb}:\n{text}"
        );
    }
}

/// Manual server configuration: the only way to reach a host the preset table never heard of.
mod manual_setup {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split(' ').map(str::to_owned).collect()
    }

    #[test]
    fn naming_both_servers_configures_an_account_the_table_does_not_know() {
        let parsed = cli::parse(&args(
            "account add me@example.test --imap imap.example.test --smtp smtp.example.test",
        ))
        .expect("manual setup parses");
        match parsed {
            cli::Command::AccountAdd {
                address,
                manual: Some(manual),
                ..
            } => {
                assert_eq!(address, "me@example.test");
                assert_eq!(manual.imap_host, "imap.example.test");
                assert_eq!(manual.smtp_host, "smtp.example.test");
                // Implicit-TLS ports, because those are the ones that cannot be downgraded.
                assert_eq!(manual.imap_port, 993);
                assert_eq!(manual.smtp_port, 465);
                assert_eq!(manual.login, None);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_port_can_be_given_with_the_host() {
        let parsed = cli::parse(&args(
            "account add me@example.test --imap imap.example.test:1993 --smtp smtp.example.test:1465",
        ))
        .unwrap();
        match parsed {
            cli::Command::AccountAdd {
                manual: Some(manual),
                ..
            } => {
                assert_eq!(manual.imap_port, 1993);
                assert_eq!(manual.smtp_port, 1465);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_login_name_that_is_not_the_address_is_carried_through() {
        let parsed = cli::parse(&args(
            "account add me@example.test --imap i.example.test --smtp s.example.test --login mylogin",
        ))
        .unwrap();
        match parsed {
            cli::Command::AccountAdd {
                manual: Some(manual),
                ..
            } => assert_eq!(manual.login.as_deref(), Some("mylogin")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn half_a_configuration_is_refused_rather_than_guessed() {
        // Defaulting the missing half to a hostname derived from the domain is how mail goes to
        // a server the user never named.
        let err = cli::parse(&args(
            "account add me@example.test --imap imap.example.test",
        ))
        .expect_err("one server is not a configuration");
        assert!(err.contains("--smtp"), "{err}");
    }

    #[test]
    fn a_domain_with_no_preset_says_how_to_configure_it() {
        let err = cli::parse(&args("account add me@nowhere.example"))
            .ok()
            .map(|_| String::new())
            .unwrap_or_default();
        // Parsing succeeds; the explanation comes from `account::add`, which needs a store.
        assert!(err.is_empty());

        let (store, _dir, _thread) = seeded();
        let command = cli::parse(&args("account add me@nowhere.example")).unwrap();
        let err = cli::run(&store, &command, now()).expect_err("no preset");
        assert!(
            err.contains("--imap"),
            "the error must show the way out: {err}"
        );
        assert!(err.contains("--smtp"), "{err}");
    }

    #[test]
    fn a_bad_port_is_explained_not_ignored() {
        let err = cli::parse(&args(
            "account add me@example.test --imap imap.example.test:notaport --smtp s.example.test",
        ))
        .expect_err("ports are numbers");
        assert!(err.contains("port"), "{err}");
    }

    #[test]
    fn an_unknown_option_does_not_pass_silently() {
        let err = cli::parse(&args(
            "account add me@example.test --imap i.example.test --smtp s.example.test --tls no",
        ))
        .expect_err("unknown flag");
        assert!(err.contains("--tls"), "{err}");
    }

    #[test]
    fn a_manual_account_is_stored_as_imap_with_implicit_tls() {
        let (store, _dir, _thread) = seeded();
        let command = cli::parse(&args(
            "account add someone@nowhere.example --imap imap.nowhere.example --smtp smtp.nowhere.example",
        ))
        .unwrap();
        // No MAILO_PASSWORD in the environment, so this reports what is still needed rather
        // than failing — what matters here is the plan it wrote.
        let _ = cli::run(&store, &command, now());

        let plan: String = store
            .connection()
            .query_row(
                "SELECT plan FROM accounts WHERE address = 'someone@nowhere.example'",
                [],
                |r| r.get(0),
            )
            .expect("the account was stored");
        let plan: AccountPlan = serde_json::from_str(&plan).unwrap();
        match plan.incoming {
            Incoming::Imap { host, port, tls } => {
                assert_eq!(host, "imap.nowhere.example");
                assert_eq!(port, 993);
                // Never StartTls and never Plaintext: an opportunistic upgrade is strippable.
                assert_eq!(tls, Tls::Implicit);
            }
            other => panic!("expected IMAP, got {other:?}"),
        }
        assert!(
            matches!(plan.auth, AuthPlan::Password { .. }),
            "a manually configured account is a password account: {:?}",
            plan.auth
        );
    }
}

/// Microsoft 365, which the address alone usually cannot reveal.
mod microsoft {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split(' ').map(str::to_owned).collect()
    }

    #[test]
    fn a_tenant_fallback_domain_is_recognised_on_its_own() {
        // `you@contoso.onmicrosoft.com` is the one Microsoft 365 address that names itself.
        let preset = mail_domain::presets::preset_for("me@contoso.onmicrosoft.com", now())
            .expect("a tenant domain is known");
        assert!(matches!(
            preset.plan.auth,
            AuthPlan::OAuth {
                issuer: OAuthIssuer::Microsoft,
                ..
            }
        ));
    }

    #[test]
    fn a_lookalike_tenant_domain_is_not() {
        assert!(mail_domain::presets::preset_for("me@notonmicrosoft.com", now()).is_none());
    }

    #[test]
    fn a_custom_tenant_domain_needs_to_be_told() {
        // The common case, and the one nothing can infer: a work mailbox on the company's own
        // domain. Guessing would mean autodiscover, and guessing wrong points the client at a
        // host the user never named.
        assert!(mail_domain::presets::preset_for("me@yourcompany.example", now()).is_none());

        let parsed = cli::parse(&args("account add me@yourcompany.example --microsoft")).unwrap();
        match parsed {
            cli::Command::AccountAdd {
                microsoft, manual, ..
            } => {
                assert!(microsoft);
                assert!(manual.is_none(), "--microsoft supplies the servers itself");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn microsoft_and_manual_servers_together_are_refused() {
        // Both name the servers. Silently letting one win is how an account ends up pointed
        // somewhere the user did not intend.
        let err = cli::parse(&args(
            "account add me@x.example --microsoft --imap i.example --smtp s.example",
        ))
        .expect_err("two sources of truth");
        assert!(err.contains("--microsoft"), "{err}");
    }

    #[test]
    fn the_preset_asks_for_both_protocols_and_a_refresh_token() {
        // One scope per protocol: Microsoft grants IMAP and SMTP separately, and asking only for
        // the first produces an account that syncs and cannot send. Without offline_access the
        // account stops working an hour after it is added.
        let preset = mail_domain::presets::microsoft_preset("me@yourcompany.example", now());
        let AuthPlan::OAuth { scopes, .. } = &preset.plan.auth else {
            panic!("microsoft is an OAuth account");
        };
        assert!(
            scopes.iter().any(|s| s.contains("IMAP.AccessAsUser.All")),
            "{scopes:?}"
        );
        assert!(scopes.iter().any(|s| s.contains("SMTP.Send")), "{scopes:?}");
        assert!(scopes.iter().any(|s| s == "offline_access"), "{scopes:?}");
    }

    #[test]
    fn submission_uses_starttls_on_587_not_implicit_tls_on_465() {
        // The one place Microsoft's shape differs from Gmail's. Exchange Online does not offer
        // implicit TLS on 465 for SMTP AUTH, and the upgrade is required rather than
        // opportunistic — a failure to upgrade aborts instead of sending a bearer token in
        // cleartext.
        let preset = mail_domain::presets::microsoft_preset("me@yourcompany.example", now());
        match preset.plan.outgoing {
            Outgoing::Smtp { host, port, tls } => {
                assert_eq!(host, "smtp.office365.com");
                assert_eq!(port, 587);
                assert_eq!(tls, Tls::StartTlsRequired);
            }
        }
    }

    #[test]
    fn the_folder_roles_are_left_for_the_server_to_say() {
        // Exchange Online localises folder names per mailbox. Guessing "Sent Items" files mail
        // into a folder that may not exist; `refresh_caps` fills these in from LIST (SPECIAL-USE).
        let preset = mail_domain::presets::microsoft_preset("me@yourcompany.example", now());
        assert!(
            preset.expected_caps.folders.0.is_empty(),
            "a folder name was guessed: {:?}",
            preset.expected_caps.folders
        );
    }
}

#[test]
fn the_suggested_rerun_reproduces_the_account_it_describes() {
    // Advice that does not work when followed is worse than none. The address alone does not
    // identify a Microsoft tenant on a custom domain — `--microsoft` is exactly the information
    // the preset table lacks — so a re-run without it finds no preset at all.
    let (store, _dir, _thread) = seeded();
    let command = cli::parse(
        &"account add me@yourcompany.example --microsoft"
            .split(' ')
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let out = cli::run(&store, &command, now()).expect("an OAuth account is added");

    let line = out
        .lines()
        .find(|l| l.contains("MAILO_OAUTH_CLIENT_ID"))
        .expect("the re-run is suggested");
    assert!(
        line.contains("--microsoft"),
        "following this would fail to find a preset: {line}"
    );

    // And what it suggests actually parses back to the same account.
    let suggested: Vec<String> = line
        .split_whitespace()
        .skip_while(|w| !w.starts_with("mailo"))
        .skip(1)
        .map(str::to_owned)
        .collect();
    match cli::parse(&suggested).expect("the suggestion parses") {
        cli::Command::AccountAdd {
            address, microsoft, ..
        } => {
            assert_eq!(address, "me@yourcompany.example");
            assert!(microsoft);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn discard_is_parsed_and_needs_an_id() {
    let parse =
        |args: &[&str]| cli::parse(&args.iter().map(|s| (*s).to_string()).collect::<Vec<_>>());
    let id = uuid::Uuid::new_v4();
    assert_eq!(
        parse(&["discard", &id.to_string()]).unwrap(),
        cli::Command::Discard {
            draft: DraftId::from_uuid(id)
        }
    );
    assert!(parse(&["discard"]).is_err(), "no id is not a discard");
    assert!(parse(&["discard", "not-a-uuid"]).is_err());
    // And it is in the usage text, because a command nobody can find is not a command.
    assert!(
        parse(&[]).unwrap_err().contains("discard <draft-id>"),
        "{}",
        parse(&[]).unwrap_err()
    );
}
