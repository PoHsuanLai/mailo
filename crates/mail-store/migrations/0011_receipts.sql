-- Read receipts (RFC 8098).
--
-- A draft may ask for one. The column holds serde(ReceiptRequest), and every draft written
-- before it existed did not ask, which is what the default says.
--
-- A message that asked for one is asked about once. What the user answered — sent or declined —
-- is kept here, beside the message rather than on it: it is a fact about this client's user, not
-- about the message's bytes, and an ingest that rewrites the message row from server truth must
-- not forget it. The server learns the same through the `$MDNSent` keyword (RFC 3503), queued
-- in the outbox when the answer is given.

ALTER TABLE drafts ADD COLUMN receipt TEXT NOT NULL DEFAULT '"unrequested"';

CREATE TABLE receipt_answers (
    message     TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    answer      TEXT NOT NULL,      -- serde(ReceiptAnswer)
    answered_at TEXT NOT NULL
);
