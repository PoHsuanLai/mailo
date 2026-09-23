-- When each account started being watched for new mail (`plan.md` 10.6).
--
-- A desktop notification is for mail that has just arrived, and "stored for the first time by
-- this pass" is not the same thing: an account's first sync stores every message it has ever
-- received, a few hundred per pass, over many passes. A message dated before the account was
-- first watched is backfill and is never announced, however late the pass that stores it.
--
-- Written once per account, by the first watched pass, and never moved. Kept here rather than
-- beside the window's preferences because it is a fact about this database's accounts: a new
-- database is a new first sync, and must arm again.
CREATE TABLE notify_floor (
    account   TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    armed_at  TEXT NOT NULL   -- RFC 3339, UTC
) WITHOUT ROWID;
