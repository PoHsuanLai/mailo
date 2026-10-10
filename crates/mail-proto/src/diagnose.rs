//! Turning a server's refusal into something the user can act on.
//!
//! A mail server that refuses you is usually right, and usually says so in a way that means
//! nothing to the person reading it. `535 5.7.139 Authentication unsuccessful` is the tenant
//! administrator having switched SMTP AUTH off; `534-5.7.9 Application-specific password
//! required` is two-factor authentication needing an App Password. Both look identical to "wrong
//! password" from the client's side, and a user who believes that will retype a correct password
//! for as long as their patience lasts.
//!
//! What this maps are **documented, provider-specific codes**, not guesses at prose. Every entry
//! is a string the provider publishes and versions; nothing here pattern-matches on wording that
//! a server might reasonably change, because a confident wrong explanation is worse than the raw
//! text it replaced.
//!
//! Advice, never a decision: this changes what is printed, never whether something is retried.
//! [`crate::Retryable`] owns that, and a diagnosis that quietly altered retry behaviour would be
//! a second policy disagreeing with the first.

use crate::machine::ProtoError;

/// What a refusal turned out to be, when it is one we recognise.
///
/// The words for each are the caller's: this crate names the cause and no front-end's wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Diagnosis {
    /// A Microsoft 365 tenant with SMTP client authentication switched off (`5.7.139`): an
    /// administrator's setting, and nothing about the password.
    TenantSmtpAuthOff,
    /// The server wants `STARTTLS` before it will authenticate (`5.7.3`).
    StartTlsFirst,
    /// A Google account with two-factor authentication needs an App Password (`5.7.9`).
    AppPasswordNeeded,
    /// Google refused the credential: account passwords no longer work for IMAP or SMTP, and an
    /// App Password does (`5.7.8`).
    GoogleRejectedCredential,
    /// The protocol is switched off for this mailbox, which is not the credential being wrong
    /// and is fixed by someone else.
    ProtocolSwitchedOff,
}

/// What a refusal means, when it is one we recognise.
///
/// `None` is the common case and the safe one: the server's own words are shown unchanged.
pub fn diagnose(error: &ProtoError) -> Option<Diagnosis> {
    let text = match error {
        ProtoError::AuthRejected(text) => text,
        ProtoError::Refused { text, .. } => text,
        _ => return None,
    };
    diagnose_text(text)
}

/// The same, for a reply that was not turned into a `ProtoError`.
pub fn diagnose_text(text: &str) -> Option<Diagnosis> {
    let upper = text.to_uppercase();

    // Microsoft publishes these as enhanced status codes. `5.7.139` in particular is the one a
    // work or school mailbox hits, and it is a tenant setting rather than anything about the
    // account: `Set-CASMailbox -SmtpClientAuthenticationDisabled $false` is what changes it.
    if has_code(&upper, "5.7.139") {
        return Some(Diagnosis::TenantSmtpAuthOff);
    }
    if has_code(&upper, "5.7.3") && upper.contains("STARTTLS") {
        return Some(Diagnosis::StartTlsFirst);
    }
    // Google's two: an account with 2FA needs an App Password, and one without it can no longer
    // use the account password at all.
    if has_code(&upper, "5.7.9") || upper.contains("APPLICATION-SPECIFIC PASSWORD") {
        return Some(Diagnosis::AppPasswordNeeded);
    }
    if has_code(&upper, "5.7.8") && upper.contains("USERNAME AND PASSWORD NOT ACCEPTED") {
        return Some(Diagnosis::GoogleRejectedCredential);
    }
    // Exchange Online and Dovecot both say this when the protocol is disabled for the mailbox,
    // which is not the same as the credential being wrong and is fixed by someone else.
    if upper.contains("IMAP4 ACCESS IS DISABLED") || upper.contains("POP3 ACCESS IS DISABLED") {
        return Some(Diagnosis::ProtocolSwitchedOff);
    }
    None
}

/// Whether `text` contains `code` as a whole enhanced status code.
///
/// `contains` is the version of this that reads `5.7.1399` as `5.7.139` — my own test caught it,
/// and it is the third appearance in this codebase of one mistake: a substring match where a
/// boundary was meant. `ends_with` on a hostname treated `evil-office365.com` as Microsoft, and
/// `contains("io")` matched every error mentioning a connection.
///
/// A code is bounded by anything that is not a digit or a dot, which is how every provider
/// writes them: `535 5.7.139 Authentication unsuccessful`.
fn has_code(text: &str, code: &str) -> bool {
    let bounded = |c: Option<char>| !c.is_some_and(|c| c.is_ascii_digit() || c == '.');
    let mut from = 0;
    while let Some(at) = text[from..].find(code) {
        let start = from + at;
        let end = start + code.len();
        if bounded(text[..start].chars().next_back()) && bounded(text[end..].chars().next()) {
            return true;
        }
        from = start + 1;
    }
    false
}
