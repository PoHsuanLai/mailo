//! What the message list says when it has nothing to list.
//!
//! A function of the links, not stored state: "still fetching" and "cannot connect" are both an
//! empty list, and which one it is decides whether the person waits or acts.

use super::need::Need;
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
    /// The quick filter's toggles left nothing.
    Filtered,
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
    /// if anything, to do about it. `account` is the one a sign-in is asked for.
    pub fn description(self, account: &str) -> String {
        match self {
            Problem::Unreachable => "Can\u{2019}t reach the server. Retrying.".to_owned(),
            Problem::Broken => {
                "Something is wrong with this account. Check its settings.".to_owned()
            }
            Problem::SignIn | Problem::Allow => {
                format!("Sign in to {account} again to load mail.")
            }
        }
    }

    /// What the person is to give, when that is the problem: the pane offers it as a button.
    pub fn need(self) -> Option<Need> {
        match self {
            Problem::SignIn => Some(Need::SignIn),
            Problem::Allow => Some(Need::Allow),
            Problem::Unreachable | Problem::Broken => None,
        }
    }
}

/// The title of the pane when nothing can be loaded.
pub const CANNOT_LOAD: &str = "Couldn\u{2019}t load this mailbox";

/// What the list pane shows, given its rows, the links of the accounts in scope and why it is
/// empty if it is.
pub fn list_face(has_rows: HasRows, links: &[&Link], nothing: &Nothing) -> ListFace {
    match (has_rows, nothing) {
        (HasRows::Yes, _) => ListFace::Rows,
        (HasRows::No, Nothing::NoAccount) => ListFace::NoAccount,
        (HasRows::No, Nothing::NoMatch(words)) => ListFace::NoMatch(words.clone()),
        (HasRows::No, Nothing::Filtered) => ListFace::Filtered,
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
    fn only_a_sign_in_is_the_persons_to_give_and_it_names_the_account() {
        let cases = [
            (
                Problem::Unreachable,
                None,
                "Can\u{2019}t reach the server. Retrying.",
            ),
            (
                Problem::Broken,
                None,
                "Something is wrong with this account. Check its settings.",
            ),
            (
                Problem::SignIn,
                Some(Need::SignIn),
                "Sign in to me@x.test again to load mail.",
            ),
            (
                Problem::Allow,
                Some(Need::Allow),
                "Sign in to me@x.test again to load mail.",
            ),
        ];
        for (problem, need, said) in cases {
            assert_eq!(problem.need(), need, "{problem:?}");
            assert_eq!(problem.description("me@x.test"), said, "{problem:?}");
        }
    }

    /// A first fetch is not an empty folder, every account stuck is a cannot-load with the worst
    /// reason, and one healthy account (or one only throttled) makes it an empty folder.
    #[test]
    fn the_list_is_what_the_accounts_are_doing() {
        let throttled = Link::Waiting {
            until: t(),
            why: Pause::Throttled,
            failures: 1,
            first: First::No,
        };
        let cases = [
            (vec![Link::Fresh], ListFace::FirstSync),
            (vec![syncing(First::Yes)], ListFace::FirstSync),
            (vec![current(), Link::Fresh], ListFace::FirstSync),
            (vec![syncing(First::No)], ListFace::Empty),
            (vec![signin()], ListFace::CannotLoad(Problem::SignIn)),
            (vec![broken()], ListFace::CannotLoad(Problem::Broken)),
            (vec![offline()], ListFace::CannotLoad(Problem::Unreachable)),
            (
                vec![offline(), broken()],
                ListFace::CannotLoad(Problem::Broken),
            ),
            (
                vec![offline(), broken(), signin()],
                ListFace::CannotLoad(Problem::SignIn),
            ),
            (vec![current(), signin()], ListFace::Empty),
            (vec![current()], ListFace::Empty),
            (vec![], ListFace::Empty),
            (vec![throttled], ListFace::Empty),
        ];
        for (links, want) in cases {
            assert_eq!(face(&links), want, "{links:?}");
        }
    }
}
