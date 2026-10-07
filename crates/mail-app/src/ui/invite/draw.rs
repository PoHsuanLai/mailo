//! The invitation card: what the event is, who is asked, and the three answers.

use super::super::menu::{MenuKey, menu_key};
use super::super::motion::{Follow, tell};
use super::super::press::{available, on_primary};
use super::{CHIPS, Card, Stand, answer, cached, lookup, save_ics};
use dioxus::prelude::*;
use ds::components::content::avatar::{
    AvatarFace, AvatarShape, AvatarSize, AvatarTone, person_hue,
};
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::{Answers, Bezel};
use ds::components::controls::chip::{Chip, ChipVariant};
use ds::components::controls::segmented::Tracking;
use ds::components::fields::fact_list::{Fact, FactList};
use ds::components::overlays::inline_banner::InlineBanner;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use mail_domain::{Attendance, BlobId, MessageId};
use mail_store::SqliteStore;
use std::sync::Arc;

/// A description longer than this many lines or characters is folded, with More.
const FOLD_LINES: usize = 3;
const FOLD_CHARS: usize = 280;

/// Where the card's answering is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Phase {
    /// As the card's standing says.
    Resting,
    /// Answered, and Change answer pressed: the three buttons again.
    Changing,
    /// An answer chosen, and its note being written.
    Noting {
        attendance: Attendance,
        note: String,
    },
    /// The answer is being queued; every button stays disabled so a second press cannot send
    /// twice.
    Working,
    /// It did not go, or the file was not written, and why, in words.
    Failed(String),
}

/// The card for one message, once its invitation is known.
///
/// Draws nothing until then: finding out reads the stored blob, so it runs on a blocking thread
/// and lands here, or is already kept from the last time the message was open. Keyed by the
/// reader on the message and its body, so one message's card is never drawn under another.
#[component]
pub(in crate::ui) fn Invitation(message: MessageId, body: Option<BlobId>) -> Element {
    let mut known = use_signal(move || cached(message, body));
    let _look = use_resource(move || {
        let store = consume_context::<Arc<SqliteStore>>();
        async move {
            if known.peek().is_some() {
                return;
            }
            let found = tokio::task::spawn_blocking(move || lookup(&store, message, body))
                .await
                .unwrap_or_default();
            known.set(Some(found));
        }
    });
    let Some(Some(card)) = known() else {
        return rsx! {};
    };
    rsx! {
        InviteCard { card, known }
    }
}

