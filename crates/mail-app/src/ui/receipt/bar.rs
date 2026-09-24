//! The bar under the reader's head: a message asking for a receipt, and the two answers.

use super::super::motion::{Follow, tell};
use super::{Bodies, Line, Standing, answer, cached, line, lookup};
use crate::receipt::ReceiptState;
use dioxus::prelude::*;
use mail_domain::{MessageId, ReceiptAnswer};
use mail_store::SqliteStore;
use std::sync::Arc;

/// Where one bar's answer is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Phase {
    Asking,
    /// An answer is being given; both buttons stay disabled so a second press cannot send twice.
    Working,
    /// It did not go, and why, in words.
    Failed(String),
}

/// Every message in the open thread that asks, or once asked, for a receipt.
///
/// Draws nothing until the standings are known: finding them reads stored blobs, so it runs on
/// a blocking thread and lands here, or is already kept from the last time this thread was
/// open. Keyed by the reader on the thread and its bodies, so one thread's answer is never
/// drawn, or acted on, under another.
#[component]
pub(in crate::ui) fn Receipts(bodies: Bodies) -> Element {
    let mut known = use_signal({
        let bodies = bodies.clone();
        move || cached(&bodies)
    });
    let _look = use_resource(move || {
        let bodies = bodies.clone();
        let store = consume_context::<Arc<SqliteStore>>();
        async move {
            if known.peek().is_some() {
                return;
            }
            let found = tokio::task::spawn_blocking(move || lookup(&store, &bodies))
                .await
                .unwrap_or_default();
            known.set(Some(found));
        }
    });
    let Some(standings) = known() else {
        return rsx! {};
    };
    rsx! {
        for standing in standings {
            if let Some(said) = line(&standing) {
                Bar { key: "{standing.message}", message: standing.message, said, known }
            }
        }
    }
}

/// One message's bar: the question and its two buttons, or the note once it is answered.
#[component]
pub(in crate::ui) fn Bar(
    message: MessageId,
    said: Line,
    known: Signal<Option<Vec<Standing>>>,
) -> Element {
    let phase = use_signal(|| Phase::Asking);
    match said {
        Line::Settled(note) => rsx! {
            p { class: "receipt-note mono", "{note}" }
        },
        Line::Asking { sentence, warning } => {
            let working = phase() == Phase::Working;
            let failed = match phase() {
                Phase::Failed(why) => Some(why),
                _ => None,
            };
            let send = "Send receipt";
            let decline = "Don't send";
            rsx! {
                div { class: "receipt", role: "group", aria_label: "Read receipt",
                    p { class: "say", "{sentence}" }
                    if let Some(warning) = warning {
                        p { class: "warn", "{warning}" }
                    }
                    if let Some(why) = failed {
                        p { class: "why", "{why}" }
                    }
                    div { class: "acts",
                        button {
                            class: "mini",
                            r#type: "button",
                            aria_label: "{decline}",
                            disabled: working,
                            onclick: move |_| give(message, ReceiptAnswer::Declined, phase, known),
                            "{decline}"
                        }
                        button {
                            class: "mini primary",
                            r#type: "button",
                            aria_label: "{send}",
                            disabled: working,
                            onclick: move |_| give(message, ReceiptAnswer::Sent, phase, known),
                            if working { "Working…" } else { "{send}" }
                        }
                    }
                }
            }
        }
    }
}

/// Give `given` on a blocking thread, then say so in the toast and settle the bar.
fn give(
    message: MessageId,
    given: ReceiptAnswer,
    mut phase: Signal<Phase>,
    mut known: Signal<Option<Vec<Standing>>>,
) {
    if *phase.peek() == Phase::Working {
        return;
    }
    phase.set(Phase::Working);
    let store = consume_context::<Arc<SqliteStore>>();
    // Spawned from a click, which is where a task is polled (F140): a future started from a
    // component body is the one thing that may never run.
    spawn(async move {
        let done =
            tokio::task::spawn_blocking(move || answer(&store, message, given, chrono::Utc::now()))
                .await;
        match done {
            Ok(Ok(said)) => {
                if let Some(all) = known.write().as_mut()
                    && let Some(slot) = all.iter_mut().find(|had| had.message == message)
                {
                    slot.state = ReceiptState::Answered(given);
                }
                phase.set(Phase::Asking);
                // Nothing to take back: a receipt queued is the sender's once it leaves, and an
                // answer, either way, is kept so the question is not put again.
                tell(said, Follow::Nothing);
            }
            Ok(Err(why)) => phase.set(Phase::Failed(why)),
            Err(error) => phase.set(Phase::Failed(format!(
                "The answer stopped before it was given: {error}"
            ))),
        }
    });
}
