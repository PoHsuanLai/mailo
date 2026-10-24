//! The messages of the conversations a filter lists, each on its own: the list with Organise by
//! Conversation off.
//!
//! The filter is compiled as it is for [`crate::Store::threads`] and answered of each message's
//! conversation, so the store says which conversations are listed and nothing new about a
//! filter. Which of their messages a list of single messages shows is the caller's decision
//! (`mail_core::single`), made with `Filter::fit` on each message alone; this only reads them,
//! ordered as the message itself sorts.

use super::SqliteStore;
use super::read::MESSAGE_COLUMNS;
use crate::{StoreError, sql};
use chrono::{DateTime, Utc};
use mail_domain::{Cursor, Message, MessageId, Page, Property, Query, SortDir, ThreadSummary};
use std::collections::HashMap;

/// One message of a listed conversation, with the conversation as the store sums it up, and
/// where a page that ends on it resumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRow {
    pub message: Message,
    pub conversation: ThreadSummary,
    /// The keyset position of this row: a [`Query`] whose page starts after it lists what
    /// follows it.
    pub at: Cursor,
}

/// The `messages` column a [`Property`] sorts a message on. Size, files and pin are not a
/// message's own to sort by, and fall back to its date, as `sort_column` does for size.
fn message_column(property: Property) -> &'static str {
    match property {
        Property::Date | Property::Size | Property::Attachments | Property::Pin => "m.date",
        Property::Subject => "m.subject",
        Property::From | Property::Sender => "m.from_email",
    }
}

/// `MESSAGE_COLUMNS`, each qualified with `m.`, in their order, so a row reads as
/// `read_message` expects.
fn qualified_message_columns() -> String {
    MESSAGE_COLUMNS
        .split(',')
        .map(|column| format!("m.{}", column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn encode(sort_value: &str, message: MessageId) -> Cursor {
    Cursor(format!("{sort_value}\u{1f}{message}"))
}

fn decode(cursor: &Cursor) -> Result<(String, String), StoreError> {
    cursor
        .0
        .split_once('\u{1f}')
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .ok_or(StoreError::BadCursor)
}

impl SqliteStore {
    /// A page of the messages whose conversations `query.filter` lists, ordered by
    /// `query.sort` read off each message: its date, its subject or its sender. Keyset on the
    /// message's id, as [`crate::Store::threads`] is on the thread's, so two messages sent in
    /// the same second never swap places between pages.
    pub fn messages_listed(
        &self,
        query: &Query,
        now: DateTime<Utc>,
    ) -> Result<Page<MessageRow>, StoreError> {
        let compiled = sql::compile(&query.filter, now);
        let column = message_column(query.sort.property);
        let dir = match query.sort.dir {
            SortDir::Asc => "ASC",
            SortDir::Desc => "DESC",
        };
        let (keyset, mut after) = match &query.page.after {
            None => (String::from("1=1"), Vec::new()),
            Some(cursor) => {
                let (value, message) = decode(cursor)?;
                let cmp = match query.sort.dir {
                    SortDir::Asc => ">",
                    SortDir::Desc => "<",
                };
                (
                    format!("({column}, m.id) {cmp} (?, ?)"),
                    vec![sql::SqlValue::Text(value), sql::SqlValue::Text(message)],
                )
            }
        };
        let sql_text = format!(
            "SELECT {}, {column} AS sort_key FROM messages m \
             JOIN thread_summary ts ON ts.thread = m.thread \
             WHERE {} AND {keyset} ORDER BY {column} {dir}, m.id {dir} LIMIT ?",
            qualified_message_columns(),
            compiled.where_clause,
        );
        let mut bound = compiled.params;
        bound.append(&mut after);
        // One extra row says whether another page follows.
        let limit = i64::from(query.page.limit);
        bound.push(sql::SqlValue::Int(limit + 1));

        let db = self.reader();
        let mut stmt = db.prepare_cached(&sql_text)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(bound.iter().map(|v| match v {
            sql::SqlValue::Text(t) => rusqlite::types::Value::Text(t.clone()),
            sql::SqlValue::Int(i) => rusqlite::types::Value::Integer(*i),
        })))?;
        let mut read = Vec::new();
        while let Some(row) = rows.next()? {
            let message = self.read_message(&db, row)?;
            let key = row.get::<_, String>("sort_key")?;
            read.push((message, key));
        }
        drop(rows);
        drop(stmt);

        // A conversation with several messages on the page is summed up once.
        let mut conversations: HashMap<_, ThreadSummary> = HashMap::new();
        let mut items = Vec::with_capacity(read.len());
        for (message, key) in read {
            let conversation = match conversations.get(&message.thread) {
                Some(known) => known.clone(),
                None => {
                    let summary = self.summary_of(&db, message.thread)?;
                    conversations.insert(message.thread, summary.clone());
                    summary
                }
            };
            items.push(MessageRow {
                at: encode(&key, message.id),
                message,
                conversation,
            });
        }
        let next = if items.len() as i64 > limit {
            items.truncate(limit as usize);
            items.last().map(|row| row.at.clone())
        } else {
            None
        };
        Ok(Page { items, next })
    }
}
