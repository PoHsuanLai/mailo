-- Search the server: messages that came to this computer because a search of the server found
-- them, not because a sync fetched them.
--
-- A server search fetches the headers of what it found and keeps them like any other message,
-- so they list, open and thread as mail does. This row is how the list can say where one came
-- from ("from the server"): it is a fact about how the message arrived, kept for as long as the
-- message is. A message already held when a search found it is not marked.
CREATE TABLE found_on_server (
    message  TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    account  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    found_at TEXT NOT NULL
);
