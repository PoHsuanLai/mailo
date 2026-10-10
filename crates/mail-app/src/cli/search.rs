//! `mailo search`: the top results, marked, then every other match, newest first.
//!
//! The same [`mail_core::search::search_list`] the window's list box runs, so `from:ada`, a prefix
//! and a phrase cannot mean one thing here and another there, and the top results are the
//! strip the window draws above its rows.

use super::render_list;
use chrono::{DateTime, Utc};
use mail_core::SqliteStore;
use mail_domain::{LabelId, ThreadSummary};

/// The column a top result is marked in. Every other line has as many spaces there, so the
/// rows stay aligned.
const TOP: &str = "top ";
const NOT_TOP: &str = "    ";

/// Search every account for `needle`, at most `limit` rows under the top results.
///
/// `label` resolves `label:`, which is the one operator that needs the store. An invalid
/// `re:/pattern/` is the `regex` crate's own message, printed rather than failed on.
pub(super) fn search(
    store: &SqliteStore,
    needle: &str,
    limit: u32,
    label: &dyn Fn(&str) -> Vec<LabelId>,
    now: DateTime<Utc>,
) -> String {
    let take = usize::try_from(limit).unwrap_or(usize::MAX);
    // Asked for the top results' worth more, so removing them still leaves `limit` rows.
    let page = mail_core::search::first(take.saturating_add(mail_core::search::STRIP));
    let searched = match mail_core::search::search_list(
        needle,
        store,
        &mail_core::search::Affinity::default(),
        &chrono::Local,
        label,
        page,
        now,
    ) {
        Ok(searched) => searched,
        Err(message) => return format!("{message}\n"),
    };
    let top: Vec<ThreadSummary> = searched
        .top
        .into_iter()
        .map(|(summary, _)| summary)
        .collect();
    let rest: Vec<ThreadSummary> = searched
        .rows
        .into_iter()
        .filter(|row| !top.iter().any(|hit| hit.id == row.id))
        .take(take)
        .collect();
    if top.is_empty() && rest.is_empty() {
        return format!("nothing matches {needle:?}\n");
    }
    let mut out = String::new();
    for line in render_list(&top).lines() {
        out.push_str(TOP);
        out.push_str(line);
        out.push('\n');
    }
    for line in render_list(&rest).lines() {
        out.push_str(NOT_TOP);
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};
    use mail_core::search::Affinity;
    use mail_domain::*;
    use mail_store::{SqliteStore, Store};
    use porter_core::AccountId;

    fn acct_account() -> AccountId {
        mail_domain::id::account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
    }

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0)
            .single()
            .expect("fixture timestamp")
    }

    fn thread_of(n: u128) -> ThreadId {
        ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n))
    }

    fn message(n: u128, subject: &str, body: &str, secs: i64) -> Message {
        Message {
            id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n)),
            thread: thread_of(n),
            account: acct_account(),
            key: MessageKey::Rfc(format!("m{n}@b.c")),
            date: at(secs),
            from: Address {
                name: None,
                email: "a@b.c".to_owned(),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: subject.to_owned(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some(format!("m{n}@b.c")),
            read: ReadState::Read,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            body: Body::Present {
                text: Some(body.to_owned()),
                raw: BlobId::from_uuid(uuid::Uuid::from_u128(0xB000 + n)),
            },
            attachments: vec![],
        }
    }

    /// A sqlite store with one account, so `mailo search` and the window's pipeline see the same rows.
    fn sqlite_with(rows: &[(&str, &str, i64)]) -> (SqliteStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SqliteStore::in_memory(dir.path()).expect("sqlite");
        mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
        for (i, (subject, body, secs)) in rows.iter().enumerate() {
            let raw = store.blobs().put(body.as_bytes()).expect("blob");
            let mut message = message(i as u128, subject, body, *secs);
            if let Body::Present { raw: slot, .. } = &mut message.body {
                *slot = raw;
            }
            store
                .apply(
                    acct_account(),
                    &Patch {
                        id: ChangeId::generate(),
                        changes: vec![Change::MessageUpsert(Box::new(message))],
                    },
                )
                .expect("sqlite apply");
        }
        (store, dir)
    }

    /// `mailo search` prints the list box's answer: its top results first, marked `top`, then the
    /// other matches newest first. Asked of the pipeline, not restated, so the two cannot drift.
    #[test]
    fn cli_search_prints_the_windows_top_results_then_the_rest_newest_first() {
        let (store, _dir) = sqlite_with(&[
            ("compose a reply", "please compose this", 10),
            ("bravo", "compose the note", 20),
            ("charlie", "nothing to see", 30),
            ("delta", "do compose it", 40),
        ]);
        let now = at(10_000);
        let out = crate::cli::run(
            &store,
            &crate::cli::Command::Search {
                needle: "compose".to_owned(),
                limit: 20,
            },
            now,
        )
        .expect("search");
        let id = |line: &str| line.split_whitespace().next_back().unwrap_or("").to_owned();
        let top: Vec<String> = out
            .lines()
            .filter(|line| line.starts_with("top "))
            .map(id)
            .collect();
        let rest: Vec<String> = out
            .lines()
            .filter(|line| !line.starts_with("top "))
            .map(id)
            .collect();

        let window = mail_core::search::search_list(
            "compose",
            &store,
            &Affinity::default(),
            &chrono::Local,
            &|_| Vec::new(),
            mail_core::search::first(100),
            now,
        )
        .expect("not a pattern");
        let window_top: Vec<String> = window
            .top
            .iter()
            .map(|(summary, _)| summary.id.to_string())
            .collect();
        let window_rest: Vec<String> = window
            .rows
            .iter()
            .map(|summary| summary.id.to_string())
            .filter(|id| !window_top.contains(id))
            .collect();
        assert_eq!(
            window_top.first(),
            Some(&thread_of(0).to_string()),
            "the subject hit is the best result"
        );
        assert_eq!(top, window_top, "top results, cli:\n{out}");
        assert_eq!(rest, window_rest, "the rest, cli:\n{out}");
        assert_eq!(
            top.len() + rest.len(),
            3,
            "every match once, and only matches:\n{out}"
        );
    }
}
