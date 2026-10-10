//! `mailo invite`: the words typed after it, as the command [`mail_core::invite`] runs.

use chrono::{DateTime, Local, TimeZone, Utc};
use mail_core::error::CoreError;
use mail_core::invite::{Answered, InviteState, repeats_words, show_when};
use mail_domain::{Address, Attendance, InviteAnswer, MessageId};
use mail_pim::ical::PartStat;
use mail_pim::{Invite, Kind, Me, Revision};
use std::fmt::Write as _;
use std::path::PathBuf;

use super::{SqliteStore, Store};

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
    let usage = || super::usage();
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
        let _ = writeln!(out, "  repeats:   {}", repeats_words(repeats));
    }
    if let Some(location) = &invite.location {
        let _ = writeln!(out, "  where:     {}", one_line(location));
    }
    if let Some(organiser) = &invite.organiser {
        let _ = writeln!(out, "  organiser: {}", party(organiser));
    }
    if !invite.attendees.is_empty() {
        let _ = writeln!(out, "  attendees:");
        for attendee in &invite.attendees {
            let _ = writeln!(
                out,
                "    {}  {}",
                party(&attendee.party),
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

/// What `mailo invite <message-id> accept|tentative|decline` prints once the answer is queued.
pub fn said(answered: &Answered) -> String {
    format!(
        "queued your answer ({}) to {}\n\ndeliver it with: mailo sync\n",
        word(answered.attendance),
        answered.to.join(", ")
    )
}

/// Run `mailo invite …`.
pub fn run(
    store: &SqliteStore,
    command: &InviteCommand,
    now: DateTime<Utc>,
) -> Result<String, CoreError> {
    match &command.action {
        InviteAction::Show => {
            let message = store.message(command.message)?;
            match mail_core::invite::state(store, &message)? {
                InviteState::Shown { invite, answered } => {
                    Ok(render(&invite, answered.as_ref(), &Local))
                }
                InviteState::Unknown => Err(CoreError::HeadersOnly),
                InviteState::NotInvite => Err(CoreError::NoCalendar),
            }
        }
        InviteAction::Answer {
            attendance,
            comment,
        } => {
            let answered = mail_core::invite::answer(
                store,
                command.message,
                *attendance,
                comment.as_deref(),
                now,
            )?;
            Ok(said(&answered))
        }
        InviteAction::Ics(path) => {
            let bytes = mail_core::invite::export(store, command.message)?;
            std::fs::write(path, &bytes)
                .map_err(|e| CoreError::cannot(format!("write {}", path.display()), e))?;
            Ok(format!("wrote {}\n", path.display()))
        }
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

fn party(who: &mail_pim::ical::Party) -> String {
    match &who.name {
        Some(name) => Address::named(one_line(name), &who.email).to_string(),
        None => who.email.clone(),
    }
}

/// A value from a stranger's calendar, cut at its first line break so it cannot draw lines of
/// its own in the terminal.
fn one_line(value: &str) -> String {
    value.chars().take_while(|c| !c.is_control()).collect()
}
