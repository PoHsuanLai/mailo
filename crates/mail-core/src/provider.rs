//! Which provider an account is on.
//!
//! The chip draws a letter until that provider's own icon has been fetched once and
//! cached. The built-in table decides first, because an address on `gmail.com` is Google even
//! when the host was typed by hand. Otherwise the incoming host's suffix, and Graph,
//! which has no host. A lookalike (`imap.gmail.com.evil.test`) is not Google.

use crate::discover::known;
use chrono::{DateTime, Utc};
use mail_domain::{AccountPlan, AuthPlan, Incoming, Outgoing};
use porter_provider::Issuer;

/// A mail host the window knows by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Google,
    Microsoft,
    Fastmail,
    Icloud,
    Yahoo,
    /// Anything else, including a host that only resembles a known one.
    Imap,
}

impl Provider {
    /// Every provider, in the order an icon cache names its files.
    pub const ALL: [Provider; 6] = [
        Provider::Google,
        Provider::Microsoft,
        Provider::Fastmail,
        Provider::Icloud,
        Provider::Yahoo,
        Provider::Imap,
    ];

    /// The short name beside a row.
    pub fn short(self) -> &'static str {
        match self {
            Provider::Google => "gmail",
            Provider::Microsoft => "m365",
            Provider::Fastmail => "fastmail",
            Provider::Icloud => "icloud",
            Provider::Yahoo => "yahoo",
            Provider::Imap => "imap",
        }
    }

    /// The name a chip's title carries.
    pub fn title(self) -> &'static str {
        match self {
            Provider::Google => "Google",
            Provider::Microsoft => "Microsoft 365",
            Provider::Fastmail => "Fastmail",
            Provider::Icloud => "iCloud",
            Provider::Yahoo => "Yahoo",
            Provider::Imap => "IMAP",
        }
    }
}

/// Which provider `plan` is on.
///
/// Pure: the preset's clock is fixed, because the match does not read it and a
/// call must not depend on when it runs.
pub fn provider(plan: &AccountPlan) -> Provider {
    // `known` stores `now` on the expected capabilities and does not branch on it.
    let now = DateTime::<Utc>::UNIX_EPOCH;
    if let Some(preset) = known(&plan.address, now) {
        return match preset.plan.auth {
            AuthPlan::OAuth {
                issuer: Issuer::Google,
                ..
            } => Provider::Google,
            AuthPlan::OAuth {
                issuer: Issuer::Microsoft,
                ..
            } => Provider::Microsoft,
            _ => from_host(plan),
        };
    }
    from_host(plan)
}

/// What a window calls the service `plan` is on: its provider's name, or, on a host it does not
/// know, how mail is fetched from it, so a POP3 account on its own server reads POP3 and not IMAP.
pub fn service_name(plan: &AccountPlan) -> &'static str {
    match (provider(plan), &plan.incoming) {
        (Provider::Imap, Incoming::Pop3 { .. }) => "POP3",
        (Provider::Imap, Incoming::Jmap { .. }) => "JMAP",
        (known, _) => known.title(),
    }
}

fn from_host(plan: &AccountPlan) -> Provider {
    if matches!(plan.outgoing, Outgoing::Graph) {
        return Provider::Microsoft;
    }
    if plan.incoming == Incoming::Graph {
        return Provider::Microsoft;
    }
    // No host, so no provider to recognise: the generic mark.
    let served = plan.incoming.served_from().unwrap_or_default();
    let host = served.as_str();
    if is_host(host, "gmail.com") || is_host(host, "googlemail.com") {
        Provider::Google
    } else if is_host(host, "office365.com") || is_host(host, "outlook.com") {
        Provider::Microsoft
    } else if is_host(host, "fastmail.com") {
        Provider::Fastmail
    } else if is_host(host, "mail.me.com") || is_host(host, "icloud.com") {
        Provider::Icloud
    } else if is_host(host, "yahoo.com") {
        Provider::Yahoo
    } else {
        Provider::Imap
    }
}

/// `host` is `root` or a subdomain of it. A suffix with no label boundary is not.
fn is_host(host: &str, root: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    host == root || host.ends_with(&format!(".{root}"))
}

#[cfg(test)]
mod tests {
    use super::{Provider, provider, service_name};
    use mail_domain::{
        AccountPlan, AuthPlan, HttpAuth, Incoming, LeaveOnServer, Outgoing, SaslMech, Tls, Username,
    };

    struct Case {
        name: &'static str,
        address: &'static str,
        host: &'static str,
        graph: bool,
        expect: Provider,
    }

    fn plan(case: &Case) -> AccountPlan {
        AccountPlan {
            address: case.address.to_owned(),
            incoming: Incoming::Imap {
                host: case.host.to_owned(),
                port: 993,
                tls: Tls::Implicit,
            },
            outgoing: if case.graph {
                Outgoing::Graph
            } else {
                Outgoing::Smtp {
                    host: "smtp.example".to_owned(),
                    port: 465,
                    tls: Tls::Implicit,
                }
            },
            auth: AuthPlan::Password {
                username: Username::SameAsAddress,
                sasl: vec![SaslMech::Plain],
            },
            identities: Vec::new(),
        }
    }

