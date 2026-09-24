-- OpenPGP (`plan.md` 10.15).
--
-- A draft says whether it is to be signed, encrypted, or both when it is sent. The column holds
-- serde(OpenPgp), and every draft written before it existed was plain, which is the default.
--
-- `pgp_keys` holds public keys only: the user's own public halves and correspondents' keys,
-- whichever way they arrived. The secret halves live in the OS keyring, named by fingerprint,
-- and `secret` records only whether the keyring holds one. `key` is the binary transferable
-- PUBLIC key; nothing writes a secret key here, and a test scans the database file to hold
-- that true.
--
-- `autocrypt_peers` is Autocrypt Level 1's per-correspondent state: when their mail and their
-- Autocrypt header were last seen, which key the header carried, and what other people's mail
-- gossiped for them. The keys it names are rows of `pgp_keys`. Separate tables because the one
-- is about keys, named by fingerprint, and the other about addresses.
--
-- Decrypted mail is kept in neither, nor anywhere else: a message is decrypted when it is read,
-- and what the reader shows is not written back.

ALTER TABLE drafts ADD COLUMN openpgp TEXT NOT NULL DEFAULT '"none"';

-- A template carries it too, so a template kept from an encrypted draft never starts a plain one.
ALTER TABLE templates ADD COLUMN openpgp TEXT NOT NULL DEFAULT '"none"';

CREATE TABLE pgp_keys (
    fingerprint TEXT PRIMARY KEY,    -- upper-case hex
    key_ids     TEXT NOT NULL,       -- JSON array of upper-case hex key ids, primary and subkeys
    user_ids    TEXT NOT NULL,       -- JSON array
    emails      TEXT NOT NULL,       -- JSON array, lower-cased
    key         BLOB NOT NULL,       -- binary transferable public key
    source      TEXT NOT NULL,       -- serde(KeySource)
    first_seen  TEXT NOT NULL,
    last_seen   TEXT NOT NULL,
    trust       TEXT NOT NULL,       -- serde(KeyTrust)
    secret      TEXT NOT NULL        -- serde(SecretHeld)
);

CREATE TABLE autocrypt_peers (
    address             TEXT PRIMARY KEY,   -- lower-cased
    last_seen           TEXT,
    autocrypt_timestamp TEXT,
    key                 TEXT,               -- fingerprint
    prefer_encrypt      TEXT NOT NULL,      -- serde(PreferEncrypt)
    gossip_timestamp    TEXT,
    gossip_key          TEXT                -- fingerprint
);
