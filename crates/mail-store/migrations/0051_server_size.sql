-- The size the server gave a message, kept apart from the size of what is held. A message held by
-- its headers alone has no text and no parts, so `messages.size` was 0 for it however large the
-- message is, and `larger:` missed it. The server says (IMAP `RFC822.SIZE`, JMAP `size`) before
-- the body is fetched; that is this column. `messages.size` is the larger of this and what is held
-- (`upsert_message`, `fill_body`, `hold_part`), so a fetched body or part only ever sharpens it
-- and a draft rewritten shorter still shrinks (it has no server size, so 0).
ALTER TABLE messages ADD COLUMN server_size INTEGER NOT NULL DEFAULT 0;
