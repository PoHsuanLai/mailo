//! Which provider an account is on.
//!
//! The chip draws a letter until that provider's own icon has been fetched once and
//! cached. The preset decides first, because an address on `gmail.com` is Google even
//! when the host was typed by hand. Otherwise the incoming host's suffix, and Graph,
//! which has no host. A lookalike (`imap.gmail.com.evil.test`) is not Google.

use chrono::{DateTime, Utc};
use mail_domain::presets::preset_for;
use mail_domain::{AccountPlan, AuthPlan, Incoming, OAuthIssuer, Outgoing};

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

    /// The letter on the chip.
    pub fn mark(self) -> &'static str {
        match self {
            Provider::Google => "G",
            Provider::Microsoft => "M",
            Provider::Fastmail => "F",
            Provider::Icloud => "i",
            Provider::Yahoo => "Y",
            Provider::Imap => "@",
        }
    }

    /// The letter's colour, or `None` for `@`, which is drawn in `--ink-soft`.
    pub fn color(self) -> Option<&'static str> {
        match self {
            Provider::Google => Some("#1A73E8"),
            Provider::Microsoft => Some("#0F6CBD"),
            Provider::Fastmail => Some("#2A5DB0"),
            Provider::Icloud => Some("#3A82F7"),
            Provider::Yahoo => Some("#6001D2"),
            Provider::Imap => None,
        }
    }

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
    // `preset_for` stores `now` on the expected capabilities and does not branch on it.
    let now = DateTime::<Utc>::UNIX_EPOCH;
    if let Some(preset) = preset_for(&plan.address, now) {
        return match preset.plan.auth {
            AuthPlan::OAuth {
                issuer: OAuthIssuer::Google,
                ..
            } => Provider::Google,
            AuthPlan::OAuth {
                issuer: OAuthIssuer::Microsoft,
                ..
            } => Provider::Microsoft,
            _ => from_host(plan),
        };
    }
    from_host(plan)
}

/// The host of a JMAP session URL, or nothing when it has none.
fn session_host(session: &str) -> &str {
    let rest = session.split_once("://").map_or(session, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    host.split(':').next().unwrap_or("")
}

fn from_host(plan: &AccountPlan) -> Provider {
    if matches!(plan.outgoing, Outgoing::Graph) {
        return Provider::Microsoft;
    }
    let host = match &plan.incoming {
        Incoming::Imap { host, .. } | Incoming::Pop3 { host, .. } => host.as_str(),
        Incoming::Jmap { session, .. } => session_host(session),
        // No host, so no provider to recognise: the generic mark.
        Incoming::Local => "",
        Incoming::Graph => return Provider::Microsoft,
    };
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
    use super::{Provider, provider};
    use mail_domain::{AccountPlan, AuthPlan, Incoming, Outgoing, SaslMech, Tls, Username};

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
            if got.short().is_empty() || got.mark().is_empty() || got.title().is_empty() {
                failures.push(format!("{}: a provider with no mark", case.name));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}

#[path = "provider/icon/mod.rs"]
pub(crate) mod icon;
