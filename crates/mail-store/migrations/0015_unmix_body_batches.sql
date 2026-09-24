-- Undo what body fetches drawn from more than one mailbox did.
--
-- The body pass took the account's whole backlog, newest first, and handed it to the backend as
-- one batch. The backend selects one mailbox for a batch, the first remote's, and names every UID
-- in it — so once Sent was fetched beside INBOX, a batch could ask INBOX for a UID that belonged
-- to Sent, or the reverse. Replies are paired by the UID they carry (F143), so a UID the selected
-- mailbox did not have simply went unanswered and was asked for again later, which is harmless.
-- A UID it did have came back with that mailbox's message, and was stored under the other
-- mailbox's address:
--
-- 1. The bytes named their own Message-ID, so the message they matched was the right one (Y),
--    and its body, if missing, was filled with its own content. But `remote_map` was upserted
--    for the address asked about: Sent/U, which named message X, now named Y. X lost that
--    address — usually its only one — so its body was never fetched and the header pass, seeing
--    Sent/U held, never mapped it again. Y gained an address on X: every sweep of Sent laid X's
--    flags and labels on Y, and anything done to Y was also done to X on the server.
--    Signature: one message holding the same UID in two mailboxes.
-- 2. Bytes with no Message-ID are keyed by a digest that includes the address asked about, so
--    they matched nothing and became a new message: a copy of Y, filed as the pass's role, on
--    X's address. Signature: a digest-keyed message on mailbox A, UID U, and a message with the
--    same subject, sender and date on another mailbox under the same UID.
-- 3. A message over a megabyte is fetched as its structure and then its sections. The
--    structure came from the other mailbox's message, and the sections, asked for one message
--    at a time, from the right one: its headers with another message's boundary. The parser
--    finds no parts in that, and the body text is the whole multipart, delimiters and all.
--    Signature: a part-left-on-the-server marker, or a delimiter and part header, in the text.
--
-- Nothing here can tell which of two addresses was the stolen one, and nothing needs to: both
-- rows go, and the header pass fetches each UID's headers again, matches each to the message
-- already held by its key, and maps it to the UID it really has — as 0008 did for F143. Bodies
-- that are wrong are dropped, so the next body pass fetches them from the right mailbox.

-- 2 first, while the rows that identify the copies are still there. Which of the two is the
--    copy cannot be told in SQL — each key is a digest of its own address — so both go, with
--    their addresses; the header pass stores the one the server holds in each mailbox again.
--    They are digest-keyed, so nothing but their flags and local labels is lost, and the flags
--    come back with the headers.
CREATE TEMP TABLE unmix_copies AS
    SELECT DISTINCT z.id AS message, z.thread AS thread
    FROM messages z
    JOIN remote_map rz ON rz.message = z.id AND rz.uid IS NOT NULL
    JOIN remote_map ry ON ry.account = rz.account AND ry.uid = rz.uid
                      AND ry.mailbox <> rz.mailbox AND ry.message <> z.id
    JOIN messages y ON y.id = ry.message
    WHERE json_extract(z.msg_key, '$.kind') = 'synthetic'
      AND y.subject = z.subject
      AND y.from_email = z.from_email
      AND y.date = z.date;

INSERT OR IGNORE INTO summaries_to_refresh (thread) SELECT thread FROM unmix_copies;
DELETE FROM messages WHERE id IN (SELECT message FROM unmix_copies);
DROP TABLE unmix_copies;

-- 1. Both addresses of a message that holds one UID in two mailboxes.
CREATE TEMP TABLE unmix_rows AS
    SELECT a.rowid AS row
    FROM remote_map a
    JOIN remote_map b ON b.message = a.message AND b.account = a.account
                     AND b.uid = a.uid AND b.mailbox <> a.mailbox
    WHERE a.uid IS NOT NULL;
DELETE FROM remote_map WHERE rowid IN (SELECT row FROM unmix_rows);
DROP TABLE unmix_rows;

-- 3. Bodies rebuilt around another message's structure: the parser, finding none of the
--    declared boundary, keeps the whole multipart as text, delimiters, part headers and the
--    markers of the parts left on the server included. The index keeps the old text until the
--    body arrives again, which rewrites it.
CREATE TEMP TABLE unmix_bodies AS
    SELECT id AS message, thread FROM messages
    WHERE body_text IS NOT NULL
      AND (instr(body_text, 'X-Mailo-Remote-Section') > 0
           OR (substr(body_text, 1, 2) = '--'
               AND instr(body_text, char(13) || char(10) || 'Content-Type:') > 0));
INSERT OR IGNORE INTO summaries_to_refresh (thread) SELECT thread FROM unmix_bodies;
UPDATE messages SET body_raw = NULL, body_text = NULL, attachments = '[]'
    WHERE id IN (SELECT message FROM unmix_bodies);
DROP TABLE unmix_bodies;