    #[test]
    fn the_preset_wins_and_a_lookalike_host_does_not() {
        let cases = [
            Case {
                name: "gmail address is Google even on another host",
                address: "a@gmail.com",
                host: "imap.evil.test",
                graph: false,
                expect: Provider::Google,
            },
            Case {
                name: "googlemail address is Google",
                address: "a@googlemail.com",
                host: "imap.example",
                graph: false,
                expect: Provider::Google,
            },
            Case {
                name: "onmicrosoft address is Microsoft",
                address: "a@contoso.onmicrosoft.com",
                host: "imap.example",
                graph: false,
                expect: Provider::Microsoft,
            },
            Case {
                name: "gmail host suffix",
                address: "a@acme.example",
                host: "imap.gmail.com",
                graph: false,
                expect: Provider::Google,
            },
            Case {
                name: "lookalike gmail host is not Google",
                address: "a@imap.gmail.com.evil.test",
                host: "imap.gmail.com.evil.test",
                graph: false,
                expect: Provider::Imap,
            },
            Case {
                name: "notgmail.com is not Google",
                address: "a@notgmail.com",
                host: "imap.notgmail.com",
                graph: false,
                expect: Provider::Imap,
            },
            Case {
                name: "office365 host",
                address: "a@corp.example",
                host: "outlook.office365.com",
                graph: false,
                expect: Provider::Microsoft,
            },
            Case {
                name: "outlook host",
                address: "a@corp.example",
                host: "imap.outlook.com",
                graph: false,
                expect: Provider::Microsoft,
            },
            Case {
                name: "Graph has no host and is Microsoft",
                address: "a@corp.example",
                host: "imap.example",
                graph: true,
                expect: Provider::Microsoft,
            },
            Case {
                name: "fastmail host",
                address: "lists@fastmail.example",
                host: "imap.fastmail.com",
                graph: false,
                expect: Provider::Fastmail,
            },
            Case {
                name: "icloud host",
                address: "a@home.example",
                host: "imap.mail.me.com",
                graph: false,
                expect: Provider::Icloud,
            },
            Case {
                name: "yahoo host",
                address: "a@home.example",
                host: "imap.mail.yahoo.com",
                graph: false,
                expect: Provider::Yahoo,
            },
            Case {
                name: "anything else is imap",
                address: "a@example.test",
                host: "imap.example.test",
                graph: false,
                expect: Provider::Imap,
            },
        ];
        let mut failures = Vec::new();
        for case in cases {
            let got = provider(&plan(&case));
            if got != case.expect {
                failures.push(format!(
                    "{}: got {got:?}, expected {:?}",
                    case.name, case.expect
                ));
            }
            if got.short().is_empty() || got.title().is_empty() {
                failures.push(format!("{}: a provider with no mark", case.name));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn an_unknown_host_is_named_by_how_its_mail_is_fetched() {
        let imap = plan(&Case {
            name: "imap",
            address: "a@example.test",
            host: "mail.example.test",
            graph: false,
            expect: Provider::Imap,
        });
        let pop3 = AccountPlan {
            incoming: Incoming::Pop3 {
                host: "mail.example.test".to_owned(),
                port: 995,
                tls: Tls::Implicit,
                leave: LeaveOnServer::DeleteAfterFetch,
            },
            ..imap.clone()
        };
        let jmap = AccountPlan {
            incoming: Incoming::Jmap {
                session: "https://mail.example.test/.well-known/jmap".to_owned(),
                auth: HttpAuth::Basic,
            },
            ..imap.clone()
        };
        let gmail_pop3 = AccountPlan {
            address: "a@gmail.com".to_owned(),
            ..pop3.clone()
        };
        assert_eq!(service_name(&imap), "IMAP");
        assert_eq!(service_name(&pop3), "POP3");
        assert_eq!(service_name(&jmap), "JMAP");
        assert_eq!(
            service_name(&gmail_pop3),
            "Google",
            "a known provider keeps its name"
        );
    }

    /// A JMAP session URL is read once, so a port, a user part, an upper-case name or an IPv6
    /// address gives the provider, the sieve server and the receiver the same host.
    #[test]
    fn a_jmap_session_url_has_one_host_for_every_reader() {
        use mail_logic::trust::registered;
        use mail_mime::auth::Receiver;
        use mail_proto::sieve::{NoSieve, endpoint};
        use porter_provider::Issuer;

        let base = plan(&Case {
            name: "base",
            address: "a@example.test",
            host: "mail.example.test",
            graph: false,
            expect: Provider::Imap,
        });
        // The session, the host it names, and the provider that host belongs to.
        let cases = [
            (
                "https://mail.example.test/.well-known/jmap",
                "mail.example.test",
                Provider::Imap,
            ),
            (
                "https://ada:pw@mail.example.test:8443/jmap",
                "mail.example.test",
                Provider::Imap,
            ),
            (
                "https://Mail.Example.Test:443/jmap?x=1#top",
                "mail.example.test",
                Provider::Imap,
            ),
            (
                "https://[2001:db8::1]:8443/jmap",
                "2001:db8::1",
                Provider::Imap,
            ),
            (
                "https://ada@imap.gmail.com:8443/jmap",
                "imap.gmail.com",
                Provider::Google,
            ),
        ];
        for (session, host, expect) in cases {
            let plan = AccountPlan {
                incoming: Incoming::Jmap {
                    session: session.to_owned(),
                    auth: HttpAuth::Basic,
                },
                ..base.clone()
            };
            assert_eq!(
                plan.incoming.served_from().as_deref(),
                Some(host),
                "{session}"
            );
            assert_eq!(provider(&plan), expect, "{session}");
            match endpoint(&plan) {
                Ok(found) => assert_eq!(found.host, host, "{session}"),
                Err(why) => {
                    assert_eq!(why, NoSieve::Provider(Issuer::Google), "{session}");
                    assert_eq!(expect, Provider::Google, "{session}");
                }
            }
            let believed = match registered(host) {
                Some(domain) if domain == "gmail.com" => {
                    Receiver::Domains(vec![domain, "google.com".to_owned()])
                }
                Some(domain) => Receiver::Domains(vec![domain]),
                None => Receiver::Topmost,
            };
            assert_eq!(crate::auth::receiver(&plan), believed, "{session}");
        }
    }
}

#[path = "provider/icon/mod.rs"]
pub mod icon;
