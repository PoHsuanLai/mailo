use super::*;
use crate::support_tests::{account, at, caps};
use mail_domain::{
    Address, BlobId, Change, ChangeId, MessageKey, Patch, ReadState, Star, ThreadId,
};

fn message(n: u128, from: &str, subject: &str, text: &str) -> Message {
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n)),
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n)),
        account: account(),
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: at(n as i64),
        from: Address::new(from),
        reply_to: vec![],
        to: vec![Address::named("Ada Lovelace", "ada@example.test")],
        cc: vec![],
        bcc: vec![],
        subject: subject.to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some(text.to_owned()),
            raw: BlobId::from_uuid(uuid::Uuid::from_u128(0xB000 + n)),
        },
        attachments: vec![],
    }
}

fn stored(store: &SqliteStore, mut message: Message, raw: &[u8]) -> Message {
    let blob = store.blobs().put(raw).unwrap();
    if let Body::Present { raw, .. } = &mut message.body {
        *raw = blob;
    }
    store
        .apply(
            account(),
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageUpsert(Box::new(message.clone()))],
            },
        )
        .unwrap();
    message
}

const SPAMMY: &str = "winner claim your free prize now lottery cash bonus";
const HAMMY: &str = "minutes from the meeting about the quarterly budget review";

/// A store whose filter has learned enough: spam about prizes, good mail about meetings.
fn trained() -> SqliteStore {
    let store = mail_store::testing::in_memory();
    mail_store::testing::seed_account(&store, account(), "ada@example.test");
    mail_store::testing::seed_default_identity(
        &store,
        account(),
        "ada@example.test",
        Some("Ada Lovelace"),
    );
    for n in 0..ENOUGH as u128 {
        let spam = stored(
            &store,
            message(n, "promo@prizes.example", "You won", SPAMMY),
            b"",
        );
        let ham = stored(
            &store,
            message(100 + n, "bob@work.example", "Minutes", HAMMY),
            b"",
        );
        learn(&store, &[spam], Judged::Spam).unwrap();
        learn(&store, &[ham], Judged::Ham).unwrap();
    }
    store
}

#[test]
fn a_messages_words_are_its_subjects_its_texts_and_its_senders_domain() {
    let words = words(&message(
        1,
        "Promo@Prizes.Example",
        "FREE prize!!",
        "a free 2026 lottery",
    ));
    assert_eq!(words, ["from:prizes.example", "free", "prize", "lottery"]);
}

#[test]
fn the_score_says_nothing_until_enough_is_learned_and_then_leans_the_right_way() {
    let counts = |spam, ham, words: Vec<(&str, u64, u64)>| JunkCounts {
        spam,
        ham,
        words: words
            .into_iter()
            .map(|(w, s, h)| (w.to_owned(), s, h))
            .collect(),
    };
    assert_eq!(
        score(&counts(4, 10, vec![("prize", 4, 0)])),
        None,
        "too little spam"
    );
    assert_eq!(
        score(&counts(10, 10, vec![])),
        Some(0.5),
        "no word it knows"
    );
    let spammy = score(&counts(10, 10, vec![("prize", 9, 0), ("winner", 8, 1)])).unwrap();
    let hammy = score(&counts(10, 10, vec![("budget", 0, 9), ("meeting", 1, 8)])).unwrap();
    assert!(spammy > THRESHOLD, "{spammy}");
    assert!(hammy < 0.1, "{hammy}");
}

#[test]
fn the_servers_verdict_is_read_from_its_headers() {
    let cases: &[(&[u8], bool)] = &[
        (b"X-Spam-Flag: YES\r\nSubject: hi\r\n\r\nbody", true),
        (b"X-Spam-Status: Yes, score=9.1\r\n\r\n", true),
        (b"X-Spam-Status: No, score=0.1\r\n\r\n", false),
        (b"Subject: hi\r\n\r\nX-Spam-Flag: YES", false),
        (b"", false),
    ];
    for (raw, want) in cases {
        assert_eq!(
            server_says_spam(raw),
            *want,
            "{}",
            String::from_utf8_lossy(raw)
        );
    }
}

#[test]
fn learning_a_message_twice_counts_it_once_and_changing_ones_mind_moves_it() {
    let store = trained();
    let counts = |store: &SqliteStore| store.junk_counts(account(), &[]).unwrap();
    let before = counts(&store);
    let again = message(0, "promo@prizes.example", "You won", SPAMMY);
    assert_eq!(
        learn(&store, std::slice::from_ref(&again), Judged::Spam).unwrap(),
        0
    );
    assert_eq!(counts(&store), before);
    learn(&store, &[again], Judged::Ham).unwrap();
    let after = counts(&store);
    assert_eq!((after.spam, after.ham), (before.spam - 1, before.ham + 1));
}

