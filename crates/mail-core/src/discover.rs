//! `account add` for a domain the preset table does not know, and `account discover`.
//!
//! The lookup is `mail_runtime::discover`'s and what it means is `mail_proto::discover`'s. This
//! is the part between the answer and the account: what was found and where it came from, as
//! words ([`describe`]) and as a typed miss ([`Failed`]). Getting a yes before any of it is used
//! is the front-end's: at the terminal, `mail_app::cli::discover`.

mod jmap;
pub use jmap::find as find_jmap;
use mail_domain::{AuthPlan, Incoming, OAuthIssuer, Outgoing, Retry, Tls, Username};
use mail_proto::discover::{Found, Unusable};
use mail_runtime::discover::NotFound;
use std::fmt::Write as _;

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

/// What a domain's servers were missing, as far as the search could tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// Nothing usable, and no more to say.
    Nothing,
    /// Servers were published, but only ones that upgrade with STARTTLS, which this client
    /// does not use.
    StartTlsOnly,
    /// A personal Microsoft mailbox, which no longer takes a password.
    PersonalMicrosoft,
}

/// Why a lookup found no servers: what a caller can act on, before any wording.
///
/// [`Failed::said`] is the command line's prose; a window matches on the variants instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// The sources answered, and none named servers this client can use.
    NoServers {
        address: String,
        gap: Gap,
        /// Every step tried and what came of it, as `NotFound` prints them.
        tried: String,
    },
    /// Nothing answered at all: most likely this computer is offline.
    Unreachable {
        address: String,
        /// Whether trying again can help.
        retry: Retry,
        why: String,
    },
    /// The lookup could not be set up: no async runtime, no HTTP client, no resolver.
    Broken(String),
}

impl Failed {
    /// What to say at a terminal: why, and the way to configure the account by hand.
    pub fn said(&self) -> String {
        let (address, why, gap) = match self {
            Failed::Broken(why) => return why.clone(),
            Failed::NoServers {
                address,
                gap,
                tried,
            } => (address, tried, *gap),
            Failed::Unreachable { address, why, .. } => (address, why, Gap::Nothing),
        };
        let mut out = format!("could not find servers for {address}: {why}\n");
        match gap {
            Gap::PersonalMicrosoft => return out,
            Gap::StartTlsOnly => out.push_str(
                "\nThis domain publishes servers that use STARTTLS, which this client does not \
                 use: the upgrade can be stripped by anyone on the path, and the password then \
                 crosses in the clear. Ask the provider whether it also offers IMAP on port 993 \
                 (or POP3 on 995) and submission on port 465; if it does, name them:\n",
            ),
            Gap::Nothing => out.push_str("\nName the servers yourself:\n"),
        }
        let _ = write!(
            out,
            "\n  mailo account add {address} --imap HOST[:993] --smtp HOST[:465] [--login NAME]\n\
             \nor --pop3 HOST[:995] in place of --imap for a POP3-only server."
        );
        out
    }
}

/// What a search that found nothing means for `address`.
pub fn classify(address: &str, why: &NotFound) -> Failed {
    if why.offline() {
        return Failed::Unreachable {
            address: address.to_owned(),
            retry: Retry::Now,
            why: why.to_string(),
        };
    }
    let gap = if why
        .unusable()
        .any(|u| matches!(u, Unusable::PersonalMicrosoft))
    {
        Gap::PersonalMicrosoft
    } else if why
        .unusable()
        .any(|u| matches!(u, Unusable::NoImplicitTls { .. }))
    {
        Gap::StartTlsOnly
    } else {
        Gap::Nothing
    };
    Failed::NoServers {
        address: address.to_owned(),
        gap,
        tried: why.to_string(),
    }
}

/// What to say when nothing usable was found, with the way to configure the account by hand.
pub fn not_found(address: &str, why: &NotFound) -> String {
    classify(address, why).said()
}

/// Look the address up, over the network.
pub fn lookup(address: &str, now: chrono::DateTime<chrono::Utc>) -> Result<Found, String> {
    search(address, now).map_err(|why| why.said())
}

/// [`lookup`], with the reason a miss was left typed.
pub fn search(address: &str, now: chrono::DateTime<chrono::Utc>) -> Result<Found, Failed> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Failed::Broken(format!("cannot start the async runtime: {e}")))?;
    runtime.block_on(async {
        let http = mail_runtime::discover::client_builder()
            .build()
            .map_err(|e| Failed::Broken(format!("cannot build an HTTP client: {e}")))?;
        let dns =
            mail_runtime::discover::SystemDns::new().map_err(|e| Failed::Broken(e.to_string()))?;
        mail_runtime::discover::discover(
            address,
            &http,
            &dns,
            &mail_runtime::discover::Sources::default(),
            now,
        )
        .await
        .map_err(|why| classify(address, &why))
    })
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
    fn a_miss_is_typed_for_the_window_and_unchanged_at_the_terminal() {
        let tried = |miss| Tried {
            what: "https://example.test/".to_owned(),
            miss,
        };
        let offline = NotFound {
            tried: vec![tried(Miss::Unreachable("no route".to_owned()))],
            timed_out: false,
        };
        assert!(matches!(
            classify("me@example.test", &offline),
            Failed::Unreachable {
                retry: Retry::Now,
                ..
            }
        ));
        let nothing = NotFound {
            tried: vec![tried(Miss::Absent)],
            timed_out: false,
        };
        let failed = classify("me@example.test", &nothing);
        assert!(matches!(
            failed,
            Failed::NoServers {
                gap: Gap::Nothing,
                ..
            }
        ));
        assert_eq!(
            failed.said(),
            format!(
                "could not find servers for me@example.test: {nothing}\n\nName the servers \
                 yourself:\n\n  mailo account add me@example.test --imap HOST[:993] --smtp \
                 HOST[:465] [--login NAME]\n\nor --pop3 HOST[:995] in place of --imap for a \
                 POP3-only server."
            )
        );
        let microsoft = NotFound {
            tried: vec![tried(Miss::Unusable(Unusable::PersonalMicrosoft))],
            timed_out: false,
        };
        let failed = classify("me@outlook.test", &microsoft);
        assert!(matches!(
            failed,
            Failed::NoServers {
                gap: Gap::PersonalMicrosoft,
                ..
            }
        ));
        assert!(!failed.said().contains("mailo account add"));
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
