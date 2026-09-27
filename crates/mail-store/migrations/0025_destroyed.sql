-- Delete forever: the server addresses of messages the user destroyed, until the server no
-- longer lists them.
--
-- A destroyed message is removed here at once, and its `remote_map` rows with it (they cascade
-- from `messages`). Until the server has expunged it, though, a sync still lists it, and a UID
-- with no row is a new message to fetch: the message would come back into Trash. So its
-- addresses are kept here, where a sync counts them as held, and the queued operation is sent to
-- them. A row goes when a sync no longer finds the address, or at once when the server refuses
-- the deletion, so the message comes back as the server still has it. POP3 deletes only here,
-- so a POP3 row stays for as long as the server keeps the message.
--
-- `message` has no foreign key: the message is gone. `outbox` is the entry sending the
-- deletion, NULL once the server has confirmed it; it has none either, since the entry is
-- dropped on settling.
CREATE TABLE destroyed (
    account     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mailbox     TEXT NOT NULL,
    uidvalidity INTEGER,
    uid         INTEGER,
    uidl        TEXT,
    message     TEXT NOT NULL,
    outbox      INTEGER,
    CHECK ((uid IS NULL) != (uidl IS NULL))
);
CREATE UNIQUE INDEX destroyed_identity ON destroyed(
    account, mailbox, COALESCE(uidvalidity, -1), COALESCE(uid, -1), COALESCE(uidl, '')
);
CREATE INDEX destroyed_message ON destroyed(message);
CREATE INDEX destroyed_outbox ON destroyed(outbox);
