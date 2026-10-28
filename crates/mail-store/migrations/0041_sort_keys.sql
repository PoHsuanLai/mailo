-- The list sorts by sender, subject, size, unread and starred, each either way, and every order
-- breaks a tie by date and then by thread, so a page boundary between two rows that sort alike is
-- never ambiguous.
--
-- A conversation sorts by its newest message, as its sender and date already are
-- (`mail_domain::SortFacts`): that message's subject as threading compares subjects, its sender
-- as the row names it, and its size, which is its text's bytes and the size each of its parts
-- says. Unread and starred are the summary's own, any message, as integers so that descending is
-- unread first and starred first.
ALTER TABLE thread_summary ADD COLUMN sort_subject TEXT NOT NULL DEFAULT '';
ALTER TABLE thread_summary ADD COLUMN sort_sender TEXT NOT NULL DEFAULT '';
ALTER TABLE thread_summary ADD COLUMN size INTEGER NOT NULL DEFAULT 0;
ALTER TABLE thread_summary ADD COLUMN unread INTEGER NOT NULL DEFAULT 0;
ALTER TABLE thread_summary ADD COLUMN starred INTEGER NOT NULL DEFAULT 0;

-- Every existing row, from its messages. The newest is the one `messages_of` reads last:
-- `ORDER BY date, id`. Size, unread and starred are exactly what Rust derives; the subject and the
-- sender are close to it (SQL lowercases ASCII only and strips no `Re:`), and are made exact by
-- the queue below.
UPDATE thread_summary SET
    unread = (read = '"unread"'),
    starred = (star = '"starred"'),
    size = coalesce((
        SELECT coalesce(length(CAST(m.body_text AS BLOB)), 0)
             + coalesce((SELECT sum(json_extract(part.value, '$.size'))
                         FROM json_each(m.attachments) AS part), 0)
        FROM messages m WHERE m.thread = thread_summary.thread
        ORDER BY m.date DESC, m.id DESC LIMIT 1
    ), 0),
    sort_subject = coalesce((
        SELECT lower(trim(m.subject))
        FROM messages m WHERE m.thread = thread_summary.thread
        ORDER BY m.date DESC, m.id DESC LIMIT 1
    ), ''),
    sort_sender = coalesce((
        SELECT lower(coalesce(nullif(trim(m.from_name), ''), m.from_email))
        FROM messages m WHERE m.thread = thread_summary.thread
        ORDER BY m.date DESC, m.id DESC LIMIT 1
    ), '');

-- The subject and sender as Rust words them, on the next open (`0007_resummarize.sql`).
INSERT OR IGNORE INTO summaries_to_refresh (thread) SELECT thread FROM thread_summary;

-- One index per order, matching `ORDER BY column, last_date, thread` exactly, so a page of fifty
-- is read in order rather than sorted out of the whole mailbox (0003, 0032). 0032's subject,
-- sender and pin indexes broke ties by thread alone; no query orders that way now.
DROP INDEX thread_summary_subject;
DROP INDEX thread_summary_from;
DROP INDEX thread_summary_pin;
CREATE INDEX thread_summary_sort_subject ON thread_summary(sort_subject, last_date, thread);
CREATE INDEX thread_summary_sort_sender ON thread_summary(sort_sender, last_date, thread);
CREATE INDEX thread_summary_sort_size ON thread_summary(size, last_date, thread);
CREATE INDEX thread_summary_sort_unread ON thread_summary(unread, last_date, thread);
CREATE INDEX thread_summary_sort_starred ON thread_summary(starred, last_date, thread);
CREATE INDEX thread_summary_sort_pin ON thread_summary(pin, last_date, thread);
