//! `account add` for a domain the preset table does not know, and `account discover`.
//!
//! The lookup is `mail_runtime::discover`'s and what it means is `mail_proto::discover`'s. This
//! is the part between the answer and the account: showing what was found, where it came from,
//! and getting a yes before any of it is used — because a configuration from a DNS record or a
//! database is a guess, and a wrong guess sends a password to a host the user never named.
//!
//! The rule for asking, in [`next`]: `--yes` proceeds; a terminal is asked, and anything but a
//! yes is a no; anywhere else — a script, a pipe — the finding is printed and the command fails,
//! since there is nobody to ask and silence is not consent.

use crate::cli::{Command, Consent, Setup};

mod jmap;
pub use jmap::{before_add_jmap, find as find_jmap};
use mail_domain::{AuthPlan, Incoming, OAuthIssuer, Outgoing, Tls, Username};
use mail_proto::discover::{Found, Unusable};
use mail_runtime::discover::NotFound;
use std::fmt::Write as _;

/// Whether anyone can be asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminal {
    /// Standard input is a terminal: a person can answer.
    Interactive,
    /// A pipe, a file, a script.
    Not,
}

impl Terminal {
    /// This process's standard input.
    pub fn of_stdin() -> Terminal {
        use std::io::IsTerminal as _;
        if std::io::stdin().is_terminal() {
            Terminal::Interactive
        } else {
            Terminal::Not
        }
    }
}

/// What to do with a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Use it.
    Proceed,
    /// Show it and ask.
    Ask,
    /// Show it and stop: nobody said yes and nobody can be asked.
    Refuse,
}

/// The rule for asking.
pub fn next(consent: Consent, terminal: Terminal) -> Next {
    match (consent, terminal) {
        (Consent::Given, _) => Next::Proceed,
        (Consent::Ask, Terminal::Interactive) => Next::Ask,
        (Consent::Ask, Terminal::Not) => Next::Refuse,
    }
}

/// Whether a typed answer is a yes. Only `y` or `yes`, in any case: the default is no.
pub fn accepted(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// The address to look up, when `command` is an `account add` that needs discovery: no servers
/// named, not `--microsoft`, and a domain the preset table does not cover.
pub fn needed(command: &Command) -> Option<String> {
    let Command::AccountAdd {
        address,
        manual: None,
        microsoft: false,
        ..
    } = command
    else {
        return None;
    };
    // Lowercased the way `account::add` stores it, so what is shown is what is saved.
    let address = address.to_lowercase();
    mail_domain::presets::preset_for(&address, chrono::Utc::now())
        .is_none()
        .then_some(address)
}

/// What was found, for the user to judge.
pub fn describe(address: &str, origin: &str, found: &mail_domain::presets::Preset) -> String {
    let plan = &found.plan;
    let mut out = format!("for {address}, {origin}:\n");
    let incoming = match &plan.incoming {
        Incoming::Imap { host, port, tls } => format!("IMAP {host}:{port}, {}", tls_said(*tls)),
        Incoming::Pop3 {
            host, port, tls, ..
        } => format!(
            "POP3 {host}:{port}, {} (mail is left on the server)",
            tls_said(*tls)
        ),
        Incoming::Local => "none".to_owned(),
        Incoming::Graph => "Microsoft Graph".to_owned(),
        Incoming::Jmap { session, auth } => format!(
            "JMAP at {session}, {}",
            match auth {
                mail_domain::HttpAuth::Basic => "signing in with a password",
                mail_domain::HttpAuth::Bearer => "signing in with a token",
            }
        ),
    };
    let outgoing = match &plan.outgoing {
        Outgoing::Smtp { host, port, tls } => format!("SMTP {host}:{port}, {}", tls_said(*tls)),
        Outgoing::Graph => "Microsoft Graph".to_owned(),
        Outgoing::Jmap => "JMAP submission, on the same server".to_owned(),
        Outgoing::Nowhere => "none".to_owned(),
    };
    let sign_in = match &plan.auth {
        AuthPlan::Password { username, .. } => format!(
            "a password, logging in as {:?}{}",
            username.resolve(address),
            match username {
                Username::SameAsAddress => " (the whole address)",
                Username::LocalPart => " (the part before the @)",
                Username::Literal(_) => "",
            }
        ),
        AuthPlan::OAuth { issuer, .. } => format!(
            "OAuth, signing in with {} in a browser; no password is stored",
            match issuer {
                OAuthIssuer::Google => "Google",
                OAuthIssuer::Microsoft => "Microsoft",
            }
        ),
    };
    let _ = writeln!(out, "  incoming  {incoming}");
    let _ = writeln!(out, "  outgoing  {outgoing}");
    let _ = writeln!(out, "  sign-in   {sign_in}");
    out
}

fn tls_said(tls: Tls) -> &'static str {
    match tls {
        Tls::Implicit => "TLS from the first byte",
        Tls::StartTlsRequired => "STARTTLS, required",
        Tls::Plaintext => "no TLS",
    }
}

