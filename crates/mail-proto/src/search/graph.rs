//! A search as Microsoft Graph asks for one: `$search` (KQL) or `$filter` on a folder's or the
//! mailbox's messages.
//!
//! Graph will not take both at once on messages, nor `$orderby` with `$search`, so a query is one
//! or the other. `$filter` holds exactly what it compares — read, flagged, has attachments, dates
//! to the second — and is used whenever the query is only those. Anything with words goes to
//! `$search`, whose KQL has `from:`, `to:`, `cc:`, `subject:`, `participants:`, `hasAttachments:`
//! and `sent` dates, and no read or flag state: such a query with words in it is [`Unsaid`].
//!
//! A free word is searched as itself (Graph's default fields: from, subject, body) or as a
//! participant, the fields the store's index holds. `sent` dates in KQL are whole days.
//!
//! Written from Graph's description of `$search` and `$filter` on messages; like
//! `permanentDelete` (FINDINGS F189), it could not be run against a tenant from where it was
//! built.

use super::{Asked, Unsaid, describe, each, kept_here, on_account, place_of, whole_days, words};
use mail_domain::{AccountId, DateRange, Filter, MailboxRole, ReadState, Star, TextMatch};

/// Where Graph is asked: a folder, or the whole mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphPlace {
    /// `/me/messages`: every folder.
    Everywhere,
    /// The folder serving a role, by Graph's well-known name.
    Role(MailboxRole),
    /// A folder by this client's path for it.
    Folder(String),
}

/// What Graph is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphQuery {
    /// No condition: the newest messages there are.
    All,
    /// `$search`, KQL, without the surrounding quotes.
    Search(String),
    /// `$filter`, OData.
    Filter(String),
}

/// A Graph search: where, and what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphPlan {
    pub place: GraphPlace,
    pub query: GraphQuery,
}

/// `filter` as a Graph search of this account, or what of it cannot be asked.
pub fn translate(filter: &Filter, account: AccountId) -> Result<Asked<GraphPlan>, Unsaid> {
    let filter = on_account(filter, account);
    if filter == Filter::Nothing {
        return Ok(Asked::Nothing);
    }
    let (place, rest) = place_of(&filter)?;
    let place = match place {
        None => GraphPlace::Everywhere,
        Some(super::Place::Role(role)) => GraphPlace::Role(role),
        Some(super::Place::Folder(path)) => GraphPlace::Folder(path),
    };
    let query = match rest.as_slice() {
        [] => GraphQuery::All,
        _ => {
            let whole = Filter::And(rest);
            match odata(&whole) {
                Ok(text) => GraphQuery::Filter(text),
                Err(_) => GraphQuery::Search(kql(&whole)?),
            }
        }
    };
    Ok(Asked::Ask(GraphPlan { place, query }))
}

/// A KQL value: always quoted, so a word is never read as an operator.
fn value(text: &str) -> Result<String, Unsaid> {
    if text.contains('"') || text.contains('\\') {
        return Err(Unsaid::one(format!(
            "{text} (KQL cannot quote a quotation mark)"
        )));
    }
    Ok(format!("\"{text}\""))
}

fn joined(parts: Vec<String>, with: &str) -> String {
    match <[String; 1]>::try_from(parts) {
        Ok([one]) => one,
        Err(parts) => format!("({})", parts.join(&format!(" {with} "))),
    }
}

fn kql(filter: &Filter) -> Result<String, Unsaid> {
    let whole = |m: &TextMatch| match m {
        TextMatch::Contains(s) => value(s),
        TextMatch::Exact(_) => Err(Unsaid::one(format!(
            "{} as a whole value",
            describe(filter)
        ))),
    };
    let anywhere = |w: &str| -> Result<String, Unsaid> {
        let v = value(w)?;
        Ok(format!("({v} OR participants:{v})"))
    };
    Ok(match filter {
        Filter::All => return Err(Unsaid::one("everything".to_owned())),
        Filter::And(parts) => joined(each(parts, kql)?, "AND"),
        Filter::Or(parts) => joined(each(parts, kql)?, "OR"),
        Filter::Not(inner) => format!("NOT {}", kql(inner)?),
        Filter::From(m) => format!("from:{}", whole(m)?),
        Filter::To(m) => {
            let who = whole(m)?;
            format!("(to:{who} OR cc:{who})")
        }
        Filter::Subject(m) => format!("subject:{}", whole(m)?),
        Filter::Text(TextMatch::Contains(needle)) => joined(
            words(needle)
                .iter()
                .map(|w| anywhere(w))
                .collect::<Result<_, _>>()?,
            "AND",
        ),
        Filter::Text(TextMatch::Exact(phrase)) => anywhere(phrase)?,
        Filter::HasAttachment => "hasAttachments:true".to_owned(),
        Filter::Date(range) => {
            let (since, before) = whole_days(range);
            let mut parts = Vec::new();
            parts.extend(since.map(|d| format!("sent>={}", d.format("%Y-%m-%d"))));
            parts.extend(before.map(|d| format!("sent<{}", d.format("%Y-%m-%d"))));
            if parts.is_empty() {
                return Err(Unsaid::one("any date".to_owned()));
            }
            joined(parts, "AND")
        }
        Filter::Read(_) | Filter::Starred(_) => {
            return Err(Unsaid::one(format!(
                "{} together with words (Graph cannot filter a search by it)",
                describe(filter)
            )));
        }
        Filter::HasLabel(_) => {
            return Err(Unsaid::one(
                "label: (labels stay on this computer for Microsoft accounts)".to_owned(),
            ));
        }
        Filter::InMailbox(_) | Filter::InFolder(_) => {
            return Err(Unsaid::one(format!(
                "{} inside another clause",
                describe(filter)
            )));
        }
        Filter::Nothing
        | Filter::Account(_)
        | Filter::Snoozed
        | Filter::SnoozeDue
        | Filter::Pinned => return Err(kept_here(filter)),
    })
}

/// `filter` as OData, where it is only what `$filter` compares.
fn odata(filter: &Filter) -> Result<String, Unsaid> {
    let instant =
        |at: &chrono::DateTime<chrono::Utc>| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    Ok(match filter {
        Filter::And(parts) => joined(each(parts, odata)?, "and"),
        Filter::Or(parts) => joined(each(parts, odata)?, "or"),
        Filter::Not(inner) => format!("not {}", odata(inner)?),
        Filter::Read(ReadState::Read) => "isRead eq true".to_owned(),
        Filter::Read(ReadState::Unread) => "isRead eq false".to_owned(),
        Filter::Starred(Star::Starred) => "flag/flagStatus eq 'flagged'".to_owned(),
        Filter::Starred(Star::Unstarred) => "flag/flagStatus ne 'flagged'".to_owned(),
        Filter::HasAttachment => "hasAttachments eq true".to_owned(),
        Filter::Date(DateRange { from, to }) => {
            let mut parts = Vec::new();
            parts.extend(
                from.iter()
                    .map(|f| format!("sentDateTime ge {}", instant(f))),
            );
            parts.extend(to.iter().map(|t| format!("sentDateTime lt {}", instant(t))));
            if parts.is_empty() {
                return Err(Unsaid::one("any date".to_owned()));
            }
            joined(parts, "and")
        }
        other => return Err(Unsaid::one(describe(other))),
    })
}
