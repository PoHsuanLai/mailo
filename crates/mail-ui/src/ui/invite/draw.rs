//! The invitation card: what the event is, who is asked, and the three answers.

use super::super::menu::{MenuKey, menu_key};
use super::super::motion::{Follow, tell};
use super::super::press::{available, on_primary};
use super::{CHIPS, Calendar, Card, Stand, answer, cached, lookup, save_ics};
use crate::ui::copy::{aria, controls, errors, fields, toasts};
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
use mail_client::message::Looks;
use mail_domain::{Attendance, BlobId, MessageId};
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
    /// Propose New Time pressed: the time being written, and a note to go with it.
    Proposing { start: String, note: String },
    /// The answer is being queued; every button stays disabled so a second press cannot send
    /// twice.
    Working,
    /// It did not go, or the file was not written, and why, in words.
    Failed(String),
}

/// Whether the card shows more than its one row: the event's facts, its people and its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui) enum Disclosure {
    #[default]
    Closed,
    Open,
}

impl Disclosure {
    fn toggled(self) -> Self {
        match self {
            Self::Closed => Self::Open,
            Self::Open => Self::Closed,
        }
    }
}

/// The card for one message, once its invitation is known.
///
/// Draws nothing until then: finding out reads the stored blob, so it runs on a blocking thread
/// and lands here, or is already kept from the last time the message was open. Keyed by the
/// reader on the message and its body, so one message's card is never drawn under another.
#[component]
pub(in crate::ui) fn Invitation(message: MessageId, body: Option<BlobId>) -> Element {
    let looks = crate::ui::reading::use_looks();
    let hours = crate::ui::clock::hours();
    let mut known = use_signal({
        let looks = looks.clone();
        move || cached(&looks, message, body, hours)
    });
    let _look = use_resource(move || {
        let client = crate::ui::live::client();
        let looks = looks.clone();
        async move {
            if known.peek().is_some() {
                return;
            }
            let found =
                tokio::task::spawn_blocking(move || lookup(&looks, &client, message, body, hours))
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
pub(in crate::ui) fn InviteCard(
    card: Card,
    known: Signal<Option<Option<Card>>>,
    /// How the card first opens: the reader draws it closed.
    #[props(default)]
    opens: Disclosure,
) -> Element {
    let looks = crate::ui::reading::use_looks();
    let mut phase = use_signal(|| Phase::Resting);
    let mut disclosure = use_signal(|| opens);
    let mut more = use_signal(|| false);
    let mut everyone = use_signal(|| false);
    let message = card.message;
    let desk = try_consume_context::<crate::calendar::Calendars>();
    let now = phase();
    let offered = match (&card.stand, &now) {
        (Stand::Open { .. }, _) => true,
        (Stand::Answered { .. }, Phase::Resting | Phase::Proposing { .. }) => false,
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
    let keeping = desk.clone();
    let send = use_callback(move |(attendance, note): (Attendance, String)| {
        give(
            looks.clone(),
            message,
            attendance,
            note,
            phase,
            known,
            keeping.clone(),
        )
    });
    let propose_it =
        use_callback(move |(start, note): (String, String)| propose(message, start, note, phase));
    let open = disclosure() == Disclosure::Open;
    let details_name = match disclosure() {
        Disclosure::Closed => aria::SHOW_DETAILS,
        Disclosure::Open => aria::HIDE_DETAILS,
    };
    rsx! {
        section { class: "invite", role: "group", aria_label: "Calendar invitation",
            div { class: "inv-row",
                span { class: card.tag.class(), Chip { variant: card.tag.chip(), text: card.tag.word().to_owned() } }
                div { class: "inv-what",
                    h3 { class: "inv-title", Label { text: card.title.clone(), style: LabelStyle::Headline } }
                    Label { text: card.when.clone(), role: LabelRole::Secondary, style: LabelStyle::Footnote }
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
                Button {
                    bezel: Bezel::Inline,
                    label: controls::DETAILS,
                    common: Common { aria_label: Some(details_name.to_owned()), ..Common::default() },
                    onclick: on_primary(move || disclosure.set(disclosure().toggled())),
                }
            }
            if desk.is_some() && (card.calendar != Calendar::None || card.propose) {
                div { class: "inv-cal",
                    match card.calendar {
                        Calendar::Add => rsx! {
                            Button {
                                bezel: Bezel::Inline,
                                label: controls::ADD_TO_CALENDAR,
                                availability: available(!working),
                                onclick: {
                                    let desk = desk.clone();
                                    on_primary(move || {
                                        if let Some(desk) = desk.clone() {
                                            into_calendar(desk, message, phase);
                                        }
                                    })
                                },
                                common: Common { aria_label: Some(controls::ADD_TO_CALENDAR.to_owned()), data: crate::ui::sidebar::tagged("invite", "add-to-calendar").data, ..Common::default() },
                            }
                        },
                        Calendar::Remove => rsx! {
                            Button {
                                bezel: Bezel::Inline,
                                label: controls::REMOVE_FROM_CALENDAR,
                                availability: available(!working),
                                onclick: {
                                    let desk = desk.clone();
                                    on_primary(move || {
                                        if let Some(desk) = desk.clone() {
                                            out_of_calendar(desk, message, phase);
                                        }
                                    })
                                },
                                common: Common { aria_label: Some(controls::REMOVE_FROM_CALENDAR.to_owned()), data: crate::ui::sidebar::tagged("invite", "remove-from-calendar").data, ..Common::default() },
                            }
                        },
                        Calendar::None => rsx! {},
                    }
                    if card.propose {
                        Button {
                            bezel: Bezel::Inline,
                            label: controls::PROPOSE_NEW_TIME,
                            availability: available(!working),
                            onclick: on_primary(move || phase.set(Phase::Proposing { start: String::new(), note: String::new() })),
                            common: Common { aria_label: Some(controls::PROPOSE_NEW_TIME.to_owned()), data: crate::ui::sidebar::tagged("invite", "propose").data, ..Common::default() },
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
                                common: Common { aria_label: Some(change.to_owned()), data: crate::ui::sidebar::tagged("invite", "change").data, ..Common::default() },
                            }
                        },
                    }
                },
                Stand::Answered { said, .. } => rsx! { p { class: "inv-before", Label { text: format!("{said}."), role: LabelRole::Secondary } } },
                Stand::Open { before: Some(before) } => rsx! { p { class: "inv-before", Label { text: before.clone(), role: LabelRole::Secondary } } },
                Stand::Open { before: None } => rsx! {},
            }
            if let Phase::Noting { attendance, note } = now.clone() {
                Noting { attendance, note, phase, on_send: send }
            }
            if let Phase::Proposing { start, note } = now.clone() {
                Proposing { start, note, phase, on_send: propose_it }
            }
            if working {
                p { class: "inv-before", Label { text: "Sending\u{2026}", role: LabelRole::Secondary } }
            }
            if let Phase::Failed(why) = now {
                InlineBanner { severity: Severity::Danger, text: why }
            }
            if open {
                div { class: "inv-details",
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
                        p { class: "inv-comment", Label { text: format!("\u{201c}{comment}\u{201d}"), role: LabelRole::Secondary } }
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
                    div { class: "inv-save",
                        Button {
                            bezel: Bezel::Inline,
                            label: save,
                            availability: available(!working),
                            onclick: on_primary(move || save_file(message, title.clone(), phase)),
                            common: Common { aria_label: Some(save.to_owned()), data: crate::ui::sidebar::tagged("invite", "save").data, ..Common::default() },
                        }
                    }
                }
            }
        }
    }
}

/// What the card states about the event, a label and its value each: when, in whose time, and
/// where, who and how often where the invitation says.
fn facts_of(card: &Card) -> Vec<Fact> {
    let mut facts = vec![Fact::new(fields::WHEN, card.when.clone())];
    if let Some(theirs) = &card.theirs {
        facts.push(Fact::new("Their time", theirs.clone()));
    }
    if let Some(occurrence) = card.occurrence {
        facts.push(Fact::new("About", occurrence));
    }
    if let Some(repeats) = &card.repeats {
        facts.push(Fact::new(fields::REPEATS, repeats.clone()));
    }
    if let Some(location) = &card.location {
        facts.push(Fact::new(fields::WHERE, location.clone()));
    }
    if let Some(organiser) = &card.organiser {
        facts.push(Fact::new(fields::ORGANISER, organiser.clone()));
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
    common: Common { aria_label: Some("Send answer".to_owned()), data: crate::ui::sidebar::tagged("invite", "send").data, ..Common::default() },
        onclick: on_primary(move || on_send.call((attendance, note.clone()))),
    }
            }
        }
}

/// Queue `attendance` with `note` on a blocking thread, then say so in the toast and settle
/// the card on what the client now holds.
fn give(
    looks: Arc<Looks>,
    message: MessageId,
    attendance: Attendance,
    note: String,
    mut phase: Signal<Phase>,
    mut known: Signal<Option<Option<Card>>>,
    desk: Option<crate::calendar::Calendars>,
) {
    if *phase.peek() == Phase::Working {
        return;
    }
    phase.set(Phase::Working);
    let client = crate::ui::live::client();
    let hours = crate::ui::clock::hours();
    let keep =
        crate::ui::prefs::current().calendar.keep_in_step == crate::settings::KeepCalendar::On;
    let calendar_client = client.clone();
    // Spawned from a click or a key, which is where a task is polled.
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || {
            let note = Some(note.as_str()).filter(|note| !note.trim().is_empty());
            answer(
                &looks,
                &client,
                message,
                attendance,
                note,
                chrono::Utc::now(),
                hours,
            )
        })
        .await;
        match done {
            Ok(Ok((said, card))) => {
                known.set(Some(card));
                phase.set(Phase::Resting);
                // The calendar follows the answer, never the other way round: a calendar that
                // cannot be reached costs the person a sentence, not their answer.
                let said = match desk.filter(|_| keep) {
                    Some(desk) => {
                        match crate::calendar::flow::follow(
                            &desk,
                            &calendar_client,
                            message,
                            attendance,
                        )
                        .await
                        {
                            Ok(crate::calendar::flow::Kept::Updated(name)) => {
                                format!("{said}. {}", toasts::updated_in_calendar(&name))
                            }
                            Ok(crate::calendar::flow::Kept::Removed(name)) => {
                                format!("{said}. {}", toasts::taken_out_of_calendar(&name))
                            }
                            Ok(crate::calendar::flow::Kept::NotThere) => said,
                            Err(why) => format!("{said}. {}", errors::calendar_not_updated(&why)),
                        }
                    }
                    None => said,
                };
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
    let client = crate::ui::live::client();
    let dir = super::super::files::save_dir();
    let saving = crate::ui::downloads::Saving::quick();
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || {
            let saved = save_ics(&client, message, &title, &dir);
            (saved, crate::ui::downloads::origin(&client, message))
        })
        .await;
        match done {
            Ok((Ok(path), origin)) => {
                saving.end(Some(&path), origin);
                tell(format!("Saved to {}", path.display()), Follow::Nothing);
            }
            Ok((Err(why), _)) => phase.set(Phase::Failed(format!("Couldn’t save the file: {why}"))),
            Err(error) => phase.set(Phase::Failed(format!("Couldn’t save the file: {error}"))),
        }
    });
}

/// Add to Calendar: put the event in the calendar the settings name, on the calendar's own
/// threads, and say where it went.
fn into_calendar(desk: crate::calendar::Calendars, message: MessageId, mut phase: Signal<Phase>) {
    let client = crate::ui::live::client();
    let wanted = crate::ui::prefs::current().calendar.default;
    phase.set(Phase::Working);
    spawn(async move {
        match crate::calendar::flow::add(&desk, &client, message, &wanted).await {
            Ok(added) => {
                phase.set(Phase::Resting);
                tell(
                    match added {
                        crate::calendar::flow::Added::In(name) => toasts::added_to_calendar(&name),
                        crate::calendar::flow::Added::HandedOver => {
                            toasts::handed_to_calendar_app()
                        }
                    },
                    Follow::Nothing,
                );
            }
            Err(why) => phase.set(Phase::Failed(errors::calendar_failed(&why))),
        }
    });
}

/// Remove from Calendar: take a cancelled event out of the calendar that has it.
fn out_of_calendar(desk: crate::calendar::Calendars, message: MessageId, mut phase: Signal<Phase>) {
    let client = crate::ui::live::client();
    phase.set(Phase::Working);
    spawn(async move {
        match crate::calendar::flow::cancel(&desk, &client, message).await {
            Ok(Some(name)) => {
                phase.set(Phase::Resting);
                tell(toasts::removed_from_calendar(&name), Follow::Nothing);
            }
            Ok(None) => {
                phase.set(Phase::Resting);
                tell(toasts::NOT_IN_CALENDAR.to_owned(), Follow::Nothing);
            }
            Err(why) => phase.set(Phase::Failed(errors::calendar_failed(&why))),
        }
    });
}

/// Queue the proposed time for the organiser on a blocking thread.
fn propose(message: MessageId, start: String, note: String, mut phase: Signal<Phase>) {
    let Some(start) = crate::calendar::flow::local_time(&start) else {
        phase.set(Phase::Failed(errors::NEW_TIME_NOT_READ.to_owned()));
        return;
    };
    let client = crate::ui::live::client();
    phase.set(Phase::Working);
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || {
            let note = Some(note.as_str()).filter(|note| !note.trim().is_empty());
            client
                .invite()
                .propose(message, start, note, chrono::Utc::now())
        })
        .await;
        match done {
            Ok(Ok(_)) => {
                phase.set(Phase::Resting);
                tell(toasts::PROPOSAL_SENT.to_owned(), Follow::Nothing);
            }
            Ok(Err(why)) => phase.set(Phase::Failed(why.to_string())),
            Err(error) => phase.set(Phase::Failed(error.to_string())),
        }
    });
}

