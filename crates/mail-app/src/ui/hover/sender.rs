//! The sender card: who they are, how often they write, and whether their name borrows a
//! brand their address does not belong to. Its actions are the shared `Menu`.

use super::super::checks::SenderChecks;
use super::super::contacts::ContactPart;
use super::super::history::History;
use super::super::menu::{Floating, MenuItem, Right, Tile};
use super::super::motion::{Follow, tell};
use super::cards::{Card, letter, who};
use super::dismiss;
use crate::rules::block::Blocked;
use crate::space::{Pinned, Spaces};
use crate::trust::spoof;
use crate::view::Shell;
use chrono::Local;
use dioxus::prelude::*;
use ds::{AvatarTone, FlagTone, HoverCardPart, HoverStat, Icon, Run, RunTone};
use mail_domain::{AccountId, Message, MessageId, ThreadId};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

pub(super) fn sender_card(
    store: &SqliteStore,
    id: ThreadId,
    known: &History,
    mut shell: Signal<Shell>,
    spaces: Option<Signal<Spaces>>,
) -> Option<Card> {
    let loaded = store.thread(id).ok()?;
    let from = loaded.summary.from.clone();
    let name = who(&from);
    let email = from.email.clone();
    // Their newest message here: the one whose checks the card shows, and whose account a block
    // is made on.
    let theirs = newest_from(store, &loaded.messages, &email);
    let seen = known.get(&email).cloned();
    let first = seen.as_ref().is_none_or(|sender| sender.threads <= 1);
    let flag = spoof(from.name.as_deref(), &email);
    let threads = seen.as_ref().map_or(1, |sender| sender.threads);
    let last = seen
        .as_ref()
        .map(|sender| crate::view::listed(sender.last, chrono::Utc::now(), &Local))
        .unwrap_or_default();
    let items = sender_actions();
    let given = from.name.clone().unwrap_or_default();
    let mut parts = vec![
        HoverCardPart::Person {
            initial: letter(&name),
            tone: AvatarTone::Ink,
            title: name.clone(),
            sub: Some(email.clone()),
        },
        HoverCardPart::Stats(vec![
            HoverStat {
                value: threads.to_string(),
                label: if threads == 1 { "thread" } else { "threads" }.to_owned(),
            },
            HoverStat {
                value: last,
                label: "last wrote".to_owned(),
            },
        ]),
    ];
    // The flags are quire's parts, the brand and the domain in the strong tone.
    if let Some(flag) = flag {
        parts.push(HoverCardPart::flag(
            FlagTone::Danger,
            Icon::X,
            vec![
                Run::new("The name says ", RunTone::Plain),
                Run::new(flag.brand, RunTone::Strong),
                Run::new("; the address is ", RunTone::Plain),
                Run::new(flag.domain, RunTone::Strong),
                Run::new(", which is not one of theirs.", RunTone::Plain),
            ],
        ));
    }
    if first {
        parts.push(HoverCardPart::flag(
            FlagTone::Info,
            Icon::Mail,
            "First mail from this address. Nothing else in the store has come from it.",
        ));
    }
    let checked = theirs
        .as_ref()
        .map(|message| (message.id, message.body.raw()));
    let account = theirs.as_ref().map(|message| message.account);
    let more = rsx! {
        div { class: "hc",
        // What the receiving server checked about them, beside the flags above: the same line
        // the reader shows, read off the thread that draws. Keyed like the reader's.
        if let Some((message, raw)) = checked {
            {rsx! { SenderChecks { key: "{message}-{raw:?}", message, body: raw } }}
        }
        // Keyed, so a card for another sender starts afresh rather than keep this one's state.
        {rsx! { ContactPart { key: "{email}", email: email.clone(), name: given } }}
        div { class: "acts",
            // The card's actions are a menu drawn in the card, not over it.
            Floating {
                kind: ds::MenuKind::Slim,
                anchor: None,
                title: String::new(),
                items,
                flow: ds::Flow::Inline,
                // No row under a cursor: the card is pointed at, never arrowed through, and a
                // first row drawn as selected reads as one already chosen.
                active: ds::Cursor::Controlled(None),
                on_pick: move |key: String| {
                    match key.as_str() {
                        "pin" => pin_person(spaces, &name, &email),
                        "mail" => shell.write().search = format!("from:{email}"),
                        "block" => block_sender(account, &email),
                        _ => copy(&email),
                    }
                    dismiss();
                },
                on_close: move |_| dismiss(),
            }
        }
        }
    };
    Some(Card { parts, more })
}

fn sender_actions() -> Vec<MenuItem> {
    [
        ("pin", Icon::Star, "Pin to sidebar"),
        ("mail", Icon::Search, "Their mail"),
        ("copy", Icon::Mail, "Copy address"),
        ("block", Icon::OctagonAlert, "Block sender"),
    ]
    .into_iter()
    .map(|(key, icon, name)| MenuItem {
        key: key.to_owned(),
        tile: Tile::Icon(icon),
        name: name.to_owned(),
        help: None,
        right: Right::None,
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    })
    .collect()
}

/// The newest of `messages` that `email` sent, or the newest of all when none reads as theirs.
fn newest_from(store: &SqliteStore, messages: &[MessageId], email: &str) -> Option<Message> {
    let mut all: Vec<Message> = messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect();
    all.sort_by_key(|message| message.date);
    let at = all
        .iter()
        .rposition(|message| message.from.email.eq_ignore_ascii_case(email))
        .or_else(|| all.len().checked_sub(1))?;
    Some(all.swap_remove(at))
}

/// "Block sender": a rule that sends their mail to Spam (`crate::rules::block`), said with the
/// way to take it back.
fn block_sender(account: Option<AccountId>, email: &str) {
    let Some(account) = account else {
        return;
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let said = match crate::rules::block::block(&store, account, email) {
        Ok(Blocked::Made(rule)) => {
            let text = format!("Blocked {email}: new mail from them goes to Spam");
            let follow = Follow::Unblock {
                rule: rule.id,
                sender: email.to_owned(),
            };
            (text, follow)
        }
        Ok(Blocked::Already(_)) => (format!("{email} is already blocked"), Follow::Nothing),
        Err(why) => (format!("Could not block {email}: {why}"), Follow::Nothing),
    };
    tell(said.0, said.1);
}

/// Add the sender to the current Space's pinned people, once.
fn pin_person(spaces: Option<Signal<Spaces>>, name: &str, email: &str) {
    let Some(mut spaces) = spaces else {
        return;
    };
    let current = spaces.peek().current;
    {
        let mut write = spaces.write();
        let Some(space) = write.spaces.get_mut(current) else {
            return;
        };
        let already = space.pins.iter().any(|pin| {
            matches!(pin, Pinned::Person { email: known, .. } if known.eq_ignore_ascii_case(email))
        });
        if !already {
            space.pins.push(Pinned::Person {
                name: name.to_owned(),
                email: email.to_owned(),
            });
        }
    }
    super::super::frame::keep(&spaces.read());
}

/// Put an address on the clipboard.
pub(in crate::ui) fn copy(email: &str) {
    crate::ui::host::Host::copy(email);
}
