//! Several conversations picked at once: Ctrl-click, Shift-click, Shift+j/k and select-all.
//!
//! Pure: every function takes the ids the list shows, in the order it shows them, and the
//! conversation open in the reader, and returns a new [`Picked`]. Nothing here reads a store or a
//! signal, so each gesture is a table test (`selection/tests.rs`).
//!
//! What is picked is always read *through the list* ([`Picked::chosen`]). A conversation picked
//! and then archived, or picked in a list that a search has since replaced, is not listed, and an
//! action on the selection never reaches something the user cannot see.

use mail_domain::ThreadId;

/// The conversations picked for an action on several at once.
///
/// Empty is the ordinary state: an action then means the conversation open in the reader, as it
/// always has. The first Ctrl-click or Shift-click starts from that open conversation, so picking
/// a second one gives two, not one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picked {
    /// The picked conversations, in no particular order. [`Picked::chosen`] gives list order.
    threads: Vec<ThreadId>,
    /// Where a Shift range is measured from: the last conversation clicked on its own or
    /// Ctrl-clicked, as in every list that selects ranges.
    anchor: Option<ThreadId>,
    /// The end a Shift+j or Shift+k moves.
    cursor: Option<ThreadId>,
}

/// Which way Shift+j and Shift+k move the end of a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toward {
    /// Down the list: Shift+j.
    Next,
    /// Up the list: Shift+k.
    Previous,
}

/// What a click on a row asks for, read from the keys held with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// No modifier: open the conversation.
    Plain,
    /// Ctrl (Cmd on a Mac): pick it, or put it back.
    Toggle,
    /// Shift: pick the range from the anchor to it.
    Range,
}

impl Picked {
    /// Nothing picked.
    pub fn none() -> Self {
        Self::default()
    }

    /// What is picked and still listed, in list order. Empty when nothing is.
    pub fn chosen(&self, ids: &[ThreadId]) -> Vec<ThreadId> {
        ids.iter()
            .copied()
            .filter(|id| self.threads.contains(id))
            .collect()
    }

    /// Whether `thread` is picked and listed.
    pub fn holds(&self, thread: ThreadId, ids: &[ThreadId]) -> bool {
        self.threads.contains(&thread) && ids.contains(&thread)
    }

    /// Whether anything listed is picked.
    pub fn any(&self, ids: &[ThreadId]) -> bool {
        ids.iter().any(|id| self.threads.contains(id))
    }

    /// Ctrl-click: `thread` in or out, and the anchor moves to it.
    ///
    /// With nothing picked, the open conversation is what was selected, so it is picked too:
    /// Ctrl-clicking a second row gives both.
    pub fn toggle(&self, thread: ThreadId, open: Option<ThreadId>, ids: &[ThreadId]) -> Self {
        let mut threads = self.base(open, ids);
        match threads.iter().position(|id| *id == thread) {
            Some(at) => {
                threads.remove(at);
            }
            None => threads.push(thread),
        }
        Self {
            threads,
            anchor: Some(thread),
            cursor: Some(thread),
        }
    }

    /// Shift-click: every listed conversation from the anchor to `thread`, both included,
    /// replacing what was picked. The anchor stays where it was, so a second Shift-click
    /// re-measures from the same place.
    ///
    /// No anchor yet means the open conversation, and nothing open means `thread` alone.
    pub fn range(&self, thread: ThreadId, open: Option<ThreadId>, ids: &[ThreadId]) -> Self {
        let from = self.anchor_in(open, ids).unwrap_or(thread);
        Self {
            threads: between(ids, from, thread),
            anchor: Some(from),
            cursor: Some(thread),
        }
    }

    /// Shift+j or Shift+k: move the range's moving end one row, and pick from the anchor to it.
    ///
    /// The end starts at the open conversation, or, with nothing open, at the end of the list
    /// the movement comes from, as `j` and `k` do (`view::step`). It stops at the ends of the
    /// list rather than wrapping.
    pub fn extend(&self, toward: Toward, open: Option<ThreadId>, ids: &[ThreadId]) -> Self {
        let here = self
            .cursor
            .filter(|id| ids.contains(id))
            .or_else(|| open.filter(|id| ids.contains(id)));
        let forward = toward == Toward::Next;
        let Some(next) = crate::view::step(here, ids, forward) else {
            return Self::none();
        };
        // Starting fresh, the first press picks where it starts and where it lands.
        let from = self.anchor_in(open, ids).or(here).unwrap_or(next);
        Self {
            threads: between(ids, from, next),
            anchor: Some(from),
            cursor: Some(next),
        }
    }

    /// Select all: every listed conversation.
    pub fn all(ids: &[ThreadId]) -> Self {
        Self {
            threads: ids.to_vec(),
            anchor: ids.first().copied(),
            cursor: ids.last().copied(),
        }
    }

    /// A plain click on `thread`: nothing picked, and a later Shift-click measures from it.
    pub fn clicked(thread: ThreadId) -> Self {
        Self {
            threads: Vec::new(),
            anchor: Some(thread),
            cursor: Some(thread),
        }
    }

    /// What a gesture starts from: the listed picks, or else the open conversation alone.
    fn base(&self, open: Option<ThreadId>, ids: &[ThreadId]) -> Vec<ThreadId> {
        let chosen = self.chosen(ids);
        if chosen.is_empty() {
            open.filter(|id| ids.contains(id)).into_iter().collect()
        } else {
            chosen
        }
    }

    /// The anchor if it is still listed, else the open conversation if that is.
    fn anchor_in(&self, open: Option<ThreadId>, ids: &[ThreadId]) -> Option<ThreadId> {
        self.anchor
            .filter(|id| ids.contains(id))
            .or_else(|| open.filter(|id| ids.contains(id)))
    }
}

/// The listed ids from `a` to `b`, both included, in list order whichever comes first. Just
/// `b` when `a` is not listed, and nothing when `b` is not.
fn between(ids: &[ThreadId], a: ThreadId, b: ThreadId) -> Vec<ThreadId> {
    let Some(end) = ids.iter().position(|id| *id == b) else {
        return Vec::new();
    };
    let start = ids.iter().position(|id| *id == a).unwrap_or(end);
    let (low, high) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    ids[low..=high].to_vec()
}

#[cfg(test)]
mod tests;
