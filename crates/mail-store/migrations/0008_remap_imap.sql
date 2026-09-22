-- Every IMAP `remote_map` row may point at the wrong message.
--
-- `ImapBackend` paired the bodies and headers a `UID FETCH` returned with the UIDs it asked for
-- by position. A server answers in mailbox order, whatever order the set was written in, and
-- once the engine started asking newest first every batch was paired backwards. Each message's
-- content is still its own — the store keys a message by its Message-ID, read from the bytes
-- that arrived — but the row saying which UID holds it was moved onto whichever message the
-- server happened to send in that position. On a real Gmail account: 1,302 messages held
-- several INBOX UIDs and 1,763 held none, so their bodies could never be fetched, and flags and
-- labels the server reported for one UID were applied to another message.
--
-- There is no telling the right rows from the wrong ones here, so all of them go. The header
-- pass fetches whatever the server lists that is not mapped, newest first; each message it
-- sees is matched to the one already held by its key, mapped to the UID it really has, and
-- given the flags the server reports for it. Nothing is deleted but the mapping, and nothing is
-- downloaded twice but headers.
--
-- POP3 rows (`uidl`) never went through that code and are kept.

DELETE FROM remote_map WHERE uid IS NOT NULL;
