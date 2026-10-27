//! Download All Mail on an account's page in Settings: the setting per account, against a
//! temporary config directory only.

use crate::ui::data::account_rows;
use crate::ui::fixtures::{Seen, Work, click, work};
use dioxus::dioxus_core::VirtualDom;
use ds::prelude::Check;
use mail_core::Store as _;
use mail_core::offline::{self, Keep};
use mail_domain::*;
use porter_core::AccountId;

const ADDRESS: &str = "poh@acme.example";

fn opened(built: &Work) -> (VirtualDom, Seen) {
    crate::ui::settings_window::account::tests::pushed(built)
}

fn label() -> String {
    crate::ui::copy::fields::DOWNLOAD_ALL_MAIL.to_owned()
}

/// Whether the account's switch is drawn on, as the page draws it: the switch is the button
/// named for the setting, inside the row of the same name.
fn shown(page: &str) -> Check {
    let label = format!("aria-label=\"{}\"", label());
    let tag = page
        .match_indices("<button")
        .map(|(at, _)| &page[at..at + page[at..].find('>').unwrap_or(0)])
        .find(|tag| tag.contains(&label))
        .unwrap_or_else(|| panic!("no switch for {ADDRESS} in:\n{page}"));
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
    let raw = built.store.blobs().put(b"rebuilt").unwrap();
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
                section: "2".parse().unwrap(),
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
                label_names: Vec::new(),
                provider_threads: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
}

#[tokio::test]
async fn the_switch_keeps_the_setting_per_account_and_draws_no_count() {
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
    // How much is here is `mailo offline`'s to say: the page draws no caption.
    assert!(
        !page.contains(&offline::said(&counted)),
        "the count is still drawn:\n{page}"
    );

    let switch = seen.after("aria-label", &label(), "aria-checked")[0];
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
