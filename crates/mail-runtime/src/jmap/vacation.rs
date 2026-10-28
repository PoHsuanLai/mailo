//! A JMAP account's vacation reply, set on its server: `VacationResponse/set` (RFC 8621 §8),
//! where the session says the account has one.

use super::JmapEngine;
use crate::RuntimeError;
use mail_domain::Vacation;
use mail_proto::ProtoError;
use mail_proto::jmap::{CORE, MAIL, VACATION, Vacations, vacation_set, vacation_set_done};

impl JmapEngine {
    /// Make the account's reply on the server `vacation`, or switch it off for `None`.
    ///
    /// A server whose session lists no vacation reply for the account is refused before anything
    /// is sent: it would only answer `unknownMethod`, and the person should hear why.
    pub async fn set_vacation(&mut self, vacation: Option<&Vacation>) -> Result<(), RuntimeError> {
        let result = self.vacation_set(vacation).await;
        if let Err(e) = &result {
            self.forget_if_stale(e);
        }
        result
    }

    async fn vacation_set(&mut self, vacation: Option<&Vacation>) -> Result<(), RuntimeError> {
        let client = self.client().await?.clone();
        if client.session.vacation == Vacations::Absent {
            return Err(ProtoError::Unsupported(
                "the JMAP server keeps no vacation reply for this account".to_owned(),
            )
            .into());
        }
        let call = vacation_set(&client.session.account, vacation, "v");
        let responses = client.call(&[CORE, MAIL, VACATION], &[call]).await?;
        Ok(vacation_set_done(
            responses.answer("v", "VacationResponse/set")?,
        )?)
    }
}
