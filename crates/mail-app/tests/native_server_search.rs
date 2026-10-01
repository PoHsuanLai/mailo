//! "Search … on the server", driven the way its user drives it in the real window on Blitz
//! (`ds_harness::Harness`): a search that finds nothing here ends its list with the offer, and
//! pressing it lists what the server found, marked as found there.
//!
//! The server is a `ServerSearcher` that answers as the runtime's search would: it keeps a
//! message the store did not hold, as headers, marks it found, and names it. The protocols are
//! tested where they are spoken (`mail-proto`'s tables and traces, `mail-runtime`'s servers);
//! this is the window. It is handed no directories, so it writes no file anywhere, and the
//! automatic search is off, as it is until someone turns it on.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_harness::{Driver, Harness, HarnessConfig, Query, Viewport};

#[path = "support/drive.rs"]
mod drive;
use drive::Drive;
use ds_blitz::{NetPolicy, PrintOutcome};
use mail_app::ui::native::ServerSearcher;
use mail_domain::*;
use mail_runtime::{Arrival, Searched, ServerHits, absorb};
use mail_store::{SqliteStore, Store};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// The server's hit: found by a word in its body, which a header fetch does not bring, so the
/// store cannot match it and it is listed under "From the server".
const HIT: &str = "Numbers for March 2019";

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn plan() -> AccountPlan {
    AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Imap {
            host: "imap.example.test".to_owned(),
            port: 993,
            tls: Tls::Implicit,
        },
        outgoing: Outgoing::Smtp {
            host: "smtp.example.test".to_owned(),
            port: 465,
            tls: Tls::Implicit,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    }
}

/// A store in `dir` with one IMAP account and one message in its inbox.
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', ?2, datetime('now'))",
            [ACCOUNT.to_string(), serde_json::to_string(&plan()).unwrap()],
        )
        .unwrap();
    let now = chrono::Utc::now();
    let raw = format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: Flight to the conference\r\n\
         Date: {}\r\nMessage-ID: <here@example.test>\r\n\r\nSee you there.\r\n",
        now.to_rfc2822()
    );
    absorb(
        &store,
        ACCOUNT,
        inbox(),
        None,
        vec![Arrival {
            remote: RemoteRef::Imap {
                mailbox: "INBOX".to_owned(),
                uidvalidity: 1,
                uid: 1,
            },
            raw: raw.into_bytes(),
        }],
        false,
        now,
    )
    .unwrap();
    Arc::new(store)
}

fn inbox() -> MailboxRef {
    MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    }
}

/// A server that holds [`HIT`] in its All Mail: asked, it is kept here as headers and marked
/// found, as `mail_runtime`'s search keeps what it finds. Records each line it was asked.
fn server(asked: Arc<Mutex<Vec<String>>>, calls: Arc<AtomicUsize>) -> ServerSearcher {
    ServerSearcher(Arc::new(move |store, account, input, now| {
        calls.fetch_add(1, Ordering::SeqCst);
        asked.lock().unwrap().push(input.to_owned());
        let head = format!(
            "From: Bob <bob@example.test>\r\nTo: me@example.test\r\nSubject: {HIT}\r\n\
             Date: Wed, 6 Mar 2019 12:00:00 +0000\r\nMessage-ID: <budget@example.test>\r\n\r\n"
        );
        let remote = RemoteRef::Imap {
            mailbox: "[Gmail]/All Mail".to_owned(),
            uidvalidity: 11,
            uid: 502,
        };
        let patch = absorb(
            &store,
            account,
            MailboxRef {
                account,
                path: "[Gmail]/All Mail".to_owned(),
            },
            None,
            vec![Arrival {
                remote: remote.clone(),
                raw: head.into_bytes(),
            }],
            true,
            now,
        )
        .map_err(|e| e.to_string())?;
        let new: Vec<MessageId> = patch
            .changes
            .iter()
            .filter_map(|c| match c {
                Change::MessageUpsert(m) => Some(m.id),
                _ => None,
            })
            .collect();
        store
            .mark_found(account, &new, now)
            .map_err(|e| e.to_string())?;
        let messages = store
            .held_at(account, &[remote])
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(_, id)| id)
            .collect();
        Ok(Searched::Found(ServerHits {
            messages,
            fetched: new.len(),
            more: 0,
        }))
    }))
}

struct Open {
    harness: Harness,
    store: Arc<SqliteStore>,
    asked: Arc<Mutex<Vec<String>>>,
    calls: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

fn open() -> Open {
    open_with(None)
}

/// The window, with `dirs` as its directories when a case needs a setting kept in them.
fn open_with(dirs: Option<mail_app::appearance::WindowDirs>) -> Open {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(&store),
        mail_app::view::Appearance::default(),
        mail_app::space::Spaces::default(),
        dirs,
        mail_app::ui::Start::Inbox,
    )
    .with(printer)
    .with(server(asked.clone(), calls.clone()));
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    Open {
        harness,
        store,
        asked,
        calls,
        _dir: dir,
    }
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

fn press(harness: &mut Harness, selector: &str) {
    let at = centre(harness, selector);
    harness.click(at);
    harness.advance(ms(400));
}

fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        harness.key(if c == ' ' { Key::Space } else { Key::Char(c) });
    }
    harness.advance(ms(100));
}