/// The new time and its note, and Send Proposal. Enter sends; Esc cancels.
#[component]
fn Proposing(
    start: String,
    note: String,
    phase: Signal<Phase>,
    on_send: Callback<(String, String)>,
) -> Element {
    let (typed_start, typed_note) = (start.clone(), note.clone());
    rsx! {
        div {
            class: "inv-noting",
            onkeydown: move |event: KeyboardEvent| {
                event.stop_propagation();
                match menu_key(&event.key().to_string()) {
                    Some(MenuKey::Enter) => {
                        event.prevent_default();
                        on_send.call((typed_start.clone(), typed_note.clone()));
                    }
                    Some(MenuKey::Escape) => phase.set(Phase::Resting),
                    _ => {}
                }
            },
            TextField {
                label: fields::NEW_TIME.to_owned(),
                value: start.clone(),
                placeholder: fields::NEW_TIME_EXAMPLE.to_owned(),
                oninput: {
                    let note = note.clone();
                    move |value: String| phase.set(Phase::Proposing { start: value, note: note.clone() })
                },
                common: Common { extra_class: ExtraClass::parse("inv-note-field").ok(), ..Common::default() },
            }
            TextField {
                label: fields::NOTE.to_owned(),
                value: note.clone(),
                placeholder: fields::NOTE_PLACEHOLDER.to_owned(),
                oninput: {
                    let start = start.clone();
                    move |value: String| phase.set(Phase::Proposing { start: start.clone(), note: value })
                },
                common: Common { extra_class: ExtraClass::parse("inv-note-field").ok(), ..Common::default() },
            }
            Button {
                label: controls::CANCEL.to_owned(),
                common: Common { aria_label: Some(aria::CANCEL_PROPOSAL.to_owned()), ..Common::default() },
                onclick: on_primary(move || phase.set(Phase::Resting)),
            }
            Button {
                answers: Answers::Return,
                label: controls::SEND_PROPOSAL.to_owned(),
                common: Common { data: crate::ui::sidebar::tagged("invite", "send-proposal").data, ..Common::default() },
                onclick: on_primary(move || on_send.call((start.clone(), note.clone()))),
            }
        }
    }
}
