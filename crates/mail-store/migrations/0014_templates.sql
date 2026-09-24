-- Templates: messages kept so that new ones can be started from them (`plan.md` 10.8).
--
-- A table of their own rather than a kind column on `drafts`. A template is never sent, has no
-- send state and answers no message, and every reader of `drafts` — the drafts list, the send
-- path, discarding, the window — would otherwise have to filter templates out, and the one that
-- forgot would send one. Local only: nothing here is uploaded to or read from a server.
--
-- Shaped like `drafts` where the two agree, so a template carries a draft's fields in the same
-- encodings.
CREATE TABLE templates (
    id          TEXT PRIMARY KEY,
    account     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    identity    TEXT NOT NULL REFERENCES identities(id),
    name        TEXT NOT NULL,
    recipients  TEXT NOT NULL,       -- serde({to, cc, bcc}), as in drafts
    subject     TEXT NOT NULL,
    body_text   TEXT NOT NULL,
    body_html   TEXT,
    attachments TEXT NOT NULL,       -- serde(Vec<PendingAttachment>)
    receipt     TEXT NOT NULL,       -- serde(ReceiptRequest), as in drafts
    updated_at  TEXT NOT NULL
);
CREATE INDEX templates_account ON templates(account);
