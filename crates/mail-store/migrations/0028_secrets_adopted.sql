-- Which accounts have had their secrets adopted into porter's store (PLAN step E2).
--
-- Earlier builds kept an account's passwords and tokens in the platform keyring under the
-- service `mailo`, named `<account>:incoming|outgoing|oauth|carddav`. From E2 they are filed
-- through porter-secrets under porter's own attributes, and `mail-runtime`'s adoption moves what
-- an earlier build left: read each entry, file it, read it back, and only then delete the old
-- one. A row here says that was finished for the account: no legacy entry of it is left to read,
-- so the next start does not ask the keyring about it again. An account without a row is one the
-- adoption has not finished (never run, a keyring that was locked, a put that failed), and the
-- next run does what is left.
--
-- The secrets themselves are never here: only that the move is done. Follows the account's row,
-- so removing an account forgets this too.
CREATE TABLE secrets_adopted (
    account     TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    adopted_at  TEXT NOT NULL
);
