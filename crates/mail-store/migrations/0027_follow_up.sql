-- Remind me if no reply: a conversation the user is waiting on an answer to.
--
-- Thread-level user state, like `snooze`, `pin` and `mute`, so it lives on `threads` and is
-- carried into the `thread_summary` cache beside them. serde(FollowUp):
-- '{"kind":"inactive"}' | '{"kind":"until","v":{"at":…,"set":…}}' | '{"kind":"returned",…}'.
--
-- Every conversation that exists before this migration had no reminder, since reminders did not
-- exist, so the default is the true value for every existing row and not a guess.
ALTER TABLE threads ADD COLUMN follow_up TEXT NOT NULL DEFAULT '{"kind":"inactive"}';
ALTER TABLE thread_summary ADD COLUMN follow_up TEXT NOT NULL DEFAULT '{"kind":"inactive"}';

-- The waiting list, read in the order reminders come due.
CREATE INDEX thread_summary_follow_up
    ON thread_summary (json_extract(follow_up, '$.v.at'))
    WHERE json_extract(follow_up, '$.kind') <> 'inactive';

-- A reminder asked for in the composer, waiting for its message to leave.
--
-- It belongs on a conversation only once the message is on its way to someone: until then Undo
-- can take the send back, and a reminder for mail that never went would bring the conversation
-- back for nothing. A reply names its conversation (`thread`) and joins it once the outbox has
-- sent it. A new message has no conversation here until its copy comes back from the Sent
-- folder, so it joins the conversation that copy lands in, found by the `Message-ID` it was sent
-- with. Keyed by the draft, so sending the same draft again replaces its reminder rather than
-- adding a second. No foreign key to the draft: a sent draft may be discarded before its copy
-- comes back, and the reminder is still wanted.
CREATE TABLE follow_up_held (
    draft       TEXT PRIMARY KEY,
    account     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_id  TEXT NOT NULL,   -- normalized, as `messages.rfc_message_id` holds it
    thread      TEXT,            -- the conversation a reply answers; NULL for a new message
    due_at      TEXT NOT NULL,   -- RFC 3339, UTC
    set_at      TEXT NOT NULL    -- RFC 3339, UTC: when the message leaves
) WITHOUT ROWID;
