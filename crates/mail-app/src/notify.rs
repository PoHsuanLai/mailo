//! New mail, said out loud — `plan.md` 10.6.
//!
//! Three questions, kept apart. *Which* of the messages a pass stored deserve a word is a pure
//! function of each message and a few facts about the user ([`verdict`]). *How many* words a burst
//! gets is another ([`batch`]). *Where* the words go is a seam ([`Notifier`]): the desktop's
//! notification server in the binary, a recorder in the tests, which must never raise a real one.
//!
//! The glue between them, [`announce`], is the only part that reads the store, and it reads it
//! after the pass: the pass applies the server's flags and fetches bodies after it stores a
//! header, so "is this still unread, is it still in the inbox" is the store's current answer and
//! not the one the header fetch built.

pub mod desktop;
pub mod floor;

use chrono::{DateTime, Utc};
use mail_domain::ThreadId;
use mail_domain::{AccountId, AccountPlan, MailboxRole, Message, MessageId, ReadState, Snooze};
use mail_store::{SqliteStore, Store as _};
use std::path::Path;

/// Whether `mailo watch` raises notifications. On unless someone turned it off.
///
/// A preference, so it lives with the window's other preferences in the config directory
/// (`notify.json`), not in the mail database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    #[default]
    On,
    Off,
}

const FILE_NAME: &str = "notify.json";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Stored {
    #[serde(default)]
    notifications: Setting,
}

/// The stored setting, or on when there is none or it cannot be read.
pub fn load(dir: &Path) -> Setting {
    crate::appearance::read_json::<Stored>(dir, FILE_NAME).notifications
}

/// Remember `setting` in `dir`.
pub fn save(dir: &Path, setting: Setting) -> Result<(), String> {
    crate::appearance::write_json(
        dir,
        FILE_NAME,
        &Stored {
            notifications: setting,
        },
    )
}

/// `mailo notify [on|off]`: change the setting, or say what it is.
///
/// `dir` is `None` when there is no home directory to keep it in, and then there is nothing to
/// change — saying so beats pretending the change took.
pub fn command(dir: Option<&Path>, set: Option<Setting>) -> Result<String, String> {
    let Some(dir) = dir else {
        return Err("no config directory (neither XDG_CONFIG_HOME nor HOME is set)".to_owned());
    };
    if let Some(setting) = set {
        save(dir, setting)?;
    }
    Ok(match load(dir) {
        Setting::On => "notifications are on\n".to_owned(),
        Setting::Off => {
            "notifications are off; `mailo notify on` to have `watch` raise them again\n".to_owned()
        }
    })
}

/// Why an arrival is not announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quiet {
    /// Read already — on another device, or by a rule, before this pass looked.
    Read,
    /// Not in the inbox: spam, trash, sent, drafts and archive are all places nobody is waiting
    /// on.
    Elsewhere(MailboxRole),
    /// From one of the user's own addresses: mail this client, or the user elsewhere, sent.
    FromSelf,
    /// Its conversation is snoozed, and waking it early is the one thing snoozing asked not to.
    Snoozed,
    /// Dated before the account was first watched: backfill, not news.
    BeforeFloor,
}

/// Whether one arrival is announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Announce,
    Quiet(Quiet),
}

/// The user's own addresses, across every account and identity, compared without case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Own(Vec<String>);

impl Own {
    pub fn new<S: AsRef<str>>(addresses: impl IntoIterator<Item = S>) -> Self {
        let mut all: Vec<String> = addresses
            .into_iter()
            .map(|a| a.as_ref().trim().to_ascii_lowercase())
            .filter(|a| !a.is_empty())
            .collect();
        all.sort();
        all.dedup();
        Own(all)
    }

    pub fn includes(&self, email: &str) -> bool {
        self.0
            .binary_search(&email.trim().to_ascii_lowercase())
            .is_ok()
    }
}

