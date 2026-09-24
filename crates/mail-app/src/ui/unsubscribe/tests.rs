//! Leaving a list from the window. No test here reaches the network: one-click is the runtime's
//! to test against its local listener, and every fixture below that could reach it stops at
//! the popover.

use super::head::{Confirm, Phase};
use super::{Ask, Offer, archive_from, ask, leave, looked_at, offer_of};
use crate::ui::app::App;
use crate::ui::fixtures::{ACCOUNT, FakePointer, dispatching, pointer, rebuild_into, seeded};
use crate::ui::reading::Reader;
use crate::unsubscribe::{Found, Outcome};
use crate::view::Shell;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_domain::*;
use mail_mime::{ListHeaders, ListId};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

const ONE_CLICK: &str = "List-Id: Rust Weekly <weekly.rust.test>\r\n\
     List-Unsubscribe: <https://lists.rust.test/u/abc>, <mailto:leave@rust.test>\r\n\
     List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n";
const MAILTO: &str = "List-Id: <announce.example.test>\r\n\
     List-Unsubscribe: <mailto:leave@example.test?subject=remove>\r\n";
const WEB: &str = "List-Id: Deals <deals.shop.test>\r\n\
     List-Unsubscribe: <https://www.shop.test/prefs?u=1>\r\n";
const NO_WAY_OUT: &str = "List-Id: Quiet <quiet.example.test>\r\n";

/// Whether the body was fetched, or only its headers.
#[derive(Clone, Copy, PartialEq)]
enum Held {
    Body,
    HeadersOnly,
}

