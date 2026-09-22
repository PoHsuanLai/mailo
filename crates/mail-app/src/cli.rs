//! The command line over the store.
//!
//! Exists before the Dioxus shell because it is testable: `plan.md` phase 4 asks for "a tiny
//! CLI can list, open and reply", and a CLI can be driven from a test where a window cannot.
//! The UI will call the same `Store` methods.

use crate::view::Stamp;
use chrono::{DateTime, Local, Utc};
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
    /// Configure an account from its address.
    ///
    /// `manual` is `Some` when the user named the servers themselves, which is the only way to
    /// reach a host the preset table has never heard of.
    AccountAdd {
        address: String,
        manual: Option<mail_domain::presets::Manual>,
        /// The address belongs to a managed Microsoft 365 tenant on its own domain, which is
        /// the one thing the preset table cannot work out for itself.
        microsoft: bool,
    },
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
    /// Delete a draft.
    Discard { draft: DraftId },
    /// Pin a conversation, or unpin it if it is already pinned.
    Pin { thread: ThreadId },
    /// Conversations kept in view.
    ListPinned { limit: u32 },
    /// Conversations that are put off, and not yet due.
    ListSnoozed { limit: u32 },
    /// Put a conversation off until later.
    Snooze { thread: ThreadId, when: String },
    /// Bring a snoozed conversation back now.
    Wake { thread: ThreadId },
    /// The attachments on a message.
    Attachments { message: MessageId },
    /// Write one attachment to a directory.
    Save {
        message: MessageId,
        index: usize,
        dir: std::path::PathBuf,
    },
    /// Forward a message. The covering note is read from stdin.
    ///
    /// Recipients are given on the command line because a forward has none of its own: nothing
    /// in the original says who it should go to next, and guessing would be inventing one.
    Forward {
        message: MessageId,
        to: Vec<Address>,
        body: String,
    },
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
            // Not a `MailboxRole`: "snoozed" is a predicate over threads rather than a folder
            // any of them is in, which is what `filter.rs` means by predicates over mirrors.
            if matches!(args.get(1).map(String::as_str), Some("snoozed")) {
                return Ok(Command::ListSnoozed {
                    limit: parse_limit(args.get(2))?,
                });
            }
            if matches!(args.get(1).map(String::as_str), Some("pinned")) {
                return Ok(Command::ListPinned {
                    limit: parse_limit(args.get(2))?,
                });
            }
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
        "forward" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("forward needs a message id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a message id"))?;
            let to = match (args.get(2).map(String::as_str), args.get(3)) {
                (Some("--to"), Some(list)) => crate::view::parse_addresses(list)?,
                (Some("--to"), None) => {
                    return Err(format!("--to needs an address\n\n{}", usage()));
                }
                (None, _) => {
                    // A forward with nobody to send it to is the F99 dead end again: a draft the
                    // CLI cannot finish and no command can repair.
                    return Err(format!(
                        "forward needs recipients: mailo forward {raw} --to someone@example.com\n\n{}",
                        usage()
                    ));
                }
                (Some(other), _) => return Err(format!("unknown option {other:?}\n\n{}", usage())),
            };
            if to.is_empty() {
                return Err("--to had no addresses in it".to_owned());
            }
            Ok(Command::Forward {
                message: MessageId::from_uuid(uuid),
                to,
                // Filled in by the caller, which owns stdin. Parsing stays pure.
                body: String::new(),
            })
        }
        "pin" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("pin needs a thread id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a thread id"))?;
            Ok(Command::Pin {
                thread: ThreadId::from_uuid(uuid),
            })
        }
        "snooze" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("snooze needs a thread id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a thread id"))?;
            // The rest of the line, joined: "next tuesday" and "2026-09-25 14:30" are two words
            // and quoting them would be a thing to remember for no reason.
            let when = args[2..].join(" ");
            if when.trim().is_empty() {
                return Err(format!(
                    "snooze needs a time: mailo snooze {raw} tomorrow\n\n{}",
                    usage()
                ));
            }
            Ok(Command::Snooze {
                thread: ThreadId::from_uuid(uuid),
                when,
            })
        }
        "wake" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("wake needs a thread id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a thread id"))?;
            Ok(Command::Wake {
                thread: ThreadId::from_uuid(uuid),
            })
        }
        "attachments" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("attachments needs a message id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a message id"))?;
            Ok(Command::Attachments {
                message: MessageId::from_uuid(uuid),
            })
        }
        "save" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("save needs a message id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a message id"))?;
            let index = args
                .get(2)
                .ok_or_else(|| format!("save needs the attachment's number\n\n{}", usage()))?
                .parse::<usize>()
                .map_err(|_| {
                    "the attachment's number comes from `mailo attachments <message-id>`".to_owned()
                })?;
            // The current directory by default, which is where someone running a command
            // expects a file to land.
            let dir = args
                .get(3)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            Ok(Command::Save {
                message: MessageId::from_uuid(uuid),
                index,
                dir,
            })
        }
        "drafts" => Ok(Command::Drafts),
        "discard" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("discard needs a draft id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a draft id"))?;
            Ok(Command::Discard {
                draft: DraftId::from_uuid(uuid),
            })
        }
        "status" => Ok(Command::Status),
        "sync" => Ok(Command::Sync),
        "account" => match args.get(1).map(String::as_str) {
            Some("add") => {
                let address = args
                    .get(2)
                    .ok_or_else(|| format!("account add needs an address\n\n{}", usage()))?;
                let rest = &args[3..];
                let microsoft = rest.iter().any(|a| a == "--microsoft");
                let others: Vec<String> = rest
                    .iter()
                    .filter(|a| *a != "--microsoft")
                    .cloned()
                    .collect();
                if microsoft && !others.is_empty() {
                    return Err(format!(
                        "--microsoft already knows the servers; drop the other options\n\n{}",
                        usage()
                    ));
                }
                Ok(Command::AccountAdd {
                    address: address.clone(),
                    manual: parse_manual(&others)?,
                    microsoft,
                })
            }
            None | Some("list") => Ok(Command::AccountList),
            Some(other) => Err(format!("unknown account command {other:?}\n\n{}", usage())),
        },
        other => Err(format!("unknown command {other:?}\n\n{}", usage())),
    }
}