/// Whether `message`, just stored, is worth interrupting someone for.
///
/// `snooze` is its conversation's, and `floor` is when its account was first watched
/// ([`floor::armed`]). Checked in the order a reader would explain it: where it is, whether it
/// has been read, who sent it, then when.
pub fn verdict(
    message: &Message,
    snooze: Snooze,
    own: &Own,
    floor: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Verdict {
    let quiet = if message.mailbox != MailboxRole::Inbox {
        Some(Quiet::Elsewhere(message.mailbox))
    } else if message.read == ReadState::Read {
        Some(Quiet::Read)
    } else if own.includes(&message.from.email) {
        Some(Quiet::FromSelf)
    } else if matches!(snooze, Snooze::Until(at) if at > now) {
        Some(Quiet::Snoozed)
    } else if message.date < floor {
        Some(Quiet::BeforeFloor)
    } else {
        None
    };
    quiet.map_or(Verdict::Announce, Verdict::Quiet)
}

/// What a click on a notification should open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opens {
    Thread(ThreadId),
    /// A summary of several: there is no one conversation to open, so the inbox.
    Inbox,
}

/// One desktop notification, before any transport has touched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub account: AccountId,
    /// The headline: who it is from, or how many arrived.
    pub summary: String,
    /// Plain text. A transport that speaks markup escapes it.
    pub body: String,
    pub opens: Opens,
}

/// At most this many arrivals are announced one by one; more become one summary.
pub const ONE_BY_ONE: usize = 3;

/// At most this many senders are named in a summary.
const NAMED: usize = 3;

/// A pass's announced arrivals as notifications: one each for a few, one summary for a burst.
///
/// A summary names the senders who sent most, ties broken by who came first, because "7 new
/// messages" alone sends the user to the window to find out whether any of them matter.
pub fn batch(account: AccountId, announced: &[&Message]) -> Vec<Notification> {
    if announced.len() <= ONE_BY_ONE {
        return announced
            .iter()
            .map(|message| Notification {
                account,
                summary: sender(message),
                body: if message.subject.trim().is_empty() {
                    "(no subject)".to_owned()
                } else {
                    message.subject.clone()
                },
                opens: Opens::Thread(message.thread),
            })
            .collect();
    }
    let mut senders: Vec<(String, usize)> = Vec::new();
    for message in announced {
        let name = sender(message);
        match senders.iter_mut().find(|(seen, _)| *seen == name) {
            Some((_, count)) => *count += 1,
            None => senders.push((name, 1)),
        }
    }
    // Stable, so equal counts keep the order they arrived in.
    senders.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let named: Vec<&str> = senders
        .iter()
        .take(NAMED)
        .map(|(name, _)| name.as_str())
        .collect();
    let others = senders.len().saturating_sub(NAMED);
    let body = match others {
        0 => format!("From {}", named.join(", ")),
        1 => format!("From {} and 1 other", named.join(", ")),
        n => format!("From {} and {n} others", named.join(", ")),
    };
    vec![Notification {
        account,
        summary: format!("{} new messages", announced.len()),
        body,
        opens: Opens::Inbox,
    }]
}

/// The name a person would recognise: the display name where there is one, else the address.
fn sender(message: &Message) -> String {
    match message.from.name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => message.from.email.clone(),
    }
}

/// Where notifications go.
///
/// A seam with two sides: the desktop's notification server, and a recorder in the tests.
/// Infallible on purpose — a notification that could not be shown is not a reason to stop
/// fetching mail, and there is nobody to tell except the terminal the watch already prints to.
pub trait Notifier: Send + Sync {
    fn show(&self, notification: &Notification);
}

/// Whether a watch announces what it fetches.
#[derive(Clone, Copy)]
pub enum Announce<'a> {
    /// `mailo sync`, `mailo watch --no-notify`, or notifications turned off.
    Quietly,
    To {
        store: &'a SqliteStore,
        notifier: &'a dyn Notifier,
    },
}

impl std::fmt::Debug for Announce<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Announce::Quietly => f.write_str("Quietly"),
            Announce::To { .. } => f.write_str("To(..)"),
        }
    }
}

