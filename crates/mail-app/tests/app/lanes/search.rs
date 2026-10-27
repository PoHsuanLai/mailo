//! Lane 15: searching. ⌘K puts the keyboard in the search panel, where typed letters are its
//! query and not shortcuts, the first Escape empties it and the second gives the keyboard back;
//! the toolbar's magnifier brings it up too, what is typed filters the rows, and ⌘K brings it
//! up over an open conversation, where ⌘F finds in the conversation instead (`find`). A search
//! kept with Save as View is a place in the sidebar, grouped as it was saved, and is deleted
//! from its editor. A search that finds nothing here offers the server, which is asked only
//! when that is pressed, or at once when Settings says so. A search the panel leaves behind is
//! the list's header until it is cleared, by its × or Esc.

use mail_app::ui::copy;
use mail_app::ui::native::ServerSearcher;
use mail_core::{Arrival, Searched, ServerHits, SqliteStore, Store, absorb};
use mail_domain::{
    AccountPlan, AuthPlan, Change, GroupKey, Incoming, MailboxRef, MessageId, Outgoing, ReadState,
    RemoteRef, SaslMech, Tls, Username, View,
};
use std::sync::{Arc, Mutex};

use super::drive::{Drive, Key, PRIMARY};
use super::look::conversation;
use super::window::{INBOX, Window, account, ms};
use crate::lanes::seed::ME;

/// The panel's field.
const FIELD: &str = ".ds-search-card input";
/// The server's hit: found by a word in its body, which a header fetch does not bring, so the
/// store cannot match it.
const HIT: &str = "Numbers for March 2019";
/// Search … on the server.
const OFFER: &str = ".server-search-one button";

/// The account as an IMAP account, which a server can be searched on.
fn on_imap(store: &SqliteStore) {
    let plan = AccountPlan {
        address: ME.to_owned(),
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
    };
    store.set_account_plan(account(), &plan).expect("the plan");
}