/// New mail in the inbox: junk moves to Spam, or is marked; a sender in Contacts, one written
/// to, and mail to the user's full name are spared; the server's own word is taken; mail the
/// server filed in Junk is learned from; nothing is judged with the filter off.
#[test]
fn new_mail_is_judged_as_the_policy_says() {
    struct Case {
        name: &'static str,
        policy: Policy,
        from: &'static str,
        to_name: Option<&'static str>,
        text: &'static str,
        raw: &'static [u8],
        arrives_in: MailboxRole,
        contact: Option<mail_store::Origin>,
        want_junk: bool,
        want_mailbox: MailboxRole,
        want_label: bool,
    }
    let base = || Case {
        name: "",
        policy: Policy::default(),
        from: "promo@prizes.example",
        to_name: None,
        text: SPAMMY,
        raw: b"Subject: You won\r\n\r\n",
        arrives_in: MailboxRole::Inbox,
        contact: None,
        want_junk: true,
        want_mailbox: MailboxRole::Spam,
        want_label: false,
    };
    let cases = vec![
        Case {
            name: "junk moves to Spam",
            ..base()
        },
        Case {
            name: "good mail stays",
            from: "carol@work.example",
            text: HAMMY,
            want_junk: false,
            want_mailbox: MailboxRole::Inbox,
            ..base()
        },
        Case {
            name: "marked, not moved",
            policy: Policy {
                treat: Treat::Mark,
                ..Policy::default()
            },
            want_mailbox: MailboxRole::Inbox,
            want_label: true,
            ..base()
        },
        Case {
            name: "a sender in Contacts is spared",
            contact: Some(mail_store::Origin::Manual),
            want_junk: false,
            want_mailbox: MailboxRole::Inbox,
            ..base()
        },
        Case {
            name: "unless Contacts are not heeded",
            policy: Policy {
                contacts: Heed::No,
                ..Policy::default()
            },
            contact: Some(mail_store::Origin::Manual),
            ..base()
        },
        Case {
            name: "mail to the user's full name is spared",
            to_name: Some("Ada Lovelace"),
            want_junk: false,
            want_mailbox: MailboxRole::Inbox,
            ..base()
        },
        Case {
            name: "the server's word is taken",
            from: "carol@work.example",
            text: HAMMY,
            raw: b"X-Spam-Flag: YES\r\n\r\n",
            ..base()
        },
        Case {
            name: "unless it is not heeded",
            policy: Policy {
                server_says: Heed::No,
                ..Policy::default()
            },
            from: "carol@work.example",
            text: HAMMY,
            raw: b"X-Spam-Flag: YES\r\n\r\n",
            want_junk: false,
            want_mailbox: MailboxRole::Inbox,
            ..base()
        },
        Case {
            name: "the server's Junk is learned from, not judged",
            arrives_in: MailboxRole::Spam,
            want_junk: false,
            want_mailbox: MailboxRole::Spam,
            ..base()
        },
        Case {
            name: "off judges nothing",
            policy: Policy {
                filtering: Filtering::Off,
                ..Policy::default()
            },
            want_junk: false,
            want_mailbox: MailboxRole::Inbox,
            ..base()
        },
    ];
    for case in cases {
        let store = trained();
        if let Some(origin) = &case.contact {
            store.put_contact(case.from, None, origin).unwrap();
        }
        let mut arriving = message(500, case.from, "Hello", case.text);
        arriving.mailbox = case.arrives_in;
        if let Some(name) = case.to_name {
            arriving.to = vec![Address::named(name, "ada@example.test")];
        } else {
            arriving.to = vec![Address::new("ada@example.test")];
        }
        let arriving = stored(&store, arriving, case.raw);
        let sifted = sift(
            &store,
            account(),
            &caps(),
            &[arriving.id],
            &case.policy,
            at(1000),
        )
        .unwrap();
        assert_eq!(
            sifted.junk.contains(&arriving.id),
            case.want_junk,
            "{}",
            case.name
        );
        let now = store.message(arriving.id).unwrap();
        assert_eq!(now.mailbox, case.want_mailbox, "{}", case.name);
        let junk_label = store
            .labels(account())
            .unwrap()
            .into_iter()
            .find(|label| label.name == JUNK_LABEL)
            .map(|label| label.id);
        assert_eq!(
            junk_label.is_some_and(|label| now.labels.contains(&label)),
            case.want_label,
            "{}",
            case.name
        );
    }
}
