//! Reading what the window shows and what it stored, for every lane: the list's rows, the
//! search panel's rows, an open menu's items, and the outbox's submissions.

use chrono::{DateTime, Utc};
use ds_harness::{Harness, Query as Read};
use mail_core::{SqliteStore, Store};
use mail_domain::{
    DraftId, Filter, MailboxRole, PageReq, Property, ProtoOp, Query, Sort, SortDir, TextMatch,
    ThreadSummary,
};

use super::seed::account;

/// The `n`th row of the list (1-based).
pub fn row(n: usize) -> String {
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"] .ds-thread")
}

/// The position of the row whose subject is `subject`.
pub fn row_of(harness: &Harness, subject: &str) -> Option<usize> {
    (1..=harness.count(".list .ds-list-item")).find(|n| {
        harness
            .text_of(&format!("{} .ds-thread-sub", row(*n)))
            .is_some_and(|text| text.trim() == subject)
    })
}

/// The position of the search panel's row that names `text`.
pub fn panel_row(harness: &Harness, text: &str) -> Option<usize> {
    (1..=harness.count(".ds-search-card-rows .ds-menu > *")).find(|n| {
        harness
            .text_of(&format!(".ds-search-card-rows .ds-menu > :nth-child({n})"))
            .is_some_and(|row| row.contains(text))
    })
}

/// Whether the search panel's rows answer what its field holds: an empty field lists Recent
/// mail first, and the rows for typed text come once the search has run on it.
pub fn panel_settled(harness: &Harness) -> bool {
    let typed = harness
        .attr(".ds-search-card input", "value")
        .is_some_and(|text| !text.is_empty());
    let first = harness
        .text_of(".ds-search-card-rows .ds-menu > :nth-child(1)")
        .unwrap_or_default();
    !typed || first != "Recent"
}

/// The position of the open menu's item named `name`.
pub fn menu_item_of(harness: &Harness, name: &str) -> Option<usize> {
    (1..=harness.count(".ds-menu > *")).find(|n| {
        harness
            .text_of(&format!(".ds-menu > :nth-child({n}) .ds-menu-label"))
            .is_some_and(|label| label.trim() == name)
    })
}

/// One submission waiting in the outbox: its draft, recipients and bytes.
pub struct Queued {
    pub draft: DraftId,
    pub rcpt_to: Vec<String>,
    pub raw: String,
}

/// Every submission in the outbox, due or not.
pub fn queued(store: &SqliteStore) -> Vec<Queued> {
    let far = chrono::Utc::now() + chrono::Duration::days(3650);
    store
        .outbox_due(account(), far)
        .expect("the outbox reads")
        .into_iter()
        .filter_map(|entry| match entry.op {
            ProtoOp::Submit {
                draft,
                raw,
                rcpt_to,
                ..
            } => {
                let bytes = store.blobs().get(raw).expect("the submission's bytes");
                Some(Queued {
                    draft,
                    rcpt_to,
                    raw: String::from_utf8_lossy(&bytes).into_owned(),
                })
            }
            _ => None,
        })
        .collect()
}

/// `raw` read as mail is read: its text, its HTML and its attachments, decoded.
pub fn parsed(raw: &str) -> mail_mime::Parsed {
    mail_mime::parse(raw.as_bytes()).expect("the submission parses")
}

/// The conversations `filter` lists at `at`, newest first.
pub fn listed(store: &SqliteStore, filter: Filter, at: DateTime<Utc>) -> Vec<ThreadSummary> {
    let query = Query {
        filter,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 50,
        },
    };
    store.threads(&query, at).expect("the store lists").items
}

/// The conversation whose subject is `subject`, as stored now.
pub fn conversation(store: &SqliteStore, subject: &str) -> ThreadSummary {
    let found = listed(
        store,
        Filter::Subject(TextMatch::Contains(subject.to_owned())),
        Utc::now(),
    );
    found
        .into_iter()
        .find(|thread| thread.subject == subject)
        .unwrap_or_else(|| panic!("no conversation is called {subject:?}"))
}

/// The subjects the Inbox lists at `at`, as the window and the CLI ask for it.
pub fn inbox_at(store: &SqliteStore, at: DateTime<Utc>) -> Vec<String> {
    listed(
        store,
        mail_core::place::place_filter(MailboxRole::Inbox),
        at,
    )
    .into_iter()
    .map(|thread| thread.subject)
    .collect()
}
