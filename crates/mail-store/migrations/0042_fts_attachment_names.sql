-- Attachment names are searched: `invoice` finds the mail that carries `invoice-2026.pdf`. The
-- indexed text (`sql::message_index`) now ends with each part's name, so every row is indexed
-- again, by the backfill that runs as the store opens. Same two statements as 0005 and 0006,
-- for the same reasons.

UPDATE messages SET fts_text = NULL;
INSERT INTO messages_fts(messages_fts) VALUES ('delete-all');
