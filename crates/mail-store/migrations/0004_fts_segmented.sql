-- Full-text search could not find a Chinese word. See FINDINGS F125.
--
-- `unicode61` classifies ideographs as token characters and Chinese is written without spaces,
-- so an unbroken run of them indexed as ONE token: the whole of a subject was a single word and
-- only typing all of it found anything. For a mailbox that is substantially Chinese, full-text
-- search did not work.
--
-- The tokenizer cannot be changed to fix it: `trigram` answers queries of three characters or
-- more and Chinese words are overwhelmingly two. So the *text* is segmented instead, which means
-- the index can no longer read the message columns directly — it reads one column that Rust
-- fills with exactly the tokens the query side will ask for. `SqliteStore::from_connection`
-- backfills it for messages that predate this migration.

ALTER TABLE messages ADD COLUMN fts_text TEXT;

DROP TRIGGER messages_fts_ins;
DROP TRIGGER messages_fts_del;
DROP TRIGGER messages_fts_upd;
DROP TABLE messages_fts;

-- One column, still external-content so the index stores no duplicate text. `Filter::Text`
-- searches the whole corpus rather than a named field, so four columns bought nothing that one
-- does not — and one is what a segmented corpus can be.
CREATE VIRTUAL TABLE messages_fts USING fts5(
    fts_text,
    content = 'messages',
    content_rowid = 'rowid',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER messages_fts_ins AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, fts_text) VALUES (new.rowid, new.fts_text);
END;
CREATE TRIGGER messages_fts_del AFTER DELETE ON messages
WHEN old.fts_text IS NOT NULL BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, fts_text)
    VALUES ('delete', old.rowid, old.fts_text);
END;

-- Two update triggers rather than one, on whether there is an entry to remove.
--
-- An external-content FTS5 index is told what the old value was so it can unindex it, and
-- telling it to remove an entry that was never inserted leaves the index inconsistent — SQLite
-- reports "database disk image is malformed" at the next query. Every row that predates this
-- migration has `fts_text IS NULL` and no entry, and the backfill is precisely an update of
-- those rows, so this is not a rare case: it is the first thing that happens.
CREATE TRIGGER messages_fts_upd AFTER UPDATE ON messages
WHEN old.fts_text IS NOT NULL BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, fts_text)
    VALUES ('delete', old.rowid, old.fts_text);
    INSERT INTO messages_fts(rowid, fts_text) VALUES (new.rowid, new.fts_text);
END;
CREATE TRIGGER messages_fts_upd_first AFTER UPDATE ON messages
WHEN old.fts_text IS NULL BEGIN
    INSERT INTO messages_fts(rowid, fts_text) VALUES (new.rowid, new.fts_text);
END;
