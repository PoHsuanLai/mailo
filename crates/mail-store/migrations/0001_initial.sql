-- Schema v1. Forward-only. Never edit a released migration; add a new numbered file.
--
-- Conventions in this file:
--   * ids are UUID text (lowercase hyphenated) except outbox.id, which is a rowid
--   * enums and nested structures are JSON, in the serde form fixed by CONVENTIONS.md §3
--   * timestamps are RFC 3339 UTC text, so they sort lexicographically
--   * every FK is declared and ON DELETE is always explicit

PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE schema_version (
    version     INTEGER NOT NULL,
    applied_at  TEXT    NOT NULL
);

-- ---------------------------------------------------------------- accounts

CREATE TABLE accounts (
    id          TEXT PRIMARY KEY,
    address     TEXT NOT NULL UNIQUE,
    -- serde(AccountPlan). Read with #[serde(default)] discipline: a field added in a later
    -- version must deserialize against rows written by an earlier one.
    plan        TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

-- Separate from accounts because capabilities are discovered, not configured, and are
-- rewritten on every connect. Folding them into `plan` would mean rewriting the user's
-- saved configuration each time we talk to the server.
CREATE TABLE account_caps (
    account     TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    caps        TEXT NOT NULL,   -- serde(AccountCaps)
    observed_at TEXT NOT NULL
);

CREATE TABLE identities (
    id          TEXT PRIMARY KEY,
    account     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    from_name   TEXT,
    from_email  TEXT NOT NULL,
    reply_to    TEXT,            -- serde(Option<Address>)
    signature   TEXT,
    is_default  TEXT NOT NULL    -- serde(IsDefault): 'default' | 'alternate'
);
CREATE INDEX identities_account ON identities(account);

-- ---------------------------------------------------------------- content

CREATE TABLE blobs (
    id          TEXT PRIMARY KEY,
    -- blake3 of the contents. Two blobs with one hash are one file on disk.
    hash        TEXT NOT NULL,
    size        INTEGER NOT NULL,
    -- NULL when the bytes are inline below (small parts), else relative to the blob root.
    path        TEXT,
    inline      BLOB,
    created_at  TEXT NOT NULL,
    CHECK ((path IS NULL) != (inline IS NULL))
);
CREATE INDEX blobs_hash ON blobs(hash);

CREATE TABLE labels (
    id          TEXT PRIMARY KEY,
    account     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    color       TEXT,
    origin      TEXT NOT NULL,   -- serde(LabelOrigin): 'user' | 'provider'
    UNIQUE (account, name)
);

-- ---------------------------------------------------------------- mail

CREATE TABLE threads (
    id              TEXT PRIMARY KEY,
    account         TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- Thread-level user state. Everything else in ThreadSummary is derived from messages
    -- and lives in the cache table below.
    snooze          TEXT NOT NULL,   -- serde(Snooze)
    pin             TEXT NOT NULL    -- serde(Pin)
);
CREATE INDEX threads_account ON threads(account);

CREATE TABLE messages (
    id              TEXT PRIMARY KEY,
    thread          TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    account         TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- serde(MessageKey). The deduplication key: several remote_map rows may resolve here.
    msg_key         TEXT NOT NULL,
    date            TEXT NOT NULL,
    from_name       TEXT,
    from_email      TEXT NOT NULL,
    recipients      TEXT NOT NULL,   -- serde({reply_to, to, cc, bcc})
    subject         TEXT NOT NULL,
    in_reply_to     TEXT,            -- normalized
    refs            TEXT NOT NULL,   -- serde(Vec<String>), normalized, oldest first
    rfc_message_id  TEXT,            -- normalized
    read            TEXT NOT NULL,   -- serde(ReadState)
    star            TEXT NOT NULL,   -- serde(Star)
    mailbox         TEXT NOT NULL,   -- serde(MailboxRole); a message really is in one place
    body_text       TEXT,
    body_raw        TEXT NOT NULL REFERENCES blobs(id),
    attachments     TEXT NOT NULL,   -- serde(Vec<Attachment>)
    UNIQUE (account, msg_key)
);
CREATE INDEX messages_thread ON messages(thread);
CREATE INDEX messages_account_date ON messages(account, date DESC);
CREATE INDEX messages_mailbox ON messages(account, mailbox);

CREATE TABLE message_labels (
    message     TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    label       TEXT NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (message, label)
) WITHOUT ROWID;
CREATE INDEX message_labels_label ON message_labels(label);

-- Derived fields of ThreadSummary, materialized so a list query is one indexed scan rather
-- than an aggregate over messages. Rewritten by ThreadSummary::derive on every change to a
-- thread's messages; it is a cache and can always be rebuilt.
CREATE TABLE thread_summary (
    thread          TEXT PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE,
    account         TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    subject         TEXT NOT NULL,
    snippet         TEXT NOT NULL,
    from_name       TEXT,
    from_email      TEXT NOT NULL,
    participants    TEXT NOT NULL,   -- serde(Vec<Address>): senders, oldest first
    -- serde(Vec<Address>): To and Cc across the thread. Bcc is excluded on purpose — a blind
    -- copy is not a fact the thread's other readers share, and a list row would leak it.
    recipients      TEXT NOT NULL,
    last_date       TEXT NOT NULL,
    message_count   INTEGER NOT NULL,
    read            TEXT NOT NULL,   -- serde(ReadState): unread if ANY message is unread
    star            TEXT NOT NULL,   -- serde(Star): starred if ANY message is starred
    mailboxes       TEXT NOT NULL,   -- serde(MailboxSet): a JSON array of role names
    labels          TEXT NOT NULL,   -- serde(Vec<LabelId>)
    attachments     TEXT NOT NULL,   -- serde(Attachments)
    snooze          TEXT NOT NULL,
    pin             TEXT NOT NULL
);
-- Keyset pagination: (sort key, id) so a page boundary is stable while mail arrives.
CREATE INDEX thread_summary_date  ON thread_summary(account, last_date DESC, thread DESC);
CREATE INDEX thread_summary_unread ON thread_summary(account, read, last_date DESC);

-- from_name is indexed because Filter::fit matches display names, not just addresses. Without
-- it, searching for "Ada Lovelace" finds nothing in SQL while fit says it matches, and the
-- parity proptest is right to fail.
--
-- Indexing per message also covers ThreadSummary.participants exactly: the participants of a
-- thread ARE the senders of its messages, and the Text predicate asks whether each token
-- appears in some message of the thread.
CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject, from_name, from_email, body_text,
    content = 'messages',
    content_rowid = 'rowid',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER messages_fts_ins AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, subject, from_name, from_email, body_text)
    VALUES (new.rowid, new.subject, new.from_name, new.from_email, new.body_text);
