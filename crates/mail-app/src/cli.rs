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
        manual: Option<Setup>,
        /// The address belongs to a managed Microsoft 365 tenant on its own domain, which is
        /// the one thing the preset table cannot work out for itself.
        microsoft: bool,
        /// `--send graph`: send through Microsoft Graph, for a tenant with SMTP AUTH off.
        graph: bool,
    },
    /// Configured accounts, and what each still needs.
    AccountList,
    /// Fetch every configured account's provider icon again.
    IconsRefresh,
    /// Set or clear the signature on an account. The text is read from stdin.
    Signature {
        address: String,
        clear: bool,
        text: String,
    },
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
    /// Put a file on a draft.
    Attach {
        draft: DraftId,
        path: std::path::PathBuf,
    },
    /// Take one back off, by the number `attached` prints.
    Detach { draft: DraftId, index: usize },
    /// What a draft is carrying.
    Attached { draft: DraftId },
    /// Keep fetching until stopped.
    Watch,
    /// Run the daemon, or stop the one that is running.
    Daemon { stop: bool },
    /// Reach the daemon, starting one if none is listening.
    Ping,
    /// A message that answers nothing. `from` names the sending account when there is a choice.
    Compose {
        from: Option<String>,
        to: Vec<Address>,
        cc: Vec<Address>,
        bcc: Vec<Address>,
        subject: String,
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
        "watch" => Ok(Command::Watch),
        "daemon" => match args.get(1).map(String::as_str) {
            None => Ok(Command::Daemon { stop: false }),
            Some("--stop") => Ok(Command::Daemon { stop: true }),
            Some(other) => Err(format!("unknown option {other:?}\n\n{}", usage())),
        },
        "ping" => Ok(Command::Ping),
        "attach" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("attach needs a draft id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a draft id"))?;
            let path = args.get(2).ok_or_else(|| {
                format!(
                    "attach needs a file: mailo attach {raw} ./report.pdf\n\n{}",
                    usage()
                )
            })?;
            Ok(Command::Attach {
                draft: DraftId::from_uuid(uuid),
                path: std::path::PathBuf::from(path),
            })
        }
        "detach" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("detach needs a draft id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a draft id"))?;
            let index = args
                .get(2)
                .ok_or_else(|| {
                    format!(
                        "detach needs a number, as `attached` prints it\n\n{}",
                        usage()
                    )
                })?
                .parse()
                .map_err(|_| "that is not an attachment number".to_owned())?;
            Ok(Command::Detach {
                draft: DraftId::from_uuid(uuid),
                index,
            })
        }
        "attached" => {
            let raw = args
                .get(1)
                .ok_or_else(|| format!("attached needs a draft id\n\n{}", usage()))?;
            let uuid = raw
                .parse()
                .map_err(|_| format!("{raw:?} is not a draft id"))?;
            Ok(Command::Attached {
                draft: DraftId::from_uuid(uuid),
            })
        }
        "compose" => {
            // A flag loop rather than fixed positions: three options, two of them optional, and
            // the order someone types them in is not something to have an opinion about.
            let mut from = None;
            let (mut to, mut cc, mut bcc) = (Vec::new(), Vec::new(), Vec::new());
            let mut subject = String::new();
            let mut rest = args[1..].iter();
            while let Some(flag) = rest.next() {
                let missing = format!("{flag} needs a value\n\n{}", usage());
                match flag.as_str() {
                    "--from" => {
                        from = Some(rest.next().ok_or(missing)?.clone());
                    }
                    "--to" => {
                        to = crate::view::parse_addresses(rest.next().ok_or(missing)?)?;
                    }
                    "--cc" => {
                        cc = crate::view::parse_addresses(rest.next().ok_or(missing)?)?;
                    }
                    "--bcc" => {
                        bcc = crate::view::parse_addresses(rest.next().ok_or(missing)?)?;
                    }
                    "--subject" => {
                        subject = rest.next().ok_or(missing)?.clone();
                    }
                    other => return Err(format!("unknown option {other:?}\n\n{}", usage())),
                }
            }
            if to.is_empty() {
                // The F99 dead end once more: a draft with nobody to send it to is one no
                // command can finish.
                return Err(format!(
                    "compose needs recipients: mailo compose --to someone@example.com\n\n{}",
                    usage()
                ));
            }
            Ok(Command::Compose {
                from,
                to,
                cc,
                bcc,
                subject,
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
        "signature" => {
            let address = args
                .get(1)
                .ok_or_else(|| {
                    format!(
                        "signature needs an address: mailo signature you@example.com\n\n{}",
                        usage()
                    )
                })?
                .to_lowercase();
            let clear = match args.get(2).map(String::as_str) {
                None => false,
                Some("--clear") => true,
                Some(other) => return Err(format!("unknown option {other:?}\n\n{}", usage())),
            };
            Ok(Command::Signature {
                address,
                clear,
                // Filled in by the caller, which owns stdin. Parsing stays pure.
                text: String::new(),
            })
        }
        "sync" => Ok(Command::Sync),
        "icons" => match args.get(1).map(String::as_str) {
            Some("refresh") if args.len() == 2 => Ok(Command::IconsRefresh),
            Some("refresh") => Err(format!("icons refresh takes no arguments\n\n{}", usage())),
            _ => Err(format!(
                "unknown icons command. Use: mailo icons refresh\n\n{}",
                usage()
            )),
        },
        "account" => match args.get(1).map(String::as_str) {
            Some("add") => {
                let address = args
                    .get(2)
                    .ok_or_else(|| format!("account add needs an address\n\n{}", usage()))?;
                let rest = &args[3..];
                let microsoft = rest.iter().any(|a| a == "--microsoft");
                let mut graph = false;
                let mut others: Vec<String> = Vec::new();
                let mut words = rest.iter().filter(|a| *a != "--microsoft");
                while let Some(word) = words.next() {
                    if word != "--send" {
                        others.push(word.clone());
                        continue;
                    }
                    graph = match words.next().map(String::as_str) {
                        Some("graph") => true,
                        Some("smtp") => false,
                        other => {
                            return Err(format!(
                                "--send takes smtp or graph, not {other:?}\n\n{}",
                                usage()
                            ));
                        }
                    };
                    if !microsoft {
                        return Err(format!(
                            "--send is for a Microsoft 365 account; add --microsoft\n\n{}",
                            usage()
                        ));
                    }
                }
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
                    graph,
                })
            }
            None | Some("list") => Ok(Command::AccountList),
            Some(other) => Err(format!("unknown account command {other:?}\n\n{}", usage())),
        },
        other => Err(format!("unknown command {other:?}\n\n{}", usage())),
    }
}

