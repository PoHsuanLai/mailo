//! Opening a folder, as data: the filter its place lists, when opening it fetches, and what the
//! list's title names. The window around it is in `sidebar/folder_place_tests.rs`.

use super::{Recent, again, title_address};
use crate::view::{Shell, folder_of, places_with};
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use mail_domain::*;

const ONE: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1"));
const TWO: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c2"));

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_750_000_000 + seconds, 0).unwrap()
}

fn mailbox(account: AccountId, path: &str) -> MailboxRef {
    MailboxRef {
        account,
        path: path.to_owned(),
    }
}

/// A shell whose sidebar has one folder place per `(name, mailbox)`, with the first chosen.
fn in_folder(folders: &[(String, MailboxRef)]) -> Shell {
    let mut shell = Shell {
        places: places_with(&[], folders),
        ..Shell::default()
    };
    let index = shell
        .places
        .iter()
        .position(|place| folder_of(place).is_some())
        .unwrap();
    shell.select(index);
    shell
}

fn summary(account: AccountId) -> ThreadSummary {
    ThreadSummary {
        id: ThreadId::from_uuid(uuid::Uuid::from_u128(7)),
        account,
        subject: "receipt".to_owned(),
        snippet: String::new(),
        from: Address {
            name: None,
            email: "shop@example.test".to_owned(),
        },
        participants: vec![],
        recipients: vec![],
        last_date: at(0),
        message_count: 1,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailboxes: MailboxSet::only(MailboxRole::Archive),
        labels: vec![],
        attachments: Attachments::None,
        snooze: Snooze::Inactive,
        pin: Pin::Unpinned,
    }
}

#[test]
fn a_folder_place_lists_its_account_and_exactly_its_path() {
    // The filter is asked of the shell, as the list asks for it, not rebuilt here.
    const PATHS: &[&str] = &["Projects/2026", "收據", "旅行/京都", "Old news"];
    for path in PATHS {
        let here = mailbox(ONE, path);
        let shell = in_folder(&[((*path).to_owned(), here.clone())]);
        let filter = shell.query(10).filter;
        assert_eq!(
            filter,
            Filter::And(vec![Filter::Account(ONE), Filter::InFolder(here.clone())]),
            "{path}"
        );
        let fits = |account: AccountId, folders: &[MailboxRef]| {
            // Each address one message's, filed as held there (a user folder: archived).
            let placed = Placed::of_message(
                MailboxRole::Archive,
                folders.to_vec(),
                &FolderRoles::default(),
                &[],
            );
            filter.fit(&MatchCtx {
                summary: &summary(account),
                corpus: None,
                folders: &placed,
                now: at(0),
            })
        };
        assert!(fits(ONE, std::slice::from_ref(&here)), "{path}");
        // A parent, a near spelling and the same path on another account are other folders.
        for other in [
            mailbox(ONE, "Projects"),
            mailbox(ONE, &format!("{path} ")),
            mailbox(ONE, &path.to_lowercase()),
        ]
        .into_iter()
        .filter(|other| other.path != *path)
        {
            assert!(
                !fits(ONE, std::slice::from_ref(&other)),
                "{path} took {}",
                other.path
            );
        }
        assert!(
            !fits(TWO, &[mailbox(TWO, path)]),
            "{path} took another account's folder of the same name"
        );
    }
}

#[test]
fn folder_places_come_after_the_labels_so_the_badges_line_up() {
    let label = LabelId::from_uuid(uuid::Uuid::from_u128(9));
    let places = places_with(
        &[("travel".to_owned(), label)],
        &[("2026".to_owned(), mailbox(ONE, "Projects/2026"))],
    );
    let defaults = crate::view::default_places().len();
    assert_eq!(places.len(), defaults + 2);
    assert!(crate::view::is_label_place(&places[defaults]));
    assert_eq!(places[defaults + 1].name, "2026");
    assert_eq!(
        folder_of(&places[defaults + 1]),
        Some(&mailbox(ONE, "Projects/2026"))
    );
    assert!(places[..=defaults].iter().all(|p| folder_of(p).is_none()));
}

#[test]
fn opening_fetches_once_a_minute_per_folder_and_sync_resets_it() {
    let receipts = mailbox(ONE, "收據");
    let projects = mailbox(ONE, "Projects/2026");
    let mut recent = Recent::default();
    assert!(recent.due(&receipts, at(0)), "never fetched");
    recent.mark(receipts.clone(), at(0));
    let window = again().num_seconds();
    const CASES: &[(i64, bool)] = &[(0, false), (1, false), (59, false), (60, true), (600, true)];
    for (later, due) in CASES {
        assert_eq!(recent.due(&receipts, at(*later)), *due, "{later} s later");
    }
    assert_eq!(window, 60);
    assert!(recent.due(&projects, at(1)), "another folder is its own");
    assert!(
        recent.due(&mailbox(TWO, "收據"), at(1)),
        "the same path on another account is another folder"
    );
    // A clock that went backwards is not "within the minute".
    assert!(recent.due(&receipts, at(0) - TimeDelta::seconds(5)));
    recent.mark(receipts.clone(), at(100));
    assert!(!recent.due(&receipts, at(130)), "marking again restarts it");
    recent.forget();
    assert!(recent.due(&receipts, at(130)), "Sync was pressed");
}

#[test]
fn the_title_names_the_account_only_when_several_are_in_view() {
    let accounts = vec![
        (ONE, "me@one.example".to_owned()),
        (TWO, "me@two.example".to_owned()),
    ];
    let shell = in_folder(&[("收據".to_owned(), mailbox(TWO, "收據"))]);
    assert_eq!(
        title_address(&shell, &accounts).as_deref(),
        Some("me@two.example")
    );
    assert_eq!(title_address(&shell, &accounts[1..]), None, "only one");
    let single = Shell {
        scope: vec![TWO],
        ..shell.clone()
    };
    assert_eq!(title_address(&single, &accounts), None, "a Space of one");
    let pair = Shell {
        scope: vec![ONE, TWO],
        ..shell.clone()
    };
    assert_eq!(
        title_address(&pair, &accounts).as_deref(),
        Some("me@two.example")
    );
    let tile = Shell {
        account: Some(TWO),
        ..shell.clone()
    };
    assert_eq!(
        title_address(&tile, &accounts),
        None,
        "the tile names it already"
    );
    let inbox = Shell {
        selected: 0,
        ..shell
    };
    assert_eq!(title_address(&inbox, &accounts), None, "not a folder");
}
