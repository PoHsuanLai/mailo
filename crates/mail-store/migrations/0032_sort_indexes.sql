-- Indexes for the orders the store reads in that still sorted every row they read.
--
-- A conversation's messages are read oldest first, `ORDER BY date, id`, every time one is opened
-- and every time its summary is rebuilt, which is once per thread an ingest touches. With only
-- `messages_thread` SQLite found the rows by the index and then sorted them in a temp B-tree.
-- This index holds them in that order, and it serves every lookup by thread alone as well, so
-- the old one is dropped rather than kept as a second copy of its first column.
CREATE INDEX messages_thread_order ON messages(thread, date, id);
DROP INDEX messages_thread;

-- The list sorted by subject, by sender and by pin: `SCAN ts` / `USE TEMP B-TREE FOR ORDER BY`,
-- the whole mailbox sorted for a page of fifty, which is what 0003 fixed for the date order.
-- Each matches its `ORDER BY column, ts.thread` exactly, and SQLite walks an index backwards as
-- readily as forwards, so one serves both directions.
CREATE INDEX thread_summary_subject ON thread_summary(subject, thread);
CREATE INDEX thread_summary_from ON thread_summary(from_email, thread);
CREATE INDEX thread_summary_pin ON thread_summary(pin, thread);
