-- Push for an account that reads through Microsoft Graph: the relay it is pushed through, and the
-- subscription Graph sends that relay its change notifications under.
--
-- Graph delivers change notifications only to a public HTTPS webhook. A relay (`relay/` in the
-- repository) is that webhook, and forwards each one over a WebSocket. One row per account that
-- has ever had a relay named. `relay` is the person's setting: NULL is no push, and the account
-- polls as it always did. The rest is what mailo set up there, kept so a later run renews rather
-- than starts again: the key it registered with the relay at `registered`, and the subscription
-- it made with Graph and when that ends. The relay's listening secret is not here: it is an
-- account secret, in the keyring. The row goes with its account.
CREATE TABLE graph_push (
    account      TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    relay        TEXT,  -- the relay's base URL, as set; NULL when push is off
    registered   TEXT,  -- the relay `key` was registered with; NULL until one is
    key          TEXT,  -- the relay's name for this account's notifications
    subscription TEXT,  -- Graph's subscription id; NULL until one is made
    expires      TEXT   -- RFC 3339, UTC, fixed width: when Graph ends the subscription
) WITHOUT ROWID;
