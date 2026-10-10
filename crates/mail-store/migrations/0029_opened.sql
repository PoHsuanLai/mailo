-- History: the conversations the person opened, and when they last did.
--
-- One row per conversation, so opening it again moves its row to the newest rather than adding a
-- second. Local only, like a saved view: nothing here is mail and nothing reaches a server. A
-- conversation that goes (its account is removed, or it is merged away) takes its row with it.
-- The store forgets rows older than ninety days whenever it records an open, so nothing else
-- needs to sweep.
CREATE TABLE opened (
    thread    TEXT PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE,
    opened_at TEXT NOT NULL  -- RFC 3339, UTC, fixed width: it sorts as the instants do
) WITHOUT ROWID;
CREATE INDEX opened_by_time ON opened(opened_at);
