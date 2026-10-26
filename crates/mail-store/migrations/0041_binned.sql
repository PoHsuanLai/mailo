-- Erasing Trash and Spam: since when each message has been in one of them, as this computer saw
-- it, and which are to be erased as soon as the server holds them there.
--
-- `since` is when a sweep first found the message in Trash or Spam, not the message's date: a
-- three-year-old message deleted today has been in Trash for no time at all. A message that
-- leaves (restored, or moved again) loses its row, so coming back starts its time again.
-- `soon` is 1 for a message Delete put there under "Delete deletes it now": it is erased once
-- the server holds it in Trash, whatever the wait. Local only: nothing here is mail. The row goes
-- with its message.
CREATE TABLE binned (
    message TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    since   TEXT NOT NULL,  -- RFC 3339, UTC, fixed width
    soon    INTEGER NOT NULL DEFAULT 0 CHECK (soon IN (0, 1))
) WITHOUT ROWID;
