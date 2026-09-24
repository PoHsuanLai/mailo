-- S/MIME (`plan.md` 10.16).
--
-- A draft says whether S/MIME signs it, encrypts it, or both when it is sent. The column holds
-- serde(Smime), and every draft written before it existed was plain, which is the default. A
-- template carries it too, so a template kept from an encrypted draft never starts a plain one.
--
-- `smime_certs` holds certificates only: the user's own, correspondents' collected from their
-- signed mail, and ones the user imported. The private keys live in the OS keyring, named by
-- certificate fingerprint, and `secret` records only whether the keyring holds one; nothing
-- writes a private key here, and a test scans the database file to hold that true. `trust` is
-- the user's own word: a certificate marked verified is a trust anchor beside the system's.
--
-- Decrypted mail is kept nowhere: a message is decrypted when it is read, and what the reader
-- shows is not written back.

-- And, for the window, when each OpenPGP key was made and when it expires, as the key says. Rows
-- kept before this have NULL in both until the application reads them from the stored key bytes
-- (`mail_runtime::pgp::date_keys`, run as the store is opened).
ALTER TABLE pgp_keys ADD COLUMN created TEXT;
ALTER TABLE pgp_keys ADD COLUMN expires TEXT;

ALTER TABLE drafts ADD COLUMN smime TEXT NOT NULL DEFAULT '"none"';

ALTER TABLE templates ADD COLUMN smime TEXT NOT NULL DEFAULT '"none"';

CREATE TABLE smime_certs (
    fingerprint TEXT PRIMARY KEY,    -- upper-case hex SHA-256 of `der`
    subject     TEXT NOT NULL,       -- RFC 4514
    issuer      TEXT NOT NULL,       -- RFC 4514
    serial      TEXT NOT NULL,       -- upper-case hex
    emails      TEXT NOT NULL,       -- JSON array, lower-cased
    not_before  TEXT NOT NULL,
    not_after   TEXT NOT NULL,
    der         BLOB NOT NULL,       -- the certificate
    chain       TEXT NOT NULL,       -- JSON array of the issuers' certificates, DER as byte arrays
    source      TEXT NOT NULL,       -- serde(CertSource)
    first_seen  TEXT NOT NULL,
    last_seen   TEXT NOT NULL,
    trust       TEXT NOT NULL,       -- serde(KeyTrust)
    secret      TEXT NOT NULL        -- serde(SecretHeld)
);
