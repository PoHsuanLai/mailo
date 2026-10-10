use mail_domain::{
    JmapAccountId, JmapBlobId, JmapEmailId, JmapIdentityId, JmapMailboxId, JmapThreadId, RemoteRef,
};

/// Each id is a bare JSON string and nothing else, and the same text comes back through
/// `Display` and `FromStr`.
macro_rules! round_trips {
    ($($name:ident: $ty:ty),* $(,)?) => {$(
        #[test]
        fn $name() {
            for text in ["Mf40b5f831", "A13824", "x", "G_d2f8-1008"] {
                let id: $ty = text.parse().unwrap();
                assert_eq!(id.to_string(), text);
                assert_eq!(id.as_str(), text);
                assert_eq!(id, <$ty>::from(text));
                let json = serde_json::to_string(&id).unwrap();
                assert_eq!(json, serde_json::to_string(text).unwrap());
                assert_eq!(serde_json::from_str::<$ty>(&json).unwrap(), id);
                assert_eq!(String::from(id), text);
            }
        }
    )*};
}

round_trips!(
    an_account_id_is_its_text: JmapAccountId,
    a_mailbox_id_is_its_text: JmapMailboxId,
    an_email_id_is_its_text: JmapEmailId,
    a_thread_id_is_its_text: JmapThreadId,
    a_blob_id_is_its_text: JmapBlobId,
    an_identity_id_is_its_text: JmapIdentityId,
);

#[test]
fn a_stored_remote_address_reads_the_same_with_a_typed_id() {
    let stored = r#"{"kind":"jmap","v":{"email_id":"Mf40b5f831"}}"#;
    let remote: RemoteRef = serde_json::from_str(stored).unwrap();
    assert_eq!(
        remote,
        RemoteRef::Jmap {
            email_id: "Mf40b5f831".into()
        }
    );
    assert_eq!(serde_json::to_string(&remote).unwrap(), stored);
}