/// Servers the user named, for an address the preset table does not cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setup {
    /// `--imap`: mail is read where it lies, in folders.
    Imap(mail_domain::presets::Manual),
    /// `--pop3`: mail is downloaded from one mailbox, and left there.
    Pop3(mail_domain::presets::ManualPop3),
}

/// `--imap HOST[:PORT] | --pop3 HOST[:PORT]`, then `--smtp HOST[:PORT] [--login NAME]`, or `None`
/// when none were given.
///
/// All or nothing: naming only the incoming server would leave the account unable to send, and
/// silently defaulting the other half to a guessed hostname is how mail goes to a server the
/// user never chose.
fn parse_manual(args: &[String]) -> Result<Option<Setup>, String> {
    if args.is_empty() {
        return Ok(None);
    }
    let (mut imap, mut pop3, mut smtp, mut login) = (None, None, None, None);
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let value = rest
            .next()
            .ok_or_else(|| format!("{flag} needs a value\n\n{}", usage()))?;
        match flag.as_str() {
            "--imap" => imap = Some(host_port(value, 993)?),
            "--pop3" => pop3 = Some(host_port(value, 995)?),
            "--smtp" => smtp = Some(host_port(value, 465)?),
            "--login" => login = Some(value.clone()),
            other => return Err(format!("unknown option {other:?}\n\n{}", usage())),
        }
    }
    if imap.is_some() && pop3.is_some() {
        return Err(format!(
            "an account reads mail over IMAP or over POP3, not both; pick one\n\n{}",
            usage()
        ));
    }
    match (imap, pop3, smtp) {
        (Some((imap_host, imap_port)), None, Some((smtp_host, smtp_port))) => {
            Ok(Some(Setup::Imap(mail_domain::presets::Manual {
                imap_host,
                imap_port,
                smtp_host,
                smtp_port,
                login,
            })))
        }
        (None, Some((pop3_host, pop3_port)), Some((smtp_host, smtp_port))) => {
            Ok(Some(Setup::Pop3(mail_domain::presets::ManualPop3 {
                pop3_host,
                pop3_port,
                smtp_host,
                smtp_port,
                login,
            })))
        }
        (None, None, None) => Err(format!(
            "--login needs --imap (or --pop3) and --smtp too\n\n{}",
            usage()
        )),
        _ => Err(format!(
            "manual setup needs --smtp and one of --imap or --pop3; an account that \
             can only receive, or only send, is not one this can configure\n\n{}",
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
  compose --to a@b[,c@d] [--cc …] [--bcc …] [--subject S] [--from address]
                             a new message; the body is read from stdin
  attach <draft-id> <path>    put a file on a draft
  attached <draft-id>         what it is carrying
  detach <draft-id> <n>       take one back off
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
  signature <address> [--clear]
                             set it from stdin, or take it off
  account add <address>      (set MAILO_PASSWORD for a password account)
  account add <address> --imap HOST[:PORT] --smtp HOST[:PORT] [--login NAME]
                             for a server the preset table does not know
  account add <address> --pop3 HOST[:PORT] --smtp HOST[:PORT] [--login NAME]
                             the same, for a server that offers only POP3
  account add <address> --microsoft [--send graph]
                             a work or school Microsoft 365 mailbox on its own domain;
                             --send graph where the tenant has SMTP sending turned off
  sync                       fetch mail and send anything queued
  icons refresh              fetch each account's provider icon again
  watch                      keep fetching until stopped; uses IDLE where the
                             server offers it, and polls where it does not
  daemon [--stop]            run the background daemon, or stop it
  ping                       reach the daemon, starting one if none is running
"
    .to_owned()
}

/// Run a command against the store, returning what to print.
///
/// Returns a `String` rather than printing, so tests assert on output instead of capturing
/// stdout. OAuth setup falls back on [`crate::account::saved_clients`].
pub fn run(store: &SqliteStore, command: &Command, now: DateTime<Utc>) -> Result<String, String> {
    run_with_clients(store, command, now, &crate::account::saved_clients())
}

/// [`run`], with the OAuth clients named.
///
/// The binary passes [`crate::account::saved_clients`]. An integration test passes an empty
/// registry: it links the ordinary library, so that function's `cfg!(test)` guard does not
/// apply, and a real client id would open a browser and wait.
pub fn run_with_clients(
    store: &SqliteStore,
    command: &Command,
    now: DateTime<Utc>,
    saved: &mail_runtime::OAuthRegistry,
) -> Result<String, String> {
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
            // The same parse → expand → rank the menu runs, so `from:ada` and a prefix cannot
            // mean one thing here and another in the window. Labels stay resolved: `label:` is
            // the one operator that needs the store, and `run` has no slot for that index.
            let ranked = match crate::search::rank_query(
                needle,
                store,
                &crate::search::Affinity::default(),
                &chrono::Local,
                &|name| labels_named(store, name),
                now,
            ) {
                Ok(ranked) => ranked,
                // The `regex` crate's own message. Shown, not panicked on, and not a failed command.
                Err(message) => return Ok(format!("{message}\n")),
            };
            let take = usize::try_from(*limit).unwrap_or(usize::MAX);
            let items: Vec<ThreadSummary> = ranked
                .hits
                .into_iter()
                .take(take)
                .map(|(summary, _)| summary)
                .collect();
            if items.is_empty() {
                return Ok(format!("nothing matches {needle:?}\n"));
            }
            Ok(render_list(&items))
        }
        // Dispatched in main: it needs an async runtime and the store by Arc, which would make
        // this function untestable without one.
        Command::Sync => Err("sync is dispatched before this point".to_owned()),
        Command::Watch => Err("watch is dispatched before this point".to_owned()),
        Command::Daemon { .. } | Command::Ping => {
            Err("the daemon commands are dispatched before this point".to_owned())
        }
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
        Command::Attach { draft, path } => crate::compose::attach_file(store, *draft, path, now)
            .map(|draft| {
                format!(
                    "attached {}; {} now carries {} file(s)\n",
                    path.display(),
                    draft.id,
                    draft.attachments.len()
                )
            }),
        Command::Detach { draft, index } => {
            crate::compose::detach(store, *draft, *index, now).map(|draft| {
                format!(
                    "{} now carries {} file(s)\n",
                    draft.id,
                    draft.attachments.len()
                )
            })
        }
        Command::Attached { draft } => crate::compose::attachments_of(store, *draft),
        Command::Compose {
            from,
            to,
            cc,
            bcc,
            subject,
            body,
        } => crate::compose::new_message(store, from.as_deref(), [to, cc, bcc], subject, body, now),
        Command::Discard { draft } => {
            crate::compose::discard(store, *draft).map(|subject| format!("discarded {subject:?}\n"))
        }
        Command::AccountAdd {
            address,
            manual,
            microsoft,
            graph,
        } => crate::account::add(
            store,
            address,
            manual.as_ref(),
            *microsoft,
            *graph,
            saved,
            now,
        ),
        Command::AccountList => crate::account::list(store),
        Command::IconsRefresh => {
            let Some(root) = crate::appearance::cache_dir() else {
                return Err("no home directory, so there is nowhere to keep an icon".to_owned());
            };
            let providers = crate::provider::icon::providers_of(store)?;
            if providers.is_empty() {
                return Ok("no accounts. Add one with: mailo account add <address>\n".to_owned());
            }
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| format!("cannot fetch icons: {err}"))?;
            let results = runtime.block_on(crate::provider::icon::refresh(
                &root.join("providers"),
                &providers,
            ));
            for (provider, result) in &results {
                if let Err(err) = result
                    && !matches!(err, crate::provider::icon::IconError::Unmapped)
                {
                    eprintln!("provider icon: {provider:?}: {err}");
                }
            }
            Ok(crate::provider::icon::report(&results))
        }
        Command::Signature {
            address,
            clear,
            text,
        } => {
            let account = account_named(store, address)?;
            crate::compose::set_signature(
                store,
                account,
                if *clear { None } else { Some(text.as_str()) },
            )
        }
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

/// The account whose address is `address`.
///
/// By address because that is what the user knows: an account id is a uuid nothing prints except
/// `account list`, and asking someone to copy one to set a signature is asking them not to.
fn account_named(store: &SqliteStore, address: &str) -> Result<AccountId, String> {
    let db = store.connection();
    let found: Option<String> = db
        .query_row(
            "SELECT id FROM accounts WHERE address = ?1",
            [address.to_lowercase()],
            |r| r.get(0),
        )
        .ok();
    let id = found.ok_or_else(|| {
        format!("no account for {address:?}. `mailo account list` says which there are.")
    })?;
    id.parse()
        .map(AccountId::from_uuid)
        .map_err(|_| "that account's id is unreadable".to_owned())
}

/// Every label with this name, across every configured account.
///
/// All of them, not the first. `UNIQUE (account, name)` means "travel" on the Gmail account and
/// "travel" on the work one are two different labels, and a user who types `label:travel` means
/// the word rather than one account's row. Taking the first silently searched one mailbox — a
/// wrong answer that looks like an empty one, which is the worst kind.
fn labels_named(store: &SqliteStore, name: &str) -> Vec<mail_domain::LabelId> {
    // Shared with the window's search box, which is the point: the two had separate answers to
    // "which label is called this", and one of them was "none, ever".
    crate::query::named(&crate::query::known_labels(store))(name)
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
    fn icons_refresh_is_a_command() {
        assert_eq!(
            parse(&args(&["icons", "refresh"])).unwrap(),
            Command::IconsRefresh
        );
        assert!(parse(&args(&["icons"])).is_err());
        assert!(parse(&args(&["icons", "refresh", "now"])).is_err());
        assert!(usage().contains("icons refresh"));
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
