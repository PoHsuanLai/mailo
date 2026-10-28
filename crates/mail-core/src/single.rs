//! A list of messages rather than conversations: Organise by Conversation turned off for a
//! place or a saved view ([`mail_domain::Threading::Single`]).
//!
//! Each message is a row of its own, with its own sender, subject and date. Which messages a
//! place lists this way is one rule, decided here and not in the window: the messages of the
//! conversations the place lists ([`mail_store::SqliteStore::messages_listed`]) that the place's
//! filter also fits on their own ([`alone`], [`fits`]). So the inbox lists the messages in the
//! inbox and not the replies sent from it, and an unread view the unread messages and not the
//! read ones beside them, by the same filter, compiled the same way, as the conversations it
//! lists otherwise.
//!
//! A row is still the conversation's to act on: its summary carries the conversation's id, so
//! opening, picking and every action reach the conversation as they do from its own row. Which
//! message the row stands for travels beside it ([`Single::message`]), and the reader shows that
//! one alone.

use crate::error::CoreError;
use chrono::{DateTime, Utc};
use mail_domain::{
    Filter, MatchCtx, Message, MessageId, Page, PageReq, Placed, Query, ThreadSummary,
};
use mail_store::{MessageRow, SqliteStore, Store};

/// One message, as a list of single messages shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Single {
    pub message: MessageId,
    /// The message as a conversation of one ([`alone`]), under its conversation's id.
    pub summary: ThreadSummary,
}

/// `message` summed up as though it were its conversation's only message: its own sender,
/// subject, date, snippet, flags, place and files, under `conversation`'s id. The snooze, pin,
/// mute and reminder are the conversation's, because those are set on a conversation and never
/// on one of its messages.
pub fn alone(message: &Message, conversation: &ThreadSummary) -> ThreadSummary {
    ThreadSummary::derive(
        conversation.id,
        std::slice::from_ref(message),
        conversation.snooze,
        conversation.pin,
        conversation.mute,
        conversation.follow_up,
    )
}

/// Whether `filter` lists `message` on its own: [`Filter::fit`] asked of the message as a
/// conversation of one (`summary`, from [`alone`]), with its own body for words and its own
/// server addresses (`folders`) for a folder, and the VIPs' addresses (`vips`) for `is:vip`.
pub fn fits(
    filter: &Filter,
    summary: &ThreadSummary,
    message: &Message,
    folders: &[Placed],
    vips: &[String],
    now: DateTime<Utc>,
) -> bool {
    filter.fit(&MatchCtx {
        summary,
        corpus: message.body.text(),
        folders,
        sizes: &[mail_domain::filter::message_size(message)],
        cc: &message.cc,
        vips,
        now,
    })
}

/// Whether `filter` names a folder anywhere, so that a message's server addresses are needed
/// to answer it. Read only then: they are a query each.
fn names_folder(filter: &Filter) -> bool {
    match filter {
        Filter::InFolder(_) => true,
        Filter::And(parts) | Filter::Or(parts) => parts.iter().any(names_folder),
        Filter::Not(inner) => names_folder(inner),
        _ => false,
    }
}

/// A page of the messages `query` lists one to a row, ordered by `query.sort` read off each
/// message. The store reads the messages of the listed conversations; those the filter does not
/// fit on their own are passed over, and the store asked again past them, until the page is
/// full or nothing is left.
pub fn listed(
    store: &SqliteStore,
    query: &Query,
    now: DateTime<Utc>,
) -> Result<Page<Single>, CoreError> {
    let limit = query.page.limit;
    let folders = names_folder(&query.filter);
    let vips: Vec<String> = store.vips()?.into_iter().map(|vip| vip.address).collect();
    let mut after = query.page.after.clone();
    let mut items: Vec<Single> = Vec::new();
    loop {
        let asked = Query {
            filter: query.filter.clone(),
            sort: query.sort,
            page: PageReq {
                after: after.clone(),
                limit: limit.saturating_add(1),
            },
        };
        let page = store.messages_listed(&asked, now)?;
        for MessageRow {
            message,
            conversation,
            at,
        } in page.items
        {
            let summary = alone(&message, &conversation);
            let placed = if folders {
                store.placed(message.id)?
            } else {
                Vec::new()
            };
            if !fits(&query.filter, &summary, &message, &placed, &vips, now) {
                after = Some(at);
                continue;
            }
            if items.len() == limit as usize {
                // One more fits: the page ends on the row before it.
                return Ok(Page { items, next: after });
            }
            items.push(Single {
                message: message.id,
                summary,
            });
            after = Some(at);
        }
        if page.next.is_none() {
            return Ok(Page { items, next: None });
        }
    }
}

#[cfg(test)]
#[path = "single_tests.rs"]
mod tests;
