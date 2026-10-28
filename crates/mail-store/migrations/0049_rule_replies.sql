-- What a rule that sends mail remembers: when it was made, how many days it leaves between two
-- answers to one sender, and whom it answered last.
--
-- A rule answers only mail dated after it was made, so mail downloaded again, or fetched anew
-- after a mailbox's UIDVALIDITY changed, never sends the history. The rules there already are
-- count from this migration.
ALTER TABLE rules ADD COLUMN made_at    TEXT NOT NULL DEFAULT '1970-01-01T00:00:00+00:00';
ALTER TABLE rules ADD COLUMN reply_days INTEGER NOT NULL DEFAULT 4;
UPDATE rules SET made_at = strftime('%Y-%m-%dT%H:%M:%S+00:00', 'now');

CREATE TABLE rule_replied (
    rule       TEXT NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
    sender     TEXT NOT NULL,   -- lower-cased address
    replied_at TEXT NOT NULL,   -- RFC 3339, UTC
    PRIMARY KEY (rule, sender)
) WITHOUT ROWID;

-- Whether a rule made a draft, not a person: it is sent as `Auto-Submitted: auto-replied`.
ALTER TABLE drafts ADD COLUMN automatic INTEGER NOT NULL DEFAULT 0;
