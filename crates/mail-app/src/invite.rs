//! Calendar invitations from the reader's side: what a message invites the user to, answering
//! it, and saving it for a calendar.
//!
//! The mail half of scheduling only. The invitation is read from the message's bytes each time
//! (`mail-mime` finds the calendar part, `mail-pim` reads it); the answer goes to the organiser
//! as an iTIP `REPLY` through the outbox like any other message, from the account the invitation
//! came to and as the identity it named; and what was answered is kept beside the message so
//! the reader can show it. Nothing is answered unless the user asks — an automatic "accepted"
//! would tell every sender that this mailbox reads its mail.

use chrono::{DateTime, Local, TimeZone, Utc};
use mail_domain::*;
use mail_pim::ical::{self, Answering, PartStat};
use mail_pim::{Invite, Kind, Me, Revision, show_when};
use mail_store::{SqliteStore, Store};
use std::fmt::Write as _;
use std::path::PathBuf;

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
pub fn state(store: &SqliteStore, message: &Message) -> Result<InviteState, String> {
    let Some(bytes) = raw_of(store, message)? else {
        return Ok(InviteState::Unknown);
    };
    let me = addresses(store, message.account);
    let me: Vec<&str> = me.iter().map(|(_, address)| address.as_str()).collect();
    let Some(invite) = invite_of(&bytes, &me) else {
        return Ok(InviteState::NotInvite);
    };
    let answered = store.invite_answer(message.id).map_err(|e| e.to_string())?;
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
) -> Result<String, String> {
    let original = store.message(message).map_err(|e| e.to_string())?;
    let bytes = raw_of(store, &original)?.ok_or_else(|| {
        "only that message's headers are here yet; run mailo sync and try again".to_owned()
    })?;
    let part = mail_mime::calendar_part(&bytes)
        .ok_or_else(|| "that message carries no calendar invitation".to_owned())?;
    let calendar = ical::parse(&part.text).map_err(|e| format!("the invitation: {e}"))?;
    let mine = addresses(store, original.account);
    let me: Vec<&str> = mine.iter().map(|(_, address)| address.as_str()).collect();
    let invite = mail_pim::summarise(&calendar, &me)
        .ok_or_else(|| "that message carries no invitation to answer".to_owned())?;

    match invite.kind {
        Kind::Request(_) => {}
        Kind::Cancelled => {
            return Err("that event was cancelled; there is nothing to answer".into());
        }
        Kind::Reply => {
            return Err(
                "that message is someone's answer to an invitation, not an invitation".into(),
            );
        }
        Kind::Published => {
            return Err(
                "that event was published to be added to a calendar and asks for no \
                        answer; save it with: mailo invite <message-id> --ics FILE"
                    .into(),
            );
        }
    }
    let address = match &invite.me {
        Me::Invited { address, .. } => address.clone(),
        Me::Organiser => return Err("you organised that event".into()),
        Me::NotListed => {
            return Err("none of this account's addresses is among that event's attendees".into());
        }
    };
    let organiser = invite
        .organiser
        .as_ref()
        .ok_or_else(|| "the invitation names no organiser to answer".to_owned())?;
    let event = calendar
        .main_event()
        .ok_or_else(|| "that message carries no invitation to answer".to_owned())?;

    let identity_id = mine
        .iter()
        .find(|(_, mail)| mail.eq_ignore_ascii_case(&address))
        .map(|(id, _)| *id);
    let identity = crate::compose::identity_of(store, original.account, identity_id)?;
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
    )
    .map_err(|e| e.to_string())?;

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
    )
    .map_err(|e| e.to_string())?;
    let frozen = store
        .blobs()
        .put(&store.connection(), &post.message)
        .map_err(|e| e.to_string())?;
    let queued = store
        .enqueue(
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
        )
        .map_err(|e| e.to_string())?;
    if queued.is_none() {
        return Err("the answer could not be queued".to_owned());
    }
    store
        .answer_invite(&InviteAnswer {
            message,
            attendance,
            sequence: invite.sequence,
            comment: comment.map(str::to_owned),
            answered_at: now,
        })
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "queued your answer ({}) to {}\n\ndeliver it with: mailo sync\n",
        word(attendance),
        post.rcpt_to.join(", ")
    ))
}

