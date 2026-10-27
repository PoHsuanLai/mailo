//! Lane 19: a notification's click. The banner's conversation reaches the running window as a
//! request (`ui/handoff`); here the socket or bus is left out and the request is pushed straight
//! into the window's queue. The window shows it in a Space that holds its account: the one it is
//! in when that one does, else the first that does, switched to as ⌘1-9 would. Two accounts, two
//! Spaces, and nothing reaches the desktop: the store and config are a `TempDir`'s.

use mail_core::{Arrival, SqliteStore, Store as _, absorb};
use mail_domain::id::account_id_from_uuid;
use mail_domain::{
    Filter, MailboxRef, PageReq, Property, Query, RemoteRef, Sort, SortDir, SyncCursor, ThreadId,
};
use mail_ui::ui::appearance::WindowDirs;
use mail_ui::ui::native::{ActivationToken, Request, Requests};
use mail_ui::ui::space::{self, Mail};
use porter_core::AccountId;
use tokio::sync::mpsc::UnboundedSender;

use super::seed::with_account;
use super::window::{Window, account, hours_ago};

/// The second account, at home.
fn home() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"))
}

/// Its address.
const HOME: &str = "me@home.test";

/// The one conversation in its Inbox.
const SUBJECT: &str = "Dinner on Sunday";

/// Add the home account and deliver [`SUBJECT`] into its Inbox.
fn with_home(store: &SqliteStore) {
    with_account(store, home(), HOME);
    let raw = format!(
        "From: Mum <mum@home.test>\r\nTo: Me <{HOME}>\r\nSubject: {SUBJECT}\r\nDate: {}\r\n\
         Message-ID: <home0@home.test>\r\n\r\nCome round at six.\r\n",
        hours_ago(1)
    );
    absorb(
        store,
        home(),
        MailboxRef {
            account: home(),
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: "home0".to_owned(),
            },
            raw: raw.into_bytes(),
        }],
        false,
        chrono::Utc::now(),
    )
    .expect("the message is absorbed");
}

/// The home conversation's id.
fn dinner(store: &SqliteStore) -> ThreadId {
    let query = Query {
        filter: Filter::Account(home()),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 10,
        },
    };
    store
        .threads(&query, chrono::Utc::now())
        .expect("the home account's mail")
        .items
        .iter()
        .find(|thread| thread.subject == SUBJECT)
        .map(|thread| thread.id)
        .expect("the home conversation")
}

/// The window over both accounts, with two Spaces: "Work" over `work` and "Home" over `home`,
/// the window in Work; and the queue a click's request is pushed into.
fn two_spaces(work: Vec<AccountId>, home: Vec<AccountId>) -> (Window, UnboundedSender<Request>) {
    let (requests, queue): (Requests, UnboundedSender<Request>) = Requests::local();
    let window = Window::open_with(chrono::Utc::now(), with_home, move |dir, contexts| {
        let look = ds::prelude::SpaceLook::default;
        let spaces = space::built(
            vec![
                ("Work".to_owned(), look(), Mail::of(work)),
                ("Home".to_owned(), look(), Mail::of(home)),
            ],
            0,
        );
        let dirs = WindowDirs {
            config: dir.join("config"),
            state: dir.join("state"),
        };
        space::save(Some(&dirs), &spaces).expect("the Spaces are written");
        contexts.with(requests)
    });
    (window, queue)
}

/// The name of the Space the window is in, as its sidebar's head shows it.
fn space_shown(window: &Window) -> String {
    window.text(".ds-space-head")
}

/// The Space the window last wrote as the one it is in.
fn space_stored(window: &Window) -> String {
    space::stored(&window.dirs())
        .map(|spaces| spaces.current().name.clone())
        .unwrap_or_default()
}

/// A click on a banner for home mail, made while the window is in Work, which holds only the
/// work account: the window moves to Home and the reader shows the conversation there.
#[test]
fn a_click_moves_the_window_to_the_space_that_holds_the_messages_account() {
    let (mut window, queue) = two_spaces(vec![account()], vec![home()]);
    assert!(
        space_shown(&window).contains("Work"),
        "{}",
        space_shown(&window)
    );
    assert_eq!(
        window.harness.count(".reader-head h2"),
        0,
        "a reader opened unasked"
    );

    let thread = dinner(&window.store);
    queue
        .send(Request::Thread {
            thread,
            token: ActivationToken::new("click-1"),
        })
        .expect("the window's queue");
    window.until("the reader shows the home conversation", |h| {
        h.text_of(".reader-head h2")
            .is_some_and(|heading| heading.contains(SUBJECT))
    });
    assert!(
        space_shown(&window).contains("Home"),
        "{}",
        space_shown(&window)
    );
    assert_eq!(space_stored(&window), "Home", "the switch was not written");
    assert_eq!(window.subjects(), [SUBJECT], "the list is not Home's Inbox");
}

/// The same click while the window is in a Space that holds the home account too: it stays.
#[test]
fn a_click_stays_in_the_space_the_window_is_in_when_it_holds_the_account() {
    let (mut window, queue) = two_spaces(vec![account(), home()], vec![home()]);
    let thread = dinner(&window.store);
    queue
        .send(Request::Thread {
            thread,
            token: None,
        })
        .expect("the window's queue");
    window.until("the reader shows the home conversation", |h| {
        h.text_of(".reader-head h2")
            .is_some_and(|heading| heading.contains(SUBJECT))
    });
    assert!(
        space_shown(&window).contains("Work"),
        "{}",
        space_shown(&window)
    );
    assert_eq!(space_stored(&window), "Work");
}

/// A summary's click asks for the home account's inbox, from Work with a work conversation open:
/// the window moves to Home and shows its Inbox, nothing open.
#[test]
fn a_summarys_click_shows_its_accounts_inbox_in_the_space_that_holds_it() {
    let (mut window, queue) = two_spaces(vec![account()], vec![home()]);
    window.click(".list .ds-thread");
    window.until("a work conversation is open", |h| {
        h.count(".reader-head h2") == 1
    });
    queue
        .send(Request::Inbox {
            account: home(),
            token: None,
        })
        .expect("the window's queue");
    window.until("the window is in Home", |h| {
        h.text_of(".ds-space-head")
            .is_some_and(|name| name.contains("Home"))
    });
    window.until("Home's Inbox, nothing open", |h| {
        h.count(".reader-head h2") == 0
            && h.text_of(".list .ds-thread-sub")
                .is_some_and(|subject| subject.contains(SUBJECT))
    });
}
