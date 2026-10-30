-- Which mailboxes and which labels each thread is in, one row apiece, so that a list of one
-- mailbox or label is a search of an index and not a walk of every summary. The summary holds
-- the same facts as JSON arrays (`mailboxes`, `labels`), which SQLite cannot index: a list of
-- Drafts read every summary in date order until it had found its page, and every unread badge
-- counted by reading them all.
--
-- A cache as `thread_summary` is, kept in step with it by the code that writes it, and dropped
-- with it when its thread goes. `mailbox` is a role name as the summary spells it; `label` is a
-- label id.
CREATE TABLE thread_mailbox (
    thread  TEXT NOT NULL REFERENCES thread_summary(thread) ON DELETE CASCADE,
    mailbox TEXT NOT NULL,
    PRIMARY KEY (thread, mailbox)
) WITHOUT ROWID;
CREATE INDEX thread_mailbox_mailbox ON thread_mailbox(mailbox, thread);

CREATE TABLE thread_label (
    thread TEXT NOT NULL REFERENCES thread_summary(thread) ON DELETE CASCADE,
    label  TEXT NOT NULL,
    PRIMARY KEY (thread, label)
) WITHOUT ROWID;
CREATE INDEX thread_label_label ON thread_label(label, thread);

INSERT OR IGNORE INTO thread_mailbox (thread, mailbox)
    SELECT ts.thread, j.value FROM thread_summary ts, json_each(ts.mailboxes) j;
INSERT OR IGNORE INTO thread_label (thread, label)
    SELECT ts.thread, j.value FROM thread_summary ts, json_each(ts.labels) j;
