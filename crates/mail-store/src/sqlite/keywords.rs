//! Labels kept as IMAP keywords, on an account with [`ServerLabels::Keywords`].
//!
//! A label is matched to a keyword by the keyword its name makes ([`mail_domain::label_keyword`]),
//! never by spelling: `Project X` is written as `Project_X`, and reading `Project_X` back must find
//! `Project X` rather than make a second label. A label whose name makes no keyword stays local,
//! and is neither written nor taken away by what the server reports.

use super::SqliteStore;
use super::row::uuid;
use crate::StoreError;
use mail_domain::{
    Change, Label, LabelId, LabelOrigin, Patch, RemoteIntent, ServerLabels, THUNDERBIRD_TAGS,
    label_keyword,
};
use porter_core::AccountId;
use rusqlite::params;

/// The account's labels that have a keyword, each with it.
pub(super) type Keyed = Vec<(LabelId, String)>;

impl SqliteStore {
    /// Whether `account` keeps its labels as keywords, as last observed.
    pub(super) fn labels_as_keywords(&self, account: AccountId) -> Result<bool, StoreError> {
        Ok(self
            .account_caps(account)?
            .is_some_and(|caps| caps.labels == ServerLabels::Keywords))
    }

    /// Every label of `account` whose name makes a keyword, with that keyword.
    pub(super) fn keyed_labels(&self, account: AccountId) -> Result<Keyed, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare_cached("SELECT id, name FROM labels WHERE account = ?1")?;
        let rows = stmt
            .query_map(params![account.to_string()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::new();
        for (id, name) in rows {
            if let Some(keyword) = label_keyword(&name) {
                out.push((LabelId::from_uuid(uuid("LabelId", &id)?), keyword));
            }
        }
        Ok(out)
    }

    /// The label the keyword-born name `name` stands for: the one whose keyword is the same,
    /// compared as IMAP compares keywords, or a new one of the person's own, in Thunderbird's
    /// colour where it is one of Thunderbird's tags. `keyed` learns a label made here.
    pub(super) fn label_by_keyword(
        &self,
        account: AccountId,
        name: &str,
        keyed: &mut Keyed,
    ) -> Result<Option<LabelId>, StoreError> {
        let Some(keyword) = label_keyword(name) else {
            return Ok(None);
        };
        if let Some((id, _)) = keyed
            .iter()
            .find(|(_, held)| held.eq_ignore_ascii_case(&keyword))
        {
            return Ok(Some(*id));
        }
        let color = THUNDERBIRD_TAGS
            .iter()
            .find(|(_, tag, _)| *tag == name)
            .map(|(_, _, hue)| hue.as_str().to_owned());
        let label = Label {
            id: LabelId::generate(),
            account,
            name: name.to_owned(),
            color,
            origin: LabelOrigin::User,
        };
        self.write_change(&Change::LabelUpsert(label.clone()))?;
        keyed.push((label.id, keyword));
        Ok(Some(label.id))
    }

    /// Queue every message's keyword-able labels for the server, once, when an account starts
    /// keeping labels as keywords.
    ///
    /// Before then they were this computer's alone, so the server's messages carry none of
    /// them, and the first survey would read that as every label taken away elsewhere. Queued,
    /// each membership is pending until the server has it, and the survey puts it back.
    pub(super) fn push_keyword_labels(
        &self,
        account: AccountId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StoreError> {
        for (label, _) in self.keyed_labels(account.clone())? {
            let messages = {
                let db = self.connection();
                let mut stmt =
                    db.prepare_cached("SELECT message FROM message_labels WHERE label = ?1")?;
                let rows = stmt
                    .query_map(params![label.to_string()], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                rows.iter()
                    .map(|id| uuid("MessageId", id).map(mail_domain::MessageId::from_uuid))
                    .collect::<Result<Vec<_>, _>>()?
            };
            if messages.is_empty() {
                continue;
            }
            let intent = RemoteIntent::SetLabels {
                messages,
                add: vec![label],
                remove: Vec::new(),
            };
            // Nothing to undo: the labels were already on the messages here.
            let nothing = Patch {
                id: mail_domain::ChangeId::generate(),
                changes: Vec::new(),
            };
            self.queue(account.clone(), intent, &nothing, now)?;
        }
        Ok(())
    }
}
