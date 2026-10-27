//! Opening a folder, as data: the filter its place lists, and when opening it fetches. The window
//! around it is in `sidebar/folder_place_tests.rs`.

use crate::ui::view::{Shell, folder_of, places_with};
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;

fn acct_one() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1"))
}
fn acct_two() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c2"))
}

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
        places: places_with(&[], folders, &[]),
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
        from: Address {
            name: None,
            email: "shop@example.test".to_owned(),
        },
        last_date: at(0),
        mailboxes: MailboxSet::only(MailboxRole::Archive),
        ..mail_domain::testing::summary()
    }
}

#[test]
fn a_folder_place_lists_its_account_and_exactly_its_path() {
    // The filter is asked of the shell, as the list asks for it, not rebuilt here.
    const PATHS: &[&str] = &["Projects/2026", "收據", "旅行/京都", "Old news"];
    for path in PATHS {
        let here = mailbox(acct_one(), path);
        let shell = in_folder(&[((*path).to_owned(), here.clone())]);
        let filter = shell.query(10).filter;
        assert_eq!(
            filter,
            Filter::And(vec![
                Filter::Account(acct_one()),
                Filter::InFolder(here.clone())
            ]),
            "{path}"
        );
        let fits = |account: AccountId, folders: &[MailboxRef]| {
            // Each address one message's, filed as held there (a user folder: archived).
            let placed = Placed::of_message(
                MailboxRole::Archive,
                folders.to_vec(),
                &FolderRoles::default(),
                &[],
                Vec::new(),
            );
            filter.fit(&MatchCtx {
                summary: &summary(account),
                corpus: None,
                folders: &placed,
                now: at(0),
            })
        };
        assert!(fits(acct_one(), std::slice::from_ref(&here)), "{path}");
        // A parent, a near spelling and the same path on another account are other folders.
        for other in [
            mailbox(acct_one(), "Projects"),
            mailbox(acct_one(), &format!("{path} ")),
            mailbox(acct_one(), &path.to_lowercase()),
        ]
        .into_iter()
        .filter(|other| other.path != *path)
        {
            assert!(
                !fits(acct_one(), std::slice::from_ref(&other)),
                "{path} took {}",
                other.path
            );
        }
        assert!(
            !fits(acct_two(), &[mailbox(acct_two(), path)]),
            "{path} took another account's folder of the same name"
        );
    }
}

#[test]
fn folder_places_come_after_the_labels_so_the_badges_line_up() {
    let label = LabelId::from_uuid(uuid::Uuid::from_u128(9));
    let places = places_with(
        &[("travel".to_owned(), label)],
        &[("2026".to_owned(), mailbox(acct_one(), "Projects/2026"))],
        &[],
    );
    let defaults = crate::ui::view::default_places().len();
    assert_eq!(places.len(), defaults + 2);
    assert!(crate::ui::view::is_label_place(&places[defaults]));
    assert_eq!(places[defaults + 1].name, "2026");
    assert_eq!(
        folder_of(&places[defaults + 1]),
        Some(&mailbox(acct_one(), "Projects/2026"))
    );
    assert!(places[..=defaults].iter().all(|p| folder_of(p).is_none()));
}
