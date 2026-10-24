//! Microsoft Graph's addresses in `remote_map`: kept and read back as Graph's,
//! never as POP3's, and moved to the id a folder move returns.
//!
//! Every scenario runs against the SQLite store and the in-memory one and compares what each
//! answers, the parity the rest of the store is held to.

use crate::support::*;
use mail_domain::*;

#[test]
fn a_graph_address_reads_back_as_graph_in_every_query() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, graph("INBOX", "AAMk/one=="));
        (
            store.remote_refs(&mailbox("INBOX")).unwrap(),
            store.remotes_of(m.id).unwrap(),
            store.unfetched_in(&mailbox("INBOX"), None, 10).unwrap(),
        )
    });
    assert_eq!(sqlite, memory);
    let expected = vec![graph("INBOX", "AAMk/one==")];
    assert_eq!(sqlite.0, expected);
    assert_eq!(sqlite.1, expected);
    assert_eq!(sqlite.2, expected);
}

#[test]
fn a_move_remaps_the_message_to_its_new_id_and_folder() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, graph("INBOX", "old"));
        store
            .remap(
                acct_account(),
                &graph("INBOX", "old"),
                &graph("Archive", "new"),
            )
            .unwrap();
        // The folder it left says it has gone: that is not a deletion any more.
        let mut left = ingest("INBOX");
        left.gone.push(graph("INBOX", "old"));
        store.ingest(acct_account(), left).unwrap();
        // And the folder it arrived in reports it read, under the new id.
        let mut arrived = ingest("Archive");
        arrived
            .flags
            .push((graph("Archive", "new"), ReadState::Read, Star::Unstarred));
        store.ingest(acct_account(), arrived).unwrap();
        let held = store.message(m.id).unwrap();
        (
            store.remote_refs(&mailbox("INBOX")).unwrap(),
            store.remotes_of(m.id).unwrap(),
            held.read,
        )
    });
    assert_eq!(sqlite, memory);
    assert!(sqlite.0.is_empty(), "{:?}", sqlite.0);
    assert_eq!(sqlite.1, vec![graph("Archive", "new")]);
    assert_eq!(sqlite.2, ReadState::Read);
}

#[test]
fn a_remap_onto_an_address_a_sync_already_mapped_leaves_one_row() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, graph("INBOX", "old"));
        let mut there = ingest("Archive");
        there.messages.push(Fetched {
            remote: graph("Archive", "new"),
            key: m.key.clone(),
            raw: BlobId::generate(),
            message: m.clone(),
        });
        store.ingest(acct_account(), there).unwrap();
        store
            .remap(
                acct_account(),
                &graph("INBOX", "old"),
                &graph("Archive", "new"),
            )
            .unwrap();
        store.remotes_of(m.id).unwrap()
    });
    assert_eq!(sqlite, memory);
    assert_eq!(sqlite, vec![graph("Archive", "new")]);
}

#[test]
fn remapping_an_address_nobody_holds_does_nothing() {
    let (sqlite, memory) = both(|store| {
        store
            .remap(
                acct_account(),
                &graph("INBOX", "never"),
                &graph("Archive", "x"),
            )
            .unwrap();
        store.remote_refs(&mailbox("Archive")).unwrap()
    });
    assert_eq!(sqlite, memory);
    assert!(sqlite.is_empty());
}
