//! Signatures: several to an identity, in the `signatures` table, in the order they were made.
//!
//! The only code that reads or writes that table. An identity has at most one default for new
//! mail and one for replies (the table's partial unique indexes), so a write that makes one the
//! default takes the flag off the one that had it in the same transaction.

use super::SqliteStore;
use super::row::uuid;
use crate::StoreError;
use crate::changes::Topics;
use mail_domain::{IdentityId, IsDefault, Signature, SignatureId, SignatureUse};
use rusqlite::{Row, Transaction, params};

/// The columns of a signature, in the order [`signature_of`] reads them.
const SIGNATURE: &str = "id, identity, name, body_html, body_text, for_new, for_replies";

/// A signature out of a row selected with [`SIGNATURE`].
fn signature_of(row: &Row<'_>) -> Result<Signature, StoreError> {
    let id: String = row.get(0)?;
    let identity: String = row.get(1)?;
    Ok(Signature {
        id: SignatureId::from_uuid(uuid("SignatureId", &id)?),
        identity: IdentityId::from_uuid(uuid("IdentityId", &identity)?),
        name: row.get(2)?,
        html: row.get(3)?,
        text: row.get(4)?,
        new_mail: flag(row.get(5)?),
        replies: flag(row.get(6)?),
    })
}

/// A flag column as the domain says it.
fn flag(set: bool) -> IsDefault {
    if set {
        IsDefault::Default
    } else {
        IsDefault::Alternate
    }
}

/// The domain's word as a flag column.
fn column(default: IsDefault) -> bool {
    default == IsDefault::Default
}

/// The flag column `purpose` is kept in.
fn column_of(purpose: SignatureUse) -> &'static str {
    match purpose {
        SignatureUse::NewMail => "for_new",
        SignatureUse::Replies => "for_replies",
    }
}

impl SqliteStore {
    /// The identity's signatures, oldest first.
    pub fn signatures(&self, identity: IdentityId) -> Result<Vec<Signature>, StoreError> {
        self.read_signatures("WHERE identity = ?1 ORDER BY rowid", [identity.to_string()])
    }

    /// The signature with this id. [`StoreError::NoSignature`] when there is none.
    pub fn signature(&self, id: SignatureId) -> Result<Signature, StoreError> {
        self.read_signatures("WHERE id = ?1", [id.to_string()])?
            .into_iter()
            .next()
            .ok_or(StoreError::NoSignature(id))
    }

    /// The identity's default for `purpose`, if it has one.
    pub fn default_signature(
        &self,
        identity: IdentityId,
        purpose: SignatureUse,
    ) -> Result<Option<Signature>, StoreError> {
        let column = column_of(purpose);
        Ok(self
            .read_signatures(
                &format!("WHERE identity = ?1 AND {column} = 1"),
                [identity.to_string()],
            )?
            .into_iter()
            .next())
    }

    /// Write a signature, a new one or over the one with its id, which keeps its place in the
    /// list. A default it is made takes that flag off the identity's other signatures.
    pub fn put_signature(&self, signature: &Signature) -> Result<(), StoreError> {
        let db = self.touch(Topics::IDENTITIES);
        let tx = db.unchecked_transaction()?;
        for purpose in [SignatureUse::NewMail, SignatureUse::Replies] {
            if signature.default_for(purpose) == IsDefault::Default {
                clear_default(&tx, signature.identity, purpose, Some(signature.id))?;
            }
        }
        tx.execute(
            "INSERT INTO signatures
                 (id, identity, name, body_html, body_text, for_new, for_replies)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET identity = excluded.identity, name = excluded.name,
                 body_html = excluded.body_html, body_text = excluded.body_text,
                 for_new = excluded.for_new, for_replies = excluded.for_replies",
            params![
                signature.id.to_string(),
                signature.identity.to_string(),
                signature.name,
                signature.html,
                signature.text,
                column(signature.new_mail),
                column(signature.replies),
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Delete a signature. [`StoreError::NoSignature`] when there was none.
    pub fn delete_signature(&self, id: SignatureId) -> Result<(), StoreError> {
        let gone = self
            .touch(Topics::IDENTITIES)
            .execute("DELETE FROM signatures WHERE id = ?1", [id.to_string()])?;
        if gone == 0 {
            return Err(StoreError::NoSignature(id));
        }
        Ok(())
    }

    /// Set the text of an identity's default signature for new mail, or delete that signature
    /// with `None`: the one signature `mailo signature` speaks of.
    ///
    /// Set, it replaces the default's words and drops any HTML it had, which said something
    /// else; with no default yet, it is a new signature, the default for new mail and, when no
    /// other is, for replies. The `signature` column on `identities` is emptied either way, so
    /// what it once said cannot come back from under a signature the user changed.
    pub fn set_signature(
        &self,
        identity: IdentityId,
        signature: Option<&str>,
    ) -> Result<(), StoreError> {
        let had = self.default_signature(identity, SignatureUse::NewMail)?;
        let replies = self.default_signature(identity, SignatureUse::Replies)?;
        match (signature, had) {
            (Some(text), Some(had)) => self.put_signature(&Signature {
                html: None,
                text: text.to_owned(),
                ..had
            })?,
            (Some(text), None) => self.put_signature(&Signature {
                id: SignatureId::generate(),
                identity,
                name: mail_domain::UNNAMED_SIGNATURE.to_owned(),
                html: None,
                text: text.to_owned(),
                new_mail: IsDefault::Default,
                replies: flag(replies.is_none()),
            })?,
            (None, Some(had)) => self.delete_signature(had.id)?,
            (None, None) => {}
        }
        self.touch(Topics::IDENTITIES).execute(
            "UPDATE identities SET signature = NULL WHERE id = ?1",
            [identity.to_string()],
        )?;
        Ok(())
    }

    fn read_signatures<P: rusqlite::Params>(
        &self,
        tail: &str,
        params: P,
    ) -> Result<Vec<Signature>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare(&format!("SELECT {SIGNATURE} FROM signatures {tail}"))?;
        let mut rows = stmt.query(params)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(signature_of(row)?);
        }
        Ok(out)
    }
}

/// Take the `purpose` flag off every signature of `identity` but `keep`.
fn clear_default(
    tx: &Transaction<'_>,
    identity: IdentityId,
    purpose: SignatureUse,
    keep: Option<SignatureId>,
) -> Result<(), StoreError> {
    let column = column_of(purpose);
    tx.execute(
        &format!("UPDATE signatures SET {column} = 0 WHERE identity = ?1 AND id IS NOT ?2"),
        params![identity.to_string(), keep.map(|id| id.to_string())],
    )?;
    Ok(())
}
