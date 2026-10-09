//! What the server that received a message says it checked about the sender: SPF, DKIM and
//! DMARC, from the `Authentication-Results` field `mail_mime::auth` believes.
//!
//! Which field is believed follows from who received the mail, and that is the account's
//! server: its registered domain is the provider's, and only a field whose authserv-id lies
//! under it is read (`mail_mime::Receiver::Domains`). An account with no server name to go on (a
//! local one, Graph, an IP literal) reads only the topmost field.
//!
//! Read from the stored raw message each time, like the list headers (`crate::unsubscribe`):
//! the raw blob already holds the field, and a column would need a migration and a change to
//! the frozen `Message` to cache something shown for one message at a time.

use mail_domain::{AccountPlan, Incoming, Message};
use mail_mime::{AuthResults, Check, Receiver, Verdict};
use mail_store::SqliteStore;
use porter_core::AccountId;

/// Providers whose receiving servers sign as a domain other than the one their mail is read
/// from, keyed by the registered domain of the account's server. Google's help pages show its
/// fields as `Authentication-Results: mx.google.com; …` for mail read at `imap.gmail.com`.
const ALSO_SIGNS_AS: &[(&str, &[&str])] = &[
    ("gmail.com", &["google.com"]),
    ("googlemail.com", &["google.com"]),
];

/// Whose `Authentication-Results` to believe for mail that came to an account with `plan`.
pub fn receiver(plan: &AccountPlan) -> Receiver {
    let host = match &plan.incoming {
        Incoming::Imap { host, .. } | Incoming::Pop3 { host, .. } => Some(host.clone()),
        Incoming::Jmap { session, .. } => url::Url::parse(session)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned)),
        Incoming::Local | Incoming::Graph => None,
    };
    let Some(registered) = host.as_deref().and_then(crate::trust::registered) else {
        return Receiver::Topmost;
    };
    let mut domains = vec![registered.clone()];
    if let Some((_, also)) = ALSO_SIGNS_AS.iter().find(|(d, _)| *d == registered) {
        domains.extend(also.iter().map(|d| (*d).to_owned()));
    }
    Receiver::Domains(domains)
}

/// The account's plan, when the store has one that reads.
fn plan_of(store: &SqliteStore, account: AccountId) -> Option<AccountPlan> {
    let plan: String = store
        .connection()
        .query_row(
            "SELECT plan FROM accounts WHERE id = ?1",
            [account.to_string()],
            |row| row.get(0),
        )
        .ok()?;
    serde_json::from_str(&plan).ok()
}

/// What the receiving server said about `message`'s sender, read from its stored bytes. `None`
/// when the body is not here (fetching it would be a POP3 `RETR`, which marks it read), or when
/// no field is believed. Reads a blob: call it off the thread that draws.
pub fn results_of(store: &SqliteStore, message: &Message) -> Option<AuthResults> {
    let raw = message.body.raw()?;
    let bytes = store.blobs().get(raw).ok()?;
    let receiver = plan_of(store, message.account.clone())
        .as_ref()
        .map_or(Receiver::Topmost, receiver);
    mail_mime::authentication_results(&bytes, &receiver)
}

/// What the checks come to, as one word the reader can show.
///
/// DMARC's rule (RFC 7489 §4.2), which is the receiving server's own when it reports DMARC: a
/// message is the sender's when a check that passed was about the sender's domain. Either check
/// is enough. SPF judges the server that last handed the mail on, so it fails whenever the mail
/// was forwarded, fetched into another mailbox or sent through a list; DKIM signs the message
/// itself and survives those, and a list that rewrites the body breaks it. A failure of one is
/// therefore never a verdict while the other passed, and a failure on its own is weak: only the
/// domain's own word that a server is not its (DMARC's fail, or SPF's hard fail for the sender's
/// domain) is shown as a warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// DMARC passed; or, with no DMARC verdict, DKIM or SPF passed for the sender's domain.
    Passed,
    /// DMARC failed; or, with no DMARC verdict and no pass for the sender's domain, that domain's
    /// SPF record says the sending server is not its.
    Failed,
    /// Neither: nothing was checked, what passed was about another domain (a mailing service
    /// signing as itself), or what failed is what forwarding breaks (SPF's softfail, a DKIM
    /// signature the trip changed).
    Unsure,
}

impl Standing {
    /// The `data-standing` word.
    pub fn word(self) -> &'static str {
        match self {
            Standing::Passed => "pass",
            Standing::Failed => "fail",
            Standing::Unsure => "unsure",
        }
    }
}