/// A server that holds [`HIT`] in its All Mail: asked, it is kept here as headers and marked
/// found, as `mail_runtime`'s search keeps what it finds. Records each line it was asked.
fn server(asked: Arc<Mutex<Vec<String>>>) -> ServerSearcher {
    ServerSearcher(Arc::new(move |client, account, input, now| {
        let store = client.store_for_test();
        asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(input.to_owned());
        let head = format!(
            "From: Bob <bob@example.test>\r\nTo: {ME}\r\nSubject: {HIT}\r\n\
             Date: Wed, 6 Mar 2019 12:00:00 +0000\r\nMessage-ID: <budget@example.test>\r\n\r\n"
        );
        let remote = RemoteRef::Imap {
            mailbox: "[Gmail]/All Mail".to_owned(),
            uidvalidity: 11,
            uid: 502,
        };
        let mailbox = MailboxRef {
            account: account.clone(),
            path: "[Gmail]/All Mail".to_owned(),
        };
        let arrival = Arrival {
            remote: remote.clone(),
            raw: head.into_bytes(),
        };
        let patch = absorb(
            &store,
            account.clone(),
            mailbox,
            None,
            vec![arrival],
            true,
            now,
        )
        .map_err(|e| e.to_string())?;
        let new: Vec<MessageId> = patch
            .changes
            .iter()
            .filter_map(|change| match change {
                Change::MessageUpsert(message) => Some(message.id),
                _ => None,
            })
            .collect();
        store
            .mark_found(account.clone(), &new, now)
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

/// The window over an IMAP account, with [`server`] as its server's search, and `settings`
/// written first.
fn open(
    asked: &Arc<Mutex<Vec<String>>>,
    settings: impl FnOnce(&mut mail_app::settings::MailSettings),
) -> Window {
    let asked = Arc::clone(asked);
    Window::open_with(chrono::Utc::now(), on_imap, move |dir, contexts| {
        mail_app::settings::change(&mail_app::settings::root_for(&dir.join("config")), settings)
            .expect("the settings are written");
        contexts.with(server(asked))
    })
}

/// The rows' subjects, top to bottom: a row still sliding out is the previous answer's.
fn rows(window: &Window) -> Vec<String> {
    let h = &window.harness;
    (1..=h.count(".list .ds-list-item[*|aria-posinset]"))
        .filter_map(|n| {
            let item = format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]");
            if h.attr(&item, "data-exit").is_some() {
                return None;
            }
            h.text_of(&format!("{item} .ds-thread-sub"))
        })
        .collect()
}

/// The list as drawn: a group header as `# title`, a row as its subject.
fn lines(window: &Window) -> Vec<String> {
    let h = &window.harness;
    (1..=h.count(".list .ds-list-item[*|aria-posinset]"))
        .filter_map(|n| {
            let at = format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]");
            if h.count(&format!("{at} .ds-section-header")) > 0 {
                h.text_of(&format!("{at} .ds-section-header-title"))
                    .map(|title| format!("# {}", title.trim()))
            } else {
                h.text_of(&format!("{at} .ds-thread-sub"))
            }
        })
        .collect()
}

/// ⌘K, and `text` typed in the panel's field.
fn search(window: &mut Window, text: &str) {
    window.press(&[PRIMARY], Key::Char('k'), 1);
    window.until("⌘K puts the keyboard in the panel", |h| {
        h.is_focused(FIELD)
    });
    window.type_text(text);
}

/// A press outside the panel: it goes, and its search stays the list's.
fn put_away(window: &mut Window) {
    window.harness.click(ds::prelude::Point {
        x: ds::prelude::Px(4.0),
        y: ds::prelude::Px(4.0),
    });
    window.until("the panel goes", |h| h.count(".ds-search-card") == 0);
}

#[test]
fn the_search_panel_filters_the_list_saves_a_view_and_offers_the_server() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let mut window = open(&asked, |_| {});

    // ⌘K, typed letters, then Escape twice.
    search(&mut window, "arch");
    window.until("the panel is up", |h| {
        h.count(".ds-search-card .ds-menu") == 1
    });
    assert_eq!(window.harness.count(".ds-palette"), 0, "a palette is drawn");
    assert_eq!(window.harness.attr(FIELD, "value").as_deref(), Some("arch"));
    window.press(&[], Key::Escape, 1);
    assert_eq!(window.harness.attr(FIELD, "value").as_deref(), Some(""));
    assert!(
        window.harness.is_focused(FIELD),
        "the first Escape left the field"
    );
    window.press(&[], Key::Escape, 1);
    window.until("the second Escape closes it", |h| {
        h.count(".ds-search-card") == 0 && h.is_focused(".app")
    });

    // The magnifier, and typing that filters the rows; out again, the rows back.
    window.click(".list-head .bar .ds-button");
    window.until("the magnifier brings it up", |h| h.is_focused(FIELD));
    window.type_text("invoice");
    window.until("only the invoice is listed", |h| {
        h.count(".list .ds-thread") == 1 && super::look::row_of(h, INBOX[1].1).is_some()
    });
    window.press(&[], Key::Escape, 2);
    window.until("the rows come back", |h| {
        h.count(".list .ds-thread") == INBOX.len() && h.count(".ds-search-card") == 0
    });

    // ⌘K over an open conversation.
    window.open_subject(INBOX[0].1);
    window.press(&[PRIMARY], Key::Char('k'), 1);
    window.until("⌘K brings it up", |h| h.is_focused(FIELD));
    window.press(&[], Key::Escape, 2);

    // Two read, two not; then in:inbox kept as a view, grouped unread first.
    for (_, subject) in &INBOX[..2] {
        window.open_subject(subject);
        window.until_stored("it is read", |store| {
            conversation(store, subject).read == ReadState::Read
        });
    }
    let place = |name: &str| format!("[*|data-place=\"{name}\"]");
    search(&mut window, "in:inbox");
    window.harness.advance(ms(800));
    put_away(&mut window);
    window.click(&format!("[*|aria-label=\"{}\"]", copy::aria::SAVE_AS_VIEW));
    window.until("the sheet opens", |h| h.count(".sheet-form") == 1);
    window.click(".sheet-form input");
    window.press(&[PRIMARY], Key::Char('a'), 1);
    window.type_text("Mine");
    window.click("[*|data-view=group]");
    window.click(".ds-menu .ds-menu-item:nth-child(3)");
    window.click("[*|data-view=save]");
    window.until("the sheet closes", |h| h.count(".sheet-form") == 0);
    let views = window.store.views().expect("the views");
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].name, "Mine");
    assert_eq!(views[0].group_by, Some(GroupKey::Read));
    window.until("the view is a place, and shown", |h| {
        h.count(&place("Mine")) == 1 && h.count(".list-head .search-face") == 0
    });
    assert_eq!(
        lines(&window),
        [
            format!("# {}", copy::headers::UNREAD),
            INBOX[2].1.to_owned(),
            INBOX[3].1.to_owned(),
            format!("# {}", copy::headers::READ),
            INBOX[0].1.to_owned(),
            INBOX[1].1.to_owned(),
        ]
    );

    // Deleted from its editor, it leaves the sidebar and the store.
    window.click(&format!("[*|aria-label=\"{}\"]", copy::aria::EDIT_VIEW));
    window.until("the editor opens", |h| h.count(".sheet-form") == 1);
    window.click("[*|data-view=delete]");
    window.until("the view goes", |h| {
        h.count(".sheet-form") == 0 && h.count(&place("Mine")) == 0
    });
    assert_eq!(window.store.views().expect("the views"), Vec::<View>::new());

    // A search that finds nothing here offers the server, asked only once that is pressed.
    window.click(&place("Inbox"));
    search(&mut window, "spreadsheet");
    window.until("the whole word has settled, with the offer", |h| {
        h.count(OFFER) == 1
            && h.attr(".server-search", "data-line").as_deref() == Some("spreadsheet")
    });
    assert!(rows(&window).is_empty(), "something here matched");
    assert!(asked.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
    window.harness.advance(ms(400));
    put_away(&mut window);
    window.click(OFFER);
    window.until("the server's hit is the one row", |h| {
        h.count(".list .ds-thread") == 1 && super::look::row_of(h, HIT).is_some()
    });
    assert_eq!(
        *asked.lock().unwrap_or_else(|e| e.into_inner()),
        ["spreadsheet"]
    );
    let server = copy::headers::FROM_SERVER;
    assert!(
        window.harness.html().contains(server),
        "no heading says where"
    );
    assert_eq!(
        window.harness.count(&format!("[*|data-chip=\"{server}\"]")),
        0,
        "the row repeats its heading"
    );
}

