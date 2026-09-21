-- remote_map accumulated one row per message per sync, on every protocol.
--
-- The table's PRIMARY KEY is (account, mailbox, uidvalidity, uid, uidl), and exactly one of
-- `uid`/`uidl` is non-NULL by the CHECK constraint — so every row has a NULL in its key. SQLite
-- treats NULLs as distinct in UNIQUE and PRIMARY KEY comparisons, so no two rows ever conflict,
-- and `INSERT OR REPLACE` had nothing to replace. An IMAP mailbox grew a duplicate row per
-- message per pass, and `refs_for` handed the outbox the same UID several times over.
--
-- Fixed with a unique index over expressions that cannot be NULL. The sentinels are outside the
-- domain of real values: a UID is a positive integer and a UIDL is a non-empty string, so -1 and
-- '' can never collide with one.

-- Collapse what has already accumulated, keeping one row per identity.
DELETE FROM remote_map
WHERE rowid NOT IN (
    SELECT min(rowid) FROM remote_map
    GROUP BY account, mailbox,
             COALESCE(uidvalidity, -1), COALESCE(uid, -1), COALESCE(uidl, ''), message
);

CREATE UNIQUE INDEX remote_map_identity ON remote_map(
    account, mailbox,
    COALESCE(uidvalidity, -1),
    COALESCE(uid, -1),
    COALESCE(uidl, '')
);
