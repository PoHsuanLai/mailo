//! What the reader says about a message's protection, a line a fact, for OpenPGP and S/MIME
//! alike. Pure, so every wording is a table test.
//!
//! The facts are the ones `mailo show` prints (`cli::pgp::describe`,
//! `cli::smime::describe`); the words are the window's.

use mail_domain::*;

use super::{grouped, short, who};
use crate::ui::copy::status::protection as words;

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
        out.push(line(Tone::Unknown, words::ONLY_PART_SIGNED));
    }
}

/// What the reader says about an OpenPGP message.
pub(in crate::ui) fn said(protected: &mail_core::pgp::Protected) -> Vec<Said> {
    let mut out = Vec::new();
    out.push(match &protected.encryption {
        Encryption::NotEncrypted => line(Tone::Plain, words::NOT_ENCRYPTED),
        Encryption::Decrypted => line(Tone::Good, words::DECRYPTED),
        Encryption::CannotDecrypt { to } => {
            let ids: Vec<String> = to.iter().map(|id| grouped(&id.to_string())).collect();
            line(Tone::Unknown, words::to_keys_not_held(&ids.join(", ")))
        }
        Encryption::Locked { key } => line(Tone::Unknown, words::passphrase_needed(&short(*key))),
        Encryption::Unreadable { why } => line(Tone::Bad, words::unreadable(why)),
    });
    let coverage = match &protected.verification {
        Verification::NoSignature => {
            out.push(line(Tone::Plain, words::NOT_SIGNED));
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
                .map_or_else(|| words::A_KEY_YOU_HOLD.to_owned(), who);
            let trust = match trust {
                KeyTrust::Verified => words::VERIFIED_BY_YOU,
                KeyTrust::Unverified => words::NOT_VERIFIED_BY_YOU,
            };
            out.push(line(
                Tone::Good,
                words::good_signature(&name, &short(*signer), trust),
            ));
            Some(*coverage)
        }
        Verification::Bad { coverage } => {
            out.push(line(Tone::Bad, words::BAD_SIGNATURE));
            Some(*coverage)
        }
        Verification::UnknownKey { issuer, coverage } => {
            out.push(line(
                Tone::Unknown,
                words::signed_by_key_not_held(&grouped(&issuer.to_string())),
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
pub(in crate::ui) fn said_smime(protected: &mail_core::smime::Protected) -> Vec<Said> {
    let mut out = Vec::new();
    out.push(match &protected.encryption {
        SmimeEncryption::NotEncrypted => line(Tone::Plain, words::NOT_ENCRYPTED),
        SmimeEncryption::DecryptedAuthenticated => line(Tone::Good, words::SMIME_DECRYPTED),
        SmimeEncryption::Decrypted => line(Tone::Unknown, words::SMIME_NOT_AUTHENTICATED),
        SmimeEncryption::CannotDecrypt { to } => line(
            Tone::Unknown,
            words::smime_to_certs_not_held(&to.join("; ")),
        ),
        SmimeEncryption::Unreadable { why } => line(Tone::Bad, words::smime_unreadable(why)),
    });
    let name = protected
        .signer
        .as_ref()
        .map_or_else(|| words::A_CERTIFICATE_WITH_NO_NAME.to_owned(), whose);
    let coverage = match &protected.verification {
        SmimeVerification::NoSignature => {
            out.push(line(Tone::Plain, words::NOT_SIGNED));
            None
        }
        SmimeVerification::Good { coverage, .. } => {
            out.push(line(Tone::Good, words::smime_good(&name)));
            Some(*coverage)
        }
        SmimeVerification::Doubtful {
            problems, coverage, ..
        } => {
            out.push(line(Tone::Warn, words::smime_doubtful(&name)));
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
            out.push(line(Tone::Unknown, words::SMIME_UNKNOWN_SIGNER));
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
    let mut parts = vec![words::certificate_of(&cert.subject)];
    if !cert.emails.is_empty() {
        parts.push(cert.emails.join(", "));
    }
    parts.push(words::issued_by(&cert.issuer));
    parts.push(words::valid(
        &cert.not_before.format("%Y-%m-%d").to_string(),
        &cert.not_after.format("%Y-%m-%d").to_string(),
    ));
    parts.join(" · ")
}

/// One thing wrong with the certificate behind a signature that matches, and what it means.
pub(in crate::ui) fn doubt(problem: &CertProblem) -> String {
    let day = |at: &chrono::DateTime<chrono::Utc>| at.format("%Y-%m-%d").to_string();
    match problem {
        CertProblem::Untrusted => words::UNTRUSTED.to_owned(),
        CertProblem::Expired { not_after } => words::expired(&day(not_after)),
        CertProblem::NotYetValid { not_before } => words::not_yet_valid(&day(not_before)),
        CertProblem::NotForEmail => words::NOT_FOR_EMAIL.to_owned(),
        CertProblem::NotFrom { from } if from.is_empty() => words::NO_SENDER.to_owned(),
        CertProblem::NotFrom { from } => words::for_someone_else(from),
    }
}

/// A signature that does not hold, in words. Changed or forged takes the danger ground; one
/// too weak to mean anything, or unreadable, is amber; one this client cannot check says so.
fn bad(why: &BadSignature) -> Said {
    match why {
        BadSignature::Altered => line(Tone::Bad, words::SMIME_ALTERED),
        BadSignature::Forged => line(Tone::Bad, words::SMIME_FORGED),
        BadSignature::Weak { algorithm } => line(Tone::Warn, words::smime_weak(algorithm)),
        BadSignature::Unsupported { algorithm } => {
            line(Tone::Unknown, words::smime_unsupported(algorithm))
        }
        BadSignature::Malformed { why } => line(Tone::Warn, words::smime_malformed(why)),
    }
}
