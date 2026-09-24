//! The invitation card: what the event is, who is asked, and the three answers.

use super::super::field::{Field, FieldKind};
use super::super::menu::{MenuKey, menu_key};
use super::super::motion::{Follow, tell};
use super::super::press::{available, on_primary};
use super::{CHIPS, Card, Stand, answer, cached, lookup, save_ics};
use dioxus::prelude::*;
use ds::{Glyph, Icon};
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
    let everyone_label = format!("Show {hidden} more attendees");
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
                span { class: card.tag.class(), "{card.tag.word()}" }
                h3 { class: "inv-title", "{card.title}" }
                span { class: "inv-save",
                    ds::Button {
                        variant: ds::ButtonVariant::Mini,
                        label: save,
                        aria_label: save.to_owned(),
                        availability: available(!working),
                        onclick: on_primary(move || save_file(message, title.clone(), phase)),
                    }
                }
            }
            dl { class: "inv-facts",
                div { class: "inv-row",
                    dt { Glyph { icon: Icon::Clock, size: ds::IconSize::Compact } }
                    dd {
                        span { class: "inv-when", "{card.when}" }
                        if let Some(theirs) = &card.theirs {
                            span { class: "inv-theirs mono", "their time: {theirs}" }
                        }
                        if let Some(occurrence) = card.occurrence {
                            span { class: "inv-theirs", "{occurrence}" }
                        }
                    }
                }
                if let Some(repeats) = &card.repeats {
                    div { class: "inv-row",
                        dt { "Repeats" }
                        dd { "{repeats}" }
                    }
                }
                if let Some(location) = &card.location {
                    div { class: "inv-row",
                        dt { "Where" }
                        dd { "{location}" }
                    }
                }
                if let Some(organiser) = &card.organiser {
                    div { class: "inv-row",
                        dt { "Organiser" }
                        dd { "{organiser}" }
                    }
                }
            }
            if !card.attendees.is_empty() {
                ul { class: "inv-people", aria_label: "Attendees",
                    for chip in card.attendees.iter().take(shown) {
                        li { class: "{chip.class}", title: "{chip.name}: {chip.said}",
                            span { class: "dot" }
                            "{chip.name}"
                        }
                    }
                    if hidden > 0 {
                        li { class: "att rest",
                            ds::Button {
                                variant: ds::ButtonVariant::Quiet,
                                label: format!("+{hidden}"),
                                aria_label: everyone_label.clone(),
                                onclick: on_primary(move || everyone.set(true)),
                            }
                        }
                    }
                }
            }
            if let Some(summary) = &card.summary {
                p { class: "inv-summary", "{summary}" }
            }
            if let Some(comment) = &card.comment {
                p { class: "inv-comment", "“{comment}”" }
            }
            if let Some(description) = &card.description {
                div { class: "inv-about",
                    p { class: "{desc_class}", "{description}" }
                    if long {
                        ds::Button {
                            variant: ds::ButtonVariant::Quiet,
                            label: more_label,
                            aria_label: more_label.to_owned(),
                            onclick: on_primary(move || more.toggle()),
                        }
                    }
                }
            }
            match &card.stand {
                Stand::Closed(Some(why)) => rsx! { p { class: "inv-why", "{why}" } },
                Stand::Closed(None) => rsx! {},
                Stand::Answered { said, note } if now == Phase::Resting => rsx! {
                    div { class: "inv-answered",
                        Glyph { icon: Icon::Check, size: ds::IconSize::Compact }
                        span { class: "said", "{said}" }
                        if let Some(note) = note {
                            span { class: "inv-note", "“{note}”" }
                        }
                        ds::Button {
                            variant: ds::ButtonVariant::Quiet,
                            label: change,
                            aria_label: change.to_owned(),
                            onclick: on_primary(move || phase.set(Phase::Changing)),
                        }
                    }
                },
                Stand::Answered { said, .. } => rsx! { p { class: "inv-before", "{said}." } },
                Stand::Open { before: Some(before) } => rsx! { p { class: "inv-before", "{before}" } },
                Stand::Open { before: None } => rsx! {},
            }
            if offered {
                div { class: "inv-acts", role: "group", aria_label: "Answer",
                    for (attendance, label) in CHOICES {
                        ds::Button {
                            key: "{label}",
                            variant: if attendance == Attendance::Accepted { ds::ButtonVariant::Primary } else { ds::ButtonVariant::Mini },
                            label,
                            aria_label: label.to_owned(),
                            pressed: if chosen == Some(attendance) { ds::Switch::On } else { ds::Switch::Off },
                            availability: available(!working),
                            onclick: on_primary(move || phase.set(Phase::Noting { attendance, note: String::new() })),
                        }
                    }
                }
            }
            if let Phase::Noting { attendance, note } = now.clone() {
                Noting { attendance, note, phase, on_send: send }
            }
            if working {
                p { class: "inv-before", "Sending…" }
            }
            if let Phase::Failed(why) = now {
                p { class: "inv-why failed", "{why}" }
            }
        }
    }
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
    let send = "Send answer";
    let cancel = "Cancel answer";
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
            Field {
                kind: FieldKind::Boxed,
                value: note.clone(),
                placeholder: "Add a note (optional)".to_owned(),
                extra: Some("inv-note-field".to_owned()),
                on_input: move |value: String| {
                    phase.set(Phase::Noting { attendance, note: value });
                },
                on_focus: |_| {},
                on_blur: |_| {},
            }
            ds::Button {
                variant: ds::ButtonVariant::Mini,
                label: "Cancel".to_owned(),
                aria_label: cancel.to_string(),
                onclick: on_primary(move || phase.set(Phase::Resting)),
            }
            ds::Button {
                variant: ds::ButtonVariant::Primary,
                label: "Send".to_owned(),
                aria_label: send.to_string(),
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
                "The answer stopped before it was queued: {error}"
            ))),
        }
    });
}

/// Write the event's `.ics` into the save directory on a blocking thread; the toast says where.
fn save_file(message: MessageId, title: String, mut phase: Signal<Phase>) {
    let store = consume_context::<Arc<SqliteStore>>();
    let dir = super::super::files::save_dir();
    spawn(async move {
        let done =
            tokio::task::spawn_blocking(move || save_ics(&store, message, &title, &dir)).await;
        match done {
            Ok(Ok(said)) => tell(said, Follow::Nothing),
            Ok(Err(why)) => phase.set(Phase::Failed(format!("The file was not saved: {why}"))),
            Err(error) => phase.set(Phase::Failed(format!("The file was not saved: {error}"))),
        }
    });
}
