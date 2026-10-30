//! When each mailbox was last swept: what keeps the sweeps on their schedule across passes.

use super::support::{Both, acct_account, at, ingest, mailbox};
use mail_domain::SyncCursor;
use mail_store::Swept;

fn surveyed(both: &Both, path: &str) {
    for (_, store) in both.each() {
        let mut survey = ingest(path);
        survey.cursor = Some(SyncCursor::Pop);
        store.ingest(acct_account(), survey).unwrap();
    }
}

#[test]
fn a_mailbox_never_swept_has_no_times_and_a_mark_reads_back() {
    let both = Both::new();
    surveyed(&both, "INBOX");
    assert_eq!(
        both.agree(|s| s.swept(&mailbox("INBOX")).unwrap()),
        Swept::default()
    );

    for (_, store) in both.each() {
        store.mark_flags_swept(&mailbox("INBOX"), at(10)).unwrap();
    }
    assert_eq!(
        both.agree(|s| s.swept(&mailbox("INBOX")).unwrap()),
        Swept {
            flags: Some(at(10)),
            expunges: None,
        }
    );

    for (_, store) in both.each() {
        store
            .mark_expunges_swept(&mailbox("INBOX"), at(20))
            .unwrap();
    }
    assert_eq!(
        both.agree(|s| s.swept(&mailbox("INBOX")).unwrap()),
        Swept {
            flags: Some(at(10)),
            expunges: Some(at(20)),
        }
    );
    // Another mailbox is its own schedule.
    surveyed(&both, "Sent");
    assert_eq!(
        both.agree(|s| s.swept(&mailbox("Sent")).unwrap()),
        Swept::default()
    );
}

#[test]
fn the_marks_survive_a_later_cursor_ingest() {
    let both = Both::new();
    surveyed(&both, "INBOX");
    for (_, store) in both.each() {
        store.mark_flags_swept(&mailbox("INBOX"), at(10)).unwrap();
        store
            .mark_expunges_swept(&mailbox("INBOX"), at(20))
            .unwrap();
    }
    surveyed(&both, "INBOX");
    assert_eq!(
        both.agree(|s| s.swept(&mailbox("INBOX")).unwrap()),
        Swept {
            flags: Some(at(10)),
            expunges: Some(at(20)),
        }
    );
}

#[test]
fn a_mailbox_with_no_cursor_yet_holds_no_marks() {
    let both = Both::new();
    for (_, store) in both.each() {
        store.mark_flags_swept(&mailbox("INBOX"), at(10)).unwrap();
    }
    assert_eq!(
        both.agree(|s| s.swept(&mailbox("INBOX")).unwrap()),
        Swept::default()
    );
}

#[test]
fn held_through_and_the_unreadable_read_back_and_go_with_the_mailbox() {
    use mail_domain::RemoteRef;
    let both = Both::new();
    surveyed(&both, "INBOX");
    let through = SyncCursor::Imap {
        uidvalidity: 7,
        uidnext: 9,
        modseq: None,
    };
    let bad = RemoteRef::Imap {
        mailbox: "INBOX".to_owned(),
        uidvalidity: 7,
        uid: 3,
    };
    assert_eq!(
        both.agree(|s| s.held_through(&mailbox("INBOX")).unwrap()),
        None
    );
    for (_, store) in both.each() {
        store
            .mark_held_through(&mailbox("INBOX"), &through)
            .unwrap();
        store
            .mark_unreadable(&mailbox("INBOX"), std::slice::from_ref(&bad), at(10))
            .unwrap();
    }
    assert_eq!(
        both.agree(|s| s.held_through(&mailbox("INBOX")).unwrap()),
        Some(through)
    );
    assert_eq!(
        both.agree(|s| s.unreadable_since(&mailbox("INBOX"), at(5)).unwrap()),
        vec![bad.clone()]
    );
    assert!(
        both.agree(|s| s.unreadable_since(&mailbox("INBOX"), at(11)).unwrap())
            .is_empty(),
        "one tried before the cut is not resting"
    );
}
