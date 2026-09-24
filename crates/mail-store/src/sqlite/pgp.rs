//! OpenPGP public keys and Autocrypt peer state.
//!
//! Public keys only. The secret halves are in the OS keyring (`mail-runtime`'s `Secrets`), and
//! the `secret` column says no more than whether it holds one.

use super::SqliteStore;
use super::row::{from_time, json, time, to_json};
use crate::StoreError;
use mail_domain::{AutocryptPeer, Fingerprint, KeyId, KeyTrust, PgpKey};
use rusqlite::{OptionalExtension, params};

const KEY_COLUMNS: &str = "fingerprint, key_ids, user_ids, emails, key, source, first_seen, \
     last_seen, trust, secret, created, expires";

impl SqliteStore {
    /// Keys matching `clause` (with `?1` bound to `param`), or every key for an empty clause.
    fn keys_where(&self, clause: &str, param: &str) -> Result<Vec<PgpKey>, StoreError> {
        let db = self.reader();
        let sql = format!("SELECT {KEY_COLUMNS} FROM pgp_keys {clause}");
        let mut stmt = db.prepare_cached(&sql)?;
        let mut rows = if clause.is_empty() {
            stmt.query([])?
        } else {
            stmt.query(params![param])?
        };
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(read_key(row)?);
        }
        Ok(out)
    }

    pub(super) fn load_pgp_keys(&self) -> Result<Vec<PgpKey>, StoreError> {
        let mut keys = self.keys_where("", "")?;
        keys.sort_by_key(|k| k.fingerprint);
        Ok(keys)
    }

    pub(super) fn load_pgp_key(&self, fp: Fingerprint) -> Result<Option<PgpKey>, StoreError> {
        Ok(self
            .keys_where("WHERE fingerprint = ?1", &fp.to_string())?
            .into_iter()
            .next())
    }

    pub(super) fn load_pgp_keys_for(&self, address: &str) -> Result<Vec<PgpKey>, StoreError> {
        let address = address.trim().to_ascii_lowercase();
        let keys = self.keys_where(
            "WHERE EXISTS (SELECT 1 FROM json_each(pgp_keys.emails) WHERE value = ?1)",
            &address,
        )?;
        Ok(crate::pgp::preferred(keys))
    }

    pub(super) fn load_pgp_keys_by_id(&self, id: KeyId) -> Result<Vec<PgpKey>, StoreError> {
        let keys = self.keys_where(
            "WHERE EXISTS (SELECT 1 FROM json_each(pgp_keys.key_ids) WHERE value = ?1)",
            &id.to_string(),
        )?;
        Ok(crate::pgp::preferred(keys))
    }

    pub(super) fn write_pgp_key(&self, key: PgpKey) -> Result<PgpKey, StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        let held = tx
            .query_row(
                &format!("SELECT {KEY_COLUMNS} FROM pgp_keys WHERE fingerprint = ?1"),
                params![key.fingerprint.to_string()],
                |row| Ok(read_key(row)),
            )
            .optional()?
            .transpose()?;
        let merged = match held {
            Some(held) => held.merged(key),
            None => key,
        };
        tx.execute(
            &format!(
                "INSERT OR REPLACE INTO pgp_keys ({KEY_COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
            ),
            params![
                merged.fingerprint.to_string(),
                to_json("PgpKey.key_ids", &merged.key_ids)?,
                to_json("PgpKey.user_ids", &merged.user_ids)?,
                to_json("PgpKey.emails", &merged.emails)?,
                merged.key,
                to_json("KeySource", &merged.source)?,
                from_time(merged.first_seen),
                from_time(merged.last_seen),
                to_json("KeyTrust", &merged.trust)?,
                to_json("SecretHeld", &merged.secret)?,
                merged.created.map(from_time),
                merged.expires.map(from_time),
            ],
        )?;
        tx.commit()?;
        Ok(merged)
    }

    pub(super) fn write_pgp_trust(
        &self,
        fp: Fingerprint,
        trust: KeyTrust,
    ) -> Result<(), StoreError> {
        let changed = self.connection().execute(
            "UPDATE pgp_keys SET trust = ?2 WHERE fingerprint = ?1",
            params![fp.to_string(), to_json("KeyTrust", &trust)?],
        )?;
        if changed == 0 {
            return Err(StoreError::NoPgpKey(fp));
        }
        Ok(())
    }

    pub(super) fn remove_pgp_key(&self, fp: Fingerprint) -> Result<bool, StoreError> {
        let changed = self.connection().execute(
            "DELETE FROM pgp_keys WHERE fingerprint = ?1",
            params![fp.to_string()],
        )?;
        Ok(changed > 0)
    }

    pub(super) fn load_autocrypt_peer(
        &self,
        address: &str,
    ) -> Result<Option<AutocryptPeer>, StoreError> {
        type Row = (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
        );
        let db = self.reader();
        let row: Option<Row> = db
            .query_row(
                "SELECT address, last_seen, autocrypt_timestamp, key, prefer_encrypt,
                        gossip_timestamp, gossip_key
                 FROM autocrypt_peers WHERE address = ?1",
                params![address.trim().to_ascii_lowercase()],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((address, last_seen, stamp, key, prefer, gossip_at, gossip_key)) = row else {
            return Ok(None);
        };
        let at = |what: &str, text: Option<String>| text.map(|t| time(what, &t)).transpose();
        let fp = |what: &str, text: Option<String>| {
            text.map(|t| {
                t.parse::<Fingerprint>().map_err(|why| StoreError::Decode {
                    what: what.to_owned(),
                    why,
                })
            })
            .transpose()
        };
        Ok(Some(AutocryptPeer {
            address,
            last_seen: at("autocrypt_peers.last_seen", last_seen)?,
            autocrypt_timestamp: at("autocrypt_peers.autocrypt_timestamp", stamp)?,
            key: fp("autocrypt_peers.key", key)?,
            prefer_encrypt: json("PreferEncrypt", &prefer)?,
            gossip_timestamp: at("autocrypt_peers.gossip_timestamp", gossip_at)?,
            gossip_key: fp("autocrypt_peers.gossip_key", gossip_key)?,
        }))
    }

    pub(super) fn write_autocrypt_peer(&self, peer: &AutocryptPeer) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT OR REPLACE INTO autocrypt_peers
                 (address, last_seen, autocrypt_timestamp, key, prefer_encrypt,
                  gossip_timestamp, gossip_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                peer.address.trim().to_ascii_lowercase(),
                peer.last_seen.map(from_time),
                peer.autocrypt_timestamp.map(from_time),
                peer.key.map(|k| k.to_string()),
                to_json("PreferEncrypt", &peer.prefer_encrypt)?,
                peer.gossip_timestamp.map(from_time),
                peer.gossip_key.map(|k| k.to_string()),
            ],
        )?;
        Ok(())
    }
}

