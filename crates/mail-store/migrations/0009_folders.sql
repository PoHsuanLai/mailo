-- Every mailbox the server lists, not only the ones a role is filed in.
--
-- Until this, the only folders this client knew were the special-use ones, folded into
-- `account_caps` as roles. A folder the user made on the web, or one they wanted to make here,
-- had nowhere to be written down, so it could not be listed, renamed or deleted.
--
-- Rewritten from each `LIST`/`LSUB` walk, with any folder work still in the outbox laid over
-- the listing, the way `pending_changes` is laid over messages. Paths are decoded from
-- modified UTF-7, as `remote_map.mailbox` is.

CREATE TABLE folders (
    account      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    path         TEXT NOT NULL,
    delimiter    TEXT,               -- one character, or NULL for a server with no hierarchy
    special      TEXT,               -- serde(SpecialUse), or NULL
    subscription TEXT NOT NULL,      -- serde(Subscription)
    holds        TEXT NOT NULL,      -- serde(Holds)
    PRIMARY KEY (account, path)
) WITHOUT ROWID;
