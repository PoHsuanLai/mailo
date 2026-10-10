//! What looking an address up comes to, and what typing the servers by hand comes to: the part of
//! adding an account that the window and the command line decide alike.
//!
//! [`resolve`] runs the search and the JMAP search side by side and [`settle`] reads the two
//! answers; the window runs the same two searches over seams of its own and calls [`settle`] too.
//! What to say about the result, and what to ask, is each front-end's.

use super::{Failed, Found, Gap, find_jmap, search};
use crate::account::Setup;
use crate::password::Password;
use chrono::{DateTime, Utc};
use mail_domain::presets::{self, Manual, ManualPop3, Preset};
use mail_domain::{HttpAuth, Incoming, Outgoing, Tls};
use porter_core::sheet::{Hop, MailServers, Manual as Typed, Security};

/// What looking an address up came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Looked {
    /// Servers were found for the address. Boxed: it is far larger than the other answers.
    Found(Box<Found>),
    /// Only a JMAP session was found, at this URL.
    Jmap(String),
    /// Nothing usable is published: the servers have to be named.
    Ask(Failed),
    /// A personal Microsoft mailbox, which no longer takes a password.
    PersonalMicrosoft(Failed),
    /// Nothing answered, or the lookup could not be set up.
    Unreachable(Failed),
}

/// The domain of what is typed, when it reads as an address: something before the last `@`, and a
/// dotted name after it. Lowercased.
pub fn typed_domain(typed: &str) -> Option<String> {
    let (local, domain) = typed.trim().rsplit_once('@')?;
    let domain = domain.to_ascii_lowercase();
    (!local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.'))
    .then_some(domain)
}

/// The two searches' answers read together. When both answer, what the autoconfig named is the
/// offer, as it was the first of the two the old sheet showed.
pub fn settle(found: Result<Found, Failed>, jmap: Option<String>) -> Looked {
    match (found, jmap) {
        (Ok(found), _) => Looked::Found(Box::new(found)),
        (Err(_), Some(session)) => Looked::Jmap(session),
        (Err(failed @ Failed::NoServers { gap, .. }), None) => match gap {
            Gap::PersonalMicrosoft => Looked::PersonalMicrosoft(failed),
            Gap::Nothing | Gap::StartTlsOnly => Looked::Ask(failed),
        },
        (Err(failed @ (Failed::Unreachable { .. } | Failed::Broken(_))), None) => {
            Looked::Unreachable(failed)
        }
    }
}

/// Look `address` up over the network: the autoconfig search and the domain's `/.well-known/jmap`
/// side by side. Each is handed only what it needs, the address and the well-known URL. The
/// built-in table ([`super::known`]) is the caller's to ask first, since it needs no lookup.
pub async fn resolve(address: &str, now: DateTime<Utc>) -> Looked {
    let jmap = async {
        match presets::well_known(address) {
            Some(url) => find_jmap(&url).await.ok(),
            None => None,
        }
    };
    let (found, jmap) = tokio::join!(search(address, now), jmap);
    settle(found, jmap)
}

/// The servers a person typed: IMAP or POP3 with SMTP, each as secure as was said, or a JMAP
/// session. The secret the sign-in is made with is a password, or, for JMAP given an API token,
/// the token (this is the one place that decides which).
pub fn typed(
    address: &str,
    typed: Typed,
    password: Option<Password>,
    now: DateTime<Utc>,
) -> (Setup, Option<Password>) {
    match typed {
        Typed::Imap(servers) => {
            let manual = Manual {
                imap_host: servers.incoming.host.clone(),
                imap_port: servers.incoming.port,
                smtp_host: servers.outgoing.host.clone(),
                smtp_port: servers.outgoing.port,
                login: servers.login.clone(),
            };
            let preset = presets::manual(address, &manual, now);
            (
                Setup::Discovered(Box::new(secured(preset, &servers))),
                password,
            )
        }
        Typed::Pop3(servers) => {
            let manual = ManualPop3 {
                pop3_host: servers.incoming.host.clone(),
                pop3_port: servers.incoming.port,
                smtp_host: servers.outgoing.host.clone(),
                smtp_port: servers.outgoing.port,
                login: servers.login.clone(),
            };
            let preset = presets::manual_pop3(address, &manual, now);
            (
                Setup::Discovered(Box::new(secured(preset, &servers))),
                password,
            )
        }
        Typed::Jmap(server) => {
            let (auth, secret) = match server.token {
                // The token is what the account signs in with; the password typed on the first
                // form is not used.
                Some(token) => (
                    HttpAuth::Bearer,
                    Some(Password::new(token.expose().to_owned())),
                ),
                None => (HttpAuth::Basic, password),
            };
            let setup = Setup::Jmap {
                session: Some(server.session.as_str().to_owned()),
                login: server.login,
                auth,
            };
            (setup, secret)
        }
    }
}

/// `preset` with the security each of its servers was typed with (the presets are made for
/// implicit TLS).
fn secured(mut preset: Preset, servers: &MailServers) -> Preset {
    let tls = |hop: &Hop| match hop.security {
        Security::Tls => Tls::Implicit,
        Security::StartTls => Tls::StartTlsRequired,
        // Porter accepts it for a server on this computer only, where nothing can listen in.
        Security::Plain => Tls::Plaintext,
    };
    match &mut preset.plan.incoming {
        Incoming::Imap { tls: at, .. } | Incoming::Pop3 { tls: at, .. } => {
            *at = tls(&servers.incoming);
        }
        _ => {}
    }
    if let Outgoing::Smtp { tls: at, .. } = &mut preset.plan.outgoing {
        *at = tls(&servers.outgoing);
    }
    preset
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing(gap: Gap) -> Failed {
        Failed::NoServers {
            address: "me@example.test".to_owned(),
            gap,
            tried: "nothing".to_owned(),
        }
    }

    #[test]
    fn only_something_dotted_after_an_at_is_a_domain() {
        assert_eq!(
            typed_domain(" Me@Example.TEST "),
            Some("example.test".to_owned())
        );
        for typed in ["me", "@example.test", "me@host", "me@.test", "me@example."] {
            assert_eq!(typed_domain(typed), None, "{typed}");
        }
    }

    #[test]
    fn a_missing_search_falls_back_to_jmap_and_then_to_what_the_miss_was() {
        let session = || Some("https://jmap.example.test/session".to_owned());
        let no = || None;
        assert_eq!(
            settle(Err(nothing(Gap::Nothing)), session()),
            Looked::Jmap("https://jmap.example.test/session".to_owned())
        );
        for gap in [Gap::Nothing, Gap::StartTlsOnly] {
            assert_eq!(settle(Err(nothing(gap)), no()), Looked::Ask(nothing(gap)));
        }
        assert_eq!(
            settle(Err(nothing(Gap::PersonalMicrosoft)), no()),
            Looked::PersonalMicrosoft(nothing(Gap::PersonalMicrosoft))
        );
        let broken = || Failed::Broken("no client".to_owned());
        assert_eq!(settle(Err(broken()), no()), Looked::Unreachable(broken()));
    }
}