/// What to say when nothing usable was found, with the way to configure the account by hand.
pub fn not_found(address: &str, why: &NotFound) -> String {
    let mut out = format!("could not find servers for {address}: {why}\n");
    let starttls = why
        .unusable()
        .any(|u| matches!(u, Unusable::NoImplicitTls { .. }));
    if why
        .unusable()
        .any(|u| matches!(u, Unusable::PersonalMicrosoft))
    {
        return out;
    }
    if starttls {
        out.push_str(
            "\nThis domain publishes servers that use STARTTLS, which this client does not use: \
             the upgrade can be stripped by anyone on the path, and the password then crosses \
             in the clear. Ask the provider whether it also offers IMAP on port 993 (or POP3 on \
             995) and submission on port 465; if it does, name them:\n",
        );
    } else {
        out.push_str("\nName the servers yourself:\n");
    }
    let _ = write!(
        out,
        "\n  mailo account add {address} --imap HOST[:993] --smtp HOST[:465] [--login NAME]\n\
         \nor --pop3 HOST[:995] in place of --imap for a POP3-only server."
    );
    out
}

/// Look the address up, over the network.
pub fn lookup(address: &str, now: chrono::DateTime<chrono::Utc>) -> Result<Found, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;
    runtime.block_on(async {
        let http = mail_runtime::discover::client_builder()
            .build()
            .map_err(|e| format!("cannot build an HTTP client: {e}"))?;
        let dns = mail_runtime::discover::SystemDns::new().map_err(|e| e.to_string())?;
        mail_runtime::discover::discover(
            address,
            &http,
            &dns,
            &mail_runtime::discover::Sources::default(),
            now,
        )
        .await
        .map_err(|why| not_found(address, &why))
    })
}

/// `account add` with discovery in front of it.
///
/// `lookup` finds the servers and `ask` puts a question to the person and returns their answer
/// (`None` when there is none to read); both are parameters so the rule can be tested without a
/// network or a terminal. `say` prints as it goes, because the finding must be on screen before
/// the question is. Returns the command to run: the same `account add`, with the servers
/// filled in.
pub fn before_add(
    command: Command,
    lookup: impl FnOnce(&str) -> Result<Found, String>,
    terminal: Terminal,
    mut say: impl FnMut(&str),
    ask: impl FnOnce(&str) -> Option<String>,
) -> Result<Command, String> {
    let Some(address) = needed(&command) else {
        return Ok(command);
    };
    let Command::AccountAdd {
        microsoft,
        graph,
        receive,
        consent,
        ..
    } = command
    else {
        return Ok(command);
    };
    say(&format!("looking up the servers for {address}…\n"));
    let found = lookup(&address)?;
    let shown = describe(&address, &found.source.to_string(), &found.preset);
    match next(consent, terminal) {
        Next::Proceed => say(&shown),
        Next::Ask => {
            say(&shown);
            let answer = ask("Use these servers? Nothing has been sent to them yet. [y/N] ");
            if !answer.as_deref().is_some_and(accepted) {
                return Err("not added; nothing was sent anywhere".to_owned());
            }
        }
        Next::Refuse => {
            return Err(format!(
                "{shown}\nnot added: there is no terminal to confirm these servers on. Check them, \
                 then re-run with --yes to accept them:\n\n  mailo account add {address} --yes"
            ));
        }
    }
    Ok(Command::AccountAdd {
        address,
        manual: Some(Setup::Discovered(Box::new(found.preset))),
        microsoft,
        graph,
        receive,
        consent,
    })
}