/// The rows' subjects, top to bottom. A row quire's list is still sliding out is the previous
/// answer, so it is not one of these.
fn subjects(harness: &Harness) -> Vec<String> {
    (1..=harness.count(".list .ds-list > *"))
        .filter_map(|n| {
            let item = format!(".list .ds-list > .ds-list-item:nth-child({n})");
            if harness.attr(&item, "data-exit").is_some() {
                return None;
            }
            harness.text_of(&format!("{item} .ds-thread-sub"))
        })
        .collect()
}

/// Advance, with time passing, until `done` holds: the list and the server's answer each land
/// from a blocking thread, which wakes nothing while it runs, so this waits for the named state
/// rather than for a quiet spell (FINDINGS F184). Bounded by the wall clock, not by frames.
fn until(harness: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(60) {
        if done(harness) {
            return;
        }
        harness.advance(ms(50));
    }
    panic!("{what} never happened:\n{}", harness.html());
}

const OFFER: &str = ".server-search-one button";

#[test]
fn a_search_with_nothing_here_offers_the_server_and_lists_what_it_finds() {
    let Open {
        mut harness,
        store,
        asked,
        calls,
        _dir,
    } = open();
    assert_eq!(
        harness.count(OFFER),
        0,
        "nothing is offered before a search"
    );

    press(&mut harness, ".search input");
    type_text(&mut harness, "spreadsheet");
    until(&mut harness, "the search settling", |h| h.count(OFFER) == 1);
    assert_eq!(
        subjects(&harness),
        Vec::<String>::new(),
        "nothing here matches"
    );
    let offer = harness.text_of(OFFER).unwrap_or_default();
    assert!(
        offer.contains("Search me@example.test on the server"),
        "{offer}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "not asked until pressed");
    let before = store.count(&Filter::All, chrono::Utc::now()).unwrap();

    // Where the list has come to rest, so the press lands on the button and not where it was.
    harness.advance(ms(400));
    press(&mut harness, OFFER);
    until(&mut harness, "the press reaching the server", |_| {
        calls.load(Ordering::SeqCst) == 1
    });
    until(&mut harness, "the server's hit being listed", |h| {
        subjects(h) == vec![HIT.to_owned()]
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        asked.lock().unwrap().clone(),
        vec!["spreadsheet".to_owned()]
    );
    assert_eq!(
        store.count(&Filter::All, chrono::Utc::now()).unwrap(),
        before + 1,
        "the hit is kept like any other message"
    );
    let html = harness.html();
    assert!(html.contains("From the server"), "{html}");
    assert_eq!(
        harness.count("[*|data-chip=\"from the server\"]"),
        1,
        "the row says where it came from:\n{html}"
    );
    let said = harness
        .text_of(".server-search-one .status")
        .unwrap_or_default();
    assert!(
        said.contains("1 conversation found on me@example.test's server, 1 new here"),
        "{said}"
    );
}

#[test]
fn a_search_that_finds_mail_here_still_ends_with_the_offer() {
    let Open {
        mut harness, calls, ..
    } = open();
    press(&mut harness, ".search input");
    type_text(&mut harness, "flight");
    until(&mut harness, "the search listing its match", |h| {
        subjects(h) == vec!["Flight to the conference".to_owned()]
    });
    until(&mut harness, "the offer at its end", |h| {
        h.count(OFFER) == 1
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0, "not asked by itself");
    // Emptying the box is the place again, with nothing offered.
    for _ in 0.."flight".len() {
        harness.key(Key::Backspace);
    }
    until(&mut harness, "the place coming back", |h| {
        h.count(OFFER) == 0
    });
}

#[test]
fn turned_on_the_server_is_asked_once_as_the_search_is_shown() {
    let config = tempfile::tempdir().unwrap();
    let dirs = mail_app::appearance::WindowDirs {
        config: config.path().join("config"),
        state: config.path().join("state"),
    };
    mail_app::server_search::save(&dirs.config, mail_app::server_search::Automatic::On).unwrap();
    let Open {
        mut harness, asked, ..
    } = open_with(Some(dirs));
    press(&mut harness, ".search input");
    type_text(&mut harness, "spreadsheet");
    until(&mut harness, "the server's hit being listed unasked", |h| {
        subjects(h) == vec![HIT.to_owned()]
    });
    // Drawing it again, and the revision the search itself moved, ask nothing more. A line the
    // box settled on while it was typed may have been asked too; none is asked twice.
    harness.advance(ms(1500));
    let asked = asked.lock().unwrap().clone();
    let lines: Vec<&String> = asked.iter().filter(|l| *l == "spreadsheet").collect();
    assert_eq!(lines.len(), 1, "asked once for the line: {asked:?}");
    let mut once = asked.clone();
    once.dedup();
    assert_eq!(once, asked, "a line asked twice: {asked:?}");
}
