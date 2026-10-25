//! Saved views and the queries behind them. Views are local rows; they never go on the wire.

use crate::filter::Filter;
use crate::id::{LabelId, ViewId};
use crate::state::{MailboxRole, Threading};
use serde::{Deserialize, Serialize};

/// What a view fundamentally *is*, which decides how the UI presents it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum ViewKind {
    /// A place you can move mail *to*, so it accepts drops.
    Place { mailbox: MailboxRole },
    /// A label you can apply by dropping.
    PlaceLabel { label: LabelId },
    /// A saved search. Read-only: dropping onto it is meaningless.
    Query,
}

/// A sortable or displayable attribute of a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Property {
    Date,
    Subject,
    From,
    Sender,
    Size,
    Attachments,
    Pin,
    /// Unread conversations apart from read ones. A sort, not a column: no row draws a word
    /// for it, the unread dot already says it.
    Unread,
    /// Starred conversations apart from the rest: IMAP's `\Flagged`. A sort, like [`Self::Unread`].
    Starred,
}

impl Property {
    /// The way a person expects this sort to run when they first pick it: the newest, the
    /// largest, the unread and the starred first, and words from A.
    pub fn natural(self) -> SortDir {
        match self {
            Property::Subject | Property::From | Property::Sender => SortDir::Asc,
            Property::Date
            | Property::Size
            | Property::Attachments
            | Property::Pin
            | Property::Unread
            | Property::Starred => SortDir::Desc,
        }
    }
}

/// What a view groups its rows by.
///
/// Deliberately not [`Property`]. Grouping and columns look like one vocabulary and are two:
/// nobody renders a column of "unread", and nobody groups by "size". Typing `group_by` as
/// `Option<Property>` made "group by read state" and "group by label" unrepresentable — two of
/// the groupings a mail client most obviously wants — because the field was typed as the thing
/// we had rather than the thing we needed.
///
/// That is the same mistake `Op::Reply(Compose)` was, and it is fixed the same way: split the
/// axis rather than widen the enum that was already right for its own job. [`Property`] is
/// unchanged and still serves `shown` and [`Sort`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum GroupKey {
    /// Group by a displayable attribute: date, sender, size.
    Property(Property),
    /// Read and unread.
    Read,
    /// Starred and unstarred.
    Star,
    /// Whether a thread carries this label.
    ///
    /// Flat labels cannot otherwise express "show me what is and is not tagged this way".
    Label(LabelId),
    /// Which mailbox roles a thread spans.
    Mailbox,
}

/// Which way a sort runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDir {
    Asc,
    #[default]
    Desc,
}

/// A sort key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sort {
    pub property: Property,
    pub dir: SortDir,
}

/// A saved view: the filter, plus how to present its results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub id: ViewId,
    pub name: String,
    pub kind: ViewKind,
    pub filter: Filter,
    pub sort: Sort,
    pub group_by: Option<GroupKey>,
    pub threading: Threading,
    /// Columns, in display order.
    pub shown: Vec<Property>,
    /// The actions a view once chose for its rows. Not read any more: a row's menu is the same
    /// in every view, built from the conversation alone. Kept, and empty in a view saved now, so
    /// that a view saved before still loads.
    #[serde(default)]
    pub hover: Vec<crate::op::OpKind>,
}

/// An opaque pagination position.
///
/// Keyset, not offset: an offset shifts under you when mail arrives mid-scroll. The encoding
/// is `mail-store`'s business and is not parsed anywhere else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cursor(pub String);

/// Which slice of the results to return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageReq {
    /// Resume after this position; `None` starts at the beginning.
    pub after: Option<Cursor>,
    pub limit: u32,
}

/// One slice of results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// The position to pass as [`PageReq::after`] next; `None` at the end of the results.
    pub next: Option<Cursor>,
}

/// A complete request against the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Query {
    pub filter: Filter,
    pub sort: Sort,
    pub page: PageReq,
}
