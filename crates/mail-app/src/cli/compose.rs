//! What the compose commands say: the draft they left behind and the command that finishes it.
//!
//! The rules live in [`mail_core::compose`], which hands back the draft and what was queued;
//! the words, and the `mailo` commands they point at, are this front-end's.

use chrono::{Local, TimeZone};
use mail_core::compose::{AttachedFile, Composed, Scheduled, SignatureChange};
use mail_core::when::Stamp;
use mail_domain::{Address, Draft, OpenPgp, ReceiptRequest, SendState, Smime};
use std::fmt::Write as _;

fn addresses(list: &[Address]) -> String {
    Address::join(list)
}

/// A new message, as `mailo compose` reports it.
pub fn composed(composed: &Composed) -> String {
    let draft = &composed.draft;
    let mut out = format!("draft {}\n", draft.id);
    let _ = writeln!(out, "  to      {}", addresses(&draft.to));
    if !draft.cc.is_empty() {
        let _ = writeln!(out, "  cc      {}", addresses(&draft.cc));
    }
    if !draft.bcc.is_empty() {
        let _ = writeln!(out, "  bcc     {}", addresses(&draft.bcc));
    }
    let _ = writeln!(
        out,
        "  subject {}",
        if draft.subject.is_empty() {
            "(none)"
        } else {
            &draft.subject
        }
    );
    if draft.receipt == ReceiptRequest::Requested {
        let _ = writeln!(out, "  asks for a read receipt");
    }
    match draft.openpgp {
        OpenPgp::None => {}
        OpenPgp::Sign => {
            let _ = writeln!(out, "  signed with OpenPGP when it is sent");
        }
        OpenPgp::Encrypt => {
            let _ = writeln!(out, "  encrypted with OpenPGP when it is sent");
        }
        OpenPgp::SignAndEncrypt => {
            let _ = writeln!(out, "  signed and encrypted with OpenPGP when it is sent");
        }
    }
    match draft.smime {
        Smime::None => {}
        Smime::Sign => {
            let _ = writeln!(out, "  signed with S/MIME when it is sent");
        }
        Smime::Encrypt => {
            let _ = writeln!(out, "  encrypted with S/MIME when it is sent");
        }
        Smime::SignAndEncrypt => {
            let _ = writeln!(out, "  signed and encrypted with S/MIME when it is sent");
        }
    }
    if let Some(why) = &composed.refused {
        let _ = writeln!(
            out,
            "  but it cannot be sent that way yet: {}",
            super::remedy::told(why)
        );
    }
    let _ = writeln!(out, "\nsend it with: mailo send {}", draft.id);
    out
}

/// A reply, as `mailo reply` reports it.
pub fn replied(draft: &Draft) -> String {
    let mut out = format!("draft {}\n", draft.id);
    let _ = writeln!(out, "  to      {}", addresses(&draft.to));
    if !draft.cc.is_empty() {
        let _ = writeln!(out, "  cc      {}", addresses(&draft.cc));
    }
    let _ = writeln!(out, "  subject {}", draft.subject);
    if draft.to.is_empty() && draft.cc.is_empty() {
        // `Draft::reply_to` drops your own address from the recipients, which is right — a reply
        // to something you sent has nobody left to go to. Saying "send it with: …" anyway meant
        // the next command failed with "cannot build a message with no recipients", and the CLI
        // has no way to add one, so the advice was not merely useless but unfollowable.
        let _ = writeln!(
            out,
            "\nnobody to send this to: the only address on the original was your own. \
             Open it in the composer to add a recipient."
        );
    } else {
        let _ = writeln!(out, "\nsend it with: mailo send {}", draft.id);
    }
    out
}

/// A forward, as `mailo forward` reports it.
pub fn forwarded(draft: &Draft) -> String {
    let mut out = format!("draft {}\n", draft.id);
    let _ = writeln!(out, "  to      {}", addresses(&draft.to));
    let _ = writeln!(out, "  subject {}", draft.subject);
    for attachment in &draft.attachments {
        let _ = writeln!(out, "  attached {}", attachment.name);
    }
    let _ = writeln!(out, "\nsend it with: mailo send {}", draft.id);
    out
}

/// A send that was queued, as `mailo send` reports it.
pub fn queued(draft: &Draft, post: &mail_mime::Posting) -> String {
    let mut out = format!("queued {} for delivery\n", draft.id);
    let _ = writeln!(out, "  from    {}", post.mail_from);
    let _ = writeln!(out, "  to      {}", post.rcpt_to.join(", "));
    let _ = writeln!(out, "  subject {}", draft.subject);
    let _ = writeln!(out, "\ndeliver it with: mailo sync");
    out
}

/// A send held for later, as `mailo send --at` reports it, the time written in `zone`.
pub fn scheduled<Tz: TimeZone>(held: &Scheduled, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let Scheduled { draft, post, at } = held;
    let when = mail_core::when::stamp(*at, zone, Stamp::Full);
    let mut out = format!("{} will leave at {when}\n", draft.id);
    let _ = writeln!(out, "  from    {}", post.mail_from);
    let _ = writeln!(out, "  to      {}", post.rcpt_to.join(", "));
    let _ = writeln!(out, "  subject {}", draft.subject);
    let _ = writeln!(
        out,
        "\nit goes with the first `mailo sync` after then, or on the minute from a running \
         `mailo watch`.\ntake it back before then with: mailo unsend {}",
        draft.id
    );
    out
}

/// A send taken back, as `mailo unsend` reports it.
pub fn unsent(back: &Draft) -> String {
    format!(
        "{} is a draft again and will not be sent.\nsend it with: mailo send {}\n",
        back.id, back.id
    )
}

/// Every draft and where it got to, as `mailo drafts` lists them.
pub fn drafts(drafts: &[Draft]) -> String {
    drafts_in(drafts, &Local)
}

/// The same, with the zone a scheduled send's time is written in named.
pub fn drafts_in<Tz: TimeZone>(drafts: &[Draft], zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let mut out = String::new();
    for draft in drafts {
        let _ = write!(
            out,
            "{}  {:<9}  {}",
            draft.id,
            state_word(&draft.state),
            if draft.subject.is_empty() {
                "(no subject)"
            } else {
                &draft.subject
            }
        );
        if let SendState::Scheduled { at } = draft.state {
            let _ = write!(
                out,
                "  (leaves {})",
                mail_core::when::stamp(at, zone, Stamp::Full)
            );
        }
        out.push('\n');
    }
    if out.is_empty() {
        out.push_str("no drafts.\n");
    }
    out
}

fn state_word(state: &SendState) -> &'static str {
    match state {
        SendState::Editing => "editing",
        SendState::Queued => "queued",
        SendState::Scheduled { .. } => "scheduled",
        SendState::Sending => "sending",
        SendState::Failed { .. } => "failed",
        SendState::Sent { .. } => "sent",
    }
}

/// What a draft is carrying, as `mailo attached` lists it.
pub fn attached(files: &[AttachedFile]) -> String {
    if files.is_empty() {
        return "nothing attached to that draft\n".to_owned();
    }
    let mut out = String::new();
    for (index, file) in files.iter().enumerate() {
        let size = file
            .size
            .map(mail_core::attach::human_size)
            .unwrap_or_else(|| "missing".to_owned());
        let _ = writeln!(out, "  {index}  {:>9}  {}  {}", size, file.mime, file.name);
    }
    out
}

/// A signature set or cleared, as `mailo signature` reports it.
pub fn signature(change: &SignatureChange) -> String {
    if change.set {
        format!("signature set for {}\n", change.email)
    } else {
        format!("signature cleared for {}\n", change.email)
    }
}
