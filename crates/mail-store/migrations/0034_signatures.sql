-- Signatures: several to an identity, each with a name, written in the composer's editor.
--
-- A table of their own rather than the one `signature` column on `identities`, which held the
-- text of one. A signature is kept as the composer's HTML beside its plain text: the HTML is what
-- an HTML part carries, the text what a text part carries and the command line reads. `body_html`
-- is NULL for one only ever set as text.
--
-- An identity has at most one default for new mail and one for replies, which may be the same
-- row; the partial unique indexes hold that, so two defaults is a write SQLite refuses rather
-- than a choice the composer has to make. Flags are 0 or 1.
CREATE TABLE signatures (
    id          TEXT PRIMARY KEY,
    identity    TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    body_html   TEXT,
    body_text   TEXT NOT NULL,
    for_new     INTEGER NOT NULL DEFAULT 0 CHECK (for_new IN (0, 1)),
    for_replies INTEGER NOT NULL DEFAULT 0 CHECK (for_replies IN (0, 1))
);
-- Listed in the order they were made, which is the rowid's.
CREATE INDEX signatures_identity ON signatures(identity);
CREATE UNIQUE INDEX signatures_one_for_new ON signatures(identity) WHERE for_new = 1;
CREATE UNIQUE INDEX signatures_one_for_replies ON signatures(identity) WHERE for_replies = 1;

-- The signature each identity had becomes its first, the default for both, as text. A random
-- version-4 UUID in the store's hyphenated lowercase, since SQLite has no function for one.
-- Blank ones are left behind: compose has always read only whitespace as no signature.
INSERT INTO signatures (id, identity, name, body_html, body_text, for_new, for_replies)
SELECT lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4'
           || substr(lower(hex(randomblob(2))), 2) || '-'
           || substr('89ab', 1 + (abs(random()) % 4), 1)
           || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))),
       id, 'Signature', NULL, signature, 1, 1
FROM identities
WHERE signature IS NOT NULL AND trim(signature, ' ' || char(9, 10, 13)) <> '';

-- The column stays, for a build that reads it, but no longer says anything: what it said is a
-- row above, and a signature deleted there must not come back from here.
UPDATE identities SET signature = NULL;
