//! A search as JMAP `Email/query` with a filter (RFC 8621 §4.4.1).
//!
//! JMAP's filter is a tree of `FilterOperator`s (`AND`, `OR`, `NOT`) over `FilterCondition`s, so
//! the query's own shape carries over whole. Each clause becomes the condition on the same field:
//! `from:` is `from`, `to:` is `to` or `cc`, a free word is `text` (From, To, Cc, Bcc, Subject and
//! the text body parts: the fields the store indexes, with Bcc beside them), `is:unread` is
//! `notKeyword: $seen`, a place is `inMailbox`. Dates are `after` and `before` on `receivedAt`,
//! the one date RFC 8621 filters on.
//!
//! Every query is also kept to the mailboxes this client follows, as a sync is: an email only in
//! Drafts or Junk is not held here, and one fetched for a search would be dropped by the next
//! sync as gone.

use super::{Asked, Unsaid, describe, each, kept_here, on_account, words};
use crate::jmap::{Call, Mailboxes};
use mail_domain::{AccountId, Filter, LabelId, ReadState, Star, TextMatch};
use serde_json::{Value, json};

/// What the translation needs to know about the account.
#[derive(Debug, Clone, Copy)]
pub struct JmapCtx<'a> {
    pub account: AccountId,
    pub mailboxes: &'a Mailboxes,
    /// Each label this client knows on the account, with its name: on JMAP, a mailbox's path.
    pub labels_named: &'a [(LabelId, String)],
}

/// `filter` as an `Email/query` filter for this account, or what of it cannot be asked.
pub fn translate(filter: &Filter, ctx: &JmapCtx<'_>) -> Result<Asked<Value>, Unsaid> {
    match on_account(filter, ctx.account) {
        Filter::Nothing => Ok(Asked::Nothing),
        filter => condition(&filter, ctx).map(Asked::Ask),
    }
}

fn operator(name: &str, conditions: Vec<Value>) -> Value {
    json!({ "operator": name, "conditions": conditions })
}

fn condition(filter: &Filter, ctx: &JmapCtx<'_>) -> Result<Value, Unsaid> {
    let whole = |m: &TextMatch| match m {
        TextMatch::Contains(s) => Ok(s.clone()),
        TextMatch::Exact(_) => Err(Unsaid::one(format!(
            "{} as a whole value",
            describe(filter)
        ))),
    };
    let inside = |id: Option<&str>, what: String| match id {
        Some(id) => Ok(json!({ "inMailbox": id })),
        None => Err(Unsaid::one(what)),
    };
    Ok(match filter {
        // An empty condition matches every email (RFC 8621 §4.4.1).
        Filter::All => json!({}),
        Filter::Nothing => operator("NOT", vec![json!({})]),
        Filter::And(parts) => operator("AND", each(parts, |p| condition(p, ctx))?),
        Filter::Or(parts) => operator("OR", each(parts, |p| condition(p, ctx))?),
        Filter::Not(inner) => operator("NOT", vec![condition(inner, ctx)?]),
        Filter::Read(ReadState::Read) => json!({ "hasKeyword": "$seen" }),
        Filter::Read(ReadState::Unread) => json!({ "notKeyword": "$seen" }),
        Filter::Starred(Star::Starred) => json!({ "hasKeyword": "$flagged" }),
        Filter::Starred(Star::Unstarred) => json!({ "notKeyword": "$flagged" }),
        Filter::From(m) => json!({ "from": whole(m)? }),
        Filter::To(m) => {
            let who = whole(m)?;
            operator("OR", vec![json!({ "to": who }), json!({ "cc": who })])
        }
        Filter::Subject(m) => json!({ "subject": whole(m)? }),
        Filter::Text(TextMatch::Contains(needle)) => operator(
            "AND",
            words(needle)
                .into_iter()
                .map(|w| json!({ "text": w }))
                .collect(),
        ),
        Filter::Text(TextMatch::Exact(phrase)) => json!({ "text": phrase }),
        Filter::HasAttachment => json!({ "hasAttachment": true }),
        Filter::Date(range) => {
            let mut out = serde_json::Map::new();
            let utc = |at: &chrono::DateTime<chrono::Utc>| {
                at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            };
            if let Some(from) = &range.from {
                out.insert("after".to_owned(), json!(utc(from)));
            }
            if let Some(to) = &range.to {
                out.insert("before".to_owned(), json!(utc(to)));
            }
            Value::Object(out)
        }
        Filter::InMailbox(role) => inside(
            ctx.mailboxes.id_for_role(*role),
            format!("{} (the server names no such mailbox)", describe(filter)),
        )?,
        Filter::InFolder(mailbox) => inside(
            ctx.mailboxes.id_for_path(&mailbox.path),
            format!("{} (the server no longer lists it)", describe(filter)),
        )?,
        Filter::HasLabel(label) => inside(
            super::named(ctx.labels_named, *label).and_then(|n| ctx.mailboxes.id_for_path(n)),
            "label: (the server lists no mailbox of that name)".to_owned(),
        )?,
        Filter::Account(_) | Filter::Snoozed | Filter::SnoozeDue | Filter::Pinned => {
            return Err(kept_here(filter));
        }
    })
}

/// `Email/query` for `filter`, newest first, at most `limit`, counting every match; kept, as a
/// sync is, to emails in some mailbox other than `unfollowed`.
pub fn query(account: &str, filter: Value, unfollowed: &[String], limit: u64, id: &str) -> Call {
    let filter = if unfollowed.is_empty() {
        filter
    } else {
        operator(
            "AND",
            vec![json!({ "inMailboxOtherThan": unfollowed }), filter],
        )
    };
    Call {
        name: "Email/query",
        args: json!({
            "accountId": account,
            "filter": filter,
            "sort": [{ "property": "receivedAt", "isAscending": false }],
            "position": 0,
            "limit": limit,
            "calculateTotal": true,
        }),
        id: id.to_owned(),
    }
}