/// What `results` come to.
pub fn standing(results: &AuthResults) -> Standing {
    match results.dmarc.as_ref().map(|check| check.verdict) {
        Some(Verdict::Pass) => return Standing::Passed,
        Some(Verdict::Fail) => return Standing::Failed,
        _ => {}
    }
    let Some(from) = results.from.as_deref() else {
        return Standing::Unsure;
    };
    let ours = |check: &Check| check.domain.as_deref().is_some_and(|d| aligned(d, from));
    let passed = |check: &Check| check.verdict == Verdict::Pass && ours(check);
    if results.signatures.iter().any(passed) || results.spf.as_ref().is_some_and(passed) {
        return Standing::Passed;
    }
    match &results.spf {
        Some(spf) if spf.verdict == Verdict::Fail && ours(spf) => Standing::Failed,
        _ => Standing::Unsure,
    }
}

/// Whether a check about `domain` speaks for mail from `from`: DMARC's relaxed alignment
/// (RFC 7489 §3.1), the two under the same registered domain. `news.example.com` signs for
/// `example.com`; `example.com.evil.test` does not.
fn aligned(domain: &str, from: &str) -> bool {
    let registered = |d: &str| psl::domain_str(d).map(str::to_owned);
    match (registered(domain), registered(from)) {
        (Some(a), Some(b)) => a == b,
        // A name with no registered domain (a bare suffix, a single label) aligns only with
        // itself.
        _ => domain == from,
    }
}

/// The checks in words: "SPF pass · DKIM pass (example.com) · DMARC pass". Only the methods
/// the field mentions.
pub fn words(results: &AuthResults) -> String {
    [
        ("SPF", &results.spf),
        ("DKIM", &results.dkim),
        ("DMARC", &results.dmarc),
    ]
    .into_iter()
    .filter_map(|(name, check)| {
        let check = check.as_ref()?;
        let said = verdict_word(check.verdict);
        Some(match (&check.domain, check.verdict) {
            // Whose signature passed is the part that matters: a DKIM pass from a domain that
            // is not the sender's proves only that someone signed it.
            (Some(domain), Verdict::Pass) if name == "DKIM" => format!("{name} {said} ({domain})"),
            _ => format!("{name} {said}"),
        })
    })
    .collect::<Vec<_>>()
    .join(" · ")
}

fn verdict_word(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Pass => "pass",
        Verdict::Fail => "fail",
        Verdict::SoftFail => "softfail",
        Verdict::Neutral => "neutral",
        Verdict::None => "none",
        Verdict::TempError => "temperror",
        Verdict::PermError => "permerror",
        Verdict::Policy => "policy",
        Verdict::Unknown => "unknown",
    }
}

/// The one line the reader and the sender card show: what it comes to, and who said so.
pub fn sentence(results: &AuthResults) -> String {
    let checks = words(results);
    let checks = if checks.is_empty() {
        "nothing checked".to_owned()
    } else {
        checks
    };
    match &results.authserv_id {
        Some(by) => format!("{checks} — checked by {by}"),
        None => checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::{AuthPlan, Identity, LeaveOnServer, Outgoing, Tls};

    fn plan(incoming: Incoming) -> AccountPlan {
        AccountPlan {
            address: "me@example.com".to_owned(),
            incoming,
            outgoing: Outgoing::Smtp {
                host: "smtp.example.com".to_owned(),
                port: 465,
                tls: Tls::Implicit,
            },
            auth: AuthPlan::Password {
                username: mail_domain::Username::SameAsAddress,
                sasl: Vec::new(),
            },
            identities: Vec::<Identity>::new(),
        }
    }

    fn imap(host: &str) -> Incoming {
        Incoming::Imap {
            host: host.to_owned(),
            port: 993,
            tls: Tls::Implicit,
        }
    }

    #[test]
    fn the_receiver_is_the_account_servers_registered_domain() {
        let domains =
            |list: &[&str]| Receiver::Domains(list.iter().map(|d| (*d).to_owned()).collect());
        let cases: Vec<(&str, Incoming, Receiver)> = vec![
            (
                "gmail signs as google",
                imap("imap.gmail.com"),
                domains(&["gmail.com", "google.com"]),
            ),
            (
                "a registered domain",
                imap("mail.example.co.uk"),
                domains(&["example.co.uk"]),
            ),
            (
                "pop3 too",
                Incoming::Pop3 {
                    host: "pop.example.com".to_owned(),
                    port: 995,
                    tls: Tls::Implicit,
                    leave: LeaveOnServer::Keep,
                },
                domains(&["example.com"]),
            ),
            ("an ip literal", imap("192.0.2.1"), Receiver::Topmost),
            ("local mail", Incoming::Local, Receiver::Topmost),
        ];
        for (name, incoming, want) in cases {
            assert_eq!(receiver(&plan(incoming)), want, "case: {name}");
        }
    }
}
