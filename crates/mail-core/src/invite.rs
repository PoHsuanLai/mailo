//! Calendar invitations from the reader's side: what a message invites the user to, answering
//! it, and saving it for a calendar.
//!
//! The mail half of scheduling only. The invitation is read from the message's bytes each time
//! (`mail-mime` finds the calendar part, `mail-pim` reads it); the answer goes to the organiser
//! as an iTIP `REPLY` through the outbox like any other message, from the account the invitation
//! came to and as the identity it named; and what was answered is kept beside the message so
//! the reader can show it. Nothing is answered unless the user asks — an automatic "accepted"
//! would tell every sender that this mailbox reads its mail.

use crate::error::CoreError;
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_pim::ical::{self, Answering};
use mail_pim::{Invite, Kind, Me};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use std::fmt::Write as _;

mod when;

pub use when::{REPEATS_OTHERWISE, WhenShown, repeats_words, show_when};

/// An answer that is queued: what was answered, and who it goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answered {
    pub attendance: Attendance,
    /// The envelope recipients of the queued reply: the organiser.
    pub to: Vec<String>,
}

/// The invitation `raw` carries, as the reader shows it to someone whose addresses are `me`.
///
/// `None` when the message carries no calendar object, or one that is not an invitation to show.
/// Pure: the window may call it on bytes it already holds.
pub fn invite_of(raw: &[u8], me: &[&str]) -> Option<Invite> {
    let part = mail_mime::calendar_part(raw)?;
    let calendar = ical::parse(&part.text).ok()?;
    mail_pim::summarise(&calendar, me)
}

/// Where a message stands as an invitation, for the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteState {
    /// It carries no invitation.
    NotInvite,
    /// Only its headers are here; whether it invites is known once the body arrives.
    Unknown,
    /// It invites, cancels, answers or publishes an event. `answered` is what the user last
    /// answered it on this machine.
    Shown {
        invite: Box<Invite>,
        answered: Option<InviteAnswer>,
    },
}

/// Where `message` stands, read from its stored raw bytes and the account's identities.
pub fn state(store: &SqliteStore, message: &Message) -> Result<InviteState, CoreError> {
    let Some(bytes) = raw_of(store, message)? else {
        return Ok(InviteState::Unknown);
    };
    let me = addresses(store, message.account.clone());
    let me: Vec<&str> = me.iter().map(|(_, address)| address.as_str()).collect();
    let Some(invite) = invite_of(&bytes, &me) else {
        return Ok(InviteState::NotInvite);
    };
    let answered = store.invite_answer(message.id)?;
    Ok(InviteState::Shown {
        invite: Box::new(invite),
        answered,
    })
}

