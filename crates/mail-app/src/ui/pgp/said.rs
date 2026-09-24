//! What the reader says about a message's protection, a line a fact, for OpenPGP and S/MIME
//! alike. Pure, so every wording is a table test.
//!
//! The facts are the ones `mailo show` prints ([`crate::pgp::describe`],
//! [`crate::smime::describe`]); the words are the window's.

use mail_domain::*;

use super::{grouped, short, who};

/// How a line about a message's protection is drawn. A bad signature is the one that must be
/// impossible to miss; a signature nobody can check must not look like a good one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Tone {
    /// A plain fact: not encrypted, not signed.
    Plain,
    /// Opened, or a signature that checks.
    Good,
    /// Nothing can be said either way: a key this client does not hold.
    Unknown,
    /// A signature that matches the message and is still not to be believed: its certificate
    /// falls short. Amber, and said in full.
    Warn,
    /// A signature that does not check, or content that could not be read.
    Bad,
    /// Who a certificate is for and who issued it, under the line about its signature.
    Detail,
}

impl Tone {
    pub(in crate::ui) fn class(self) -> &'static str {
        match self {
            Tone::Plain => "seal-line",
            Tone::Good => "seal-line good",
            Tone::Unknown => "seal-line unknown",
            Tone::Warn => "seal-line warn",
            Tone::Bad => "seal-line bad",
            Tone::Detail => "seal-line detail",
        }
    }
}

/// One line the reader says about a message's protection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Said {
    pub tone: Tone,
    pub text: String,
}

fn line(tone: Tone, text: impl Into<String>) -> Said {
    Said {
        tone,
        text: text.into(),
    }
}

/// The line said when only part of a message is signed.
fn partly(coverage: Option<Coverage>, out: &mut Vec<Said>) {
    if coverage == Some(Coverage::Part) {
        out.push(line(
            Tone::Unknown,
            "Only part of this message is signed; the rest could say anything",
        ));
    }
}

/// What the reader says about an OpenPGP message.
pub(in crate::ui) fn said(protected: &crate::pgp::Protected) -> Vec<Said> {
    let mut out = Vec::new();
    out.push(match &protected.encryption {
        Encryption::NotEncrypted => line(Tone::Plain, "Not encrypted"),
        Encryption::Decrypted => line(Tone::Good, "Encrypted, and decrypted for reading"),
        Encryption::CannotDecrypt { to } => {
            let ids: Vec<String> = to.iter().map(|id| grouped(&id.to_string())).collect();
            line(
                Tone::Unknown,
                format!(
                    "Encrypted to keys you do not hold ({}), so it cannot be read here",
                    ids.join(", ")
                ),
            )
        }
        Encryption::Locked { key } => line(
            Tone::Unknown,
            format!(
                "Encrypted to your key {}, which needs its passphrase",
                short(*key)
            ),
        ),
        Encryption::Unreadable { why } => line(
            Tone::Bad,
            format!("Encrypted, and could not be read: {why}"),
        ),
    });
    let coverage = match &protected.verification {
        Verification::NoSignature => {
            out.push(line(Tone::Plain, "Not signed"));
            None
        }
        Verification::Good {
            signer,
            trust,
            coverage,
        } => {
            let name = protected
                .signer
                .as_ref()
                .map_or_else(|| "a key you hold".to_owned(), who);
            let trust = match trust {
                KeyTrust::Verified => "verified by you",
                KeyTrust::Unverified => "not verified by you",
            };
            out.push(line(
                Tone::Good,
                format!("Good signature by {name} · {} · {trust}", short(*signer)),
            ));
            Some(*coverage)
        }
        Verification::Bad { coverage } => {
            out.push(line(
                Tone::Bad,
                "Bad signature: the message was changed after it was signed, or the signature \
                 is forged",
            ));
            Some(*coverage)
        }
        Verification::UnknownKey { issuer, coverage } => {
            out.push(line(
                Tone::Unknown,
                format!(
                    "Signed by key {}, which you do not have, so the signature cannot be checked",
                    grouped(&issuer.to_string())
                ),
            ));
            Some(*coverage)
        }
    };
    partly(coverage, &mut out);
    out
}

/// Who a certificate is for, as a person reads it: its first address, else its subject.
pub(in crate::ui) fn whose(cert: &SmimeCert) -> String {
    cert.emails
        .first()
        .cloned()
        .unwrap_or_else(|| cert.subject.clone())
}