/// `--imap HOST[:PORT] --smtp HOST[:PORT] [--login NAME]`, or `None` when none were given.
///
/// All or nothing: naming only the incoming server would leave the account unable to send, and
/// silently defaulting the other half to a guessed hostname is how mail goes to a server the
/// user never chose.
fn parse_manual(args: &[String]) -> Result<Option<mail_domain::presets::Manual>, String> {
    if args.is_empty() {
        return Ok(None);
    }
    let (mut imap, mut smtp, mut login) = (None, None, None);
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let value = rest
            .next()
            .ok_or_else(|| format!("{flag} needs a value\n\n{}", usage()))?;
        match flag.as_str() {
            "--imap" => imap = Some(host_port(value, 993)?),
            "--smtp" => smtp = Some(host_port(value, 465)?),
            "--login" => login = Some(value.clone()),
            other => return Err(format!("unknown option {other:?}\n\n{}", usage())),
        }
    }
    match (imap, smtp) {
        (Some((imap_host, imap_port)), Some((smtp_host, smtp_port))) => {
            Ok(Some(mail_domain::presets::Manual {
                imap_host,
                imap_port,
                smtp_host,
                smtp_port,
                login,
            }))
        }
        (None, None) => Err(format!(
            "--login needs --imap and --smtp too\n\n{}",
            usage()
        )),
        _ => Err(format!(
            "manual setup needs both --imap and --smtp; an account that can \
             only receive is not one this can configure\n\n{}",
            usage()
        )),
    }
}

