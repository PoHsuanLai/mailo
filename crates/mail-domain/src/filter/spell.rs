//! A [`Filter`] written back as the search line, as near as the language goes.
//!
//! Two callers say a filter in words: a saved view or rule, which is read again later, and an
//! error that names the clause a server could not be asked. They agree on every operator and
//! differ only in what the language has no spelling for, which they pass in as [`Words`].

use super::{DateRange, Filter, TextMatch};
use crate::id::LabelId;
use crate::state::{ReadState, Star};

/// The parts of a filter the search language has no word for, spelled by the caller.
pub struct Words<'a> {
    /// A label's name, from the caller's list of labels.
    pub label: &'a dyn Fn(LabelId) -> String,
    /// A server folder, given its path.
    pub folder: &'a dyn Fn(&str) -> String,
    /// A clause that names an account, which the line has no way to write.
    pub account: &'a str,
    /// A snooze that has come due, which `is:snoozed` does not say.
    pub snooze_due: &'a str,
    /// A date range with neither end.
    pub any_date: &'a str,
}

impl std::fmt::Debug for Words<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Words").finish_non_exhaustive()
    }
}

impl Filter {
    /// This filter in the search line's words, with `words` for what that line cannot say.
    ///
    /// A `Contains` needle with a space in it is quoted, as it must be to be read back as one
    /// term rather than as two.
    pub fn spelled(&self, words: &Words<'_>) -> String {
        let text = |m: &TextMatch| match m {
            TextMatch::Contains(s) if s.contains(char::is_whitespace) => format!("\"{s}\""),
            TextMatch::Contains(s) => s.clone(),
            TextMatch::Exact(s) => format!("\"{s}\""),
        };
        let each = |fs: &[Filter]| fs.iter().map(|f| f.spelled(words)).collect::<Vec<_>>();
        match self {
            Filter::All => "everything".to_owned(),
            Filter::Nothing => "nothing".to_owned(),
            Filter::And(fs) => each(fs).join(" "),
            // A group inside an either-or is parenthesised: the line has no way to say it, so it
            // does not read back, but it reads as what it is.
            Filter::Or(fs) => fs
                .iter()
                .map(|f| match f {
                    Filter::And(_) | Filter::Or(_) => format!("({})", f.spelled(words)),
                    _ => f.spelled(words),
                })
                .collect::<Vec<_>>()
                .join(" OR "),
            Filter::Not(f) => format!("-{}", f.spelled(words)),
            Filter::Account(_) => words.account.to_owned(),
            Filter::InMailbox(role) => format!("in:{role}"),
            Filter::InFolder(mailbox) => (words.folder)(&mailbox.path),
            Filter::Read(ReadState::Read) => "is:read".to_owned(),
            Filter::Read(ReadState::Unread) => "is:unread".to_owned(),
            Filter::Starred(Star::Starred) => "is:starred".to_owned(),
            Filter::Starred(Star::Unstarred) => "is:unstarred".to_owned(),
            Filter::HasLabel(id) => format!("label:{}", (words.label)(*id)),
            Filter::From(m) => format!("from:{}", text(m)),
            Filter::To(m) => format!("to:{}", text(m)),
            Filter::Subject(m) => format!("subject:{}", text(m)),
            Filter::Text(m) => text(m),
            Filter::Date(range) => dates(range, words.any_date),
            Filter::HasAttachment => "has:attachment".to_owned(),
            Filter::Snoozed => "is:snoozed".to_owned(),
            Filter::SnoozeDue => words.snooze_due.to_owned(),
            Filter::Pinned => "is:pinned".to_owned(),
            Filter::Larger(bytes) => format!("larger:{}", size(*bytes)),
            Filter::Smaller(bytes) => format!("smaller:{}", size(*bytes)),
            Filter::NewerThan(days) => format!("newer_than:{days}d"),
            Filter::OlderThan(days) => format!("older_than:{days}d"),
            Filter::Category(category) => format!("category:{}", category.name()),
        }
    }
}

/// A size as the line writes it: whole mebibytes as `M`, whole kibibytes as `K`, else bytes,
/// the units [`crate::human_size`] shows.
fn size(bytes: u64) -> String {
    const K: u64 = 1024;
    const M: u64 = K * K;
    match bytes {
        0 => "0".to_owned(),
        b if b % M == 0 => format!("{}M", b / M),
        b if b % K == 0 => format!("{}K", b / K),
        b => b.to_string(),
    }
}

fn dates(range: &DateRange, any: &str) -> String {
    let day = |d: &chrono::DateTime<chrono::Utc>| d.format("%Y-%m-%d").to_string();
    match (&range.from, &range.to) {
        (Some(from), Some(to)) => format!("after:{} before:{}", day(from), day(to)),
        (Some(from), None) => format!("after:{}", day(from)),
        (None, Some(to)) => format!("before:{}", day(to)),
        (None, None) => any.to_owned(),
    }
}
