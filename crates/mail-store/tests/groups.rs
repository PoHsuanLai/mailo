//! Contact groups: stored, listed, replaced and forgotten the same way by the SQLite store and
//! the in-memory one, and surviving the migration that made room for them.

use mail_store::{Edit, Group, GroupHome, GroupId, MemoryStore, SqliteStore, Store};

/// Run `scenario` on both stores and hand back what each produced.
fn both<T>(scenario: impl Fn(&dyn Store) -> T) -> (T, T) {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::in_memory(dir.path()).unwrap();
    let memory = MemoryStore::new();
    (scenario(&sqlite), scenario(&memory))
}

fn local(uid: &str, name: &str, members: &[&str]) -> Group {
    Group {
        id: GroupId::local(uid),
        uid: Some(uid.to_owned()),
        name: name.to_owned(),
        members: members.iter().map(|m| (*m).to_owned()).collect(),
        home: GroupHome::Local,
    }
}

fn synced(href: &str, name: &str, members: &[&str]) -> Group {
    Group {
        id: GroupId(href.to_owned()),
        uid: None,
        name: name.to_owned(),
        members: members.iter().map(|m| (*m).to_owned()).collect(),
        home: GroupHome::Book {
            url: "https://dav.example.test/book/".to_owned(),
            href: href.to_owned(),
            edit: Edit::Synced,
        },
    }
}

#[test]
fn groups_are_kept_listed_replaced_and_forgotten_alike() {
    let (a, b) = both(|store| {
        let before = store.groups().unwrap();
        let team = local(
            "urn:uuid:03a0e51f-d1aa-4385-8a53-e29025acd8af",
            "team",
            &[
                "mailto:ada@example.test",
                // Names a card nobody here has: kept all the same.
                "urn:uuid:ffffffff-0000-4000-8000-000000000000",
            ],
        );
        let book = synced(
            "https://dav.example.test/book/family.vcf",
            "Family",
            &["mailto:mum@example.test"],
        );
        let empty = local("empty-1", "Āfter everyone", &[]);
        store.put_group(&team).unwrap();
        store.put_group(&book).unwrap();
        store.put_group(&empty).unwrap();
        let listed = store.groups().unwrap();

        let mut edited = book.clone();
        edited.name = "The family".to_owned();
        edited.members.push("mailto:dad@example.test".to_owned());
        edited.home = GroupHome::Book {
            url: "https://dav.example.test/book/".to_owned(),
            href: "https://dav.example.test/book/family.vcf".to_owned(),
            edit: Edit::Edited,
        };
        store.put_group(&edited).unwrap();
        let replaced = store.group(&book.id).unwrap();

        let forgot = store.delete_group(&team.id).unwrap();
        let again = store.delete_group(&team.id).unwrap();
        let missing = store.group(&team.id).unwrap();
        let left: Vec<String> = store
            .groups()
            .unwrap()
            .into_iter()
            .map(|g| g.name)
            .collect();
        (before, listed, replaced, forgot, again, missing, left)
    });
    assert_eq!(a, b);
    let (before, listed, replaced, forgot, again, missing, left) = a;
    assert!(before.is_empty());
    let names: Vec<&str> = listed.iter().map(|g| g.name.as_str()).collect();
    // ASCII case folded, anything else by its bytes: "Ā" sorts after every ASCII letter.
    assert_eq!(names, ["Family", "team", "Āfter everyone"]);
    assert_eq!(
        listed[1].members,
        [
            "mailto:ada@example.test",
            "urn:uuid:ffffffff-0000-4000-8000-000000000000"
        ]
    );
    assert!(listed[2].members.is_empty(), "a group of nobody is kept");
    let replaced = replaced.unwrap();
    assert_eq!(replaced.name, "The family");
    assert_eq!(replaced.members.len(), 2);
    assert!(matches!(
        replaced.home,
        GroupHome::Book {
            edit: Edit::Edited,
            ..
        }
    ));
    assert!(forgot);
    assert!(!again);
    assert_eq!(missing, None);
    assert_eq!(left, ["The family", "Āfter everyone"]);
}

#[test]
fn a_store_from_before_groups_opens_with_none() {
    let dir = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(dir.path().join("mail.db")).unwrap();
    for (version, sql) in mail_store::migrate::MIGRATIONS {
        if *version >= 22 {
            break;
        }
        db.execute_batch(sql).unwrap();
        if *version > 1 {
            db.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                [version],
            )
            .unwrap();
        }
    }
    drop(db);
    std::fs::create_dir_all(dir.path().join("blobs")).unwrap();
    let store = SqliteStore::open(dir.path().join("mail.db"), dir.path().join("blobs")).unwrap();
    assert!(store.groups().unwrap().is_empty());
    let group = local("u", "g", &["mailto:x@example.test"]);
    store.put_group(&group).unwrap();
    assert_eq!(store.group(&group.id).unwrap(), Some(group));
}
