//! The command line over the store.
//!
//! Exists before the Dioxus shell because it is testable: `plan.md` phase 4 asks for "a tiny
//! CLI can list, open and reply", and a CLI can be driven from a test where a window cannot.
//! The UI will call the same `Store` methods.

use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::fmt::Write as _;

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Threads in a mailbox, newest first.
    List { mailbox: MailboxRole, limit: u32 },
    /// One thread and its messages.
    Show { thread: ThreadId },
    /// Full-text search across every account.
    Search { needle: String, limit: u32 },
    /// Unread counts per mailbox.
    Status,
    /// Configure an account from its address, using the preset table.
    AccountAdd { address: String },
    /// Configured accounts, and what each still needs.
    AccountList,
    /// Fetch mail for every configured account, and drain the outbox.
    Sync,
    /// Start a reply to a message. The body is read from stdin.
    Reply {
        message: MessageId,
        scope: ReplyScope,
        body: String,
    },
    /// Queue a draft for delivery on the next sync.
    Send { draft: DraftId },
    /// Every draft, and where it got to.
    Drafts,
}

/// Parse arguments, or explain what was wrong.
///
/// Returns `Err` with usage rather than exiting, so the parser is testable and the binary owns
/// the only call to `std::process::exit`.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let Some(verb) = args.first().map(String::as_str) else {
        return Err(usage());
    };
    match verb {
        "list" => {
            let mailbox = match args.get(1).map(String::as_str) {
                None | Some("inbox") => MailboxRole::Inbox,
                Some("archive") => MailboxRole::Archive,
                Some("sent") => MailboxRole::Sent,
                Some("drafts") => MailboxRole::Drafts,
                Some("trash") => MailboxRole::Trash,
                Some("spam") => MailboxRole::Spam,
                Some(other) => return Err(format!("unknown mailbox {other:?}\n\n{}", usage())),
            };
            Ok(Command::List {
                mailbox,
                limit: parse_limit(args.get(2))?,
            })
        }
        "show" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("show needs a thread id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a thread id"))?;
            Ok(Command::Show {
                thread: ThreadId::from_uuid(uuid),
            })
        }
        "search" => {
            let needle = args
                .get(1..)
                .filter(|rest| !rest.is_empty())
                .map(|rest| rest.join(" "))
                .ok_or_else(|| format!("search needs something to look for\n\n{}", usage()))?;
            Ok(Command::Search { needle, limit: 20 })
        }
        "reply" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("reply needs a message id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a message id"))?;
            // `--all` rather than a separate verb: it is the same operation with a wider
            // audience, and two verbs would be two code paths for one question.
            let scope = match args.get(2).map(String::as_str) {
                None => ReplyScope::Sender,
                Some("--all") => ReplyScope::All,
                Some(other) => return Err(format!("unknown option {other:?}\n\n{}", usage())),
            };
            Ok(Command::Reply {
                message: MessageId::from_uuid(uuid),
                scope,
                // Filled in by the caller, which owns stdin. Parsing stays pure.
                body: String::new(),
            })
        }
        "send" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("send needs a draft id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a draft id"))?;
            Ok(Command::Send {
                draft: DraftId::from_uuid(uuid),
            })
        }
        "drafts" => Ok(Command::Drafts),
        "status" => Ok(Command::Status),
        "sync" => Ok(Command::Sync),
        "account" => match args.get(1).map(String::as_str) {
            Some("add") => {
                let address = args
                    .get(2)
                    .ok_or_else(|| format!("account add needs an address\n\n{}", usage()))?;
                if !address.contains('@') {
                    return Err(format!("{address:?} is not an email address"));
                }
                Ok(Command::AccountAdd {
                    address: address.clone(),
                })
            }
            None | Some("list") => Ok(Command::AccountList),
            Some(other) => Err(format!("unknown account command {other:?}\n\n{}", usage())),
        },
        other => Err(format!("unknown command {other:?}\n\n{}", usage())),
    }
}

fn parse_limit(raw: Option<&String>) -> Result<u32, String> {
    match raw {
        None => Ok(20),
        Some(text) => text
            .parse()
            .map_err(|_| format!("{text:?} is not a number of rows")),
    }
}

pub fn usage() -> String {
    "\
usage: mailo <command>

  list [inbox|archive|sent|drafts|trash|spam] [limit]
  show <thread-id>
  search <words...>
  reply <message-id> [--all]  compose a reply; the body is read from stdin
  send <draft-id>             queue a draft for the next sync
  drafts                      drafts and where each one got to
  status
  account [list]
  account add <address>      (set MAILO_PASSWORD for a password account)
  sync                       fetch mail and send anything queued
"
    .to_owned()
}

