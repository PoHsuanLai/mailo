-- The thread id a server gave a message, kept against its address on the server.
--
-- Gmail names a conversation itself (`X-GM-THRID`) and says so in the survey, before the message
-- is fetched and a row exists to hold it. So it is held against the address, as the survey
-- states it, and threading reads it back when the message arrives and when a later one names the
-- same value. Only IMAP addresses are held: no other protocol has one.

CREATE TABLE provider_thread (
    account     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mailbox     TEXT NOT NULL,
    uidvalidity INTEGER NOT NULL,
    uid         INTEGER NOT NULL,
    thrid       TEXT NOT NULL,
    PRIMARY KEY (account, mailbox, uidvalidity, uid)
);
CREATE INDEX provider_thread_thrid ON provider_thread(account, thrid);
