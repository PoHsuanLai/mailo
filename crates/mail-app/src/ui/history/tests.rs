use super::history;
use crate::ui::fixtures::work;

#[test]
fn senders_are_counted_by_conversation_and_matched_whole() {
    let built = work();
    let _ = (built.root.path(), built.dana, built.sam);
    let seen = history(&built.store);

    let dana = seen.get("Dana@Example.com").expect("Dana has written");
    assert_eq!(dana.threads, 2, "Dana started two conversations");
    assert_eq!(dana.name.as_deref(), Some("Dana Okafor"));
    assert!(!dana.replied, "nobody has written to Dana yet");

    let stranger = seen
        .get("no-reply@g00gle-security.xyz")
        .expect("the spoof is in the fixture");
    assert_eq!(stranger.threads, 1, "one conversation is a first mail");

    assert!(
        seen.get("example.com").is_none(),
        "a domain is not a sender"
    );
    assert!(
        seen.get("ana@example.com").is_none(),
        "an address is compared whole"
    );
}

#[test]
fn writing_to_someone_is_what_replied_means() {
    let built = work();
    let before = history(&built.store);
    assert!(!before.get("dana@example.com").is_some_and(|d| d.replied));
    // One of the fixture's messages, rewritten as sent from the Google account to Dana.
    built
        .store
        .connection()
        .execute(
            "UPDATE messages SET from_email = 'poh@acme.example', from_name = NULL,
                    recipients = '{\"reply_to\":[],\"to\":[{\"name\":null,\"email\":\"Dana@Example.com\"}],\"cc\":[],\"bcc\":[]}'
              WHERE subject = 'Accepted: Design review, Thursday 14:00'",
            [],
        )
        .unwrap();
    let after = history(&built.store);
    assert!(
        after.get("dana@example.com").is_some_and(|d| d.replied),
        "a message to Dana from one of your accounts did not count"
    );
    let affinity = after.affinity();
    assert!(
        affinity.get("dana@example.com").is_some_and(|s| s.replied),
        "the ranker did not get the same history"
    );
    assert_eq!(
        after.names().get("dana@example.com").map(String::as_str),
        Some("Dana Okafor")
    );
}
