//! S/MIME certificates.
//!
//! Certificates only. The private keys are in the OS keyring (`mail-runtime`'s `Secrets`), and
//! the `secret` column says no more than whether it holds one.

use super::SqliteStore;
use super::row::{from_time, json, time, to_json};
use crate::StoreError;
use mail_domain::{CertFingerprint, KeyTrust, SmimeCert};
use rusqlite::{OptionalExtension, params};

const CERT_COLUMNS: &str = "fingerprint, subject, issuer, serial, emails, not_before, not_after, \
     der, chain, source, first_seen, last_seen, trust, secret";

impl SqliteStore {
    /// Certificates matching `clause` (with `?1` bound to `param`), or all for an empty clause.
    fn certs_where(&self, clause: &str, param: &str) -> Result<Vec<SmimeCert>, StoreError> {
        let db = self.reader();
        let sql = format!("SELECT {CERT_COLUMNS} FROM smime_certs {clause}");
        let mut stmt = db.prepare_cached(&sql)?;
        let mut rows = if clause.is_empty() {
            stmt.query([])?
        } else {
            stmt.query(params![param])?
        };
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(read_cert(row)?);
        }
        Ok(out)
    }

    pub(super) fn load_smime_certs(&self) -> Result<Vec<SmimeCert>, StoreError> {
        let mut certs = self.certs_where("", "")?;
        certs.sort_by_key(|c| c.fingerprint);
        Ok(certs)
    }

    pub(super) fn load_smime_cert(
        &self,
        fp: CertFingerprint,
    ) -> Result<Option<SmimeCert>, StoreError> {
        Ok(self
            .certs_where("WHERE fingerprint = ?1", &fp.to_string())?
            .into_iter()
            .next())
    }

    pub(super) fn load_smime_certs_for(&self, address: &str) -> Result<Vec<SmimeCert>, StoreError> {
        let address = address.trim().to_ascii_lowercase();
        let certs = self.certs_where(
            "WHERE EXISTS (SELECT 1 FROM json_each(smime_certs.emails) WHERE value = ?1)",
            &address,
        )?;
        Ok(crate::smime::preferred(certs))
    }

    pub(super) fn write_smime_cert(&self, cert: SmimeCert) -> Result<SmimeCert, StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        let held = tx
            .query_row(
                &format!("SELECT {CERT_COLUMNS} FROM smime_certs WHERE fingerprint = ?1"),
                params![cert.fingerprint.to_string()],
                |row| Ok(read_cert(row)),
            )
            .optional()?
            .transpose()?;
        let merged = match held {
            Some(held) => held.merged(cert),
            None => cert,
        };
        tx.execute(
            &format!(
                "INSERT OR REPLACE INTO smime_certs ({CERT_COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"
            ),
            params![
                merged.fingerprint.to_string(),
                merged.subject,
                merged.issuer,
                merged.serial,
                to_json("SmimeCert.emails", &merged.emails)?,
                from_time(merged.not_before),
                from_time(merged.not_after),
                merged.der,
                to_json("SmimeCert.chain", &merged.chain)?,
                to_json("CertSource", &merged.source)?,
                from_time(merged.first_seen),
                from_time(merged.last_seen),
                to_json("KeyTrust", &merged.trust)?,
                to_json("SecretHeld", &merged.secret)?,
            ],
        )?;
        tx.commit()?;
        Ok(merged)
    }

    pub(super) fn write_smime_trust(
        &self,
        fp: CertFingerprint,
        trust: KeyTrust,
    ) -> Result<(), StoreError> {
        let changed = self.connection().execute(
            "UPDATE smime_certs SET trust = ?2 WHERE fingerprint = ?1",
            params![fp.to_string(), to_json("KeyTrust", &trust)?],
        )?;
        if changed == 0 {
            return Err(StoreError::NoSmimeCert(fp));
        }
        Ok(())
    }

    pub(super) fn remove_smime_cert(&self, fp: CertFingerprint) -> Result<bool, StoreError> {
        let changed = self.connection().execute(
            "DELETE FROM smime_certs WHERE fingerprint = ?1",
            params![fp.to_string()],
        )?;
        Ok(changed > 0)
    }
}

fn read_cert(row: &rusqlite::Row<'_>) -> Result<SmimeCert, StoreError> {
    let fingerprint: String = row.get(0)?;
    Ok(SmimeCert {
        fingerprint: fingerprint.parse().map_err(|why| StoreError::Decode {
            what: "smime_certs.fingerprint".to_owned(),
            why,
        })?,
        subject: row.get(1)?,
        issuer: row.get(2)?,
        serial: row.get(3)?,
        emails: json("SmimeCert.emails", &row.get::<_, String>(4)?)?,
        not_before: time("smime_certs.not_before", &row.get::<_, String>(5)?)?,
        not_after: time("smime_certs.not_after", &row.get::<_, String>(6)?)?,
        der: row.get(7)?,
        chain: json("SmimeCert.chain", &row.get::<_, String>(8)?)?,
        source: json("CertSource", &row.get::<_, String>(9)?)?,
        first_seen: time("smime_certs.first_seen", &row.get::<_, String>(10)?)?,
        last_seen: time("smime_certs.last_seen", &row.get::<_, String>(11)?)?,
        trust: json("KeyTrust", &row.get::<_, String>(12)?)?,
        secret: json("SecretHeld", &row.get::<_, String>(13)?)?,
    })
}