END;
CREATE TRIGGER messages_fts_del AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_email, body_text)
    VALUES ('delete', old.rowid, old.subject, old.from_name, old.from_email, old.body_text);
END;
CREATE TRIGGER messages_fts_upd AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_email, body_text)
    VALUES ('delete', old.rowid, old.subject, old.from_name, old.from_email, old.body_text);
    INSERT INTO messages_fts(rowid, subject, from_name, from_email, body_text)
    VALUES (new.rowid, new.subject, new.from_name, new.from_email, new.body_text);
END;

-- ---------------------------------------------------------------- sync

-- MANY-TO-ONE. The same message is in INBOX and [Gmail]/All Mail under different UIDs, and
-- again in [Gmail]/Sent if we sent it. The primary key is the remote address; `message` is
-- not unique. Making it unique duplicates every message on the first Gmail sync.
CREATE TABLE remote_map (
    account     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mailbox     TEXT NOT NULL,       -- IMAP folder path, or 'INBOX' for POP3
    uidvalidity INTEGER,             -- NULL for POP3
    uid         INTEGER,             -- NULL for POP3
    uidl        TEXT,                -- NULL for IMAP
    message     TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    PRIMARY KEY (account, mailbox, uidvalidity, uid, uidl),
    CHECK ((uid IS NULL) != (uidl IS NULL))
);
CREATE INDEX remote_map_message ON remote_map(message);

CREATE TABLE sync_state (
    account     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mailbox     TEXT NOT NULL,
    cursor      TEXT NOT NULL,       -- serde(SyncCursor)
    synced_at   TEXT NOT NULL,
    PRIMARY KEY (account, mailbox)
) WITHOUT ROWID;

-- ---------------------------------------------------------------- outbound

CREATE TABLE drafts (
    id          TEXT PRIMARY KEY,
    account     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    identity    TEXT NOT NULL REFERENCES identities(id),
    recipients  TEXT NOT NULL,       -- serde({to, cc, bcc})
    subject     TEXT NOT NULL,
    in_reply_to TEXT REFERENCES messages(id) ON DELETE SET NULL,
    forward_of  TEXT REFERENCES messages(id) ON DELETE SET NULL,
    body_text   TEXT NOT NULL,
    body_html   TEXT,
    attachments TEXT NOT NULL,       -- serde(Vec<PendingAttachment>)
    state       TEXT NOT NULL,       -- serde(SendState)
    updated_at  TEXT NOT NULL
);
CREATE INDEX drafts_account ON drafts(account);

CREATE TABLE outbox (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,  -- monotonic: drains in insertion order
    account      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    op           TEXT NOT NULL,      -- serde(ProtoOp)
    -- serde(Patch). Applied if this entry fails with Retry::Fatal, so an optimistic local
    -- change does not survive an operation the server refused.
    undo         TEXT NOT NULL,
    attempts     INTEGER NOT NULL DEFAULT 0,
    next_attempt TEXT NOT NULL,
    last_error   TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX outbox_due ON outbox(account, next_attempt);

-- Which messages carry a local change that the server has not confirmed.
--
-- This is what makes optimistic apply safe. When an Ingest brings server truth for a message
-- listed here, the store writes the server value and then RE-APPLIES the pending change on
-- top before committing. Without that step, the next poll after starring a message flips the
-- star back and the UI flickers.
CREATE TABLE pending_changes (
    message     TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    outbox      INTEGER NOT NULL REFERENCES outbox(id) ON DELETE CASCADE,
    -- serde(Vec<Change>): the subset of the patch affecting this message.
    changes     TEXT NOT NULL,
    PRIMARY KEY (message, outbox)
) WITHOUT ROWID;
CREATE INDEX pending_changes_outbox ON pending_changes(outbox);

-- ---------------------------------------------------------------- local-only

CREATE TABLE views (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL,       -- serde(ViewKind)
    filter      TEXT NOT NULL,       -- serde(Filter)
    sort        TEXT NOT NULL,       -- serde(Sort)
    group_by    TEXT,                -- serde(Option<Property>)
    threading   TEXT NOT NULL,       -- serde(Threading)
    shown       TEXT NOT NULL,       -- serde(Vec<Property>)
    hover       TEXT NOT NULL,       -- serde(Vec<OpKind>)
    position    INTEGER NOT NULL
);
CREATE INDEX views_position ON views(position);

INSERT INTO schema_version (version, applied_at) VALUES (1, datetime('now'));
