//! Lane 13b: the sidebar follows the accounts it is for. Two accounts with labels, one with a
//! folder, in one Space; after each step the Labels section, the Folders section and their
//! badges are what the store holds for the accounts in scope, and nothing of an account that
//! left it:
//!
//! - every account taken out of the Space, one by one, from its tile's menu;
//! - every account removed from the app, from a terminal;
//! - each tile pressed, then another Space;
//! - every message read, then every message deleted.

use ds_harness::Harness;
use mail_core::{Arrival, SqliteStore, Store};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_ui::ui::appearance::WindowDirs;
use mail_ui::ui::space::{self, Mail};
use mail_ui_model::copy;
use porter_core::AccountId;

use super::drive::Drive;
use super::hands::menu_item;
use super::window::{Window, account, hours_ago, ms};
use crate::lanes::seed::ME;

/// The second account's address.
const YOU: &str = "you@example.test";
/// The first account's label, on one of its conversations.
const TRAVEL: &str = "Travel";
/// The second account's label, on its inbox conversation.
const WORK: &str = "Work";
/// The second account's folder, which holds a conversation of its own.
const PROJECTS: &str = "Projects";

/// The second account.
fn other() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"))
}

/// One message into `account`'s IMAP folder `path`, the `uid`th there.
fn deliver_to(store: &SqliteStore, account: AccountId, path: &str, uid: u32, subject: &str) {
    let raw = format!(
        "From: Sam <sam@example.test>\r\nTo: <{YOU}>\r\nSubject: {subject}\r\nDate: {}\r\n\
         Message-ID: <{path}{uid}@example.test>\r\n\r\nThe body of {subject}.\r\n",
        hours_ago(1)
    );
    // A folder of the person's files its mail as Archive, as a sync does.
    let role = if path == "INBOX" {
        MailboxRole::Inbox
    } else {
        MailboxRole::Archive
    };
    mail_core::absorb_into(
        store,
        account.clone(),
        mail_core::Destination {
            mailbox: MailboxRef {
                account,
                path: path.to_owned(),
            },
            role,
        },
        None,
        vec![Arrival {
            remote: RemoteRef::Imap {
                mailbox: path.to_owned(),
                uidvalidity: 1,
                uid,
            },
            raw: raw.into_bytes(),
        }],
        false,
        chrono::Utc::now(),
    )
    .expect("the message is absorbed");
}

/// Every conversation of `account`'s.
fn threads_of(store: &SqliteStore, account: AccountId) -> Vec<ThreadId> {
    store
        .threads(
            &Query {
                filter: Filter::Account(account),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 100,
                },
            },
            chrono::Utc::now(),
        )
        .expect("the conversations")
        .items
        .into_iter()
        .map(|summary| summary.id)
        .collect()
}

/// The conversation called `subject`.
fn thread_called(store: &SqliteStore, subject: &str) -> ThreadId {
    store
        .threads(
            &Query {
                filter: Filter::Subject(TextMatch::Contains(subject.to_owned())),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 1,
                },
            },
            chrono::Utc::now(),
        )
        .expect("the conversations")
        .items
        .first()
        .map(|summary| summary.id)
        .unwrap_or_else(|| panic!("no conversation called {subject:?}"))
}

/// `op` on `thread`, as the window would do it.
fn act(store: &SqliteStore, thread: ThreadId, op: Op) {
    // The undo is the window's; nothing here takes it back.
    let _ = mail_core::act::perform(store, thread, op);
}

/// The first account's Travel label on its first conversation, and a second account, an IMAP
/// one whose folders have been listed, with a Work label on its inbox conversation and a
/// conversation in Projects.
fn two_accounts(store: &SqliteStore) {
    let travel = store.create_label(account(), TRAVEL).expect("Travel");
    act(
        store,
        thread_called(store, super::window::INBOX[0].1),
        Op::Label(travel, Membership::In),
    );

    crate::store::add_account(store, other(), YOU);
    let manual = mail_domain::presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let preset = mail_domain::presets::manual(YOU, &manual, chrono::Utc::now());
    store
        .set_account_plan(other(), &preset.plan)
        .expect("the plan");
    store
        .put_caps(other(), &preset.expected_caps, chrono::Utc::now())
        .expect("the capabilities");
    let folder = |path: &str| Folder {
        account: other(),
        path: path.to_owned(),
        delimiter: Some('/'),
        special: None,
        subscription: Subscription::Subscribed,
        holds: Holds::Mail,
    };
    store
        .put_folders(other(), vec![folder("INBOX"), folder(PROJECTS)])
        .expect("the folders");
    deliver_to(store, other(), "INBOX", 1, "The quarterly plan");
    deliver_to(store, other(), PROJECTS, 1, "The project brief");
    let work = store.create_label(other(), WORK).expect("Work");
    act(
        store,
        thread_called(store, "The quarterly plan"),
        Op::Label(work, Membership::In),
    );
}

