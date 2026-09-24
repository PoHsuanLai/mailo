-- Which messages a queued operation is about (FINDINGS F153).
--
-- An operation on messages was resolved into server addresses when it was queued, and sent to
-- those addresses however long it waited. A message moved by an operation queued before it has
-- a new address by then, and the old one names nothing: `UID MOVE` of a UID no longer in the
-- mailbox answers OK, so a second move queued behind a first was settled and lost. The outbox
-- now keeps the messages themselves and looks their addresses up when the operation is sent.
--
-- serde(Vec<MessageId>): the messages that had a server address when the operation was queued.
-- NULL for an operation that names no message — a send, an upload, folder work — which is sent
-- as it was queued. The `op` column is unchanged, and its addresses are still written.
ALTER TABLE outbox ADD COLUMN messages TEXT;

-- Rows already queued: their messages are the ones their pending changes are about, where those
-- still have an address. A keyword has no pending changes, and a row with none keeps NULL and is
-- sent to the addresses it was queued with, as before.
UPDATE outbox SET messages = (
    SELECT json_group_array(p.message) FROM pending_changes p
    WHERE p.outbox = outbox.id
      AND EXISTS (SELECT 1 FROM remote_map r
                  WHERE r.message = p.message AND r.account = outbox.account)
)
WHERE json_extract(op, '$.kind') IN ('set_flags', 'set_mailbox', 'set_labels', 'file')
  AND EXISTS (SELECT 1 FROM pending_changes p
              WHERE p.outbox = outbox.id
                AND EXISTS (SELECT 1 FROM remote_map r
                            WHERE r.message = p.message AND r.account = outbox.account));

-- Messages the server moved without saying where (no `COPYUID`): their old address was let go,
-- and until a sync finds them again they have none. An operation the user makes on one meanwhile
-- is queued all the same and waits for that sync; without this row it looked like mail the
-- server never held, and was applied here and never sent. Gone as soon as the message has an
-- address again, and with the message.
CREATE TABLE unplaced (
    account TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    PRIMARY KEY (account, message)
);

CREATE TRIGGER unplaced_found_ins AFTER INSERT ON remote_map BEGIN
    DELETE FROM unplaced WHERE account = NEW.account AND message = NEW.message;
END;

CREATE TRIGGER unplaced_found_upd AFTER UPDATE OF message ON remote_map BEGIN
    DELETE FROM unplaced WHERE account = NEW.account AND message = NEW.message;
END;
