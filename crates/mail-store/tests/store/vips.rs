//! VIPs, kept alike by both stores: marked once per address whatever its case, listed by
//! address, a name kept until another is given, taken off again, and kept across a reopen.

use mail_store::{MemoryStore, SqliteStore, Store, StoreError, Vip};

fn vip(address: &str, name: Option<&str>) -> Vip {
    Vip {
        address: address.to_owned(),
        name: name.map(str::to_owned),
    }
}

#[test]
fn both_stores_mark_list_rename_and_unmark_a_vip() {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::open(dir.path().join("mail.db"), dir.path().join("blobs")).unwrap();
    let memory = MemoryStore::new();
    let stores: [(&str, &dyn Store); 2] = [("sqlite", &sqlite), ("memory", &memory)];
    for (name, store) in stores {
        assert_eq!(store.vips().unwrap(), [], "{name}: nobody yet");
        let marked = store.put_vip(" Dana@Example.TEST ", Some("Dana")).unwrap();
        assert_eq!(marked, vip("dana@example.test", Some("Dana")), "{name}");
        store.put_vip("<ada@example.test>", None).unwrap();
        assert_eq!(
            store.vips().unwrap(),
            [
                vip("ada@example.test", None),
                vip("dana@example.test", Some("Dana"))
            ],
            "{name}: by address, the address folded as the book folds it"
        );
        // Marked again: still one, and no name given keeps the one there was.
        store.put_vip("DANA@example.test", None).unwrap();
        store.put_vip("ada@example.test", Some("Ada L")).unwrap();
        assert_eq!(
            store.vips().unwrap(),
            [
                vip("ada@example.test", Some("Ada L")),
                vip("dana@example.test", Some("Dana"))
            ],
            "{name}: marked twice"
        );
        assert!(
            matches!(
                store.put_vip("not an address", None),
                Err(StoreError::BadAddress(_))
            ),
            "{name}: no @"
        );
        assert!(store.delete_vip("Ada@Example.test").unwrap(), "{name}");
        assert!(!store.delete_vip("ada@example.test").unwrap(), "{name}");
        assert!(!store.delete_vip("nobody").unwrap(), "{name}");
        assert_eq!(
            store.vips().unwrap(),
            [vip("dana@example.test", Some("Dana"))],
            "{name}: unmarked"
        );
    }
}

#[test]
fn a_vip_outlives_its_contact_and_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let open = || SqliteStore::open(dir.path().join("mail.db"), dir.path().join("blobs")).unwrap();
    {
        let store = open();
        store
            .put_contact(
                "dana@example.test",
                Some("Dana"),
                &mail_store::Origin::Manual,
            )
            .unwrap();
        store.put_vip("dana@example.test", Some("Dana")).unwrap();
        assert!(store.delete_contact("dana@example.test").unwrap());
    }
    assert_eq!(
        open().vips().unwrap(),
        [vip("dana@example.test", Some("Dana"))]
    );
}
