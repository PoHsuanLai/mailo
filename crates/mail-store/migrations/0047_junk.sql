-- The local junk filter's word counts, one account at a time: how often each word was seen in
-- mail called spam and in mail called not spam, and how many messages of each were learned.
CREATE TABLE junk_words (
    account TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    word    TEXT NOT NULL,
    spam    INTEGER NOT NULL DEFAULT 0,
    ham     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, word)
) WITHOUT ROWID;

CREATE TABLE junk_totals (
    account TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    spam    INTEGER NOT NULL DEFAULT 0,
    ham     INTEGER NOT NULL DEFAULT 0
);

-- What each message was last learned as ('spam' or 'ham'), so learning it again changes
-- nothing and learning it the other way first takes back what it taught. The words it taught
-- are kept (one per line), so taking it back subtracts exactly those, whatever the message
-- says by then.
CREATE TABLE junk_learned (
    message TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    verdict TEXT NOT NULL,
    words   TEXT NOT NULL DEFAULT ''
);
