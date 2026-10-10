//! The four providers whose servers nobody has checked: Fastmail, iCloud, Yahoo and GMX.
//!
//! Their domains, MX suffixes, hosts and ports are in porter's provider files
//! (compiled into `porter-provider`), written from each provider's public documentation. mailo's
//! old preset table did not have them (it branded an account by its incoming host, and found the
//! servers through the ISPDB), so no host here was ever recorded against the real server. Nothing
//! else tests them; the suite proves only that an address resolves to its file.
//!
//! **`#[ignore]` on purpose.** It reaches four third parties. Run it deliberately:
//!
//! ```text
//! cargo test -p mail-core --test core -- --ignored --nocapture live_presets::
//! ```
//!
//! No credentials and no mailbox access: it resolves
//! an address to its file, opens each server with the TLS the file says, and reads what the
//! server says first (IMAP `CAPABILITY` is answered before any login; an SMTP server greets
//! before anything is sent). A server that cannot be reached is skipped, not failed: a test that
//! fails because someone is on a train is a test people learn to ignore. A server that answers
//! and is not what the file says is a failure, and the file is what to correct.

use mail_core::discover::{self, Source};
use mail_domain::{AuthPlan, Incoming, Outgoing, SaslMech, Tls, Username};
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use mail_runtime::{Transport, drive};
use porter_core::{Credential, SecretText};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::watch;

/// One address per provider, and the file it must resolve to.
const PROVIDERS: &[(&str, &str)] = &[
    ("someone@fastmail.com", "Fastmail"),
    ("someone@icloud.com", "iCloud"),
    ("someone@yahoo.com", "Yahoo Mail"),
    ("someone@gmx.de", "GMX"),
];

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

/// The unauthenticated `CAPABILITY` line of the IMAP server, or `None` when it cannot be reached.
async fn capability(host: &str, port: u16) -> Option<String> {
    let mut transport = match Transport::connect(host, port, Tls::Implicit).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("skipping {host}:{port}: unreachable ({e})");
            return None;
        }
    };
    // No login is sent: the session names `CAPABILITY` alone, so the credential is never used.
    let auth = ImapAuth {
        username: "unused".to_owned(),
        credential: Credential::Password(SecretText::new("unused".to_owned())),
        sasl: vec![SaslMech::Plain],
    };
    let mut session =
        ImapSession::new(auth, vec![ImapCommand::Capability]).expect("a clean credential");
    let (_tx, mut cancel) = watch::channel(false);
    let transcript = drive(&mut session, &mut transport, &mut cancel)
        .await
        .unwrap_or_else(|e| panic!("{host}:{port} does not take the bytes this client sends: {e}"));
    Some(
        transcript
            .untagged
            .iter()
            .map(|u| u.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .to_uppercase(),
    )
}

/// What an SMTP server says first on `host:port`, or `None` when it cannot be reached.
///
/// Implicit TLS is opened with this client's own transport, which proves the certificate is for
/// the name. A submission port that upgrades is read in the clear: the greeting is all that is
/// asked, and nothing is sent.
async fn smtp_greeting(host: &str, port: u16, tls: Tls) -> Option<String> {
    match tls {
        Tls::Implicit => match Transport::connect(host, port, Tls::Implicit).await {
            Ok(_) => Some("220 (TLS from the first byte)".to_owned()),
            Err(e) => {
                eprintln!("skipping {host}:{port}: unreachable ({e})");
                None
            }
        },
        Tls::StartTlsRequired | Tls::Plaintext => {
            let stream = match TcpStream::connect((host, port)).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("skipping {host}:{port}: unreachable ({e})");
                    return None;
                }
            };
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).await.ok()?;
            Some(line)
        }
    }
}

#[tokio::test]
#[ignore = "live: resolves real addresses and reaches Fastmail, iCloud, Yahoo and GMX servers"]
async fn the_four_providers_servers_are_what_their_files_say() {
    for (address, label) in PROVIDERS {
        // A provider file answers before any fetch, so this asks nothing of the network.
        let found = discover::search(address, now())
            .await
            .unwrap_or_else(|e| panic!("{address}: {e:?}"));
        assert!(
            matches!(&found.source, Source::Provider { label: got } if got == label),
            "{address}: {}",
            found.source
        );
        let plan = &found.preset.plan;
        assert!(matches!(
            &plan.auth,
            AuthPlan::Password {
                username: Username::SameAsAddress,
                ..
            }
        ));
        let Incoming::Imap { host, port, tls } = &plan.incoming else {
            panic!("{address}: {:?}", plan.incoming)
        };
        assert_eq!(*tls, Tls::Implicit);
        if let Some(said) = capability(host, *port).await {
            eprintln!("{label}: {host}:{port} says {said}");
            assert!(said.contains("IMAP4REV1"), "{label}: {said}");
        }
        let Outgoing::Smtp { host, port, tls } = &plan.outgoing else {
            panic!("{address}: {:?}", plan.outgoing)
        };
        if let Some(said) = smtp_greeting(host, *port, *tls).await {
            eprintln!("{label}: {host}:{port} says {said}");
            assert!(said.starts_with("220"), "{label}: {said}");
        }
    }
}
