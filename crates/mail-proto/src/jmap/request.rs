//! Method calls, and the request that carries them (RFC 8620 §3.3).
//!
//! Each builder returns one [`Call`] with the id the caller chose, so a later call in the same
//! request can name its result: `Email/get` given `#ids` from an `Email/query` fetches what the
//! query found without a second round trip (RFC 8620 §3.7).

use serde_json::{Map, Value, json};

/// One method call: its name, its arguments, and the id its response will carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub name: &'static str,
    pub args: Value,
    pub id: String,
}

/// Which records a `/get` or `/set` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ids {
    /// These, by id.
    Listed(Vec<String>),
    /// Every record of the type. Only `Mailbox/get` asks for this: a mailbox list is small, and
    /// an email list is not.
    All,
    /// Whatever an earlier call in the same request produced, at a JSON Pointer into its
    /// arguments: `/ids` of a query, `/updated` of a changes call.
    ResultOf {
        call: String,
        name: &'static str,
        path: &'static str,
    },
}

/// The request body: the capabilities it uses, and its calls in order.
pub fn request(using: &[&str], calls: &[Call]) -> Value {
    json!({
        "using": using,
        "methodCalls": calls
            .iter()
            .map(|c| json!([c.name, c.args, c.id]))
            .collect::<Vec<_>>(),
    })
}

/// `ids` placed in `args` under `key`, as a value or as a back-reference (`#key`).
fn with_ids(mut args: Map<String, Value>, key: &str, ids: Ids) -> Value {
    match ids {
        Ids::Listed(ids) => {
            args.insert(key.to_owned(), json!(ids));
        }
        Ids::All => {
            args.insert(key.to_owned(), Value::Null);
        }
        Ids::ResultOf { call, name, path } => {
            args.insert(
                format!("#{key}"),
                json!({ "resultOf": call, "name": name, "path": path }),
            );
        }
    }
    Value::Object(args)
}

fn base(account: &str) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert("accountId".to_owned(), json!(account));
    args
}

/// `Mailbox/get` for every mailbox, with the properties this client files by.
pub fn mailbox_get(account: &str, id: &str) -> Call {
    let mut args = base(account);
    args.insert(
        "properties".to_owned(),
        json!([
            "id",
            "name",
            "parentId",
            "role",
            "sortOrder",
            "isSubscribed"
        ]),
    );
    Call {
        name: "Mailbox/get",
        args: with_ids(args, "ids", Ids::All),
        id: id.to_owned(),
    }
}

/// `Mailbox/changes` since `state`.
pub fn mailbox_changes(account: &str, since: &str, id: &str) -> Call {
    let mut args = base(account);
    args.insert("sinceState".to_owned(), json!(since));
    Call {
        name: "Mailbox/changes",
        args: Value::Object(args),
        id: id.to_owned(),
    }
}

/// `Email/query`, newest first, for the emails this client follows: every email in at least one
/// mailbox that is not one of `unfollowed` (Drafts and Junk).
///
/// `limit: 0` with `calculate_total` is the cheap question "how many are there", which is how a
/// pass decides whether it is caught up without listing anything.
pub fn email_query(
    account: &str,
    unfollowed: &[String],
    position: u64,
    limit: u64,
    id: &str,
) -> Call {
    let mut args = base(account);
    if !unfollowed.is_empty() {
        args.insert(
            "filter".to_owned(),
            json!({ "inMailboxOtherThan": unfollowed }),
        );
    }
    args.insert(
        "sort".to_owned(),
        json!([{ "property": "receivedAt", "isAscending": false }]),
    );
    args.insert("position".to_owned(), json!(position));
    args.insert("limit".to_owned(), json!(limit));
    args.insert("calculateTotal".to_owned(), json!(true));
    Call {
        name: "Email/query",
        args: Value::Object(args),
        id: id.to_owned(),
    }
}

/// The count of followed emails alone: [`email_query`] with nothing listed.
pub fn total_query(account: &str, unfollowed: &[String], id: &str) -> Call {
    email_query(account, unfollowed, 0, 0, id)
}

/// `Email/get` of `properties` for `ids`.
pub fn email_get(account: &str, ids: Ids, properties: &[&str], id: &str) -> Call {
    let mut args = base(account);
    args.insert("properties".to_owned(), json!(properties));
    Call {
        name: "Email/get",
        args: with_ids(args, "ids", ids),
        id: id.to_owned(),
    }
}

/// `Email/get` of the blob ids only, for downloading bodies.
pub fn blob_ids(account: &str, ids: Vec<String>, id: &str) -> Call {
    email_get(account, Ids::Listed(ids), &["id", "blobId", "size"], id)
}

/// `Email/changes` since `state`, at most `max` per call.
pub fn email_changes(account: &str, since: &str, max: u64, id: &str) -> Call {
    let mut args = base(account);
    args.insert("sinceState".to_owned(), json!(since));
    args.insert("maxChanges".to_owned(), json!(max));
    Call {
        name: "Email/changes",
        args: Value::Object(args),
        id: id.to_owned(),
    }
}

/// `Email/set`: `update` is each email id with its patch, `destroy` the ids to delete for good.
pub fn email_set(
    account: &str,
    update: Vec<(String, Map<String, Value>)>,
    destroy: Vec<String>,
    id: &str,
) -> Call {
    let mut args = base(account);
    if !update.is_empty() {
        let map: Map<String, Value> = update
            .into_iter()
            .map(|(email, patch)| (email, Value::Object(patch)))
            .collect();
        args.insert("update".to_owned(), Value::Object(map));
    }
    if !destroy.is_empty() {
        args.insert("destroy".to_owned(), json!(destroy));
    }
    Call {
        name: "Email/set",
        args: Value::Object(args),
        id: id.to_owned(),
    }
}

/// `Mailbox/set`, with the create, update and destroy arguments already built.
pub fn mailbox_set(account: &str, work: Map<String, Value>, id: &str) -> Call {
    let mut args = base(account);
    args.extend(work);
    Call {
        name: "Mailbox/set",
        args: Value::Object(args),
        id: id.to_owned(),
    }
}

/// `Identity/get`: every address this account may send as.
pub fn identity_get(account: &str, id: &str) -> Call {
    let mut args = base(account);
    args.insert("ids".to_owned(), Value::Null);
    Call {
        name: "Identity/get",
        args: Value::Object(args),
        id: id.to_owned(),
    }
}

/// `Email/import` of one uploaded message into `mailbox`, with `keywords` and, where given, the
/// date it was received — so imported mail sorts where it belongs rather than as today's.
pub fn email_import(
    account: &str,
    blob: &str,
    mailbox: &str,
    keywords: &[&str],
    received_at: Option<chrono::DateTime<chrono::Utc>>,
    id: &str,
) -> Call {
    let mut mailbox_ids = Map::new();
    mailbox_ids.insert(mailbox.to_owned(), json!(true));
    let keywords: Map<String, Value> = keywords
        .iter()
        .map(|k| ((*k).to_owned(), json!(true)))
        .collect();
    let mut email = Map::new();
    email.insert("blobId".to_owned(), json!(blob));
    email.insert("mailboxIds".to_owned(), Value::Object(mailbox_ids));
    email.insert("keywords".to_owned(), Value::Object(keywords));
    if let Some(at) = received_at {
        email.insert(
            "receivedAt".to_owned(),
            json!(at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        );
    }
    let mut args = base(account);
    args.insert("emails".to_owned(), json!({ "import": email }));
    Call {
        name: "Email/import",
        args: Value::Object(args),
        id: id.to_owned(),
    }
}
