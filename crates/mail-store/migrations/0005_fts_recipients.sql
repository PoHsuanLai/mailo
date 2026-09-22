-- Full-text search now reads `To` and `Cc` as well as the sender (phase 9.2).
--
-- The indexed text is built in Rust, so SQL cannot rebuild it: this clears it, and the backfill
-- in `SqliteStore::from_connection` refills every row on the next open, exactly as it did after
-- 0004.
--
-- Nulling the column fires `messages_fts_upd`, which unindexes the old text and indexes an empty
-- row in its place. `delete-all` then empties the index, so every row is `NULL` with no entry —
-- precisely the state `messages_fts_upd_first` was written for. Without it the backfill inserts a
-- second entry for a rowid FTS5 already holds; measured, the bundled SQLite accepts that and the
-- integrity check passes, but this does not lean on it.
--
-- The order is not optional. `delete-all` first, and the update would ask FTS5 to unindex
-- entries it no longer has, which `tests/upgrade.rs` shows failing the integrity check.

UPDATE messages SET fts_text = NULL;
INSERT INTO messages_fts(messages_fts) VALUES ('delete-all');
