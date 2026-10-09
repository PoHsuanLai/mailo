//! Discovery at the terminal: the question put to a person before an account is added at a server
//! that was only found, and `account discover`.
//!
//! The lookup is [`mail_core::discover`]'s. This is the part between the answer and the command:
//! showing what was found, where it came from, and getting a yes before any of it is used,
//! because a configuration from a DNS record or a database is a guess, and a wrong guess sends a
//! password to a host the user never named.
//!
//! The rule for asking, in [`next`]: `--yes` proceeds; a terminal is asked, and anything but a
//! yes is a no; anywhere else — a script, a pipe — the finding is printed and the command fails,
//! since there is nobody to ask and silence is not consent.

use super::{Command, Consent};
use mail_core::account::Setup;
use mail_core::discover::Found;
use mail_core::discover::describe;

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
    mail_core::discover::known(&address, chrono::Utc::now())
        .is_none()
        .then_some(address)
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
    if let Some(preset) = mail_core::discover::known(&address, chrono::Utc::now()) {
        return Ok(describe(&address, "from the built-in table", &preset));
    }
    let found = lookup(&address)?;
    Ok(describe(&address, &found.source.to_string(), &found.preset))
}

/// Fill in the session URL of a `--jmap` account that named none.
///
/// `find` follows `https://<domain>/.well-known/jmap` to wherever it leads, sending nothing but
/// the request; the answer is shown, and a password goes there only after a yes (or `--yes`).
/// Any other command passes through untouched.
pub fn before_add_jmap(
    command: Command,
    find: impl FnOnce(&str) -> Result<String, String>,
    terminal: Terminal,
    mut say: impl FnMut(&str),
    ask: impl FnOnce(&str) -> Option<String>,
) -> Result<Command, String> {
    let Command::AccountAdd {
        address,
        manual:
            Some(Setup::Jmap {
                session: None,
                login,
                auth,
            }),
        microsoft,
        graph,
        receive,
        consent,
    } = command
    else {
        return Ok(command);
    };
    let address = address.to_lowercase();
    let url = mail_domain::presets::well_known(&address)
        .ok_or_else(|| format!("{address:?} has no domain to look for a JMAP server on"))?;
    say(&format!("looking for a JMAP session at {url}…\n"));
    let found = find(&url).map_err(|why| {
        format!(
            "{why}\n\nName the session URL yourself:\n\n  \
             mailo account add {address} --jmap https://jmap.example.com/.well-known/jmap"
        )
    })?;
    let shown = format!(
        "for {address}, from {url}:\n  JMAP session at {found}\n  sign-in   {}\n",
        match auth {
            mail_domain::HttpAuth::Bearer => "a bearer token",
            // The command line decides on a token only when it adds the account.
            mail_domain::HttpAuth::Basic => {
                "a password (HTTP Basic), or the bearer token in MAILO_JMAP_TOKEN if set"
            }
        }
    );
    match next(consent, terminal) {
        Next::Proceed => say(&shown),
        Next::Ask => {
            say(&shown);
            let answer = ask("Use this server? Nothing has been sent to it yet. [y/N] ");
            if !answer.as_deref().is_some_and(accepted) {
                return Err("not added; nothing was sent anywhere".to_owned());
            }
        }
        Next::Refuse => {
            return Err(format!(
                "{shown}\nnot added: there is no terminal to confirm this server on. Check it, \
                 then re-run with --yes to accept it:\n\n  \
                 mailo account add {address} --jmap --yes"
            ));
        }
    }
    Ok(Command::AccountAdd {
        address,
        manual: Some(Setup::Jmap {
            session: Some(found),
            login,
            auth,
        }),
        microsoft,
        graph,
        receive,
        consent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::discover::Source;

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
            receive: mail_core::account::Receive::Imap,
            consent,
        }
    }

    fn found(address: &str) -> Found {
        Found {
            source: Source::Autoconfig,
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
        assert!(err.contains("from autoconfig"), "{err}");
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
                receive: mail_core::account::Receive::Imap,
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
}

#[cfg(test)]
mod jmap_tests {
    use super::*;

    fn add(consent: Consent) -> Command {
        Command::AccountAdd {
            address: "Me@Example.com".to_owned(),
            manual: Some(Setup::Jmap {
                session: None,
                login: None,
                auth: mail_domain::HttpAuth::Basic,
            }),
            microsoft: false,
            graph: false,
            receive: mail_core::account::Receive::Imap,
            consent,
        }
    }

    fn session_of(command: &Command) -> Option<String> {
        match command {
            Command::AccountAdd {
                manual: Some(Setup::Jmap { session, .. }),
                ..
            } => session.clone(),
            _ => None,
        }
    }

    #[test]
    fn the_well_known_url_is_followed_and_what_it_found_is_used_after_a_yes() {
        let mut asked_about = String::new();
        let command = before_add_jmap(
            add(Consent::Ask),
            |url| {
                assert_eq!(url, "https://example.com/.well-known/jmap");
                Ok("https://jmap.example.net/session".to_owned())
            },
            Terminal::Interactive,
            |text| asked_about.push_str(text),
            |_| Some("yes\n".to_owned()),
        )
        .unwrap();
        assert!(
            asked_about.contains("https://jmap.example.net/session"),
            "{asked_about}"
        );
        assert_eq!(
            session_of(&command).as_deref(),
            Some("https://jmap.example.net/session")
        );
    }

    #[test]
    fn anything_but_a_yes_adds_nothing_and_nobody_to_ask_is_a_no() {
        let found = |_: &str| Ok("https://jmap.example.net/session".to_owned());
        let no = before_add_jmap(
            add(Consent::Ask),
            found,
            Terminal::Interactive,
            |_| {},
            |_| Some("\n".to_owned()),
        );
        assert!(no.is_err());
        let script = before_add_jmap(add(Consent::Ask), found, Terminal::Not, |_| {}, |_| None);
        assert!(script.unwrap_err().contains("--yes"));
        let given = before_add_jmap(add(Consent::Given), found, Terminal::Not, |_| {}, |_| None);
        assert!(session_of(&given.unwrap()).is_some());
    }

    #[test]
    fn a_named_url_is_not_looked_up() {
        let named = Command::AccountAdd {
            address: "me@example.com".to_owned(),
            manual: Some(Setup::Jmap {
                session: Some("https://jmap.example.com/s".to_owned()),
                login: None,
                auth: mail_domain::HttpAuth::Basic,
            }),
            microsoft: false,
            graph: false,
            receive: mail_core::account::Receive::Imap,
            consent: Consent::Ask,
        };
        let out = before_add_jmap(
            named.clone(),
            |_| panic!("looked up a URL the user named"),
            Terminal::Not,
            |_| {},
            |_| None,
        )
        .unwrap();
        assert_eq!(out, named);
    }
}
