-- How large each message is, kept beside it: `larger:` and `smaller:` ask for messages by size,
-- and worked it out per message from the text and the parts' JSON on every search, scanning every
-- message there is. The size is `mail_domain::filter::message_size`: the bytes of the text (a TEXT
-- cast to a BLOB counts bytes, not characters) and each part at the size the part says, held or
-- still on the server.
--
-- Written wherever the text or the parts are (`upsert_message`, `fill_body`, `hold_part`), and
-- here for every message already held, from the same two columns.
ALTER TABLE messages ADD COLUMN size INTEGER NOT NULL DEFAULT 0;

-- The index holds `fts_text` and nothing else, so only a write to that column changes what it
-- should hold. The two triggers (0004) fired on every update of a message, so a flag, a move or
-- this backfill took each message out of the index and put it back. Same triggers, same bodies,
-- on `UPDATE OF fts_text`: the update that sets it still fires them, the backfill of 0004 and the
-- clearing of 0005 among them.
DROP TRIGGER messages_fts_upd;
DROP TRIGGER messages_fts_upd_first;
CREATE TRIGGER messages_fts_upd AFTER UPDATE OF fts_text ON messages
WHEN old.fts_text IS NOT NULL BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, fts_text)
    VALUES ('delete', old.rowid, old.fts_text);
    INSERT INTO messages_fts(rowid, fts_text) VALUES (new.rowid, new.fts_text);
END;
CREATE TRIGGER messages_fts_upd_first AFTER UPDATE OF fts_text ON messages
WHEN old.fts_text IS NULL BEGIN
    INSERT INTO messages_fts(rowid, fts_text) VALUES (new.rowid, new.fts_text);
END;

-- A row whose parts are not JSON (nothing this build wrote) is no bytes of parts rather than a
-- migration that stops the database opening.
UPDATE messages SET size =
    coalesce(length(CAST(body_text AS BLOB)), 0)
    + CASE WHEN json_valid(attachments)
        THEN coalesce((SELECT sum(json_extract(part.value, '$.size'))
                       FROM json_each(messages.attachments) AS part), 0)
        ELSE 0 END;

-- `m.size > ?` and `m.size < ?` read the thread out of the index alone.
CREATE INDEX messages_size ON messages(size, thread);
