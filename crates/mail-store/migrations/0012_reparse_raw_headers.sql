-- Header fields in raw 8-bit legacy charsets (Big5, GBK, Shift_JIS, KOI8-R…) are now decoded
-- instead of shown as U+FFFD, and so are RFC 2047 encoded-words in multi-byte charsets, which the
-- parser had been reading as UTF-8. Mail already stored was parsed the old way and keeps its
-- replacement characters until something rewrites the row: for a message held whole, nothing
-- ever would.
--
-- So every message held whole whose subject, sender or recipients carry U+FFFD is queued here,
-- and the next `mailo` run re-parses its stored bytes and rewrites those fields. A table rather
-- than SQL, for the reason 0007 gives: the parse is Rust, and this crate cannot run it. A
-- message held as headers only is not queued; its fields are rewritten when its body arrives.

CREATE TABLE messages_to_reparse (
    message TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE
);
INSERT INTO messages_to_reparse (message)
    SELECT id FROM messages
    WHERE body_raw IS NOT NULL
      AND (instr(subject, char(65533)) > 0
           OR instr(coalesce(from_name, ''), char(65533)) > 0
           OR instr(recipients, char(65533)) > 0);