#[test]
fn turned_on_in_settings_the_server_is_asked_once_as_the_search_is_shown() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let mut window = open(&asked, |settings| {
        settings.search.server_automatically = mail_app::settings::ServerSearch::On;
    });
    search(&mut window, "spreadsheet");
    window.until("the server's hit is listed unasked", |h| {
        super::look::row_of(h, HIT).is_some()
    });
    // A prefix asked while the line was typed can be what listed it: wait for the whole line.
    let recorded = Arc::clone(&asked);
    window.until("the whole line is asked", move |_| {
        recorded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|line| line == "spreadsheet")
    });
    window.harness.advance(ms(1500));
    let asked = asked.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut once = asked.clone();
    once.dedup();
    assert_eq!(once, asked, "a line asked twice: {asked:?}");
}

/// While a search narrows the list and the panel is down, the search is the list's header: the
/// search as the panel's field shows it, its operators as chips, where it looked at its end,
/// Save as View and the ×, and none of the place's own tools. A press on it brings the panel back
/// over it; the ×, or Esc in the list, lets go of it and the place's header comes back.
#[test]
fn a_search_shown_is_the_lists_header_until_it_is_cleared() {
    let mut window = Window::open(|store| {
        let raw = format!(
            "From: Ada Lovelace <ada@example.test>\r\nTo: {ME}\r\nSubject: Lunch on Friday?\r\n\
             Date: {}\r\nMessage-ID: <lunch@example.test>\r\n\r\nShall we?\r\n",
            super::window::hours_ago(1)
        );
        super::window::deliver(store, "lunch", &raw);
    });
    let all = INBOX.len() + 1;
    let named = |name: &str| format!(".list-head [*|aria-label=\"{name}\"]");
    let face = ".list-head .search-face";
    let applied = |window: &mut Window| {
        search(window, "from:ada lunch");
        window.harness.advance(ms(800));
        window.press(&[], Key::Enter, 1);
        window.until("Return leaves the search in the list", |h| {
            h.count(".ds-search-card") == 0 && h.count(face) == 1
        });
        window.until("only Ada's lunch is listed", |h| {
            h.count(".list .ds-thread") == 1
        });
    };

    applied(&mut window);
    let h = &window.harness;
    assert_eq!(
        h.attr(face, "aria-label").as_deref(),
        Some(copy::aria::EDIT_SEARCH)
    );
    assert_eq!(
        h.text_of(&format!("{face} .ds-chip"))
            .as_deref()
            .map(str::trim),
        Some("from:ada")
    );
    assert_eq!(
        h.text_of(&format!("{face} .search-words"))
            .as_deref()
            .map(str::trim),
        Some("lunch")
    );
    assert_eq!(
        h.text_of(&format!("{face} .search-scope"))
            .as_deref()
            .map(str::trim),
        Some(copy::status::SEARCHED_ALL_MAIL)
    );
    assert_eq!(h.count(&named(copy::aria::SAVE_AS_VIEW)), 1);
    assert_eq!(h.count(&named(copy::aria::CLEAR_SEARCH)), 1);
    assert_eq!(h.count(&named(copy::aria::NEW_MESSAGE)), 0, "New Message");
    assert_eq!(h.count(&named(copy::aria::MORE)), 0, "the ⋯");
    assert_eq!(h.count(".list-head .list-title"), 0, "the place's title");
    // The search takes the header's width but for Save as View and the ×.
    let head = h.rect(".list-head").expect("the header");
    let shown = h.rect(face).expect("the search");
    assert!(
        shown.size.width.0 > head.size.width.0 * 0.6,
        "{shown:?} in {head:?}"
    );

    // A press on it brings the panel back over the search.
    window.click(face);
    window.until("the panel comes back over it", |h| {
        h.is_focused(FIELD) && h.attr(FIELD, "value").as_deref() == Some("from:ada lunch")
    });
    put_away(&mut window);
    assert_eq!(
        window.harness.count(face),
        1,
        "the search went with the panel"
    );

    // The × lets go of it: the place's header and its rows come back.
    window.click(&named(copy::aria::CLEAR_SEARCH));
    window.until("the × gives the place back", |h| {
        h.count(face) == 0
            && h.count(&named(copy::aria::NEW_MESSAGE)) == 1
            && h.count(&named(copy::aria::MORE)) == 1
            && h.count(".list .ds-thread") == all
    });

    // So does Esc in the list, with nothing open.
    applied(&mut window);
    window.press(&[], Key::Escape, 1);
    window.until("Esc gives the place back", |h| {
        h.count(face) == 0
            && h.count(&named(copy::aria::NEW_MESSAGE)) == 1
            && h.count(".list .ds-thread") == all
    });
}