/// `host` or `host:port`, with `default` when no port is given.
fn host_port(raw: &str, default: u16) -> Result<(String, u16), String> {
    match raw.rsplit_once(':') {
        Some((host, port)) => {
            let port: u16 = port
                .parse()
                .map_err(|_| format!("{port:?} is not a port number"))?;
            if host.is_empty() {
                return Err(format!("{raw:?} has no hostname"));
            }
            Ok((host.to_owned(), port))
        }
        None if raw.is_empty() => Err("a server name cannot be empty".to_owned()),
        None => Ok((raw.to_owned(), default)),
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

  list [inbox|archive|sent|drafts|trash|spam|snoozed|pinned] [limit]
  show <thread-id>          prints each message's id, for `reply`
  search <words...>          from:ada to:bob subject:lunch is:unread is:starred
                             in:archive has:attachment before:2026-01-01
                             after:2025-12-25 -from:newsletter, or a quoted phrase
  reply <message-id> [--all]  compose a reply; the body is read from stdin
  forward <message-id> --to a@b[,c@d]
                             forward it; the covering note is read from stdin
  send <draft-id>             queue a draft for the next sync
  snooze <thread-id> <when>   put it off: later, tonight, tomorrow, weekend,
                             monday…sunday, +2h, +3d, or a date like 2026-09-25
  wake <thread-id>            bring a snoozed conversation back now
  pin <thread-id>             keep it in view, or unpin it again
  attachments <message-id>    what is attached to a message
  save <message-id> <n> [dir] write one of them out (default: here)
  drafts                      drafts and where each one got to
  discard <draft-id>          delete a draft
  status
  account [list]
  account add <address>      (set MAILO_PASSWORD for a password account)
  account add <address> --imap HOST[:PORT] --smtp HOST[:PORT] [--login NAME]
                             for a server the preset table does not know
  account add <address> --microsoft
                             a work or school Microsoft 365 mailbox on its own domain
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
            // `view::place_filter`, not `Filter::InMailbox`: the shell and the command list the
            // same mailbox, and two spellings of "the inbox" is how one of them keeps showing
            // what the other has put away.
            let page = store
                .threads(
                    &list_query(crate::view::place_filter(*mailbox), *limit),
                    now,
                )
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
                // The id is here because `reply` needs one and nothing else prints it: the
                // sequence `list` → `show` → `reply` was unusable without opening the database
                // by hand. Found by running the three in order rather than each on its own.
                let _ = writeln!(
                    out,
                    "\n--- {} <{}>  {}\n    {}",
                    message.from.name.as_deref().unwrap_or(""),
                    message.from.email,
                    crate::view::stamp(message.date, &Local, Stamp::Full),
                    message.id
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
                    // The same parser the shell's box uses, so `from:ada` means one thing.
                    &list_query(
                        crate::query::parse_with(needle, &chrono::Local, &|name| {
                            labels_named(store, name)
                        }),
                        *limit,
                    ),
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
        Command::ListSnoozed { limit } => {
            let page = store
                .threads(&list_query(crate::view::pending_snooze(), *limit), now)
                .map_err(|e| e.to_string())?;
            if page.items.is_empty() {
                return Ok("nothing is snoozed\n".to_owned());
            }
            Ok(render_list(&page.items))
        }
        Command::ListPinned { limit } => {
            let page = store
                .threads(&list_query(Filter::Pinned, *limit), now)
                .map_err(|e| e.to_string())?;
            if page.items.is_empty() {
                return Ok("nothing is pinned\n".to_owned());
            }
            Ok(render_list(&page.items))
        }
        Command::Pin { thread } => crate::snooze::pin(store, *thread, now),
        Command::Snooze { thread, when } => crate::snooze::snooze(store, *thread, when, now),
        Command::Wake { thread } => crate::snooze::wake(store, *thread, now),
        Command::Attachments { message } => crate::attach::list(store, *message),
        Command::Save {
            message,
            index,
            dir,
        } => crate::attach::save(store, *message, *index, dir)
            .map(|path| format!("wrote {}\n", path.display())),
        Command::Forward { message, to, body } => {
            crate::compose::forward(store, *message, to, body, now)
        }
        Command::Discard { draft } => {
            crate::compose::discard(store, *draft).map(|subject| format!("discarded {subject:?}\n"))
        }
        Command::AccountAdd {
            address,
            manual,
            microsoft,
        } => crate::account::add(store, address, manual.as_ref(), *microsoft, now),
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

/// Every label with this name, across every configured account.
///
/// All of them, not the first. `UNIQUE (account, name)` means "travel" on the Gmail account and
/// "travel" on the NTU one are two different labels, and a user who types `label:travel` means
/// the word rather than one account's row. Taking the first silently searched one mailbox — a
/// wrong answer that looks like an empty one, which is the worst kind.
fn labels_named(store: &SqliteStore, name: &str) -> Vec<mail_domain::LabelId> {
    let accounts: Vec<AccountId> = {
        let db = store.connection();
        let Ok(mut stmt) = db.prepare("SELECT id FROM accounts ORDER BY created_at") else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
            return Vec::new();
        };
        rows.filter_map(Result::ok)
            .filter_map(|id| id.parse().ok())
            .map(AccountId::from_uuid)
            .collect()
    };
    accounts
        .into_iter()
        .filter_map(|account| store.labels(account).ok())
        .flatten()
        .filter(|l| l.name.eq_ignore_ascii_case(name))
        .map(|l| l.id)
        .collect()
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
            crate::view::stamp(summary.last_date, &Local, Stamp::Row),
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
