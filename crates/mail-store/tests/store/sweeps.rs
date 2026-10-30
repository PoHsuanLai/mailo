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