/// The card itself. `known` is where an answer's fresh card is put.
#[component]
pub(in crate::ui) fn InviteCard(card: Card, known: Signal<Option<Option<Card>>>) -> Element {
    let mut phase = use_signal(|| Phase::Resting);
    let mut more = use_signal(|| false);
    let mut everyone = use_signal(|| false);
    let message = card.message;
    let now = phase();
    let offered = match (&card.stand, &now) {
        (Stand::Open { .. }, _) => true,
        (Stand::Answered { .. }, Phase::Resting) => false,
        (Stand::Answered { .. }, _) => true,
        (Stand::Closed(_), _) => false,
    };
    let working = now == Phase::Working;
    let chosen = match &now {
        Phase::Noting { attendance, .. } => Some(*attendance),
        _ => None,
    };
    let shown = if everyone() || card.attendees.len() <= CHIPS + 1 {
        card.attendees.len()
    } else {
        CHIPS
    };
    let hidden = card.attendees.len() - shown;
    let long = card
        .description
        .as_deref()
        .is_some_and(|text| text.lines().count() > FOLD_LINES || text.chars().count() > FOLD_CHARS);
    let desc_class = if long && !more() {
        "inv-desc folded"
    } else {
        "inv-desc"
    };
    let title = card.title.clone();
    let save = "Save .ics";
    let change = "Change answer";
    let more_label = if more() { "Less" } else { "More" };
    // Made here, not in the note row: the row unmounts the moment the answer starts, and a task
    // spawned from a component that is gone is dropped with it. A callback runs in the scope
    // that made it, and this card stays.
    let send = use_callback(move |(attendance, note): (Attendance, String)| {
        give(message, attendance, note, phase, known)
    });
    rsx! {
            section { class: "invite", role: "group", aria_label: "Calendar invitation",
                div { class: "inv-head",
                    span { class: card.tag.class(), Chip { variant: card.tag.chip(), text: card.tag.word().to_owned() } }
                    h3 { class: "inv-title", Label { text: card.title.clone(), style: LabelStyle::Title } }
                    Button {
        label: save,
        availability: available(!working),
        onclick: on_primary(move || save_file(message, title.clone(), phase)),
        common: Common { extra_class: ExtraClass::parse("inv-save").ok(), aria_label: Some(save.to_owned()), ..Common::default() },
    }
                }
                FactList { facts: facts_of(&card) }
                if !card.attendees.is_empty() {
                    ul { class: "inv-people", aria_label: "Attendees",
                        for chip in card.attendees.iter().take(shown) {
                            li { class: "{chip.class}", title: "{chip.name}: {chip.said}",
                                Chip {
                                    variant: ChipVariant::Person(AvatarFace {
                                        initial: chip.name.chars().next().unwrap_or('?'),
                                        size: AvatarSize::Size18,
                                        tone: AvatarTone::Person(person_hue(&chip.name)),
                                        shape: AvatarShape::Round,
                                    }),
                                    text: chip.name.clone(),
                                }
                            }
                        }
                        if hidden > 0 {
                            li { class: "att rest",
                                Button {
        bezel: Bezel::Inline,
        label: format!("+{hidden}"),
    common: Common { aria_label: Some(format!("Show {hidden} more attendees")), ..Common::default() },
        onclick: on_primary(move || everyone.set(true)),
    }
                            }
                        }
                    }
                }
                if let Some(summary) = &card.summary {
                    p { class: "inv-summary", Label { text: summary.clone(), style: LabelStyle::Headline } }
                }
                if let Some(comment) = &card.comment {
                    p { class: "inv-comment", Label { text: format!("“{comment}”"), role: LabelRole::Secondary } }
                }
                if let Some(description) = &card.description {
                    div { class: "inv-about",
                        p { class: "{desc_class}", Label { text: description.clone(), role: LabelRole::Secondary } }
                        if long {
                            Button {
        bezel: Bezel::Inline,
        label: more_label,
    common: Common { aria_label: Some(more_label.to_owned()), ..Common::default() },
        onclick: on_primary(move || more.toggle()),
    }
                        }
                    }
                }
                match &card.stand {
                    Stand::Closed(Some(why)) => rsx! { InlineBanner { severity: Severity::Info, text: (*why).to_owned() } },
                    Stand::Closed(None) => rsx! {},
                    Stand::Answered { said, note } if now == Phase::Resting => rsx! {
                        InlineBanner {
                            severity: Severity::Ok,
                            text: said.clone(),
                            detail: note.as_ref().map(|note| TextLine::from(format!("\u{201c}{note}\u{201d}"))),
                            actions: rsx! {
                                Button {
                                    label: change,
                                    onclick: on_primary(move || phase.set(Phase::Changing)),
                                    common: Common { aria_label: Some(change.to_owned()), ..Common::default() },
                                }
                            },
                        }
                    },
                    Stand::Answered { said, .. } => rsx! { p { class: "inv-before", Label { text: format!("{said}."), role: LabelRole::Secondary } } },
                    Stand::Open { before: Some(before) } => rsx! { p { class: "inv-before", Label { text: before.clone(), role: LabelRole::Secondary } } },
                    Stand::Open { before: None } => rsx! {},
                }
                if offered {
                    div { class: "inv-acts",
                        SegmentedControl::<Attendance> {
                            label: "Answer",
                            choices: CHOICES.into_iter().map(|(attendance, label)| Choice::new(attendance, label)).collect::<Vec<_>>(),
                            tracking: match chosen {
                                Some(attendance) => Tracking::SelectOne(attendance),
                                None => Tracking::Momentary,
                            },
                            availability: available(!working),
                            onchange: move |attendance| phase.set(Phase::Noting { attendance, note: String::new() }),
                        }
                    }
                }
                if let Phase::Noting { attendance, note } = now.clone() {
                    Noting { attendance, note, phase, on_send: send }
                }
                if working {
                    p { class: "inv-before", Label { text: "Sending…", role: LabelRole::Secondary } }
                }
                if let Phase::Failed(why) = now {
                    InlineBanner { severity: Severity::Danger, text: why }
                }
            }
        }
}

