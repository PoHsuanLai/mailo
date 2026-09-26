-- Muting a conversation: its new mail arrives read and out of the inbox.
--
-- Thread-level user state, like `snooze` and `pin`, so it lives on `threads` and is carried into
-- the `thread_summary` cache beside them. serde(Mute): '"unmuted"' | '"muted"'.
--
-- Every conversation that exists before this migration was unmuted, since muting did not exist,
-- so the default is the true value for every existing row and not a guess.
ALTER TABLE threads ADD COLUMN mute TEXT NOT NULL DEFAULT '"unmuted"';
ALTER TABLE thread_summary ADD COLUMN mute TEXT NOT NULL DEFAULT '"unmuted"';