/// `account discover`: what would be used, and from where. Adds nothing.
pub fn show(
    address: &str,
    lookup: impl FnOnce(&str) -> Result<Found, String>,
) -> Result<String, String> {
    let address = address.to_lowercase();
    if let Some(preset) = mail_domain::presets::preset_for(&address, chrono::Utc::now()) {
        return Ok(describe(&address, "from the built-in table", &preset));
    }
    let found = lookup(&address)?;
    Ok(describe(&address, &found.source.to_string(), &found.preset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_proto::discover::{Offer, Side, Source};
    use mail_runtime::discover::{Miss, Tried};

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-09-24T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn add(address: &str, consent: Consent) -> Command {
        Command::AccountAdd {
            address: address.to_owned(),
            manual: None,
            microsoft: false,
            graph: false,
            receive: crate::cli::Receive::Imap,
            consent,
        }
    }

    fn found(address: &str) -> Found {
        Found {
            source: Source::Ispdb,
            preset: mail_domain::presets::manual(
                address,
                &mail_domain::presets::Manual {
                    imap_host: "imap.example.test".to_owned(),
                    imap_port: 993,
                    smtp_host: "smtp.example.test".to_owned(),
                    smtp_port: 465,
                    login: None,
                },
                now(),
            ),
        }
    }

    /// Runs `before_add` with a lookup that answers `found` and a person who answers `answer`.
    fn run(
        command: Command,
        terminal: Terminal,
        answer: Option<&str>,
    ) -> (Result<Command, String>, String, bool) {
        let mut said = String::new();
        let mut asked = false;
        let result = before_add(
            command,
            |address| Ok(found(address)),
            terminal,
            |text| said.push_str(text),
            |_| {
                asked = true;
                answer.map(str::to_owned)
            },
        );
        (result, said, asked)
    }

    #[test]
    fn yes_and_discover_parse() {
        let args = |s: &str| s.split(' ').map(str::to_owned).collect::<Vec<_>>();
        assert!(matches!(
            crate::cli::parse(&args("account add me@example.test --yes")).unwrap(),
            Command::AccountAdd {
                consent: Consent::Given,
                manual: None,
                ..
            }
        ));
        assert!(matches!(
            crate::cli::parse(&args("account add me@example.test")).unwrap(),
            Command::AccountAdd {
                consent: Consent::Ask,
                ..
            }
        ));
        assert!(matches!(
            crate::cli::parse(&args(
                "account add me@example.test --imap i.example.test --smtp s.example.test --yes"
            ))
            .unwrap(),
            Command::AccountAdd {
                manual: Some(Setup::Imap(_)),
                ..
            }
        ));
        assert_eq!(
            crate::cli::parse(&args("account discover me@example.test")).unwrap(),
            Command::AccountDiscover {
                address: "me@example.test".to_owned()
            }
        );
        assert!(crate::cli::parse(&args("account discover")).is_err());
    }

    #[test]
    fn the_rule_for_asking() {
        let table = [
            (Consent::Given, Terminal::Interactive, Next::Proceed),
            (Consent::Given, Terminal::Not, Next::Proceed),
            (Consent::Ask, Terminal::Interactive, Next::Ask),
            (Consent::Ask, Terminal::Not, Next::Refuse),
        ];
        for (consent, terminal, want) in table {
            assert_eq!(next(consent, terminal), want, "{consent:?} {terminal:?}");
        }
    }

    #[test]
    fn only_a_yes_is_a_yes() {
        for yes in ["y", "Y", "yes", " YES\n"] {
            assert!(accepted(yes), "{yes:?}");
        }
        for no in ["", "\n", "n", "no", "yeah", "sure", "y es"] {
            assert!(!accepted(no), "{no:?}");
        }
    }

    #[test]
    fn without_a_terminal_or_yes_nothing_is_added_and_the_finding_is_shown() {
        let (result, _, asked) = run(add("me@example.test", Consent::Ask), Terminal::Not, None);
        let err = result.unwrap_err();
        assert!(!asked);
        assert!(err.contains("imap.example.test:993"), "{err}");
        assert!(err.contains("from the Thunderbird ISPDB"), "{err}");
        assert!(err.contains("--yes"), "{err}");
    }

    #[test]
    fn yes_accepts_what_was_found_and_shows_it() {
        let (result, said, asked) =
            run(add("Me@Example.test", Consent::Given), Terminal::Not, None);
        assert!(!asked);
        assert!(said.contains("smtp.example.test:465"), "{said}");
        let Command::AccountAdd {
            address,
            manual: Some(Setup::Discovered(preset)),
            ..
        } = result.unwrap()
        else {
            panic!("the servers were not filled in")
        };
        assert_eq!(
            address, "me@example.test",
            "lowercased as it will be stored"
        );
        assert_eq!(preset.plan.address, "me@example.test");
    }

    #[test]
    fn a_terminal_is_asked_and_the_default_is_no() {
        let (result, said, asked) = run(
            add("me@example.test", Consent::Ask),
            Terminal::Interactive,
            Some(""),
        );
        assert!(asked);
        assert!(
            said.contains("imap.example.test"),
            "shown before asking: {said}"
        );
        assert!(result.is_err());
        let (result, _, _) = run(
            add("me@example.test", Consent::Ask),
            Terminal::Interactive,
            None,
        );
        assert!(result.is_err(), "no answer at all is a no");
        let (result, _, _) = run(
            add("me@example.test", Consent::Ask),
            Terminal::Interactive,
            Some("y\n"),
        );
        assert!(matches!(
            result.unwrap(),
            Command::AccountAdd {
                manual: Some(Setup::Discovered(_)),
                ..
            }
        ));
    }

    #[test]
    fn named_servers_and_known_domains_are_not_looked_up() {
        let looked = std::cell::Cell::new(false);
        for command in [
            add("me@gmail.com", Consent::Ask),
            Command::AccountAdd {
                address: "me@firm.example".to_owned(),
                manual: None,
                microsoft: true,
                graph: false,
                receive: crate::cli::Receive::Imap,
                consent: Consent::Ask,
            },
        ] {
            let same = command.clone();
            let out = before_add(
                command,
                |_| {
                    looked.set(true);
                    Err("looked".to_owned())
                },
                Terminal::Not,
                |_| {},
                |_| None,
            )
            .unwrap();
            assert_eq!(out, same);
        }
        assert!(!looked.get());
    }

    #[test]
    fn a_starttls_only_domain_is_told_how_to_configure_it_by_hand() {
        let why = NotFound {
            tried: vec![Tried {
                what: "https://autoconfig.example.test/mail/config-v1.1.xml".to_owned(),
                miss: Miss::Unusable(Unusable::NoImplicitTls {
                    side: Side::Incoming,
                    offered: vec![Offer {
                        protocol: "IMAP".to_owned(),
                        host: "imap.example.test".to_owned(),
                        port: 143,
                        socket: "STARTTLS".to_owned(),
                    }],
                }),
            }],
            timed_out: false,
        };
        let said = not_found("me@example.test", &why);
        assert!(said.contains("STARTTLS"), "{said}");
        assert!(said.contains("imap.example.test:143"), "{said}");
        assert!(said.contains("--imap HOST[:993]"), "{said}");
    }

    #[test]
    fn a_google_hosted_domain_is_described_as_an_oauth_sign_in() {
        let preset =
            mail_domain::presets::preset_for_mail_exchanger("google.com", "me@firm.example", now())
                .unwrap();
        let text = describe(
            "me@firm.example",
            &Source::Mx {
                exchanger: "aspmx.l.google.com".to_owned(),
                provider: "google.com".to_owned(),
            }
            .to_string(),
            &preset,
        );
        assert!(text.contains("via MX → google.com"), "{text}");
        assert!(text.contains("imap.gmail.com:993"), "{text}");
        assert!(text.contains("OAuth"), "{text}");
    }
}
