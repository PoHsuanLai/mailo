//! A search typed here, asked of a server: IMAP `UID SEARCH`, JMAP `Email/query`, Microsoft
//! Graph `$search` and `$filter`.
//!
//! The store answers a search over what it holds. Mail it never fetched — older than a sync has
//! reached, in a folder nobody follows, on Gmail anywhere but the inbox and Sent — only the server
//! can find. Each protocol here turns the one [`Filter`] a search line parses to into what that
//! protocol can be asked, clause for clause, and names every clause it cannot ask faithfully in an
//! [`Unsaid`] instead of dropping it: a dropped clause widens the search, and a wider answer
//! shown as the answer is a wrong one.
//!
//! What "faithfully" can mean has a floor each protocol sets. How a server matches a string
//! within a field is the server's: IMAP matches substrings (RFC 3501 §6.4.4), JMAP leaves it
//! undefined (RFC 8621 §4.4.1), and Graph's KQL matches words and prefixes. What is never done is
//! to leave out a clause, or to search a wider field than the one the clause names. Dates are the
//! one place a bound moves: a protocol whose dates are whole days gets the days the reader named
//! (see [`whole_days`]).
//!
//! Pure: values in, values out. Sending what these build is `mail-runtime`'s.

pub mod graph;
pub mod imap;
pub mod jmap;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use mail_domain::{DateRange, Filter, MailboxRole, ReadState, Star, TextMatch};
use porter_core::AccountId;

/// What of a query cannot be asked of a server faithfully, each as the words a person typed or
/// would have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsaid(pub Vec<String>);

impl Unsaid {
    fn one(what: String) -> Self {
        Unsaid(vec![what])
    }

    /// The clauses together: an error over two clauses names both.
    fn join(parts: impl IntoIterator<Item = Unsaid>) -> Self {
        Unsaid(parts.into_iter().flat_map(|u| u.0).collect())
    }
}

impl std::fmt::Display for Unsaid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.join(", "))
    }
}

/// A query ready for a server, or an answer known without asking one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked<T> {
    Ask(T),
    /// The query matches nothing on this account: another account's, or text with no words.
    /// There is nothing to send.
    Nothing,
}

/// Every child, or the first failure of each child that failed.
fn each<T>(parts: &[Filter], one: impl Fn(&Filter) -> Result<T, Unsaid>) -> Result<Vec<T>, Unsaid> {
    let (done, failed): (Vec<_>, Vec<_>) = parts.iter().map(one).partition(Result::is_ok);
    if failed.is_empty() {
        Ok(done.into_iter().filter_map(Result::ok).collect())
    } else {
        Err(Unsaid::join(failed.into_iter().filter_map(Result::err)))
    }
}

/// `filter` as it reads on one account: an `Account` clause is true there or false, and `All`
/// and `Nothing` are folded away wherever they decide the clause holding them.
///
/// A server search is of one account, so an `Account` clause is never sent; what it says is
/// already known.
pub fn on_account(filter: &Filter, account: AccountId) -> Filter {
    match filter {
        Filter::Account(id) if *id == account => Filter::All,
        Filter::Account(_) => Filter::Nothing,
        Filter::And(parts) => {
            let mut kept = Vec::new();
            for part in parts.iter().map(|p| on_account(p, account.clone())) {
                match part {
                    Filter::All => {}
                    Filter::Nothing => return Filter::Nothing,
                    Filter::And(inner) => kept.extend(inner),
                    other => kept.push(other),
                }
            }
            match kept.len() {
                0 => Filter::All,
                1 => kept.remove(0),
                _ => Filter::And(kept),
            }
        }
        Filter::Or(parts) => {
            let mut kept = Vec::new();
            for part in parts.iter().map(|p| on_account(p, account.clone())) {
                match part {
                    Filter::Nothing => {}
                    Filter::All => return Filter::All,
                    other => kept.push(other),
                }
            }
            match kept.len() {
                0 => Filter::Nothing,
                1 => kept.remove(0),
                _ => Filter::Or(kept),
            }
        }
        Filter::Not(inner) => match on_account(inner, account) {
            Filter::All => Filter::Nothing,
            Filter::Nothing => Filter::All,
            other => Filter::Not(Box::new(other)),
        },
        // Text with no words matches nothing (`Filter::Text`): said here once, for every protocol.
        Filter::Text(TextMatch::Contains(needle) | TextMatch::Exact(needle))
            if mail_domain::filter::search_tokens(needle).is_empty() =>
        {
            Filter::Nothing
        }
        other => other.clone(),
    }
}

/// The top-level clauses of `filter`, which a server takes joined by AND.
fn clauses(filter: &Filter) -> Vec<Filter> {
    match filter {
        Filter::All => Vec::new(),
        Filter::And(parts) => parts.clone(),
        other => vec![other.clone()],
    }
}

/// Where a search runs, when its query names one place at the top: `in:sent`, or a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Role(MailboxRole),
    Folder(String),
}