/// Announce what one account's pass stored, returning how many notifications were raised.
///
/// Arms the account's floor on its first watched pass, whether or not anything arrived — the
/// pass that arms it is the start of "new", and everything dated before it is backfill.
pub fn announce(
    store: &SqliteStore,
    account: AccountId,
    arrived: &[MessageId],
    notifier: &dyn Notifier,
    now: DateTime<Utc>,
) -> Result<usize, String> {
    let floor = floor::armed(store, account, now)?;
    if arrived.is_empty() {
        return Ok(0);
    }
    let own = own_addresses(store)?;
    let mut held = Vec::with_capacity(arrived.len());
    for id in arrived {
        // Gone again already — expunged by the sweep of the same pass. Nothing to announce.
        let Ok(message) = store.message(*id) else {
            continue;
        };
        let snooze = store
            .thread(message.thread)
            .map(|t| t.summary.snooze)
            .unwrap_or_default();
        held.push((message, snooze));
    }
    let announced: Vec<&Message> = held
        .iter()
        .filter(|(message, snooze)| {
            verdict(message, *snooze, &own, floor, now) == Verdict::Announce
        })
        .map(|(message, _)| message)
        .collect();
    let notifications = batch(account, &announced);
    for notification in &notifications {
        notifier.show(notification);
    }
    Ok(notifications.len())
}

