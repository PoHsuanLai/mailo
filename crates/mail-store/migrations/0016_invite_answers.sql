-- Answers to calendar invitations (`plan.md` 10.12).
--
-- The invitation is read from the message's bytes whenever it is shown; what the user answered
-- is not in those bytes — it left in a message of its own — so it is kept here, beside the
-- message, for the same reason read-receipt answers are: an ingest rewrites the message row from
-- server truth, and the answer is not something the server's copy knows.
--
-- One row per message, replaced when the user changes their mind: the reader shows the answer
-- that stands, and answering again sends a new reply that supersedes the last (RFC 5546 §3.2.3).
-- `sequence` is the invitation's revision at the time, so an answer to an older revision is not
-- mistaken for one to its update.
CREATE TABLE invite_answers (
    message     TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    attendance  TEXT NOT NULL,      -- serde(Attendance)
    sequence    INTEGER NOT NULL,
    comment     TEXT,
    answered_at TEXT NOT NULL
);