/// One message from `sender` in its own thread, with `headers` in its stored bytes.
fn put(store: &SqliteStore, sender: &str, headers: &str, held: Held) -> ThreadId {
    let rfc = format!("{}@example.test", uuid::Uuid::new_v4());
    let bytes = format!(
        "From: {sender}\r\nTo: me@example.test\r\nSubject: news\r\nMessage-ID: <{rfc}>\r\n\
         {headers}\r\nthis week's news\r\n"
    );
    let raw = store
        .blobs()
        .put(&store.connection(), bytes.as_bytes())
        .unwrap();
    let id = MessageId::generate();
    let message = Message {
        id,
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc(rfc.clone()),
        date: chrono::Utc::now(),
        from: Address {
            name: Some("News".to_owned()),
            email: sender.to_owned(),
        },
        reply_to: vec![],
        to: vec![Address {
            name: None,
            email: "me@example.test".to_owned(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: "news".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(rfc.clone()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: match held {
            Held::Body => Body::Present {
                text: Some("this week's news".to_owned()),
                raw,
            },
            Held::HeadersOnly => Body::Absent,
        },
        attachments: vec![],
    };
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: Some(SyncCursor::Pop),
                messages: vec![Fetched {
                    remote: RemoteRef::Pop { uidl: rfc },
                    key: message.key.clone(),
                    raw,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                label_names: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
    // The ingest threads it; ask the store which thread it landed in.
    store.message(id).unwrap().thread
}

fn address(email: &str) -> Address {
    Address {
        name: None,
        email: email.to_owned(),
    }
}

fn offer_from(raw_headers: &str) -> Option<Offer> {
    let bytes = format!("From: news@example.test\r\n{raw_headers}\r\nbody\r\n");
    let found = Found {
        message: MessageId::generate(),
        account: ACCOUNT,
        addressed: vec![address("me@example.test")],
        list: mail_mime::list_headers(bytes.as_bytes()),
    };
    offer_of(
        found,
        address("news@example.test"),
        address("me@example.test"),
    )
}

async fn settle(dom: &mut VirtualDom, for_ms: u64) {
    let until = tokio::time::Instant::now() + std::time::Duration::from_millis(for_ms);
    loop {
        let left = until.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let _ = tokio::time::timeout(left, dom.wait_for_work()).await;
        dom.render_immediate(&mut NoOpMutations);
    }
}

#[component]
fn Open(thread: ThreadId) -> Element {
    let shell = use_signal(Shell::default);
    rsx! { Reader { thread, shell } }
}

/// The reader on `thread`, once its lookup has landed.
async fn reader_on(store: Arc<SqliteStore>, thread: ThreadId) -> (VirtualDom, String) {
    let mut dom = VirtualDom::new_with_props(Open, OpenProps { thread }).with_root_context(store);
    dom.rebuild_in_place();
    settle(&mut dom, 400).await;
    let markup = dioxus_ssr::render(&dom);
    (dom, markup)
}

#[test]
fn there_is_an_offer_only_when_there_is_a_way_out() {
    let cases = [
        (ONE_CLICK, true),
        (MAILTO, true),
        (WEB, true),
        (NO_WAY_OUT, false),
        ("", false),
    ];
    for (headers, offered) in cases {
        assert_eq!(offer_from(headers).is_some(), offered, "{headers:?}");
    }
}

#[test]
fn the_list_is_named_by_its_description_then_its_id_then_its_sender() {
    let named = |id: Option<ListId>| {
        let found = Found {
            message: MessageId::generate(),
            account: ACCOUNT,
            addressed: vec![],
            list: ListHeaders {
                id,
                unsubscribe: vec![mail_mime::Unsubscribe::Web {
                    url: "https://example.test/".to_owned(),
                }],
            },
        };
        let sender = Address {
            name: Some("News".to_owned()),
            email: "news@example.test".to_owned(),
        };
        offer_of(found, sender, address("me@example.test"))
            .unwrap()
            .list
    };
    let id = |description: Option<&str>| ListId {
        description: description.map(str::to_owned),
        id: "weekly.example.test".to_owned(),
    };
    assert_eq!(named(Some(id(Some("Weekly")))), "Weekly");
    assert_eq!(named(Some(id(None))), "weekly.example.test");
    assert_eq!(named(None), "News");
}

#[tokio::test]
async fn the_head_offers_unsubscribe_only_when_the_thread_has_a_way_out() {
    let (store, _dir) = seeded();
    let listed = put(&store, "weekly@rust.test", MAILTO, Held::Body);
    let quiet = put(&store, "quiet@example.test", NO_WAY_OUT, Held::Body);
    let unfetched = put(&store, "later@example.test", ONE_CLICK, Held::HeadersOnly);

    let (_, markup) = reader_on(store.clone(), listed).await;
    assert!(markup.contains("aria-label=\"Unsubscribe\""), "{markup}");
    for thread in [quiet, unfetched] {
        let (_, markup) = reader_on(store.clone(), thread).await;
        assert!(!markup.contains("Unsubscribe"), "{markup}");
    }
}

/// The popover for `offer`, as it opens.
#[component]
fn Popover(offer: Offer) -> Element {
    let phase = use_signal(|| Phase::Asking);
    let asked = ask(&offer).unwrap();
    rsx! { Confirm { offer, asked, phase } }
}

fn popover(headers: &str) -> String {
    let offer = offer_from(headers).unwrap();
    let mut dom = VirtualDom::new_with_props(Popover, PopoverProps { offer });
    dom.rebuild_in_place();
    // The renderer escapes the apostrophe; the words are compared as they read.
    dioxus_ssr::render(&dom).replace("&#39;", "'")
}

#[test]
fn the_popover_says_what_each_way_out_will_do() {
    let one_click = popover(ONE_CLICK);
    assert!(
        one_click.contains("Leave Rust Weekly? mailo sends the list's server a one-click request."),
        "{one_click}"
    );
    assert!(
        one_click.contains("aria-label=\"Unsubscribe\""),
        "{one_click}"
    );

    let mailto = popover(MAILTO);
    assert!(
        mailto.contains(
            "Leave announce.example.test? mailo sends a message to leave@example.test from \
             me@example.test."
        ),
        "{mailto}"
    );
    assert!(mailto.contains("aria-label=\"Send\""), "{mailto}");

    let web = popover(WEB);
    assert!(
        web.contains("This list can only be left on its web page."),
        "{web}"
    );
    // The registered domain is the part in bold, as the link pill draws it.
    assert!(web.contains("<b>shop.test</b>"), "{web}");
    assert!(web.contains("aria-label=\"Copy\""), "{web}");
    assert!(!web.contains("aria-label=\"Send\"") && !web.contains("aria-label=\"Unsubscribe\""));
}

#[test]
fn a_web_page_is_never_given_a_client() {
    // `leave` asks for a client only when it could use one. A page returns first, so there is
    // no request to make and nothing to make it with.
    let (store, _dir) = seeded();
    let offer = offer_from(WEB).unwrap();
    assert_eq!(ask(&offer).and_then(|asked| asked.action()), None);
    let outcome = leave(&store, &offer.found, chrono::Utc::now(), || {
        panic!("a client was built for a page")
    });
    assert_eq!(
        outcome,
        Ok(Outcome::Page {
            url: "https://www.shop.test/prefs?u=1".to_owned()
        })
    );
}

#[test]
fn a_mailto_way_out_queues_a_message() {
    let (store, _dir) = seeded();
    let thread = put(&store, "announce@example.test", MAILTO, Held::Body);
    let offer = super::look(&store, thread).expect("the thread offers a way out");
    assert!(matches!(ask(&offer), Some(Ask::Mailto { .. })));

    let outcome = leave(&store, &offer.found, chrono::Utc::now(), super::client).unwrap();
    let Outcome::Queued { draft } = outcome else {
        panic!("a mailto was not queued: {outcome:?}");
    };
    let draft = store.draft(draft).unwrap();
    assert_eq!(draft.state, SendState::Queued);
    assert_eq!(draft.to, vec![address("leave@example.test")]);
    assert_eq!(draft.subject, "remove");
}

#[test]
fn archive_all_takes_the_senders_inbox_and_nothing_else() {
    let (store, _dir) = seeded();
    let first = put(&store, "weekly@rust.test", ONE_CLICK, Held::Body);
    let second = put(&store, "weekly@rust.test", ONE_CLICK, Held::Body);
    let other = put(&store, "quiet@example.test", NO_WAY_OUT, Held::Body);

    let undone = archive_from(&store, "weekly@rust.test");
    let archived: Vec<ThreadId> = undone.iter().filter_map(|undo| undo.thread).collect();
    assert!(
        archived.contains(&first) && archived.contains(&second),
        "{archived:?}"
    );
    assert!(!archived.contains(&other));
    for thread in [first, second] {
        let loaded = store.thread(thread).unwrap();
        assert!(!loaded.summary.mailboxes.contains(MailboxRole::Inbox));
    }
}

#[tokio::test]
async fn neither_the_list_nor_a_hover_card_reads_a_list_header() {
    // The lookup reads a stored blob. The list draws many rows and the hover card opens on any
    // of them; neither may reach it. Only the open reader does.
    dispatching();
    let (store, _dir) = seeded();
    let thread = put(&store, "weekly@rust.test", MAILTO, Held::Body);
    let mut dom = VirtualDom::new(App).with_root_context(store.clone());
    let seen = rebuild_into(&mut dom);
    let row = seen.one("data-hc", &format!("thread:{thread}"));
    let resting = || FakePointer {
        client: (400.0, 120.0),
        offset: (10.0, 10.0),
        held: false,
    };
    pointer(&mut dom, "pointerenter", row, resting());
    pointer(&mut dom, "pointerover", row, resting());
    settle(&mut dom, 700).await;
    assert!(
        dioxus_ssr::render(&dom).contains("role=\"tooltip\""),
        "no hover card opened"
    );
    assert!(
        !looked_at(thread),
        "the list or its hover card read a list header"
    );

    let (_, markup) = reader_on(store, thread).await;
    assert!(looked_at(thread) && markup.contains("aria-label=\"Unsubscribe\""));
}

#[tokio::test]
#[ignore = "writes target/unsubscribe.html for a person or a headless browser to look at"]
async fn render_the_unsubscribe_to_a_file() {
    let (store, _dir) = seeded();
    let thread = put(&store, "weekly@rust.test", ONE_CLICK, Held::Body);
    let (_, reader) = reader_on(store, thread).await;
    let mut body = format!(
        "<section class=\"reader\" style=\"width:620px;height:230px;margin:16px\">{reader}</section>"
    );
    // Each popover as it opens, under the head's button the way the reader anchors it.
    for headers in [ONE_CLICK, MAILTO, WEB] {
        let open = popover(headers);
        body.push_str(&format!(
            "<section class=\"reader\" style=\"width:620px;height:270px;margin:16px\">\
             <div class=\"reader-head\"><h2>This week in the list</h2><div class=\"reader-meta\">\
             <div class=\"reader-av\">N</div><div><div class=\"reader-from\">News</div></div>\
             <div class=\"leave\"><button class=\"mini\" type=\"button\" aria-expanded=\"true\">\
             Unsubscribe</button>{open}</div></div></div></section>"
        ));
    }
    crate::ui::fixtures::dump("unsubscribe", &body);
}