/// Answer the invitation in `message`: queue the iTIP reply to its organiser, and record the
/// answer. Answering again sends a new reply, which supersedes the last, and replaces the record.
///
/// The reply leaves from the account the invitation came to, as the identity whose address the
/// invitation lists — the one the organiser's calendar will match the answer to — through the
/// outbox, so nothing here touches the network; the next sync delivers it.
pub fn answer(
    store: &SqliteStore,
    message: MessageId,
    attendance: Attendance,
    comment: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Answered, CoreError> {
    let original = store.message(message)?;
    let bytes = raw_of(store, &original)?.ok_or(CoreError::HeadersOnly)?;
    let part = mail_mime::calendar_part(&bytes).ok_or(CoreError::NoCalendar)?;
    let calendar = ical::parse(&part.text).map_err(|e| CoreError::context("the invitation", e))?;
    let mine = addresses(store, original.account.clone());
    let me: Vec<&str> = mine.iter().map(|(_, address)| address.as_str()).collect();
    let invite = mail_pim::summarise(&calendar, &me).ok_or(CoreError::NoInvitation)?;

    match invite.kind {
        Kind::Request(_) => {}
        Kind::Cancelled => {
            return Err(CoreError::EventCancelled);
        }
        Kind::Reply => {
            return Err(CoreError::IsAnAnswer);
        }
        Kind::Published => {
            return Err(CoreError::PublishedEvent);
        }
    }
    let address = match &invite.me {
        Me::Invited { address, .. } => address.clone(),
        Me::Organiser => return Err(CoreError::YouOrganised),
        Me::NotListed => return Err(CoreError::NotInvited),
    };
    let organiser = invite.organiser.as_ref().ok_or(CoreError::NoOrganiser)?;
    let event = calendar.main_event().ok_or(CoreError::NoInvitation)?;

    let identity_id = mine
        .iter()
        .find(|(_, mail)| mail.eq_ignore_ascii_case(&address))
        .map(|(id, _)| *id);
    let identity = crate::compose::identity_of(store, original.account.clone(), identity_id)?;
    let comment = comment.map(str::trim).filter(|c| !c.is_empty());
    let calendar_text = ical::reply(
        &calendar,
        event,
        &Answering {
            attendee: &address,
            attendance,
            comment,
            at: now,
            product: &format!("-//mailo//mailo {}//EN", env!("CARGO_PKG_VERSION")),
        },
    )?;

    let title = invite.title.as_deref().unwrap_or("(no title)");
    let subject = format!("{}: {title}", verb(attendance));
    let text = human_text(&invite, &identity.from, attendance, comment);
    let to = Address {
        name: organiser.name.clone(),
        email: organiser.email.clone(),
    };
    let id = DraftId::generate();
    let post = mail_mime::calendar_reply(
        &bytes,
        &mail_mime::CalendarReply {
            from: &identity.from,
            to: &to,
            subject: &subject,
            text: &text,
            calendar: &calendar_text,
            id,
            at: now,
        },
    )?;
    let frozen = store.blobs().put(&post.message)?;
    let queued = store.enqueue(
        original.account,
        // Named like a draft because the outbox keys a submission by one; no draft row
        // exists, which the drain already treats as "nothing to mark", as for a receipt.
        RemoteIntent::Send {
            draft: id,
            raw: frozen,
            mail_from: post.mail_from.clone(),
            rcpt_to: post.rcpt_to.clone(),
        },
        // A reply that left cannot be taken back by a patch; answering again is how.
        &Patch {
            id: ChangeId::generate(),
            changes: Vec::new(),
        },
        now,
    )?;
    if queued.is_none() {
        return Err(CoreError::AnswerNotQueued);
    }
    store.answer_invite(&InviteAnswer {
        message,
        attendance,
        sequence: invite.sequence,
        comment: comment.map(str::to_owned),
        answered_at: now,
    })?;
    Ok(Answered {
        attendance,
        to: post.rcpt_to,
    })
}

/// The calendar object in `message`, as it arrived, for saving as an `.ics` file any calendar
/// can import.
pub fn export(store: &SqliteStore, message: MessageId) -> Result<Vec<u8>, CoreError> {
    let original = store.message(message)?;
    let bytes = raw_of(store, &original)?.ok_or(CoreError::HeadersOnly)?;
    let part = mail_mime::calendar_part(&bytes).ok_or(CoreError::NoCalendar)?;
    let mut text = part.text;
    // The line break before a MIME boundary belongs to the boundary, so a part's last content
    // line arrives without its own; a file ends with one (RFC 5545 §3.1).
    if !text.ends_with('\n') {
        text.push_str("\r\n");
    }
    Ok(text.into_bytes())
}

/// The message's stored raw bytes, or `None` when only its headers are here.
fn raw_of(store: &SqliteStore, message: &Message) -> Result<Option<Vec<u8>>, CoreError> {
    let Some(raw) = message.body.raw() else {
        return Ok(None);
    };
    Ok(store.blobs().get(raw).map(Some)?)
}

/// Every address this account answers to — each identity's own and its reply-to — with the
/// identity it belongs to. An invitation sent to any of them is to the user.
pub fn addresses(store: &SqliteStore, account: AccountId) -> Vec<(IdentityId, String)> {
    let mut out = Vec::new();
    for identity in store.identities(account).unwrap_or_default() {
        out.push((identity.id, identity.from.email));
        if let Some(reply_to) = identity.reply_to {
            out.push((identity.id, reply_to.email));
        }
    }
    out
}

/// The part a person reads in the answer: who answered what, when, and their note.
fn human_text(
    invite: &Invite,
    from: &Address,
    attendance: Attendance,
    comment: Option<&str>,
) -> String {
    let who = from.name.as_deref().unwrap_or(&from.email);
    let title = invite.title.as_deref().unwrap_or("(no title)");
    let when = show_when(&invite.when, &Utc);
    let mut out = format!(
        "{who} has {} the invitation to \"{}\".\r\n",
        match attendance {
            Attendance::Accepted => "accepted",
            Attendance::Tentative => "tentatively accepted",
            Attendance::Declined => "declined",
        },
        one_line(title)
    );
    let _ = write!(out, "\r\nWhen: {}\r\n", when.theirs.unwrap_or(when.yours));
    if let Some(comment) = comment {
        let _ = write!(out, "\r\n{}\r\n", comment.replace('\n', "\r\n"));
    }
    out
}

/// The word calendars put before an answer's title.
fn verb(attendance: Attendance) -> &'static str {
    match attendance {
        Attendance::Accepted => "Accepted",
        Attendance::Tentative => "Tentative",
        Attendance::Declined => "Declined",
    }
}

/// A value from a stranger's calendar, cut at its first line break so it cannot draw lines of
/// its own in the terminal.
fn one_line(value: &str) -> String {
    value.chars().take_while(|c| !c.is_control()).collect()
}