/// Run a command against the store, returning what to print.
///
/// Returns a `String` rather than printing, so tests assert on output instead of capturing
/// stdout.
pub fn run(store: &SqliteStore, command: &Command, now: DateTime<Utc>) -> Result<String, String> {
    match command {
        Command::List { mailbox, limit } => {
            let page = store
                .threads(&list_query(Filter::InMailbox(*mailbox), *limit), now)
                .map_err(|e| e.to_string())?;
            if page.items.is_empty() {
                return Ok(format!("no threads in {}\n", role_name(*mailbox)));
            }
            Ok(render_list(&page.items))
        }
        Command::Show { thread } => {
            let loaded = store.thread(*thread).map_err(|e| e.to_string())?;
            let mut out = format!("{}\n", loaded.summary.subject);
            let _ = writeln!(
                out,
                "{} message(s), {}",
                loaded.summary.message_count,
                if loaded.summary.read == ReadState::Unread {
                    "unread"
                } else {
                    "read"
                }
            );
            for id in &loaded.messages {
                let message = store.message(*id).map_err(|e| e.to_string())?;
                let _ = writeln!(
                    out,
                    "\n--- {} <{}>  {}",
                    message.from.name.as_deref().unwrap_or(""),
                    message.from.email,
                    message.date.format("%Y-%m-%d %H:%M")
                );
                match message.body.text() {
                    Some(text) => {
                        let _ = writeln!(out, "{}", text.trim_end());
                    }
                    // Normal during a first sync, not an error: headers arrive before bodies.
                    None => {
                        let _ = writeln!(out, "(body not fetched yet)");
                    }
                }
            }
            Ok(out)
        }
        Command::Search { needle, limit } => {
            let page = store
                .threads(
                    &list_query(Filter::Text(TextMatch::Contains(needle.clone())), *limit),
                    now,
                )
                .map_err(|e| e.to_string())?;
            if page.items.is_empty() {
                return Ok(format!("nothing matches {needle:?}\n"));
            }
            Ok(render_list(&page.items))
        }
        // Dispatched in main: it needs an async runtime and the store by Arc, which would make
        // this function untestable without one.
        Command::Sync => Err("sync is dispatched before this point".to_owned()),
        Command::Reply {
            message,
            scope,
            body,
        } => crate::compose::reply(store, *message, *scope, body, now),
        Command::Send { draft } => crate::compose::send(store, *draft, now),
        Command::Drafts => crate::compose::drafts(store),
        Command::AccountAdd { address } => crate::account::add(store, address, now),
        Command::AccountList => crate::account::list(store),
        Command::Status => {
            let mut out = String::new();
            for role in MailboxRole::ALL {
                let total = store
                    .count(&Filter::InMailbox(role), now)
                    .map_err(|e| e.to_string())?;
                let unread = store
                    .count(
                        &Filter::And(vec![
                            Filter::InMailbox(role),
                            Filter::Read(ReadState::Unread),
                        ]),
                        now,
                    )
                    .map_err(|e| e.to_string())?;
                if total > 0 {
                    let _ = writeln!(
                        out,
                        "{:<8} {:>5} total  {:>5} unread",
                        role_name(role),
                        total,
                        unread
                    );
                }
            }
            if out.is_empty() {
                out.push_str("no mail yet\n");
            }
            Ok(out)
        }
    }
}

fn list_query(filter: Filter, limit: u32) -> Query {
    Query {
        filter,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq { after: None, limit },
    }
}

fn render_list(items: &[ThreadSummary]) -> String {
    let mut out = String::new();
    for summary in items {
        let _ = writeln!(
            out,
            "{} {} {:<28.28} {:<40.40} {}",
            if summary.read == ReadState::Unread {
                "*"
            } else {
                " "
            },
            summary.last_date.format("%m-%d %H:%M"),
            summary.from.name.as_deref().unwrap_or(&summary.from.email),
            summary.subject,
            summary.id
        );
    }
    out
}

fn role_name(role: MailboxRole) -> &'static str {
    match role {
        MailboxRole::Inbox => "inbox",
        MailboxRole::Archive => "archive",
        MailboxRole::Sent => "sent",
        MailboxRole::Drafts => "drafts",
        MailboxRole::Trash => "trash",
        MailboxRole::Spam => "spam",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn list_defaults_to_the_inbox() {
        assert_eq!(
            parse(&args(&["list"])).unwrap(),
            Command::List {
                mailbox: MailboxRole::Inbox,
                limit: 20
            }
        );
    }

    #[test]
    fn search_joins_its_words() {
        // So `mailo search lunch on friday` is one phrase rather than a parse error.
        assert_eq!(
            parse(&args(&["search", "lunch", "on", "friday"])).unwrap(),
            Command::Search {
                needle: "lunch on friday".to_owned(),
                limit: 20
            }
        );
    }

    #[test]
    fn bad_input_explains_itself_rather_than_panicking() {
        for bad in [
            vec![],
            args(&["wat"]),
            args(&["list", "nowhere"]),
            args(&["show"]),
            args(&["show", "not-a-uuid"]),
            args(&["list", "inbox", "many"]),
            args(&["search"]),
        ] {
            let err = parse(&bad).expect_err("should be rejected");
            assert!(!err.is_empty(), "an error must say something");
        }
    }
}
