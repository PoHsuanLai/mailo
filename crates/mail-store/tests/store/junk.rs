//! The junk filter's counts: each message is counted once, changing one's mind takes the old
//! count back, and each account's counts are its own.

use crate::support::*;
use mail_store::junk::{Judged, Learned};

fn words(of: &[&str]) -> Vec<String> {
    of.iter().map(|w| (*w).to_owned()).collect()
}

#[test]
fn a_message_is_counted_once_and_a_changed_mind_takes_the_first_count_back() {
    let store = sqlite();
    let (a, b) = (message(1), message(2));
    for m in [&a, &b] {
        deliver(&store, m, imap("INBOX", m.id.as_uuid().as_u128() as u32));
    }
    let prize = words(&["prize", "winner"]);
    let counts = |store: &mail_store::SqliteStore| {
        let c = store.junk_counts(acct_account(), &prize).unwrap();
        (c.spam, c.ham, c.words)
    };

    let steps = [
        (&a, Judged::Spam, Learned::New),
        (&a, Judged::Spam, Learned::Unchanged),
        (&b, Judged::Spam, Learned::New),
        (&a, Judged::Ham, Learned::Turned),
    ];
    let mut seen = vec![];
    for (m, judged, learned) in steps {
        let did = store
            .junk_learn(acct_account(), m.id, &prize, judged)
            .unwrap();
        assert_eq!(did, learned);
        seen.push(counts(&store));
    }
    let both = |s, h| {
        (
            s,
            h,
            vec![("prize".to_owned(), s, h), ("winner".to_owned(), s, h)],
        )
    };
    assert_eq!(seen, [both(1, 0), both(1, 0), both(2, 0), both(1, 1)]);
}

#[test]
fn one_accounts_counts_are_not_anothers() {
    let store = sqlite();
    let m = message(1);
    deliver(&store, &m, imap("INBOX", 1));
    store
        .junk_learn(acct_account(), m.id, &words(&["prize"]), Judged::Spam)
        .unwrap();
    let other = store.junk_counts(acct_other(), &words(&["prize"])).unwrap();
    assert_eq!((other.spam, other.ham, other.words.len()), (0, 0, 0));
}
