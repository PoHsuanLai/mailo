//! What the message list says when it has nothing to list.
//!
//! A function of the links, not stored state: "still fetching" and "cannot connect" are both an
//! empty list, and which one it is decides whether the person waits or acts.

use crate::ui::view::Nothing;
use mail_core::fetch::{First, Link, Pause};

/// Whether the list has any rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HasRows {
    Yes,
    No,
}

/// What the list pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListFace {
    /// The rows.
    Rows,
    /// Nothing yet, because nothing has been fetched yet.
    FirstSync,
    /// A folder with no mail in it.
    Empty,
    /// A search that matched nothing, with the words.
    NoMatch(String),
    /// No account has been added.
    NoAccount,
    /// Nothing, and the reason is that no account can be reached.
    CannotLoad(Problem),
}

/// Why accounts cannot be reached, worst first when ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Problem {
    Unreachable,
    Broken,
    SignIn,
    Allow,
}

impl Problem {
    /// What the pane says under the title when nothing can be loaded: what is wrong and what,
    /// if anything, to do about it.
    pub fn description(self) -> &'static str {
        match self {
            Problem::Unreachable => {
                "Mailo can\u{2019}t reach the server right now. It will keep trying."
            }
            Problem::Broken => "Something is wrong with this account. Check its settings.",
            Problem::SignIn => "Sign in again to load your mail.",
            Problem::Allow => "Allow Mail to use this account again to load your mail.",
        }
    }
}

/// The title of the pane when nothing can be loaded.
pub const CANNOT_LOAD: &str = "Couldn\u{2019}t load this mailbox";

/// The caption under the placeholder rows while the first mail is on its way.
pub const FIRST_SYNC: &str = "Downloading your mail\u{2026}";

/// What the list pane shows, given its rows, the links of the accounts in scope and why it is
/// empty if it is.
pub fn list_face(has_rows: HasRows, links: &[&Link], nothing: &Nothing) -> ListFace {
    match (has_rows, nothing) {
        (HasRows::Yes, _) => ListFace::Rows,
        (HasRows::No, Nothing::NoAccount) => ListFace::NoAccount,
        (HasRows::No, Nothing::NoMatch(words)) => ListFace::NoMatch(words.clone()),
        (HasRows::No, Nothing::EmptyFolder) => {
            if links.iter().any(|link| still_filling(link)) {
                ListFace::FirstSync
            } else {
                cannot_load(links).map_or(ListFace::Empty, ListFace::CannotLoad)
            }
        }
    }
}

fn still_filling(link: &Link) -> bool {
    matches!(
        link,
        Link::Fresh
            | Link::Syncing {
                first: First::Yes,
                ..
            }
    )
}

/// The worst problem, when every link has one.
fn cannot_load(links: &[&Link]) -> Option<Problem> {
    links
        .iter()
        .map(|link| problem(link))
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .max()
}

fn problem(link: &Link) -> Option<Problem> {
    match link {
        Link::NeedsSignIn { .. } => Some(Problem::SignIn),
        Link::NeedsAllow { .. } => Some(Problem::Allow),
        Link::Broken { .. } => Some(Problem::Broken),
        Link::Waiting {
            why: Pause::Unreachable,
            ..
        } => Some(Problem::Unreachable),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use mail_core::fetch::{Live, Step};

    fn t() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap()
    }

    fn current() -> Link {
        Link::Current {
            at: t(),
            trouble: vec![],
            live: Live::Polling,
        }
    }

    fn syncing(first: First) -> Link {
        Link::Syncing {
            first,
            step: Step::Folders,
            count: None,
            after: Box::new(Link::Fresh),
        }
    }

    fn signin() -> Link {
        Link::NeedsSignIn {
            why: "x".into(),
            first: First::No,
        }
    }

    fn broken() -> Link {
        Link::Broken {
            why: "x".into(),
            first: First::No,
        }
    }

    fn offline() -> Link {
        Link::Waiting {
            until: t(),
            why: Pause::Unreachable,
            failures: 1,
            first: First::No,
        }
    }

    fn face(links: &[Link]) -> ListFace {
        let refs: Vec<&Link> = links.iter().collect();
        list_face(HasRows::No, &refs, &Nothing::EmptyFolder)
    }

    #[test]
    fn rows_always_win() {
        for link in [Link::Fresh, signin(), broken(), offline()] {
            for nothing in [
                Nothing::NoAccount,
                Nothing::NoMatch("a".into()),
                Nothing::EmptyFolder,
            ] {
                assert_eq!(list_face(HasRows::Yes, &[&link], &nothing), ListFace::Rows);
            }
        }
    }

    #[test]
    fn search_and_setup_come_before_fetching() {
        let fresh = Link::Fresh;
        assert_eq!(
            list_face(HasRows::No, &[&fresh], &Nothing::NoAccount),
            ListFace::NoAccount
        );
        assert_eq!(
            list_face(HasRows::No, &[&fresh], &Nothing::NoMatch("tax".into())),
            ListFace::NoMatch("tax".into())
        );
    }

    #[test]
    fn a_first_fetch_is_not_an_empty_folder() {
        assert_eq!(face(&[Link::Fresh]), ListFace::FirstSync);
        assert_eq!(face(&[syncing(First::Yes)]), ListFace::FirstSync);
        assert_eq!(face(&[current(), Link::Fresh]), ListFace::FirstSync);
        assert_eq!(face(&[syncing(First::No)]), ListFace::Empty);
    }

    #[test]
    fn every_account_stuck_is_a_cannot_load_with_the_worst_reason() {
        let cases = [
            (vec![signin()], Problem::SignIn),
            (vec![broken()], Problem::Broken),
            (vec![offline()], Problem::Unreachable),
            (vec![offline(), broken()], Problem::Broken),
            (vec![offline(), broken(), signin()], Problem::SignIn),
        ];
        for (links, want) in cases {
            assert_eq!(face(&links), ListFace::CannotLoad(want), "{links:?}");
        }
    }

    #[test]
    fn one_healthy_account_makes_it_an_empty_folder() {
        assert_eq!(face(&[current(), signin()]), ListFace::Empty);
        assert_eq!(face(&[current()]), ListFace::Empty);
        assert_eq!(face(&[]), ListFace::Empty);
        let throttled = Link::Waiting {
            until: t(),
            why: Pause::Throttled,
            failures: 1,
            first: First::No,
        };
        assert_eq!(face(&[throttled]), ListFace::Empty);
    }
}
