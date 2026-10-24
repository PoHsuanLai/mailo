-- What the person says about an account beyond its plan: what to call it, whether it syncs, and
-- which server folder plays which role when auto-detection guessed wrong.
--
-- A table of its own rather than fields on the plan (`accounts.plan`), which is the server
-- configuration and is rewritten whole when an account is added again; nor on `account_caps`,
-- which is what the server was last observed to support and is replaced by every refresh. A
-- folder the person picked is theirs, so it is kept here and laid over the observed roles
-- whenever they are read or written.
--
-- `folder_roles` is serde(Vec<(MailboxRole, String)>): each role picked, and the folder's path.
-- An account with no row has no description, syncs, and has picked nothing.
CREATE TABLE account_prefs (
    account      TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    description  TEXT,
    paused       INTEGER NOT NULL DEFAULT 0 CHECK (paused IN (0, 1)),
    folder_roles TEXT NOT NULL DEFAULT '[]'
);
