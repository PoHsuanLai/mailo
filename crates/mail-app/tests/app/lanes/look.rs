//! Reading what the window shows and what it stored, for every lane: the list's rows, the
//! search panel's rows, an open menu's items, and the outbox's submissions.

use chrono::{DateTime, Utc};
use ds_harness::Harness;
use mail_core::{SqliteStore, Store};
use mail_domain::{
    DraftId, Filter, MailboxRole, PageReq, Property, ProtoOp, Query, Sort, SortDir, TextMatch,
    ThreadSummary,
};

use super::seed::account;

/// The composer's Send button.
pub fn send() -> String {
    format!(
        ".c-send [*|aria-label=\"{}\"]",
        mail_app::ui::copy::controls::SEND
    )
}

/// How many recipient chips `scope` holds.
pub fn chips(harness: &Harness, scope: &str) -> usize {
    harness.count(&format!("{scope} .ds-chip"))
}

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
/// mail or the places first, and the rows for typed text come once the search has run on it.
pub fn panel_settled(harness: &Harness) -> bool {
    let typed = harness
        .attr(".ds-search-card input", "value")
        .is_some_and(|text| !text.is_empty());
    let first = harness
        .text_of(".ds-search-card-rows .ds-menu > :nth-child(1)")
        .unwrap_or_default();
    !typed || !["Recent", "Places"].contains(&first.trim())
}

/// The position of the open menu's item named `name`.
pub fn menu_item_of(harness: &Harness, name: &str) -> Option<usize> {
    (1..=harness.count(".ds-menu-list > *")).find(|n| {
        harness
            .text_of(&format!(".ds-menu-list > :nth-child({n}) .ds-menu-label"))
            .is_some_and(|label| label.trim() == name)
    })
}

/// The position of the item named `name` in the submenu a pick of the open menu opened.
pub fn submenu_item_of(harness: &Harness, name: &str) -> Option<usize> {
    let list = SUBMENU_LIST;
    (1..=harness.count(&format!("{list} > *"))).find(|n| {
        harness
            .text_of(&format!("{list} > :nth-child({n}) .ds-menu-label"))
            .is_some_and(|label| label.trim() == name)
    })
}

/// A submenu's rows: quire draws each submenu as a menu panel one level deeper.
pub const SUBMENU_LIST: &str = ".ds-menu[*|data-depth=\"1\"] .ds-menu-list";

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

/// The first element `selector` matches, by its class and accessible name: what a size failure
/// says, rather than the whole document.
fn named(harness: &Harness, selector: &str) -> String {
    format!(
        "class={:?} aria-label={:?} text={:?}",
        harness.attr(selector, "class"),
        harness.attr(selector, "aria-label"),
        harness.text_of(selector)
    )
}

/// The button size rule, on what `harness` draws now (`screen` names it): nothing mailo sizes is
/// Mini (its list, reader and composer, and every button; quire's own parts, a Space tile's badge
/// among them, keep the sizes quire gives them); a Large button is a toolbar's; and a glyph on a
/// row, in the reader or in the composer is 14 px or larger, quire's 11 and 13 px rungs unused.
pub fn sizes_hold(harness: &Harness, screen: &str) {
    for mini in [
        ".list [*|data-size=\"mini\"]",
        ".reader [*|data-size=\"mini\"]",
        ".cpage [*|data-size=\"mini\"]",
        "button.ds-button[*|data-size=\"mini\"]",
    ] {
        assert_eq!(
            harness.count(mini),
            0,
            "{screen}: something is drawn at Mini: {}",
            named(harness, mini)
        );
    }
    let large = "button.ds-button[*|data-size=\"large\"]:not([*|data-variant=\"toolbar\"])";
    assert_eq!(
        harness.count(large),
        0,
        "{screen}: a Large button is not a toolbar's: {}",
        named(harness, large)
    );
    for place in [".list", ".reader", ".cpage"] {
        for side in ["11", "13"] {
            let retired = format!("{place} svg.ds-ic[*|data-size=\"{side}\"]");
            assert_eq!(
                harness.count(&retired),
                0,
                "{screen}: a {side} px glyph in {place}: {}",
                named(harness, &retired)
            );
        }
    }
}

/// [`sizes_hold`] on the open composer, and its bar (Attach, ⋯, Send and its ▾) Regular
/// throughout, with one filled button, Send.
pub fn composer_sizes_hold(harness: &Harness) {
    sizes_hold(harness, "the composer");
    let bar = ".cpage .c-head .bar-tools";
    let under = |tail: &str| {
        harness.count(&format!("{bar} > button.ds-button{tail}"))
            + harness.count(&format!("{bar} > * > button.ds-button{tail}"))
            + harness.count(&format!("{bar} > * > * > button.ds-button{tail}"))
    };
    let all = under("");
    let regular = under("[*|data-size=\"regular\"]");
    assert!(all >= 4, "the composer's bar has {all} buttons");
    assert_eq!(regular, all, "the composer's bar is not Regular throughout");
    assert_eq!(
        harness.count(&format!(
            "{bar} button.ds-button[*|data-answers=\"return\"]"
        )),
        1,
        "the composer's bar fills one button, Send"
    );
}