/// What the card states about the event, a label and its value each: when, in whose time, and
/// where, who and how often where the invitation says.
fn facts_of(card: &Card) -> Vec<Fact> {
    let mut facts = vec![Fact::new("When", card.when.clone())];
    if let Some(theirs) = &card.theirs {
        facts.push(Fact::new("Their time", theirs.clone()));
    }
    if let Some(occurrence) = card.occurrence {
        facts.push(Fact::new("About", occurrence));
    }
    if let Some(repeats) = &card.repeats {
        facts.push(Fact::new("Repeats", repeats.clone()));
    }
    if let Some(location) = &card.location {
        facts.push(Fact::new("Where", location.clone()));
    }
    if let Some(organiser) = &card.organiser {
        facts.push(Fact::new("Organiser", organiser.clone()));
    }
    facts
}

/// The three answers, in the order calendars put them.
const CHOICES: [(Attendance, &str); 3] = [
    (Attendance::Accepted, "Accept"),
    (Attendance::Tentative, "Maybe"),
    (Attendance::Declined, "Decline"),
];

/// The one-line note under a chosen answer, and Send. Enter sends; Esc cancels.
#[component]
fn Noting(
    attendance: Attendance,
    note: String,
    phase: Signal<Phase>,
    on_send: Callback<(Attendance, String)>,
) -> Element {
    let typed = note.clone();
    rsx! {
            div {
                class: "inv-noting",
                onkeydown: move |event: KeyboardEvent| {
                    // Everything typed here stays here: a letter in the note is not a shortcut.
                    event.stop_propagation();
                    match menu_key(&event.key().to_string()) {
                        Some(MenuKey::Enter) => {
                            event.prevent_default();
                            on_send.call((attendance, typed.clone()));
                        }
                        Some(MenuKey::Escape) => phase.set(Phase::Resting),
                        _ => {}
                    }
                },
                TextField {
                    label: "Note".to_owned(),
                    value: note.clone(),
                    placeholder: "Add a note (optional)".to_owned(),
                    oninput: move |value: String| {
                        phase.set(Phase::Noting { attendance, note: value });
                    },
                    common: Common { extra_class: ExtraClass::parse("inv-note-field").ok(), ..Common::default() },
                }
                Button {
        label: "Cancel".to_owned(),
    common: Common { aria_label: Some("Cancel answer".to_owned()), ..Common::default() },
        onclick: on_primary(move || phase.set(Phase::Resting)),
    }
                Button {
        answers: Answers::Return,
        label: "Send".to_owned(),
    common: Common { aria_label: Some("Send answer".to_owned()), ..Common::default() },
        onclick: on_primary(move || on_send.call((attendance, note.clone()))),
    }
            }
        }
}

/// Queue `attendance` with `note` on a blocking thread, then say so in the toast and settle
/// the card on what the store now holds.
fn give(
    message: MessageId,
    attendance: Attendance,
    note: String,
    mut phase: Signal<Phase>,
    mut known: Signal<Option<Option<Card>>>,
) {
    if *phase.peek() == Phase::Working {
        return;
    }
    phase.set(Phase::Working);
    let store = consume_context::<Arc<SqliteStore>>();
    // Spawned from a click or a key, which is where a task is polled (F140).
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || {
            let note = Some(note.as_str()).filter(|note| !note.trim().is_empty());
            answer(&store, message, attendance, note, chrono::Utc::now())
        })
        .await;
        match done {
            Ok(Ok((said, card))) => {
                known.set(Some(card));
                phase.set(Phase::Resting);
                // Nothing to take back: an answer queued is the organiser's once it leaves, and
                // answering again is how a person changes their mind.
                tell(said, Follow::Nothing);
            }
            Ok(Err(why)) => phase.set(Phase::Failed(why)),
            Err(error) => phase.set(Phase::Failed(format!(
                "Couldn\u{2019}t send the answer: {error}"
            ))),
        }
    });
}

/// Write the event's `.ics` into the save directory on a blocking thread; the toast says where.
fn save_file(message: MessageId, title: String, mut phase: Signal<Phase>) {
    let store = consume_context::<Arc<SqliteStore>>();
    let dir = super::super::files::save_dir();
    let saving = crate::ui::downloads::Saving::quick();
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || {
            let saved = save_ics(&store, message, &title, &dir);
            (saved, crate::ui::downloads::origin(&store, message))
        })
        .await;
        match done {
            Ok((Ok(path), origin)) => {
                saving.end(Some(&path), origin);
                tell(format!("Saved to {}", path.display()), Follow::Nothing);
            }
            Ok((Err(why), _)) => phase.set(Phase::Failed(format!("The file was not saved: {why}"))),
            Err(error) => phase.set(Phase::Failed(format!("The file was not saved: {error}"))),
        }
    });
}