/// The window over [`two_accounts`], with a Space of both and a Space of the first alone.
fn opened() -> Window {
    Window::open_with(chrono::Utc::now(), two_accounts, |dir, contexts| {
        let look = ds::prelude::SpaceLook::default();
        let spaces = space::built(
            vec![
                (
                    "Both".to_owned(),
                    look.clone(),
                    Mail::of(vec![account(), other()]),
                ),
                ("Mine".to_owned(), look, Mail::of(vec![account()])),
            ],
            0,
        );
        let dirs = WindowDirs {
            config: dir.join("config"),
            state: dir.join("state"),
        };
        space::save(Some(&dirs), &spaces).expect("the Spaces are written");
        contexts
    })
}

/// How the sidebar draws one label or folder.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Drawn {
    Not,
    Plain,
    Badged(String),
}

/// What the sidebar draws of each label and folder this lane knows: Labels, then Folders.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Side {
    labels: Vec<(&'static str, Drawn)>,
    folders: Vec<(&'static str, Drawn)>,
}

/// How `name` is drawn in the list called `list`.
fn drawn(h: &Harness, list: &str, name: &str) -> Drawn {
    let row = format!("[*|aria-label=\"{list}\"] .ds-row[*|data-place=\"{name}\"]");
    if h.count(&row) == 0 {
        return Drawn::Not;
    }
    // A place's count is its row's; a folder's is a badge of quire's beside its name.
    let count = h
        .text_of(&format!("{row} .ds-row-count"))
        .or_else(|| h.text_of(&format!("{row} .ds-badge")));
    match count {
        Some(count) => Drawn::Badged(count.trim().to_owned()),
        None => Drawn::Plain,
    }
}

/// How many counts the sidebar draws, on places and folders.
fn badges(h: &Harness) -> usize {
    h.count(".side .ds-row-count") + h.count(".side .ds-badge")
}

/// What the sidebar draws now.
fn seen(h: &Harness) -> Side {
    Side {
        labels: [TRAVEL, WORK]
            .map(|name| (name, drawn(h, copy::aria::PLACES, name)))
            .to_vec(),
        folders: vec![(PROJECTS, drawn(h, copy::aria::FOLDERS, PROJECTS))],
    }
}

/// What the store says the sidebar should draw for `scope`: each label and folder of the
/// accounts in it, badged with its unread conversations in those accounts.
fn expected(store: &SqliteStore, scope: &[AccountId]) -> Side {
    let now = chrono::Utc::now();
    let accounts = Filter::Or(scope.iter().cloned().map(Filter::Account).collect());
    let unread = |place: Filter| {
        let n = store
            .count(
                &Filter::And(vec![
                    accounts.clone(),
                    place,
                    Filter::Read(ReadState::Unread),
                ]),
                now,
            )
            .expect("a count");
        if n == 0 {
            Drawn::Plain
        } else {
            Drawn::Badged(n.to_string())
        }
    };
    let label = |name: &'static str| {
        let found = scope.iter().find_map(|account| {
            store
                .labels(account.clone())
                .expect("the labels")
                .into_iter()
                .find(|label| label.name == name)
        });
        let drawn = found.map_or(Drawn::Not, |label| unread(Filter::HasLabel(label.id)));
        (name, drawn)
    };
    let projects = if scope.contains(&other())
        && store
            .folders(other())
            .expect("the folders")
            .iter()
            .any(|folder| folder.path == PROJECTS)
    {
        unread(Filter::InFolder(MailboxRef {
            account: other(),
            path: PROJECTS.to_owned(),
        }))
    } else {
        Drawn::Not
    };
    Side {
        labels: vec![label(TRAVEL), label(WORK)],
        folders: vec![(PROJECTS, projects)],
    }
}

/// Wait for the sidebar to draw what the store says for `scope`, and say what it drew if not.
fn draws_for(window: &mut Window, what: &str, scope: &[AccountId]) -> Side {
    let want = expected(&window.store, scope);
    let deadline = std::time::Instant::now() + crate::settle::WAIT_BOUND;
    while std::time::Instant::now() < deadline && seen(&window.harness) != want {
        window.harness.advance(ms(10));
        std::thread::sleep(ms(1));
    }
    assert_eq!(seen(&window.harness), want, "{what}");
    want
}

/// The tile of the account at `address`.
fn tile(address: &str) -> String {
    format!(
        "[*|role=\"group\"][*|aria-label=\"{}\"] [*|aria-label=\"{address}\"]",
        copy::aria::ACCOUNTS_IN_SPACE
    )
}

