-- When each mailbox was last swept, so the checks that cost a whole-mailbox listing run on their
-- schedule and not on every pass. The engine is built afresh for each pass, so its own memory of
-- when it last swept was always empty and every pass did both.
--
-- On the mailbox's row, so a mailbox that is renamed or forgotten takes its times with it. NULL
-- until that sweep has run; a mailbox with no row has no cursor yet either, and is swept.
ALTER TABLE sync_state ADD COLUMN flags_swept_at TEXT;     -- RFC 3339, UTC, fixed width
ALTER TABLE sync_state ADD COLUMN expunges_swept_at TEXT;  -- RFC 3339, UTC, fixed width
