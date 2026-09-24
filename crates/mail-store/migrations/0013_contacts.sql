-- The address book: every address the user corresponds with, learned from their mail, plus the
-- ones they typed in or synced. See `crate::contact` for the rules; this is only where they live.
--
-- One book across accounts: the person is the same whichever account wrote to them, and each row
-- remembers which account did so last.

CREATE TABLE contacts (
    address        TEXT PRIMARY KEY,   -- lower-cased and trimmed
    name           TEXT,
    name_rank      INTEGER NOT NULL,   -- learn::NameRank: which source the name came from
    name_at        TEXT,               -- when that name was seen, for "most recent wins"
    written_count  INTEGER NOT NULL,
    written_last   TEXT,
    received_count INTEGER NOT NULL,
    received_last  TEXT,
    score          REAL,               -- log2 of the frecency sum; NULL before any event
    kind           TEXT NOT NULL,      -- serde(Kind)
    origin         TEXT NOT NULL,      -- serde(Origin)
    account        TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    -- ' word word … ': what a typed prefix may begin. Derived from the columns above.
    keys           TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX contacts_score ON contacts(score DESC);

-- Messages already counted, so a body arriving after its headers, or a message seen again in a
-- second mailbox, is not counted twice.
CREATE TABLE contacts_counted (
    message TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE
) WITHOUT ROWID;

-- Submissions counted when the outbox confirmed them, by recipients and subject, waiting for
-- their copy to arrive from the Sent folder and be recognised rather than counted again.
CREATE TABLE contacts_sent (
    fingerprint TEXT NOT NULL,
    at          TEXT NOT NULL
);
CREATE INDEX contacts_sent_fingerprint ON contacts_sent(fingerprint);

-- Where each synced CardDAV address book got to. serde(AddressBook); credentials are never here.
CREATE TABLE address_books (
    url   TEXT PRIMARY KEY,
    state TEXT NOT NULL
) WITHOUT ROWID;

-- The mail already held predates the book. `SqliteStore::from_connection` reads it through once
-- on the next open, with the same rules ingest uses, and empties this table: the rules are Rust,
-- and restating them in SQL would be a second implementation to keep in step.
CREATE TABLE contacts_to_backfill (
    pending INTEGER NOT NULL
);
INSERT INTO contacts_to_backfill (pending) VALUES (1);
