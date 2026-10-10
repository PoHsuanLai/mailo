//! An invitation answered, in words: the command line prints it, the window shows it.

use mail_core::invite::Answered;
use mail_domain::Attendance;

/// What `mailo invite <message-id> accept|tentative|decline` prints once the answer is queued.
pub fn said(answered: &Answered) -> String {
    format!(
        "queued your answer ({}) to {}\n\ndeliver it with: mailo sync\n",
        word(answered.attendance),
        answered.to.join(", ")
    )
}

pub(crate) fn word(attendance: Attendance) -> &'static str {
    match attendance {
        Attendance::Accepted => "accepted",
        Attendance::Tentative => "tentative",
        Attendance::Declined => "declined",
    }
}
