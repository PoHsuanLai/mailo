//! Rules and vacation replies. Local rows; the server's copy is a Sieve script built from them.

use super::SqliteStore;
use super::row::{from_time, json, to_json, uuid};
use crate::StoreError;
use chrono::{DateTime, Utc};
use mail_domain::{AccountId, Rule, RuleId, Vacation};
use rusqlite::{OptionalExtension, params};

const RULE_COLUMNS: &str = "id, account, name, position, state, filter, actions, after";

impl SqliteStore {
    pub(super) fn load_rules(&self, account: AccountId) -> Result<Vec<Rule>, StoreError> {
        // `name` in binary order, as the in-memory store sorts `String`s.
        let sql =
            format!("SELECT {RULE_COLUMNS} FROM rules WHERE account = ?1 ORDER BY position, name");
        let db = self.reader();
        let mut stmt = db.prepare_cached(&sql)?;
        let mut rows = stmt.query(params![account.to_string()])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(Rule {
                id: RuleId::from_uuid(uuid("RuleId", &row.get::<_, String>(0)?)?),
                account: AccountId::from_uuid(uuid("AccountId", &row.get::<_, String>(1)?)?),
                name: row.get(2)?,
                position: row.get(3)?,
                state: json("RuleState", &row.get::<_, String>(4)?)?,
                filter: json("Rule.filter", &row.get::<_, String>(5)?)?,
                actions: json("Rule.actions", &row.get::<_, String>(6)?)?,
                after: json("AfterMatch", &row.get::<_, String>(7)?)?,
            });
        }
        Ok(out)
    }

    pub(super) fn write_rule(&self, rule: &Rule) -> Result<(), StoreError> {
        let db = self.connection();
        // Asked rather than left to the UNIQUE constraint, so the refusal names the rule instead
        // of quoting SQLite.
        let taken: Option<String> = db
            .query_row(
                "SELECT id FROM rules WHERE account = ?1 AND name = ?2 AND id <> ?3",
                params![rule.account.to_string(), rule.name, rule.id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if taken.is_some() {
            return Err(StoreError::RuleNameTaken(rule.name.clone()));
        }
        db.execute(
            "INSERT INTO rules (id, account, name, position, state, filter, actions, after)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (id) DO UPDATE SET
                 account = excluded.account, name = excluded.name,
                 position = excluded.position, state = excluded.state,
                 filter = excluded.filter, actions = excluded.actions, after = excluded.after",
            params![
                rule.id.to_string(),
                rule.account.to_string(),
                rule.name,
                rule.position,
                to_json("RuleState", &rule.state)?,
                to_json("Rule.filter", &rule.filter)?,
                to_json("Rule.actions", &rule.actions)?,
                to_json("AfterMatch", &rule.after)?,
            ],
        )?;
        Ok(())
    }

    pub(super) fn remove_rule(&self, id: RuleId) -> Result<(), StoreError> {
        let gone = self
            .connection()
            .execute("DELETE FROM rules WHERE id = ?1", params![id.to_string()])?;
        if gone == 0 {
            return Err(StoreError::NoRule(id));
        }
        Ok(())
    }

    pub(super) fn load_vacation(&self, account: AccountId) -> Result<Option<Vacation>, StoreError> {
        let text: Option<String> = self
            .reader()
            .query_row(
                "SELECT vacation FROM vacations WHERE account = ?1",
                params![account.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        text.map(|t| json("Vacation", &t)).transpose()
    }

    pub(super) fn write_vacation(
        &self,
        account: AccountId,
        vacation: Option<&Vacation>,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let db = self.connection();
        match vacation {
            None => {
                db.execute(
                    "DELETE FROM vacations WHERE account = ?1",
                    params![account.to_string()],
                )?;
            }
            Some(vacation) => {
                db.execute(
                    "INSERT OR REPLACE INTO vacations (account, vacation, updated_at)
                     VALUES (?1, ?2, ?3)",
                    params![
                        account.to_string(),
                        to_json("Vacation", vacation)?,
                        from_time(now)
                    ],
                )?;
            }
        }
        Ok(())
    }
}
