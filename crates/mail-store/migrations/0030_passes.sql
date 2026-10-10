-- Freshness: how each account's passes last ended, so a window opened later can tell an account
-- that is quiet from one that has not been fetched in days.
--
-- `sync_state.synced_at` is written per mailbox as a pass ingests, which measures progress, not a
-- pass that ran to its end. One row per account, rewritten as each pass ends: the time the last
-- one finished, and the failure that ended a later one, if one did. A pass that finishes clears
-- the failure. Cancelled passes leave the row as it was.
CREATE TABLE account_passes (
    account     TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    finished_at TEXT,  -- RFC 3339, UTC, fixed width; NULL until a pass finishes
    failed_at   TEXT,  -- the latest failure since `finished_at`; NULL when the latest pass finished
    failed_why  TEXT   -- what a person was told of it
) WITHOUT ROWID;
