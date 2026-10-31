//! A read that fails, as a frame shows it.

mod tests {
    use chrono::{DateTime, Utc};
    use mail_client::ClientError;
    use mail_client::Query;
    use mail_core::SqliteStore;
    use mail_ui_kit::live::read_now;

    /// A reading that always fails.
    #[derive(Clone, PartialEq, Eq)]
    struct Unreadable;

    impl Query for Unreadable {
        type Output = Vec<u8>;
        const READS: mail_client::Topics = mail_client::Topics::ACCOUNTS;

        fn run(&self, _: &SqliteStore, _: DateTime<Utc>) -> Result<Vec<u8>, ClientError> {
            Err(ClientError::Store("no such table".to_owned()))
        }
    }

    /// A failed read is the empty answer, as a frame shows one, and not a panic.
    #[test]
    fn a_read_that_fails_answers_nothing_and_does_not_panic() {
        let built = crate::ui::fixtures::work();
        let client = crate::ui::fixtures::client(&built.store);
        assert!(read_now(&client, Unreadable).is_empty());
    }
}