/// The calendar object in `message`, as it arrived, for saving as an `.ics` file any calendar
/// can import.
pub fn export(store: &SqliteStore, message: MessageId) -> Result<Vec<u8>, String> {
    let original = store.message(message).map_err(|e| e.to_string())?;
    let bytes = raw_of(store, &original)?.ok_or_else(|| {
        "only that message's headers are here yet; run mailo sync and try again".to_owned()
    })?;
    let part = mail_mime::calendar_part(&bytes)
        .ok_or_else(|| "that message carries no calendar invitation".to_owned())?;
    let mut text = part.text;
    // The line break before a MIME boundary belongs to the boundary, so a part's last content
    // line arrives without its own; a file ends with one (RFC 5545 §3.1).
    if !text.ends_with('\n') {
        text.push_str("\r\n");
    }
    Ok(text.into_bytes())
}

/// The lines `mailo show` prints under a message that carries an invitation, or nothing.
pub fn describe(state: &InviteState, message: MessageId) -> String {
    let InviteState::Shown { invite, answered } = state else {
        return String::new();
    };
    let title = invite.title.as_deref().unwrap_or("(no title)");
    let when = show_when(&invite.when, &Local).yours;
    let head = match invite.kind {
        Kind::Request(Revision::First) => "invitation",
        Kind::Request(Revision::Update { .. }) => "updated invitation",
        Kind::Cancelled => "cancelled",
        Kind::Reply => "answer to your invitation",
        Kind::Published => "event",
    };
    let mut out = format!("    {head}: {title}, {when}\n");
    if let Some(answered) = answered {
        let _ = writeln!(out, "    you answered: {}", word(answered.attendance));
    }
    let _ = writeln!(out, "    see it with: mailo invite {message}");
    out
}

/// Everything `mailo invite <message-id>` prints, with times in `zone`.
pub fn render<Z: TimeZone>(invite: &Invite, answered: Option<&InviteAnswer>, zone: &Z) -> String {
    let mut out = String::new();
    let title = invite.title.as_deref().unwrap_or("(no title)");
    let _ = writeln!(out, "{title}");
    let status = match invite.kind {
        Kind::Request(Revision::First) => "an invitation".to_owned(),
        Kind::Request(Revision::Update { sequence }) => {
            format!("an updated invitation (revision {sequence})")
        }
        Kind::Cancelled => "CANCELLED: this event will not take place".to_owned(),
        Kind::Reply => "an answer to an invitation you sent".to_owned(),
        Kind::Published => "an event to add to a calendar; it asks for no answer".to_owned(),
    };
    let _ = writeln!(out, "  {status}");
    if invite.recurrence_id.is_some() {
        let _ = writeln!(out, "  about one occurrence of a repeating event");
    }
    let when = show_when(&invite.when, zone);
    let _ = writeln!(out, "  when:      {}", when.yours);
    if let Some(theirs) = &when.theirs {
        let _ = writeln!(out, "             {theirs}, the organiser's time");
    }
    if let Some(repeats) = &invite.repeats {
        let _ = writeln!(out, "  repeats:   {repeats}");
    }
    if let Some(location) = &invite.location {
        let _ = writeln!(out, "  where:     {}", one_line(location));
    }
    if let Some(organiser) = &invite.organiser {
        let _ = writeln!(
            out,
            "  organiser: {}",
            party(&organiser.name, &organiser.email)
        );
    }
    if !invite.attendees.is_empty() {
        let _ = writeln!(out, "  attendees:");
        for attendee in &invite.attendees {
            let _ = writeln!(
                out,
                "    {}  {}",
                party(&attendee.party.name, &attendee.party.email),
                partstat_word(attendee.answer)
            );
        }
    }
    if let Some(comment) = &invite.comment {
        let _ = writeln!(out, "  comment:   {}", one_line(comment));
    }
    match (&invite.me, answered) {
        (_, Some(answered)) => {
            let _ = write!(out, "\nyou answered: {}", word(answered.attendance));
            if answered.sequence < invite.sequence {
                out.push_str(" (to an earlier version)");
            }
            out.push('\n');
        }
        (Me::Invited { answer, .. }, None) if matches!(invite.kind, Kind::Request(_)) => {
            let _ = writeln!(out, "\nyour answer: {}", partstat_word(*answer));
        }
        _ => {}
    }
    if matches!(invite.kind, Kind::Request(_)) && matches!(invite.me, Me::Invited { .. }) {
        out.push_str(
            "answer with: mailo invite <message-id> accept|tentative|decline [--comment TEXT]\n",
        );
    }
    if let Some(description) = &invite.description {
        let _ = write!(out, "\n{}\n", description.trim_end());
    }
    out
}

/// `mailo invite …`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteCommand {
    pub message: MessageId,
    pub action: InviteAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteAction {
    /// Show the invitation.
    Show,
    /// Answer it.
    Answer {
        attendance: Attendance,
        comment: Option<String>,
    },
    /// Write its calendar object to a file.
    Ics(PathBuf),
}

