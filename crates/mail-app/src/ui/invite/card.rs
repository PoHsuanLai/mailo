//! What the invitation card says, worked out from the invitation. Pure, so every wording is a
//! table test away, and the time zone is an argument so a test can name one.

use chrono::TimeZone;
use mail_domain::{Attendance, InviteAnswer, MessageId};
use mail_pim::ical::{PartStat, Party};
use mail_pim::{Invite, Kind, Me, Revision, show_when};

/// How many attendees the card names before folding the rest into "+N".
pub(in crate::ui) const CHIPS: usize = 6;

/// The word at the card's head, which says what the message asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Tag {
    Invitation,
    Updated,
    Cancelled,
    Answer,
    Event,
}

impl Tag {
    pub(in crate::ui) fn word(self) -> &'static str {
        match self {
            Tag::Invitation => "Invitation",
            Tag::Updated => "Updated",
            Tag::Cancelled => "Cancelled",
            Tag::Answer => "Answer",
            Tag::Event => "Event",
        }
    }

    pub(in crate::ui) fn class(self) -> &'static str {
        match self {
            Tag::Invitation | Tag::Event => "inv-tag",
            Tag::Updated => "inv-tag updated",
            Tag::Cancelled => "inv-tag cancelled",
            Tag::Answer => "inv-tag answer",
        }
    }
}

/// One attendee, as a chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Chip {
    pub name: String,
    /// Their answer, in words.
    pub said: &'static str,
    /// The chip's class, which colours its mark by the answer.
    pub class: &'static str,
}

/// Where the reader stands on answering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Stand {
    /// Accept, Maybe and Decline are offered. `before` is an answer given to an earlier
    /// version, or recorded in the invitation, said as a sentence.
    Open { before: Option<String> },
    /// Answered, this version: "You accepted", the note sent with it, and Change answer.
    Answered { said: String, note: Option<String> },
    /// Nothing to answer here, and why when the reader would wonder.
    Closed(Option<&'static str>),
}

/// Everything the card draws for one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Card {
    pub message: MessageId,
    pub tag: Tag,
    pub title: String,
    pub when: String,
    /// The organiser's time, when it reads differently.
    pub theirs: Option<String>,
    /// Said when the message is about one occurrence of a repeating event.
    pub occurrence: Option<&'static str>,
    pub repeats: Option<String>,
    pub location: Option<String>,
    pub organiser: Option<String>,
    pub attendees: Vec<Chip>,
    pub description: Option<String>,
    /// On an answer: who answered what, and the note they sent.
    pub summary: Option<String>,
    pub comment: Option<String>,
    pub stand: Stand,
}

/// The card for `invite` in `message`, with times in `zone`.
pub(in crate::ui) fn card_of<Z: TimeZone>(
    message: MessageId,
    invite: &Invite,
    answered: Option<&InviteAnswer>,
    zone: &Z,
) -> Card {
    let when = show_when(&invite.when, zone);
    let tag = match invite.kind {
        Kind::Request(Revision::First) => Tag::Invitation,
        Kind::Request(Revision::Update { .. }) => Tag::Updated,
        Kind::Cancelled => Tag::Cancelled,
        Kind::Reply => Tag::Answer,
        Kind::Published => Tag::Event,
    };
    let summary = matches!(invite.kind, Kind::Reply).then(|| reply_summary(invite));
    Card {
        message,
        tag,
        title: invite
            .title
            .as_deref()
            .map(one_line)
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "(no title)".to_owned()),
        when: when.yours,
        theirs: when.theirs,
        occurrence: invite
            .recurrence_id
            .is_some()
            .then_some("About one occurrence of a repeating event"),
        repeats: invite.repeats.clone(),
        location: invite.location.as_deref().map(one_line),
        organiser: invite.organiser.as_ref().map(name_of),
        attendees: invite
            .attendees
            .iter()
            .map(|attendee| {
                let (said, class) = partstat(
                    own_answer(invite, answered, &attendee.party.email).unwrap_or(attendee.answer),
                );
                Chip {
                    name: name_of(&attendee.party),
                    said,
                    class,
                }
            })
            .collect(),
        description: invite
            .description
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        summary,
        comment: invite.comment.as_deref().map(one_line),
        stand: stand(invite, answered),
    }
}

