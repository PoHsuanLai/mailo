-- When this computer first stored a message, apart from the date its sender wrote on it. A
-- POP3 account's "after N days" counts from the later of the two, so a late or forged Date
-- header cannot make a message just downloaded old enough to remove from the server. A message
-- stored before this has only its date, which is the best there is.
ALTER TABLE messages ADD COLUMN stored_at TEXT;
UPDATE messages SET stored_at = date;
