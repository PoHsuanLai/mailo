//! `account add <address> --jmap` with no URL: the domain's `/.well-known/jmap` (RFC 8620 §2.2),
//! found without a credential and confirmed before one is sent — the same rule for asking as
//! every other discovery here ([`super::next`]).

use super::{Next, Terminal, accepted, next};
use crate::cli::{Command, Setup};

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

/// Follow `url` over the network, without a credential. See [`before_add_jmap`].
pub fn find(url: &str) -> Result<String, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;
    runtime.block_on(async {
        let http = mail_runtime::discover::client_builder()
            .build()
            .map_err(|e| format!("cannot build an HTTP client: {e}"))?;
        mail_runtime::jmap::find_session(&http, url)
            .await
            .map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Consent;

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
            receive: crate::cli::Receive::Imap,
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
            receive: crate::cli::Receive::Imap,
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
