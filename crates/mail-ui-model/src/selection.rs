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
    /// The anchor's row, in a list that gives a conversation several (a message to a row), where
    /// the conversation alone does not say which. Only read while that row still holds the anchor.
    anchor_row: Option<usize>,
    /// The cursor's row, as `anchor_row` is the anchor's.
    cursor_row: Option<usize>,
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

    /// What is picked and still listed, in list order, each once: a conversation listed a
    /// message to a row has several rows, and an action reaches it once. Empty when nothing is.
    pub fn chosen(&self, ids: &[ThreadId]) -> Vec<ThreadId> {
        let mut chosen: Vec<ThreadId> = Vec::new();
        for id in ids {
            if self.threads.contains(id) && !chosen.contains(id) {
                chosen.push(*id);
            }
        }
        chosen
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
            ..Self::default()
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
            ..Self::default()
        }
    }

    /// Shift-click on the row `at` of a list that gives a conversation several rows: the range
    /// is of rows, from the anchor's row to that one, so a conversation that comes twice is
    /// measured from the row that was clicked or moved to, not from its first.
    ///
    /// The anchor is the row a plain click or a Shift range left ([`Self::clicked_row`]), else
    /// the open conversation's first row, else `at`.
    pub fn range_rows(&self, at: usize, open: Option<ThreadId>, ids: &[ThreadId]) -> Self {
        let Some(clicked) = ids.get(at).copied() else {
            return self.clone();
        };
        let from = self.anchor_at(open, ids).unwrap_or(at);
        Self {
            threads: rows_between(ids, from, at),
            anchor: Some(ids[from]),
            cursor: Some(clicked),
            anchor_row: Some(from),
            cursor_row: Some(at),
        }
    }

    /// Shift+j or Shift+k: move the range's moving end one row, and pick from the anchor to it.
    ///
    /// The end starts at the open conversation, or, with nothing open, at the end of the list
    /// the movement comes from, as `j` and `k` do (`mail_client::conversation::step`). It stops at the ends of the
    /// list rather than wrapping.
    pub fn extend(&self, toward: Toward, open: Option<ThreadId>, ids: &[ThreadId]) -> Self {
        let here = self
            .cursor
            .filter(|id| ids.contains(id))
            .or_else(|| open.filter(|id| ids.contains(id)));
        let forward = toward == Toward::Next;
        let Some(next) = mail_client::conversation::step(here, ids, forward) else {
            return Self::none();
        };
        // Starting fresh, the first press picks where it starts and where it lands.
        let from = self.anchor_in(open, ids).or(here).unwrap_or(next);
        Self {
            threads: between(ids, from, next),
            anchor: Some(from),
            cursor: Some(next),
            ..Self::default()
        }
    }

    /// [`Self::extend`] by rows, for a list that gives a conversation several: the moving end is
    /// a row, so Shift+j from the second of a conversation's rows goes to the one after it, not
    /// after the conversation's first.
    pub fn extend_rows(&self, toward: Toward, open: Option<ThreadId>, ids: &[ThreadId]) -> Self {
        if ids.is_empty() {
            return Self::none();
        }
        let here = self.cursor_at(ids).or_else(|| first_row(open, ids));
        let next = match (here, toward) {
            (None, Toward::Next) => 0,
            (None, Toward::Previous) => ids.len() - 1,
            (Some(here), Toward::Next) => (here + 1).min(ids.len() - 1),
            (Some(here), Toward::Previous) => here.saturating_sub(1),
        };
        let from = self.anchor_at(open, ids).or(here).unwrap_or(next);
        Self {
            threads: rows_between(ids, from, next),
            anchor: Some(ids[from]),
            cursor: Some(ids[next]),
            anchor_row: Some(from),
            cursor_row: Some(next),
        }
    }

    /// Select all: every listed conversation.
    pub fn all(ids: &[ThreadId]) -> Self {
        Self {
            threads: ids.to_vec(),
            anchor: ids.first().copied(),
            cursor: ids.last().copied(),
            ..Self::default()
        }
    }

    /// A plain click on `thread`: nothing picked, and a later Shift-click measures from it.
    pub fn clicked(thread: ThreadId) -> Self {
        Self {
            threads: Vec::new(),
            anchor: Some(thread),
            cursor: Some(thread),
            ..Self::default()
        }
    }

    /// A plain click on the row `at`, which lists `thread`: as [`Self::clicked`], and a later
    /// Shift-click measures from that row.
    pub fn clicked_row(thread: ThreadId, at: usize) -> Self {
        Self {
            anchor_row: Some(at),
            cursor_row: Some(at),
            ..Self::clicked(thread)
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

    /// The anchor's row: the one remembered while it still holds the anchor, else the first row
    /// of the anchor, else the open conversation's.
    fn anchor_at(&self, open: Option<ThreadId>, ids: &[ThreadId]) -> Option<usize> {
        self.anchor_row
            .filter(|row| self.anchor.is_some_and(|a| ids.get(*row) == Some(&a)))
            .or_else(|| first_row(self.anchor, ids))
            .or_else(|| first_row(open, ids))
    }

    /// The cursor's row, while the row remembered still holds the cursor; else its first row.
    fn cursor_at(&self, ids: &[ThreadId]) -> Option<usize> {
        self.cursor_row
            .filter(|row| self.cursor.is_some_and(|c| ids.get(*row) == Some(&c)))
            .or_else(|| first_row(self.cursor, ids))
    }

    /// The anchor if it is still listed, else the open conversation if that is.
    fn anchor_in(&self, open: Option<ThreadId>, ids: &[ThreadId]) -> Option<ThreadId> {
        self.anchor
            .filter(|id| ids.contains(id))
            .or_else(|| open.filter(|id| ids.contains(id)))
    }
}

/// The first row that lists `thread`.
fn first_row(thread: Option<ThreadId>, ids: &[ThreadId]) -> Option<usize> {
    thread.and_then(|id| ids.iter().position(|it| *it == id))
}

/// The conversations of rows `a` to `b`, both included, each once, in list order whichever row
/// comes first.
fn rows_between(ids: &[ThreadId], a: usize, b: usize) -> Vec<ThreadId> {
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    let mut found: Vec<ThreadId> = Vec::new();
    for id in &ids[low..=high] {
        if !found.contains(id) {
            found.push(*id);
        }
    }
    found
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
