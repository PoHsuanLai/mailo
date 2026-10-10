//! The inbox `native_latency` times against: one account, [`THREADS`] conversations of a few
//! kilobytes of HTML each, and the window's configuration over it.

#![allow(dead_code)]

use ds_blitz::{NetPolicy, PrintOutcome};
use ds_harness::{Clock, HarnessConfig, Viewport};
use mail_app::ui::appearance::WindowDirs;
use mail_core::SqliteStore;
use mail_core::{Arrival, absorb};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
use std::sync::Arc;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c3"))
}

pub const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// Conversations in the inbox: more than a page, so the list is as long as it gets.
pub const THREADS: usize = 300;

/// What the subjects are about; a search for one of these finds a fifth of the inbox.
pub const TOPICS: [&str; 5] = ["invoices", "travel", "hiring", "roadmap", "lunches"];

/// An HTML newsletter-sized body: a few kilobytes of markup, the shape that costs a render.
fn raw(n: usize, date: &str) -> Vec<u8> {
    let topic = TOPICS[n % TOPICS.len()];
    let paragraphs = format!("<p>About {topic}, item {n}, and what comes next.</p>").repeat(40);
    format!(
        "From: sender{n}@example.test\r\nTo: me@example.test\r\n\
         Subject: Note {n} about {topic}\r\nDate: {date}\r\n\
         Message-ID: <latency{n}@example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n\
         <html><body><h1>Note {n}</h1>{paragraphs}</body></html>\r\n"
    )
    .into_bytes()
}

/// A store in `dir` with one account and [`THREADS`] conversations of one message each.
pub fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
        mail_store::testing::seed_identity_for(
            &store,
            IdentityId::generate(),
            acct_account(),
            "me@example.test",
            None,
        );
    }
    let now = chrono::Utc::now();
    let arrivals = (0..THREADS)
        .map(|n| {
            let date = (now - chrono::Duration::minutes(n as i64 + 1)).to_rfc2822();
            Arrival {
                remote: RemoteRef::Pop {
                    uidl: format!("latency{n}"),
                },
                raw: raw(n, &date),
            }
        })
        .collect();
    absorb(
        &store,
        acct_account(),
        MailboxRef {
            account: acct_account(),
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        arrivals,
        false,
        now,
    )
    .unwrap();
    Arc::new(store)
}

pub fn config(store: &Arc<SqliteStore>, dir: &std::path::Path) -> HarnessConfig {
    let dirs = WindowDirs {
        config: dir.join("config"),
        state: dir.join("state"),
    };
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(store),
        mail_app::ui::view::Appearance::default(),
        None,
        Some(dirs),
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts)
}