/// Take the account at `address` out of the Space, from its tile's menu.
fn take_out(window: &mut Window, address: &str) {
    let at = window.centre(&tile(address));
    window
        .harness
        .press(at, ds::base::press::PointerButton::Secondary);
    menu_item(&mut window.harness, copy::menus::REMOVE_FROM_SPACE);
    window.until("the menu goes", |h| h.count(".ds-menu") == 0);
}

/// A connection of the store's own, as `mailo` in a terminal opens.
fn terminal(window: &Window) -> SqliteStore {
    window.second_connection()
}

#[test]
fn accounts_removed_from_the_space_take_their_labels_folders_and_badges() {
    let mut window = opened();
    let both = draws_for(
        &mut window,
        "both accounts' labels and folder",
        &[account(), other()],
    );
    assert_eq!(
        both.labels,
        [
            (TRAVEL, Drawn::Badged("1".to_owned())),
            (WORK, Drawn::Badged("1".to_owned()))
        ],
        "the seed is not what this lane means"
    );
    assert_eq!(both.folders, [(PROJECTS, Drawn::Badged("1".to_owned()))]);

    take_out(&mut window, ME);
    draws_for(&mut window, "only the second account's", &[other()]);
    take_out(&mut window, YOU);
    let none = draws_for(&mut window, "nothing, with no account in the Space", &[]);
    assert!(
        none.labels.iter().all(|(_, drawn)| *drawn == Drawn::Not),
        "{none:?}"
    );
    assert_eq!(badges(&window.harness), 0, "a badge is left");
}

#[test]
fn accounts_removed_from_the_app_take_their_labels_folders_and_badges() {
    let mut window = opened();
    draws_for(
        &mut window,
        "both accounts' labels and folder",
        &[account(), other()],
    );

    let terminal = terminal(&window);
    terminal
        .remove_account(other())
        .expect("the store")
        .expect("the account was there");
    assert!(
        window.store.labels(other()).expect("the labels").is_empty(),
        "the removed account's labels stayed in the store"
    );
    draws_for(&mut window, "only the first account's", &[account()]);

    terminal
        .remove_account(account())
        .expect("the store")
        .expect("the account was there");
    draws_for(&mut window, "nothing, with no account left", &[]);
    assert_eq!(badges(&window.harness), 0, "a badge is left");
}

#[test]
fn pressing_a_tile_or_another_space_draws_that_scope_alone() {
    let mut window = opened();
    draws_for(&mut window, "both accounts'", &[account(), other()]);

    window.click(&tile(ME));
    draws_for(&mut window, "the first account's tile", &[account()]);
    window.click(&tile(YOU));
    draws_for(&mut window, "the second account's tile", &[other()]);

    window.click(".ds-spaces-dot-hold:nth-child(2)");
    window.harness.advance(ms(100));
    draws_for(&mut window, "the Space of the first account", &[account()]);
    window.click(".ds-spaces-dot-hold:nth-child(1)");
    draws_for(&mut window, "both again", &[account(), other()]);
}

/// Every conversation of both accounts'.
fn every(store: &SqliteStore) -> Vec<ThreadId> {
    [account(), other()]
        .into_iter()
        .flat_map(|account| threads_of(store, account))
        .collect()
}

#[test]
fn reading_then_deleting_every_message_clears_every_badge_and_keeps_the_labels() {
    let mut window = opened();
    draws_for(&mut window, "both accounts'", &[account(), other()]);

    let terminal = terminal(&window);
    for thread in every(&terminal) {
        act(&terminal, thread, Op::SetRead(ReadState::Read));
    }
    let read = draws_for(
        &mut window,
        "every badge gone once read",
        &[account(), other()],
    );
    // Labels the person or the server made stay with no mail, as Gmail keeps them.
    assert_eq!(read.labels, [(TRAVEL, Drawn::Plain), (WORK, Drawn::Plain)]);
    assert_eq!(read.folders, [(PROJECTS, Drawn::Plain)]);

    for thread in every(&terminal) {
        act(&terminal, thread, Op::Trash);
    }
    let deleted = draws_for(
        &mut window,
        "still none once deleted",
        &[account(), other()],
    );
    assert_eq!(deleted, read);
    assert_eq!(badges(&window.harness), 0, "a badge is left");
}

#[test]
#[ignore = "gap: a label's list and badge count its conversations in Trash; leaving them out \
            is per conversation, and a conversation is in Trash when any message of it is"]
fn deleting_unread_mail_takes_its_labels_badges() {
    let mut window = opened();
    draws_for(&mut window, "both accounts'", &[account(), other()]);

    let terminal = terminal(&window);
    for thread in every(&terminal) {
        act(&terminal, thread, Op::Trash);
    }
    window.until("no label badged once its mail is deleted", |h| {
        seen(h).labels == [(TRAVEL, Drawn::Plain), (WORK, Drawn::Plain)]
    });
}