/// The words after `invite`.
pub fn parse(args: &[String]) -> Result<InviteCommand, String> {
    let usage = || crate::cli::usage();
    let raw = args
        .first()
        .ok_or_else(|| format!("invite needs a message id\n\n{}", usage()))?;
    let uuid = raw
        .parse()
        .map_err(|_| format!("{raw:?} is not a message id"))?;
    let message = MessageId::from_uuid(uuid);
    let rest = args.get(1..).unwrap_or_default();
    let action = match rest.first().map(String::as_str) {
        None => InviteAction::Show,
        Some("--ics") => match rest {
            [_, path] => InviteAction::Ics(PathBuf::from(path)),
            [_] => return Err(format!("--ics needs a file to write\n\n{}", usage())),
            [_, _, extra, ..] => return Err(format!("unexpected {extra:?}\n\n{}", usage())),
            [] => return Err(usage()),
        },
        Some(word) => {
            let attendance = match word {
                "accept" => Attendance::Accepted,
                "tentative" => Attendance::Tentative,
                "decline" => Attendance::Declined,
                other => return Err(format!("unknown answer {other:?}\n\n{}", usage())),
            };
            let comment = match &rest[1..] {
                [] => None,
                [flag, text] if flag == "--comment" => Some(text.clone()),
                [flag] if flag == "--comment" => {
                    return Err(format!("--comment needs the text to send\n\n{}", usage()));
                }
                [extra, ..] => return Err(format!("unexpected {extra:?}\n\n{}", usage())),
            };
            InviteAction::Answer {
                attendance,
                comment,
            }
        }
    };
    Ok(InviteCommand { message, action })
}

/// Run `mailo invite …`.
pub fn run(
    store: &SqliteStore,
    command: &InviteCommand,
    now: DateTime<Utc>,
) -> Result<String, String> {
    match &command.action {
        InviteAction::Show => {
            let message = store.message(command.message).map_err(|e| e.to_string())?;
            match state(store, &message)? {
                InviteState::Shown { invite, answered } => {
                    Ok(render(&invite, answered.as_ref(), &Local))
                }
                InviteState::Unknown => Err(
                    "only that message's headers are here yet; run mailo sync and try again"
                        .to_owned(),
                ),
                InviteState::NotInvite => {
                    Err("that message carries no calendar invitation".to_owned())
                }
            }
        }
        InviteAction::Answer {
            attendance,
            comment,
        } => answer(store, command.message, *attendance, comment.as_deref(), now),
        InviteAction::Ics(path) => {
            let bytes = export(store, command.message)?;
            std::fs::write(path, &bytes)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            Ok(format!("wrote {}\n", path.display()))
        }
    }
}

/// The message's stored raw bytes, or `None` when only its headers are here.
fn raw_of(store: &SqliteStore, message: &Message) -> Result<Option<Vec<u8>>, String> {
    let Some(raw) = message.body.raw() else {
        return Ok(None);
    };
    store
        .blobs()
        .get(&store.connection(), raw)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// Every address this account answers to — each identity's own and its reply-to — with the
/// identity it belongs to. An invitation sent to any of them is to the user.
pub fn addresses(store: &SqliteStore, account: AccountId) -> Vec<(IdentityId, String)> {
    let db = store.connection();
    let Ok(mut stmt) =
        db.prepare("SELECT id, from_email, reply_to FROM identities WHERE account = ?1")
    else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([account.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (id, email, reply_to) in rows.filter_map(Result::ok) {
        let Some(id) = id.parse().ok().map(IdentityId::from_uuid) else {
            continue;
        };
        out.push((id, email));
        let reply_to: Option<Address> = reply_to
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
            .flatten();
        if let Some(reply_to) = reply_to {
            out.push((id, reply_to.email));
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

fn word(attendance: Attendance) -> &'static str {
    match attendance {
        Attendance::Accepted => "accepted",
        Attendance::Tentative => "tentative",
        Attendance::Declined => "declined",
    }
}

fn partstat_word(answer: PartStat) -> &'static str {
    match answer {
        PartStat::NeedsAction => "not answered",
        PartStat::Accepted => "accepted",
        PartStat::Declined => "declined",
        PartStat::Tentative => "tentative",
        PartStat::Delegated => "delegated",
    }
}

fn party(name: &Option<String>, email: &str) -> String {
    match name {
        Some(name) => format!("{} <{email}>", one_line(name)),
        None => email.to_owned(),
    }
}

/// A value from a stranger's calendar, cut at its first line break so it cannot draw lines of
/// its own in the terminal.
fn one_line(value: &str) -> String {
    value.chars().take_while(|c| !c.is_control()).collect()
}
