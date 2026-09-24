-- Rules and the vacation reply (`plan.md` 10.11).
--
-- A rule is a filter and what to do with the mail it matches, per account and in order. The
-- filter and the actions are kept as their serde form, like `views.filter`: they are closed
-- domain vocabulary that the database never queries into, and a column per clause would be a
-- schema change every time the search language grew one.
--
-- The vacation reply is one row per account. It runs only on a server (a reply that needs this
-- client to be running stops when the laptop closes), and it is kept here so that the next
-- `mailo sieve push`, which rewrites the whole server script, writes it again.
CREATE TABLE rules (
    id        TEXT PRIMARY KEY,
    account   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name      TEXT NOT NULL,
    position  INTEGER NOT NULL,
    state     TEXT NOT NULL,       -- serde(RuleState)
    filter    TEXT NOT NULL,       -- serde(Filter)
    actions   TEXT NOT NULL,       -- serde(Vec<RuleAction>)
    after     TEXT NOT NULL,       -- serde(AfterMatch)
    UNIQUE (account, name)
);
CREATE INDEX rules_account ON rules(account, position);

CREATE TABLE vacations (
    account    TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    vacation   TEXT NOT NULL,      -- serde(Vacation)
    updated_at TEXT NOT NULL
);