/// What the reader says about an S/MIME message.
pub(in crate::ui) fn said_smime(protected: &crate::smime::Protected) -> Vec<Said> {
    let mut out = Vec::new();
    out.push(match &protected.encryption {
        SmimeEncryption::NotEncrypted => line(Tone::Plain, "Not encrypted"),
        SmimeEncryption::Decrypted => line(
            Tone::Good,
            "Encrypted with S/MIME, and decrypted for reading",
        ),
        SmimeEncryption::CannotDecrypt { to } => line(
            Tone::Unknown,
            format!(
                "Encrypted with S/MIME to certificates you hold no key for ({}), so it cannot \
                 be read here",
                to.join("; ")
            ),
        ),
        SmimeEncryption::Unreadable { why } => line(
            Tone::Bad,
            format!("Encrypted with S/MIME, and could not be read: {why}"),
        ),
    });
    let name = protected
        .signer
        .as_ref()
        .map_or_else(|| "a certificate with no name".to_owned(), whose);
    let coverage = match &protected.verification {
        SmimeVerification::NoSignature => {
            out.push(line(Tone::Plain, "Not signed"));
            None
        }
        SmimeVerification::Good { coverage, .. } => {
            out.push(line(
                Tone::Good,
                format!(
                    "Good S/MIME signature by {name}, from a certificate an authority you trust \
                     issued"
                ),
            ));
            Some(*coverage)
        }
        SmimeVerification::Doubtful {
            problems, coverage, ..
        } => {
            out.push(line(
                Tone::Warn,
                format!(
                    "S/MIME signature by {name}: the message is as it was signed, but the \
                     certificate does not vouch for the sender"
                ),
            ));
            out.extend(
                problems
                    .iter()
                    .map(|problem| line(Tone::Warn, doubt(problem))),
            );
            Some(*coverage)
        }
        SmimeVerification::Bad { why, coverage } => {
            out.push(bad(why));
            Some(*coverage)
        }
        SmimeVerification::UnknownSigner { coverage } => {
            out.push(line(
                Tone::Unknown,
                "Signed with S/MIME by a certificate neither the message nor you hold, so the \
                 signature cannot be checked",
            ));
            Some(*coverage)
        }
    };
    if protected.verification != SmimeVerification::NoSignature
        && let Some(signer) = &protected.signer
    {
        out.push(line(Tone::Detail, certificate(signer)));
    }
    partly(coverage, &mut out);
    out
}

/// The signer's certificate, in one line: whom it names, for which addresses, and who issued it.
fn certificate(cert: &SmimeCert) -> String {
    let mut parts = vec![format!("Certificate of {}", cert.subject)];
    if !cert.emails.is_empty() {
        parts.push(cert.emails.join(", "));
    }
    parts.push(format!("issued by {}", cert.issuer));
    parts.push(format!(
        "valid {} to {}",
        cert.not_before.format("%Y-%m-%d"),
        cert.not_after.format("%Y-%m-%d")
    ));
    parts.join(" · ")
}

/// One thing wrong with the certificate behind a signature that matches, and what it means.
pub(in crate::ui) fn doubt(problem: &CertProblem) -> String {
    match problem {
        CertProblem::Untrusted => "Untrusted: no authority you trust issued its certificate, so \
                                   anyone could have made it. Trust it in Keys and certificates \
                                   only after checking it with its owner"
            .to_owned(),
        CertProblem::Expired { not_after } => format!(
            "Expired: a certificate in its chain stopped being valid on {}, before the message \
             says it was signed",
            not_after.format("%Y-%m-%d")
        ),
        CertProblem::NotYetValid { not_before } => format!(
            "Not yet valid: a certificate in its chain was not valid until {}, after the message \
             says it was signed",
            not_before.format("%Y-%m-%d")
        ),
        CertProblem::NotForEmail => "Not for mail: its certificate was issued for something \
                                     other than signing mail"
            .to_owned(),
        CertProblem::NotFrom { from } if from.is_empty() => {
            "The message names no sender to check its certificate against".to_owned()
        }
        CertProblem::NotFrom { from } => format!(
            "Signed with a certificate for someone else's address: it is not for {from}, who \
             the message says it is from"
        ),
    }
}

/// A signature that does not hold, in words. Changed or forged takes the danger ground; one
/// too weak to mean anything, or unreadable, is amber; one this client cannot check says so.
fn bad(why: &BadSignature) -> Said {
    match why {
        BadSignature::Altered => line(
            Tone::Bad,
            "Bad S/MIME signature: the message was changed after it was signed",
        ),
        BadSignature::Forged => line(
            Tone::Bad,
            "Bad S/MIME signature: it does not match its certificate, so it is forged or damaged",
        ),
        BadSignature::Weak { algorithm } => line(
            Tone::Warn,
            format!(
                "S/MIME signature made with {algorithm}, which is too weak to mean anything now"
            ),
        ),
        BadSignature::Unsupported { algorithm } => line(
            Tone::Unknown,
            format!("S/MIME signature made with {algorithm}, which this client does not check"),
        ),
        BadSignature::Malformed { why } => line(
            Tone::Warn,
            format!("S/MIME signature that cannot be read: {why}"),
        ),
    }
}
