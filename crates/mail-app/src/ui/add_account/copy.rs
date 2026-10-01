//! What the sheet says when something did not work: a sentence, perhaps a line of detail, and
//! at most one thing to do about it.
//!
//! Every wording lives here, as functions of the typed reason, so that none of it can lean on
//! text written for a terminal and so that one test can read all of it.

use mail_domain::{OAuthIssuer, Retry};

use super::flow::{self, Miss, Refusal, What};
use crate::discover::Gap;

/// What the sheet offers to do about a [`Notice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Action {
    /// Leave looking up and type the server in.
    EnterServerSettings,
    /// Look the address up again.
    TryAgain,
}

impl Action {
    /// The button's label. An ellipsis marks a button that opens more to fill in.
    pub(in crate::ui) fn label(self) -> &'static str {
        match self {
            Action::EnterServerSettings => "Enter Server Settings\u{2026}",
            Action::TryAgain => "Try Again",
        }
    }
}

/// A failure as the sheet says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Notice {
    /// One sentence, saying what happened.
    pub headline: String,
    /// What else the person needs, said plainly; nothing when the headline is enough.
    pub detail: Option<String>,
    pub action: Option<Action>,
}

fn notice(headline: &str, detail: Option<String>, action: Option<Action>) -> Notice {
    Notice {
        headline: headline.to_owned(),
        detail,
        action,
    }
}

/// What to say of a lookup that found nothing.
pub(in crate::ui) fn missed(miss: &Miss) -> Notice {
    match miss {
        Miss::NotAnAddress => notice(
            "Enter a full email address, like ada@example.com.",
            None,
            None,
        ),
        Miss::NoServers { domain, gap } => {
            let headline = format!("Mailo couldn\u{2019}t find the mail servers for {domain}.");
            match gap {
                Gap::Nothing => Notice {
                    headline,
                    detail: None,
                    action: Some(Action::EnterServerSettings),
                },
                Gap::StartTlsOnly => Notice {
                    headline,
                    detail: Some(
                        "This provider only offers connections Mailo considers unsafe \
                         (STARTTLS). Ask them for IMAP on port 993 and SMTP on port 465."
                            .to_owned(),
                    ),
                    action: Some(Action::EnterServerSettings),
                },
                Gap::PersonalMicrosoft => Notice {
                    headline: format!(
                        "Microsoft no longer lets apps sign in to personal accounts like \
                         {domain}."
                    ),
                    detail: Some(
                        "Mailo works with Microsoft 365 work and school accounts.".to_owned(),
                    ),
                    action: None,
                },
            }
        }
        Miss::Unreachable { domain, retry, .. } => Notice {
            headline: format!("Mailo couldn\u{2019}t reach the internet to look up {domain}."),
            detail: None,
            action: match retry {
                Retry::Now | Retry::After(_) => Some(Action::TryAgain),
                Retry::NeedsReauth | Retry::Fatal(_) => Some(Action::EnterServerSettings),
            },
        },
        Miss::Broken(why) => Notice {
            headline: "Mailo couldn\u{2019}t look up the mail servers.".to_owned(),
            detail: plain(why),
            action: Some(Action::TryAgain),
        },
    }
}

/// What to say of an add that did not go ahead.
pub(in crate::ui) fn refused(refusal: &Refusal) -> Notice {
    match refusal {
        Refusal::Blank(What::Password) => notice("Type the password first.", None, None),
        Refusal::Blank(What::Token) => notice("Type the token first.", None, None),
        Refusal::NeedsClientId(issuer) => needs_client_id(*issuer),
        Refusal::Other(why) => notice("Couldn\u{2019}t add the account.", plain(why), None),
    }
}

/// Signing in with `issuer`, when this build has no client id to do it with.
pub(in crate::ui) fn needs_client_id(issuer: OAuthIssuer) -> Notice {
    let set = match issuer {
        OAuthIssuer::Google => "MAILO_OAUTH_CLIENT_ID and MAILO_OAUTH_CLIENT_SECRET",
        OAuthIssuer::Microsoft => "MAILO_OAUTH_CLIENT_ID",
    };
    Notice {
        headline: format!(
            "Signing in with {} isn\u{2019}t set up in this build of Mailo.",
            flow::provider(issuer)
        ),
        detail: Some(format!(
            "Use an app password with IMAP instead, or start Mailo with {set} set."
        )),
        action: Some(Action::EnterServerSettings),
    }
}

/// `text` without the lines that tell a person at a terminal what to type or run: what is left
/// is a reason, or nothing.
fn plain(text: &str) -> Option<String> {
    let kept: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| {
            let lower = line.to_lowercase();
            !lower.starts_with("mailo ")
                && !lower.contains("terminal")
                && !lower.contains("mailo account")
                && !lower.contains(" --")
        })
        .collect();
    (!kept.is_empty()).then(|| kept.join(" "))
}
