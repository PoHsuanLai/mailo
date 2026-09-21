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

/// What a refusal means, when it is one we recognise.
///
/// `None` is the common case and the safe one: the server's own words are shown unchanged.
pub fn explain(error: &ProtoError) -> Option<&'static str> {
    let text = match error {
        ProtoError::AuthRejected(text) => text,
        ProtoError::Refused { text, .. } => text,
        _ => return None,
    };
    explain_text(text)
}

/// The same, for a reply that was not turned into a `ProtoError`.
pub fn explain_text(text: &str) -> Option<&'static str> {
    let upper = text.to_uppercase();

    // Microsoft publishes these as enhanced status codes. `5.7.139` in particular is the one a
    // work or school mailbox hits, and it is a tenant setting rather than anything about the
    // account: `Set-CASMailbox -SmtpClientAuthenticationDisabled $false` is what changes it.
    if has_code(&upper, "5.7.139") {
        return Some(
            "the tenant has SMTP client authentication switched off. An administrator enables it \
             per mailbox with Set-CASMailbox -SmtpClientAuthenticationDisabled $false. Nothing \
             about the password is wrong.",
        );
    }
    if has_code(&upper, "5.7.3") && upper.contains("STARTTLS") {
        return Some("the server requires STARTTLS before authenticating.");
    }
    // Google's two: an account with 2FA needs an App Password, and one without it can no longer
    // use the account password at all.
    if has_code(&upper, "5.7.9") || upper.contains("APPLICATION-SPECIFIC PASSWORD") {
        return Some(
            "this account has two-factor authentication, so it needs an App Password rather than \
             the account password.",
        );
    }
    if has_code(&upper, "5.7.8") && upper.contains("USERNAME AND PASSWORD NOT ACCEPTED") {
        return Some(
            "Google rejected the credential. Account passwords no longer work for IMAP or SMTP; \
             an App Password does.",
        );
    }
    // Exchange Online and Dovecot both say this when the protocol is disabled for the mailbox,
    // which is not the same as the credential being wrong and is fixed by someone else.
    if upper.contains("IMAP4 ACCESS IS DISABLED") || upper.contains("POP3 ACCESS IS DISABLED") {
        return Some(
            "the protocol is switched off for this mailbox. An administrator enables it; the \
             credential is not the problem.",
        );
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
