//! How much of a JMAP account's storage is used: `Quota/get` (RFC 9425), where the session says
//! the account has quotas.

use super::JmapEngine;
use crate::RuntimeError;
use mail_proto::Storage;
use mail_proto::jmap::{CORE, QUOTA, Quotas, quota_get, storage};

impl JmapEngine {
    /// The account's mail storage as the server counts it: `None` when the server keeps no
    /// quota for it.
    pub async fn quota_jmap(&mut self) -> Result<Option<Storage>, RuntimeError> {
        let result = self.quota().await;
        if let Err(e) = &result {
            self.forget_if_stale(e);
        }
        result
    }

    async fn quota(&mut self) -> Result<Option<Storage>, RuntimeError> {
        let client = self.client().await?.clone();
        if client.session.quotas == Quotas::Absent {
            return Ok(None);
        }
        let call = quota_get(&client.session.account, "q");
        let responses = client.call(&[CORE, QUOTA], &[call]).await?;
        Ok(storage(responses.answer("q", "Quota/get")?)?)
    }
}
