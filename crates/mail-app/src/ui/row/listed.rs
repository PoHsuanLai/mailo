//! What a row of the list stands for, and everything about it that is worked out rather than
//! drawn: its lines, its menu, and what a click on it opens. Pure, so each is a table test; the
//! one component that draws every row is [`super::ListRow`].
//!
//! A conversation and a draft are the same row. Each opens with a click — a conversation in the
//! reader, a draft in the composer, a message held for its time on a page that says when it
//! leaves — and each has its menu on a right click, built here from its own state.

use super::super::list_search::RowHit;
use super::super::text::sender;
use super::menu::{self, Pick};
use super::runs;
use crate::ui::copy;
use crate::ui::menus::when_words;
use crate::ui::selection::Click;
use chrono::{DateTime, TimeZone, Utc};
use ds::components::app::thread_height::ThreadLines;
use ds::components::menus::item::item::MenuItem;
use ds::prelude::*;
use mail_domain::{Draft, DraftId, ReadState, SendState, ThreadId, ThreadSummary};

/// One row of the list.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) enum Listed {
    Thread(ThreadSummary),
    Draft(Draft),
}

/// What a row draws.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) struct RowModel {
    pub name: String,
    pub subject: TextLine,
    pub snippet: Option<TextLine>,
    pub time: String,
    pub emphasis: Emphasis,
    /// What a press on the row reads as: "Open Offsite plans".
    pub label: String,
    /// How many lines of text it draws, which is what its height is computed from.
    pub lines: ThreadLines,
}

/// What a click on a row opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Opens {
    /// The conversation, as the click asks: opened, or picked with the others.
    Thread(ThreadId, Click),
    /// The draft, in the composer; one held for its time, on its page.
    Draft(DraftId),
}

impl Listed {
    /// The row's key in the list.
    pub(in crate::ui) fn key(&self) -> String {
        match self {
            Listed::Thread(summary) => summary.id.to_string(),
            Listed::Draft(draft) => draft.id.to_string(),
        }
    }

    /// The subject, or what stands for one.
    pub(in crate::ui) fn subject(&self) -> String {
        let subject = match self {
            Listed::Thread(summary) => &summary.subject,
            Listed::Draft(draft) => &draft.subject,
        };
        mail_domain::subject_or_none(subject).into_owned()
    }

    /// When a draft is held to leave; `None` for any other row.
    pub(in crate::ui) fn held(&self) -> Option<DateTime<Utc>> {
        match self {
            Listed::Draft(Draft {
                state: SendState::Scheduled { at },
                ..
            }) => Some(*at),
            _ => None,
        }
    }

    /// The row as drawn at `now` in `zone`, its hours by `clock`: with a search's marks (`hit`), and a line of
    /// snippet when `snippets` are shown.
    pub(in crate::ui) fn row<Tz: TimeZone>(
        &self,
        hit: Option<&RowHit>,
        snippets: bool,
        now: DateTime<Utc>,
        zone: &Tz,
        clock: mail_core::when::Clock,
    ) -> RowModel
    where
        Tz::Offset: std::fmt::Display,
    {
        let subject = self.subject();
        let label = copy::aria::open_row(&subject);
        match self {
            Listed::Thread(summary) => {
                // While a search is active the snippet is cut around its first match, and both
                // lines mark.
                let (subject_marks, snippet, snippet_marks) = match hit {
                    Some(hit) => (&hit.subject[..], &hit.snippet, &hit.snippet_marks[..]),
                    None => (&[][..], &summary.snippet, &[][..]),
                };
                let snippet =
                    (snippets && !snippet.is_empty()).then(|| runs(snippet, snippet_marks));
                RowModel {
                    name: sender(summary),
                    subject: runs(&summary.subject, subject_marks),
                    lines: lines(snippet.is_some()),
                    snippet,
                    time: mail_core::when::listed(summary.last_date, now, zone, clock),
                    emphasis: if summary.read == ReadState::Unread {
                        Emphasis::Strong
                    } else {
                        Emphasis::Plain
                    },
                    label,
                }
            }
            Listed::Draft(draft) => {
                let who = mail_core::compose::addresses::join_addresses(&draft.to);
                // A held one says when it leaves where the time is; any other, when it was last
                // touched, and its state on the snippet's line.
                let (time, snippet) = match self.held() {
                    Some(at) => (when_words(at, now, zone, clock), None),
                    None => (
                        mail_core::when::listed(draft.updated, now, zone, clock),
                        snippets.then(|| TextLine::from(draft.state.word())),
                    ),
                };
                RowModel {
                    name: if who.is_empty() {
                        copy::empty::NO_RECIPIENT.to_owned()
                    } else {
                        who
                    },
                    subject: TextLine::from(subject),
                    lines: lines(snippet.is_some()),
                    snippet,
                    time,
                    emphasis: Emphasis::Plain,
                    label,
                }
            }
        }
    }

    /// The row's right-click menu at `now`: a conversation's actions, as its state offers them;
    /// a draft's Delete Draft, or a held one's Cancel Sending.
    pub(in crate::ui) fn menu(&self, now: DateTime<Utc>) -> Vec<MenuItem<Pick>> {
        match self {
            Listed::Thread(summary) => menu::rows(&menu::groups(summary), summary, now),
            Listed::Draft(_) => menu::draft_rows(self.held().is_some()),
        }
    }

    /// What `click` on the row opens. A draft is not picked with others: any click opens it.
    pub(in crate::ui) fn open(&self, click: Click) -> Opens {
        match self {
            Listed::Thread(summary) => Opens::Thread(summary.id, click),
            Listed::Draft(draft) => Opens::Draft(draft.id),
        }
    }
}

fn lines(snippet: bool) -> ThreadLines {
    if snippet {
        ThreadLines::Three
    } else {
        ThreadLines::Two
    }
}