/// Every address the user sends from: each account's own, and each identity's.
pub fn own_addresses(store: &SqliteStore) -> Result<Own, String> {
    let db = store.connection();
    let mut addresses: Vec<String> = Vec::new();
    let mut plans = db
        .prepare("SELECT address, plan FROM accounts")
        .map_err(|e| e.to_string())?;
    let rows = plans
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (address, plan) = row.map_err(|e| e.to_string())?;
        addresses.push(address);
        // An unreadable plan still has its address; the identities are a bonus.
        if let Ok(plan) = serde_json::from_str::<AccountPlan>(&plan) {
            addresses.extend(plan.identities.into_iter().map(|i| i.from.email));
        }
    }
    let mut identities = db
        .prepare("SELECT from_email FROM identities")
        .map_err(|e| e.to_string())?;
    let rows = identities
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    for row in rows {
        addresses.push(row.map_err(|e| e.to_string())?);
    }
    Ok(Own::new(addresses))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mail_domain::{Address, Body, MessageKey, Star};

    const ACCOUNT: AccountId = AccountId::from_uuid(uuid::Uuid::from_u128(0xa1));

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 24, hour, 0, 0).unwrap()
    }

    fn message(n: u128, from: (&str, &str), subject: &str) -> Message {
        Message {
            id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n)),
            thread: ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n)),
            account: ACCOUNT,
            key: MessageKey::Rfc(format!("m{n}@example.test")),
            date: at(12),
            from: Address {
                name: (!from.0.is_empty()).then(|| from.0.to_owned()),
                email: from.1.to_owned(),
            },
            reply_to: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: subject.to_owned(),
            in_reply_to: None,
            references: Vec::new(),
            rfc_message_id: None,
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: Vec::new(),
            body: Body::Absent,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn only_new_unread_inbox_mail_from_someone_else_is_announced() {
        let own = Own::new(["Me@Example.test", "alias@example.test"]);
        let floor = at(9);
        let now = at(13);
        let base = message(1, ("Ada", "ada@example.test"), "lunch");
        let with = |change: &dyn Fn(&mut Message)| {
            let mut m = base.clone();
            change(&mut m);
            m
        };
        let inactive = Snooze::Inactive;
        let cases: [(&str, Message, Snooze, Verdict); 12] = [
            ("plain new mail", base.clone(), inactive, Verdict::Announce),
            (
                "spam",
                with(&|m| m.mailbox = MailboxRole::Spam),
                inactive,
                Verdict::Quiet(Quiet::Elsewhere(MailboxRole::Spam)),
            ),
            (
                "sent",
                with(&|m| m.mailbox = MailboxRole::Sent),
                inactive,
                Verdict::Quiet(Quiet::Elsewhere(MailboxRole::Sent)),
            ),
            (
                "trash",
                with(&|m| m.mailbox = MailboxRole::Trash),
                inactive,
                Verdict::Quiet(Quiet::Elsewhere(MailboxRole::Trash)),
            ),
            (
                "archived",
                with(&|m| m.mailbox = MailboxRole::Archive),
                inactive,
                Verdict::Quiet(Quiet::Elsewhere(MailboxRole::Archive)),
            ),
            (
                "read on another device",
                with(&|m| m.read = ReadState::Read),
                inactive,
                Verdict::Quiet(Quiet::Read),
            ),
            (
                "from my own address, in another case",
                with(&|m| m.from.email = "ME@example.TEST".to_owned()),
                inactive,
                Verdict::Quiet(Quiet::FromSelf),
            ),
            (
                "from another identity of mine",
                with(&|m| m.from.email = "alias@example.test".to_owned()),
                inactive,
                Verdict::Quiet(Quiet::FromSelf),
            ),
            (
                "older than the floor: the first backfill",
                with(&|m| m.date = at(8)),
                inactive,
                Verdict::Quiet(Quiet::BeforeFloor),
            ),
            (
                "dated exactly at the floor",
                with(&|m| m.date = floor),
                inactive,
                Verdict::Announce,
            ),
            (
                "its conversation is snoozed",
                base.clone(),
                Snooze::Until(at(18)),
                Verdict::Quiet(Quiet::Snoozed),
            ),
            (
                "its snooze is already due",
                base.clone(),
                Snooze::Until(at(10)),
                Verdict::Announce,
            ),
        ];
        for (name, message, snooze, expect) in cases {
            assert_eq!(
                verdict(&message, snooze, &own, floor, now),
                expect,
                "{name}"
            );
        }
    }

    #[test]
    fn up_to_three_arrivals_are_announced_one_by_one() {
        let a = message(1, ("Ada Lovelace", "ada@example.test"), "lunch on friday");
        let b = message(2, ("", "bob@example.test"), "  ");
        assert_eq!(
            batch(ACCOUNT, &[&a, &b]),
            [
                Notification {
                    account: ACCOUNT,
                    summary: "Ada Lovelace".to_owned(),
                    body: "lunch on friday".to_owned(),
                    opens: Opens::Thread(a.thread),
                },
                Notification {
                    account: ACCOUNT,
                    summary: "bob@example.test".to_owned(),
                    body: "(no subject)".to_owned(),
                    opens: Opens::Thread(b.thread),
                },
            ]
        );
        let c = message(3, ("Cy", "cy@example.test"), "hi");
        assert_eq!(batch(ACCOUNT, &[&a, &b, &c]).len(), 3);
        assert!(
            batch(ACCOUNT, &[]).is_empty(),
            "nothing arrived, nothing said"
        );
    }

    #[test]
    fn a_burst_becomes_one_summary_naming_the_busiest_senders() {
        let from = [
            ("Newsletter", "news@example.test"),
            ("Ada", "ada@example.test"),
            ("Bob", "bob@example.test"),
            ("Ada", "ada@example.test"),
            ("Cy", "cy@example.test"),
            ("Ada", "ada@example.test"),
            ("Dee", "dee@example.test"),
        ];
        let all: Vec<Message> = from
            .iter()
            .enumerate()
            .map(|(i, f)| message(i as u128, *f, "x"))
            .collect();
        let refs: Vec<&Message> = all.iter().collect();
        assert_eq!(
            batch(ACCOUNT, &refs),
            [Notification {
                account: ACCOUNT,
                summary: "7 new messages".to_owned(),
                // Ada sent three; the rest one each, in the order they came.
                body: "From Ada, Newsletter, Bob and 2 others".to_owned(),
                opens: Opens::Inbox,
            }]
        );
        assert_eq!(
            batch(ACCOUNT, &refs[..4])[0].body,
            "From Ada, Newsletter, Bob",
            "four from three people names all three"
        );
        assert_eq!(
            batch(ACCOUNT, &refs[..5])[0].body,
            "From Ada, Newsletter, Bob and 1 other"
        );
    }

    #[test]
    fn the_setting_is_on_until_turned_off_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Setting::On, "on by default");
        let said = command(Some(dir.path()), Some(Setting::Off)).unwrap();
        assert!(said.contains("off"), "{said}");
        assert_eq!(load(dir.path()), Setting::Off);
        assert!(command(Some(dir.path()), None).unwrap().contains("off"));
        command(Some(dir.path()), Some(Setting::On)).unwrap();
        assert_eq!(load(dir.path()), Setting::On);
        assert!(command(None, Some(Setting::Off)).is_err());
    }
}
