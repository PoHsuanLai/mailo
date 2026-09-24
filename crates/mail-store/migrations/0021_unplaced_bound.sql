-- How long an operation waits for a sync to find a message (FINDINGS F155).
--
-- A message the server moved without saying where (no `COPYUID`) is kept in `unplaced` until a
-- sync finds it, and an operation on it waits meanwhile. That wait had no end: if no sync ever
-- found the message — its folder is not one this client syncs, or it was deleted elsewhere —
-- the operation waited for good, held back every later one on the message, and nobody was told.
--
-- Each row now says where the move put the message and counts the passes that could have found
-- it and did not. Passes, not time: a laptop shut for a week has made no pass, and its queue must
-- not expire while it was shut.
--
-- mailbox: the folder the move filed it into, by path. NULL for a row queued before this, whose
--          destination was never kept; only `passes` can end its wait.
-- syncs:   complete syncs of that folder since, none of which found it.
-- passes:  passes of the account since that did not sync that folder in full.
ALTER TABLE unplaced ADD COLUMN mailbox TEXT;
ALTER TABLE unplaced ADD COLUMN syncs INTEGER NOT NULL DEFAULT 0;
ALTER TABLE unplaced ADD COLUMN passes INTEGER NOT NULL DEFAULT 0;
