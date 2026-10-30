//! How much of an IMAP account's storage is used, from its engine: the backend's quota walk
//! ([`mail_proto::backend::Quota`]) on a connection of its own, signed in as every walk is.

use super::AccountEngine;
use mail_domain::{Retry, Retryable};
use mail_proto::Storage;
use mail_proto::backend::ImapBackend;
use mail_runtime::error::refused_fresh;
use mail_runtime::{Cancel, RuntimeError, drive};
use mail_secrets::tokens::{AfterRefusal, Token};

impl AccountEngine<ImapBackend> {
    /// The account's storage as the server counts it: `None` when the server offers no quota.
    ///
    /// With a fresh token, and once more with a new one if the server refuses it, as a search is
    /// run.
    pub async fn quota_imap(
        &mut self,
        cancel: &mut Cancel,
    ) -> Result<Option<Storage>, RuntimeError> {
        if let Some(tokens) = &self.tokens {
            tokens.ahead(Token::Incoming).await?;
        }
        let first = self.quota_once(cancel).await;
        let refused = matches!(&first, Err(e) if matches!(e.retry(), Retry::NeedsReauth));
        let Some(tokens) = self.tokens.as_ref().filter(|_| refused) else {
            return first;
        };
        match tokens.after_refusal(Token::Incoming).await? {
            AfterRefusal::TryAgain => self.quota_once(cancel).await.map_err(refused_fresh),
            AfterRefusal::StillRefused => first.map_err(refused_fresh),
            AfterRefusal::NotRenewable => first,
        }
    }

    async fn quota_once(&mut self, cancel: &mut Cancel) -> Result<Option<Storage>, RuntimeError> {
        let mut transport = self.connect().await?;
        let mut walk = self.backend.quota();
        drive(&mut walk, &mut transport, cancel).await
    }
}