/// The top-level clauses, less the one that names a place, and that place.
///
/// Only a place every match must be in can choose where to search. One under `Or` or `Not`, or
/// two different places at once, cannot: that is returned as unsaid.
fn place_of(filter: &Filter) -> Result<(Option<Place>, Vec<Filter>), Unsaid> {
    let mut place = None;
    let mut rest = Vec::new();
    for clause in clauses(filter) {
        let here = match &clause {
            Filter::InMailbox(role) => Place::Role(*role),
            Filter::InFolder(mailbox) => Place::Folder(mailbox.path.clone()),
            _ => {
                rest.push(clause);
                continue;
            }
        };
        match &place {
            Some(was) if *was != here => {
                return Err(Unsaid::one(format!(
                    "{} and {} at once",
                    describe(&clause),
                    place_word(was)
                )));
            }
            _ => place = Some(here),
        }
    }
    Ok((place, rest))
}

fn place_word(place: &Place) -> String {
    match place {
        Place::Role(role) => format!("in:{}", role_word(*role)),
        Place::Folder(path) => format!("the folder {path}"),
    }
}

fn role_word(role: MailboxRole) -> &'static str {
    match role {
        MailboxRole::Inbox => "inbox",
        MailboxRole::Archive => "archive",
        MailboxRole::Sent => "sent",
        MailboxRole::Drafts => "drafts",
        MailboxRole::Trash => "trash",
        MailboxRole::Spam => "spam",
    }
}

/// A clause as the search line spells it, for saying what could not be asked.
pub fn describe(filter: &Filter) -> String {
    let text = |m: &TextMatch| match m {
        TextMatch::Contains(s) => s.clone(),
        TextMatch::Exact(s) => format!("\"{s}\""),
    };
    match filter {
        Filter::All => "everything".to_owned(),
        Filter::Nothing => "nothing".to_owned(),
        Filter::And(parts) => parts.iter().map(describe).collect::<Vec<_>>().join(" "),
        Filter::Or(parts) => format!(
            "({})",
            parts.iter().map(describe).collect::<Vec<_>>().join(" or ")
        ),
        Filter::Not(inner) => format!("-{}", describe(inner)),
        Filter::Account(_) => "an account".to_owned(),
        Filter::InMailbox(role) => format!("in:{}", role_word(*role)),
        Filter::Read(ReadState::Read) => "is:read".to_owned(),
        Filter::Read(ReadState::Unread) => "is:unread".to_owned(),
        Filter::Starred(Star::Starred) => "is:starred".to_owned(),
        Filter::Starred(Star::Unstarred) => "is:unstarred".to_owned(),
        Filter::HasLabel(_) => "label:".to_owned(),
        Filter::InFolder(mailbox) => format!("the folder {}", mailbox.path),
        Filter::From(m) => format!("from:{}", text(m)),
        Filter::To(m) => format!("to:{}", text(m)),
        Filter::Subject(m) => format!("subject:{}", text(m)),
        Filter::Text(m) => text(m),
        Filter::Date(range) => describe_dates(range),
        Filter::HasAttachment => "has:attachment".to_owned(),
        Filter::Snoozed => "is:snoozed".to_owned(),
        Filter::SnoozeDue => "a snooze that is due".to_owned(),
        Filter::Pinned => "is:pinned".to_owned(),
    }
}

fn describe_dates(range: &DateRange) -> String {
    let day = |d: &DateTime<Utc>| d.format("%Y-%m-%d").to_string();
    match (&range.from, &range.to) {
        (Some(from), Some(to)) => format!("after:{} before:{}", day(from), day(to)),
        (Some(from), None) => format!("after:{}", day(from)),
        (None, Some(to)) => format!("before:{}", day(to)),
        (None, None) => "any date".to_owned(),
    }
}

/// Why a clause is kept on this computer: said once, in the words every protocol uses.
fn kept_here(filter: &Filter) -> Unsaid {
    Unsaid::one(format!("{} (kept on this computer)", describe(filter)))
}

/// The days a range names, as `[since, before)`, for a server whose dates are whole days.
///
/// Each bound goes to its nearest midnight. A bound this client made is the reader's own
/// midnight (`before:` and `after:` are resolved in their zone), so the nearest UTC midnight is
/// the date they typed wherever they are within twelve hours of UTC — which is also the date a
/// day-grained server compares, since IMAP's `SENTSINCE` reads the `Date` header's own calendar
/// day and ignores its zone (RFC 3501 §6.4.4). Moving each bound inward instead would turn
/// `after:2026-01-10 before:2026-01-11`, typed east of Greenwich, into no day at all.
///
/// `None` for an end the range leaves open.
fn whole_days(range: &DateRange) -> (Option<NaiveDate>, Option<NaiveDate>) {
    let nearest = |at: DateTime<Utc>| {
        let day = at.date_naive();
        if at.time() >= chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap_or(chrono::NaiveTime::MIN)
        {
            day.succ_opt().unwrap_or(day)
        } else {
            day
        }
    };
    (range.from.map(nearest), range.to.map(nearest))
}

/// `date` as IMAP's `date` (RFC 3501 §9: `d-Mon-yyyy`), in English whatever the locale.
fn imap_date(date: NaiveDate) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{}-{}-{}",
        date.day(),
        MONTHS[date.month0() as usize],
        date.year()
    )
}

/// The name `labels` gives `label`.
fn named(labels: &[(mail_domain::LabelId, String)], label: mail_domain::LabelId) -> Option<&str> {
    labels
        .iter()
        .find(|(id, _)| *id == label)
        .map(|(_, name)| name.as_str())
}

/// The words of a free-text needle, as typed: each must be found.
fn words(needle: &str) -> Vec<String> {
    needle.split_whitespace().map(str::to_owned).collect()
}

#[cfg(test)]
mod tests;