fn stand(invite: &Invite, answered: Option<&InviteAnswer>) -> Stand {
    match invite.kind {
        Kind::Request(_) => {}
        Kind::Cancelled => return Stand::Closed(Some("This event will not take place.")),
        Kind::Reply => return Stand::Closed(None),
        Kind::Published => {
            return Stand::Closed(Some(
                "An event to add to a calendar; it asks for no answer.",
            ));
        }
    }
    let recorded = match &invite.me {
        Me::Organiser => {
            return Stand::Closed(Some(
                "You organised this event, so there is nothing to answer.",
            ));
        }
        Me::NotListed => {
            return Stand::Closed(Some(
                "None of this account's addresses is among the attendees, so it cannot be \
                 answered from here.",
            ));
        }
        Me::Invited { answer, .. } => *answer,
    };
    match answered {
        Some(answer) if answer.sequence >= invite.sequence => Stand::Answered {
            said: you(answer.attendance).to_owned(),
            note: answer.comment.clone(),
        },
        Some(answer) => Stand::Open {
            before: Some(format!("{} an earlier version.", you(answer.attendance))),
        },
        None => Stand::Open {
            before: match recorded {
                PartStat::NeedsAction => None,
                other => Some(format!("Your answer on record: {}.", partstat(other).0)),
            },
        },
    }
}

/// "You accepted", and its kin.
fn you(attendance: Attendance) -> &'static str {
    match attendance {
        Attendance::Accepted => "You accepted",
        Attendance::Tentative => "You said maybe",
        Attendance::Declined => "You declined",
    }
}

/// On an answer to the reader's invitation: "Charles accepted".
fn reply_summary(invite: &Invite) -> String {
    let answers: Vec<String> = invite
        .attendees
        .iter()
        .map(|attendee| {
            let verb = match attendee.answer {
                PartStat::Accepted => "accepted",
                PartStat::Declined => "declined",
                PartStat::Tentative => "said maybe",
                PartStat::Delegated => "delegated it",
                PartStat::NeedsAction => "has not answered",
            };
            format!("{} {verb}", name_of(&attendee.party))
        })
        .collect();
    if answers.is_empty() {
        "An answer that names nobody".to_owned()
    } else {
        answers.join(", ")
    }
}

/// What this reader answered, for their own chip: the invitation still carries the PARTSTAT it
/// was sent with, so without this the chip says "not answered" beside "You accepted".
fn own_answer(invite: &Invite, answered: Option<&InviteAnswer>, email: &str) -> Option<PartStat> {
    let Me::Invited { address, .. } = &invite.me else {
        return None;
    };
    let answer = answered.filter(|answer| answer.sequence >= invite.sequence)?;
    address
        .eq_ignore_ascii_case(email)
        .then_some(match answer.attendance {
            Attendance::Accepted => PartStat::Accepted,
            Attendance::Tentative => PartStat::Tentative,
            Attendance::Declined => PartStat::Declined,
        })
}

/// An answer in words, and the class of the chip that carries it.
fn partstat(answer: PartStat) -> (&'static str, &'static str) {
    match answer {
        PartStat::Accepted => ("accepted", "att yes"),
        PartStat::Declined => ("declined", "att no"),
        PartStat::Tentative => ("maybe", "att maybe"),
        PartStat::Delegated => ("delegated", "att waiting"),
        PartStat::NeedsAction => ("not answered", "att waiting"),
    }
}

fn name_of(party: &Party) -> String {
    party
        .name
        .as_deref()
        .map(one_line)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| party.email.clone())
}

/// A value from a stranger's calendar, cut at its first control character, so a title cannot
/// break into lines of its own.
fn one_line(value: &str) -> String {
    value.chars().take_while(|c| !c.is_control()).collect()
}
