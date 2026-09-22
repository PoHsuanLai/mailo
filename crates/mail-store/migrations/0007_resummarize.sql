-- Thread summaries are derived in Rust, and the derivation changed: the attachment count no longer
-- includes images an HTML body embeds (phase 9). Stored summaries keep the old count until their
-- thread is next written, which for old mail is never.
--
-- So every thread is queued here, and `SqliteStore::from_connection` re-derives each one on the
-- next open and empties the queue. A table rather than one-off SQL because the derivation is
-- Rust: restating it in SQL would be a second implementation to keep in step, and the next change
-- to `ThreadSummary::derive` can queue its threads the same way.

CREATE TABLE summaries_to_refresh (
    thread TEXT PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE
);
INSERT INTO summaries_to_refresh (thread) SELECT thread FROM thread_summary;