fn read_key(row: &rusqlite::Row<'_>) -> Result<PgpKey, StoreError> {
    let fingerprint: String = row.get(0)?;
    Ok(PgpKey {
        fingerprint: fingerprint.parse().map_err(|why| StoreError::Decode {
            what: "pgp_keys.fingerprint".to_owned(),
            why,
        })?,
        key_ids: json("PgpKey.key_ids", &row.get::<_, String>(1)?)?,
        user_ids: json("PgpKey.user_ids", &row.get::<_, String>(2)?)?,
        emails: json("PgpKey.emails", &row.get::<_, String>(3)?)?,
        key: row.get(4)?,
        source: json("KeySource", &row.get::<_, String>(5)?)?,
        first_seen: time("pgp_keys.first_seen", &row.get::<_, String>(6)?)?,
        last_seen: time("pgp_keys.last_seen", &row.get::<_, String>(7)?)?,
        trust: json("KeyTrust", &row.get::<_, String>(8)?)?,
        secret: json("SecretHeld", &row.get::<_, String>(9)?)?,
        created: row
            .get::<_, Option<String>>(10)?
            .map(|t| time("pgp_keys.created", &t))
            .transpose()?,
        expires: row
            .get::<_, Option<String>>(11)?
            .map(|t| time("pgp_keys.expires", &t))
            .transpose()?,
    })
}
