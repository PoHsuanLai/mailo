-- What lets a pass ask a mailbox only for what arrived since the last one, and stop asking for
-- mail that cannot be read.
--
-- `held_through` is the mailbox state (a `SyncCursor`, as JSON) up to which every message the
-- server listed was held, headers at least. It moves only when a pass found nothing left to
-- fetch, so what arrived since it is exactly what is still wanted, and a survey that starts
-- there misses nothing. It is apart from `cursor`, which a flag sweep moves on past messages
-- that no pass has fetched yet. NULL until a pass has held everything: the next one lists the
-- whole mailbox.
ALTER TABLE sync_state ADD COLUMN held_through TEXT;

-- Messages that arrived and could not be read as messages, by when each was last tried, so a
-- pass does not ask the server for the same bytes again and again. One is tried again after a
-- day, in case it was the server's fault. `remote` is a `RemoteRef` as JSON.
CREATE TABLE unreadable (
    account  TEXT NOT NULL,
    mailbox  TEXT NOT NULL,
    remote   TEXT NOT NULL,
    tried_at TEXT NOT NULL,  -- RFC 3339, UTC, fixed width
    PRIMARY KEY (account, mailbox, remote)
) WITHOUT ROWID;
