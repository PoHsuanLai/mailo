//! An earlier message of a conversation, folded to one line: who wrote it, the start of what
//! they said, and when. A press opens it where it stands and it stays open while the
//! conversation does.
//!
//! The newest message is always open, and so is any message that was unread when the
//! conversation was opened: reading a conversation marks it read, and a message that folded
//! away the moment it had been seen would be a message the person never saw.

use super::header::{named, when_label};
use super::sender_face;
use super::thumb::pressed;
use crate::ui::common::classed;
use dioxus::prelude::*;
use ds::components::content::avatar::AvatarSize;
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::prelude::*;
use mail_domain::{Message, MessageId};
use mail_ui_model::copy::aria;
use std::collections::HashSet;

/// How a message of a conversation is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fold {
    /// Its header, what is said about it, and its body.
    Open,
    /// One line.
    Folded,
}

/// What keeps an earlier message open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hold {
    /// Nothing does.
    Nothing,
    /// It was unread when the conversation was opened, or a press opened it.
    Open,
}

/// How the message at `at` of `count` is drawn: the last is always open, an earlier one when
/// something [`Hold`] it so.
pub(super) fn fold_at(at: usize, count: usize, kept: Hold) -> Fold {
    match (at + 1 == count, kept) {
        (true, _) | (_, Hold::Open) => Fold::Open,
        (false, Hold::Nothing) => Fold::Folded,
    }
}

/// The words a folded line shows of `message`: its own text, without the lines it quotes or the
/// line that introduces them. The line's end is cut by its width, not here.
pub(super) fn snippet(message: &Message) -> String {
    const WORDS: usize = 40;
    message
        .body
        .text()
        .unwrap_or_default()
        .lines()
        .take_while(|line| !line.trim_end().ends_with("wrote:"))
        .filter(|line| !line.trim_start().starts_with('>'))
        .flat_map(str::split_whitespace)
        .take(WORDS)
        .collect::<Vec<_>>()
        .join(" ")
}

/// One folded message. `unfolded` is the reader's set of messages a press opened.
#[component]
pub(super) fn FoldedLine(
    message: Message,
    when: (String, String),
    unfolded: Signal<HashSet<MessageId>>,
) -> Element {
    let face = sender_face(&message);
    let (name, _) = named(&message);
    let id = message.id;
    let label = aria::open_message(&name);
    let mut open = move || {
        unfolded.write().insert(id);
    };
    let (short, full) = when;
    rsx! {
        div {
            class: "fold-line",
            role: "button",
            tabindex: "0",
            "aria-label": label,
            onclick: move |_| open(),
            onkeydown: move |event: KeyboardEvent| {
                if pressed(&event) {
                    event.prevent_default();
                    event.stop_propagation();
                    open();
                }
            },
            Avatar { initial: face.initial, size: AvatarSize::Size22, tone: face.tone }
            Label { text: name, style: LabelStyle::Headline, common: classed("fold-name") }
            Label { text: snippet(&message), role: LabelRole::Tertiary, style: LabelStyle::Footnote, common: classed("fold-snippet") }
            {when_label(short, full)}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Fold, Hold, fold_at};

    #[test]
    fn only_the_last_and_the_kept_messages_are_open() {
        let cases: &[(usize, usize, Hold, Fold)] = &[
            (0, 1, Hold::Nothing, Fold::Open),
            (0, 3, Hold::Nothing, Fold::Folded),
            (1, 3, Hold::Nothing, Fold::Folded),
            (1, 3, Hold::Open, Fold::Open),
            (2, 3, Hold::Nothing, Fold::Open),
        ];
        for &(at, count, kept, want) in cases {
            assert_eq!(fold_at(at, count, kept), want, "{at} of {count} {kept:?}");
        }
    }
}
