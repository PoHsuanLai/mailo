//! Keep all mail offline on Settings' Accounts page: the setting per account, and the count beside it,
//! against a temporary config directory only.

use crate::ui::data::account_rows;
use crate::ui::fixtures::{Seen, Work, click, work};
use dioxus::dioxus_core::VirtualDom;
use ds::prelude::Check;
use mail_core::offline::{self, Keep};
use mail_domain::*;
use mail_store::Store as _;
use porter_core::AccountId;

const ADDRESS: &str = "poh@acme.example";

fn opened(built: &Work) -> (VirtualDom, Seen) {
    crate::ui::settings_window::tests::opened_on(built, crate::ui::view::SettingsPage::Accounts)
}

fn label() -> String {
    format!("Keep all mail offline for {ADDRESS}")
}

/// Whether the account's switch is drawn on, as the page draws it.
fn shown(page: &str) -> Check {
    let label = format!("aria-label=\"{}\"", label());
    let at = page
        .find(&label)
        .unwrap_or_else(|| panic!("no switch for {ADDRESS} in:\n{page}"));
    let open = page[..at].rfind("<button").unwrap_or(0);
    let tag = &page[open..at];
    assert!(
        tag.contains("role=\"switch\""),
        "the setting is not a switch: {tag}"
    );
    if tag.contains("aria-checked=\"true\"") {
        Check::On
    } else {
        Check::Off
    }
}

fn account(built: &Work) -> AccountId {
    account_rows(&built.store)
        .into_iter()
        .find(|row| row.address == ADDRESS)
        .map(|row| row.id)
        .unwrap_or_else(|| panic!("the fixture has no {ADDRESS}"))
}

/// A large message rebuilt from its parts, its 2 MB attachment still on the server.
fn with_a_part_on_the_server(built: &Work, id: AccountId) {
    let raw = built
        .store
        .blobs()
        .put(&built.store.connection(), b"rebuilt")
        .unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: id.clone(),
        key: MessageKey::Rfc("large@example.com".to_owned()),
        date: chrono::Utc::now() - chrono::TimeDelta::days(3),
        from: Address {
            name: None,
            email: "large@example.com".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "The scans".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("large@example.com".to_owned()),
        read: ReadState::Read,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some("Scans attached.".to_owned()),
            raw,
        },
        attachments: vec![Attachment {
            name: "scans.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            size: 2 * 1024 * 1024,
            content: PartContent::Remote {
                section: "2".to_owned(),
            },
            inline: Inline::Attached,
        }],
    };
    built
        .store
        .ingest(
            id.clone(),
            Ingest {
                mailbox: MailboxRef {
                    account: id,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![Fetched {
                    remote: RemoteRef::Imap {
                        mailbox: "INBOX".to_owned(),
                        uidvalidity: 1,
                        uid: 9_001,
                    },
                    key: message.key.clone(),
                    raw,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                label_names: vec![],
                gone: vec![],
            },
        )
        .unwrap();
}

#[tokio::test]
async fn each_account_says_how_much_is_here_and_the_switch_keeps_the_setting() {
    let built = work();
    let id = account(&built);
    let before = built.store.offline(id.clone()).unwrap();
    with_a_part_on_the_server(&built, id.clone());
    let counted = built.store.offline(id.clone()).unwrap();
    assert_eq!(
        (counted.messages, counted.held, counted.parts_remote),
        (before.messages + 1, before.held, before.parts_remote + 1),
        "the new message is here but not in full"
    );

    let config = &built.dirs.config;
    assert_eq!(
        offline::load(config).of(id.clone()),
        Keep::Bodies,
        "off by default"
    );
    let (mut dom, seen) = opened(&built);
    let page = dioxus_ssr::render(&dom);
    assert_eq!(shown(&page), Check::Off);
    // The fixture's own mail has a file left on the server too, so the numbers are the store's.
    let said = format!(
        "{} of {} messages offline; {} attachments ({}) on the server",
        counted.held,
        counted.messages,
        counted.parts_remote,
        mail_core::attach::human_size(counted.remote_bytes)
    );
    assert_eq!(said, offline::said(&counted));
    assert!(page.contains(&said), "no {said:?} in:\n{page}");

    let switch = seen.one("aria-label", &label());
    click(&mut dom, switch);
    assert_eq!(
        offline::load(config).of(id.clone()),
        Keep::Everything,
        "On was not kept"
    );
    assert_eq!(shown(&dioxus_ssr::render(&dom)), Check::On);
    for other in account_rows(&built.store).iter().filter(|row| row.id != id) {
        assert_eq!(
            offline::load(config).of(other.id.clone()),
            Keep::Bodies,
            "{} was turned on with it",
            other.address
        );
    }

    click(&mut dom, switch);
    assert_eq!(
        offline::load(config).of(id),
        Keep::Bodies,
        "Off was not kept"
    );
}

#[tokio::test]
async fn the_switch_opens_on_what_was_kept() {
    let built = work();
    let id = account(&built);
    offline::save(&built.dirs.config, id, Keep::Everything).unwrap_or_else(|why| panic!("{why}"));
    let (dom, _) = opened(&built);
    assert_eq!(shown(&dioxus_ssr::render(&dom)), Check::On);
}
