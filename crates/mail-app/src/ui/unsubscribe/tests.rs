//! Leaving a list from the window. No test here reaches the network: one-click is the runtime's
//! to test against its local listener, and every fixture below that could reach it stops at
//! the popover.

use super::head::{Confirm, Phase};
use super::{Ask, Offer, archive_from, ask, leave};
use crate::ui::app::App;
use crate::ui::copy::menus::UNSUBSCRIBE;
use crate::ui::fixtures::settle_for;
use crate::ui::fixtures::{
    Seen, acct_account, click, dispatching, menu_names, rebuild_into, seeded,
};
use crate::ui::reading::Reader;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use ds::prelude::*;
use mail_core::message::offer_of;
use mail_core::message::{Looks, Read};
use mail_core::unsubscribe::{Found, Outcome};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use std::sync::Arc;
use std::time::Duration;

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
    let raw = store.blobs().put(bytes.as_bytes()).unwrap();
    let id = MessageId::generate();
    let message = Message {
        id,
        thread: ThreadId::generate(),
        account: acct_account(),
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
            acct_account(),
            Ingest {
                mailbox: MailboxRef {
                    account: acct_account(),
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
        account: acct_account(),
        addressed: vec![address("me@example.test")],
        list: mail_mime::list_headers(bytes.as_bytes()),
    };
    offer_of(
        found,
        address("news@example.test"),
        address("me@example.test"),
    )
}

#[component]
fn Open(thread: ThreadId) -> Element {
    let shell = use_signal(Shell::default);
    let revision = use_signal(|| 0u64);
    rsx! { ds::prelude::Ds { appearance: ds::prelude::Appearance::default(), material: ds::prelude::Material::Window, Reader { thread, shell, revision: Some(revision) } } }
}

/// Open the reader's ⋯ and return the names it lists.
fn more_names(dom: &mut VirtualDom, seen: &Seen) -> Vec<String> {
    let more = *seen
        .all("aria-label", "More")
        .last()
        .expect("the reader's ⋯");
    let opened = click(dom, more);
    crate::ui::fixtures::settle(dom, opened);
    menu_names(&dioxus_ssr::render(dom))
}

/// The reader on `thread`, once its lookups have landed: the names its ⋯ lists, its markup
/// before the ⋯ was opened, and the lookups it used.
async fn reader_looking(
    store: Arc<SqliteStore>,
    thread: ThreadId,
) -> (Vec<String>, String, Arc<Looks>) {
    dispatching();
    let looks = Arc::new(Looks::new());
    let mut dom = VirtualDom::new_with_props(Open, OpenProps { thread })
        .with_root_context(store)
        .with_root_context(looks.clone());
    let mut seen = rebuild_into(&mut dom);
    settle_for(&mut dom, &mut seen, Duration::from_millis(400)).await;
    let markup = dioxus_ssr::render(&dom);
    let names = more_names(&mut dom, &seen);
    (names, markup, looks)
}

/// [`reader_looking`]'s markup alone.
async fn reader_on(store: Arc<SqliteStore>, thread: ThreadId) -> String {
    reader_looking(store, thread).await.1
}

#[tokio::test]
async fn the_more_menu_offers_unsubscribe_only_when_the_thread_has_a_way_out() {
    let (store, _dir) = seeded();
    let listed = put(&store, "weekly@rust.test", MAILTO, Held::Body);
    let quiet = put(&store, "quiet@example.test", NO_WAY_OUT, Held::Body);
    let unfetched = put(&store, "later@example.test", ONE_CLICK, Held::HeadersOnly);

    let (names, markup, _) = reader_looking(store.clone(), listed).await;
    assert!(names.iter().any(|name| name == UNSUBSCRIBE), "{names:?}");
    // The head no longer carries it: the ⋯ is its one home.
    assert!(!markup.contains("aria-label=\"Unsubscribe\""), "{markup}");
    for thread in [quiet, unfetched] {
        let (names, ..) = reader_looking(store.clone(), thread).await;
        assert!(!names.iter().any(|name| name == UNSUBSCRIBE), "{names:?}");
    }
}

/// One reader, whose thread the test moves through the signal it provides.
#[component]
fn Moving(first: ThreadId) -> Element {
    let shell = use_signal(Shell::default);
    let revision = use_signal(|| 0u64);
    let thread = use_context_provider(|| Signal::new(first));
    rsx! { ds::prelude::Ds { appearance: ds::prelude::Appearance::default(), material: ds::prelude::Material::Window, Reader { thread: thread(), shell, revision: Some(revision) } } }
}

#[tokio::test]
async fn the_answer_for_one_thread_is_never_offered_under_the_next() {
    // The same reader, moved to another thread, must ask again: the lookout is keyed on the
    // thread and its bodies, and its answer is read back only under that key.
    dispatching();
    let (store, _dir) = seeded();
    let listed = put(&store, "weekly@rust.test", MAILTO, Held::Body);
    let quiet = put(&store, "quiet@example.test", NO_WAY_OUT, Held::Body);
    let mut dom =
        VirtualDom::new_with_props(Moving, MovingProps { first: listed }).with_root_context(store);
    let mut seen = rebuild_into(&mut dom);
    settle_for(&mut dom, &mut seen, Duration::from_millis(400)).await;
    let before = more_names(&mut dom, &seen);
    assert!(before.iter().any(|name| name == UNSUBSCRIBE), "{before:?}");
    let mut thread = dom.in_scope(
        dioxus_core::ScopeId::APP,
        consume_context::<Signal<ThreadId>>,
    );
    dom.in_runtime(|| thread.set(quiet));
    let mut seen = Seen::default();
    dom.render_immediate(&mut seen);
    settle_for(&mut dom, &mut seen, Duration::from_millis(400)).await;
    let markup = dioxus_ssr::render(&dom);
    // The ⋯ opened above is still up, and lists what the quiet thread offers.
    let after = if markup.contains("class=\"ds-menu-item\"") {
        menu_names(&markup)
    } else {
        more_names(&mut dom, &seen)
    };
    assert!(
        !after.iter().any(|name| name == UNSUBSCRIBE),
        "the listed thread's way out is offered under the quiet one: {after:?}"
    );
}

/// The popover for `offer`, as it opens.
#[component]
fn Popover(offer: Offer) -> Element {
    let phase = use_signal(|| Phase::Asking);
    let asked = ask(&offer).unwrap();
    rsx! {
        Ds { appearance: Appearance::default(), material: Material::Window,
            Confirm { offer, asked, phase, anchor: ds::host::measure::Anchor::Point(Point::default()) }
        }
    }
}

fn popover(headers: &str) -> String {
    let offer = offer_from(headers).unwrap();
    let mut dom = VirtualDom::new_with_props(Popover, PopoverProps { offer });
    dom.rebuild_in_place();
    crate::ui::fixtures::drain(&mut dom);
    // The renderer escapes the apostrophe; the words are compared as they read.
    dioxus_ssr::render(&dom).replace("&#39;", "'")
}

#[test]
fn the_popover_says_what_each_way_out_will_do() {
    let one_click = popover(ONE_CLICK);
    assert!(one_click.contains("Leave Rust Weekly?"), "{one_click}");
    assert!(
        one_click.contains("aria-label=\"Unsubscribe\""),
        "{one_click}"
    );

    let mailto = popover(MAILTO);
    assert!(
        mailto.contains(
            "Leave announce.example.test? A message goes to leave@example.test from \
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
    let offer = Looks::new()
        .offer(&store, thread, &Vec::new())
        .expect("the thread offers a way out");
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

#[test]
fn archive_all_is_one_undo_for_the_whole_list() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Clone)]
    struct Handed(Arc<SqliteStore>, Arc<AtomicUsize>);
    #[component]
    fn Archiver() -> Element {
        let Handed(store, entries) = consume_context::<Handed>();
        let shell = use_signal(Shell::default);
        let revision = use_signal(|| 0u64);
        use_hook(|| {
            super::archive_list(&store, shell, revision, "weekly@rust.test", "Rust Weekly");
            entries.store(shell.peek().undo.len(), Ordering::SeqCst);
        });
        rsx! {}
    }
    let (store, _dir) = seeded();
    for _ in 0..2 {
        put(&store, "weekly@rust.test", ONE_CLICK, Held::Body);
    }
    let entries = Arc::new(AtomicUsize::new(usize::MAX));
    let mut dom = VirtualDom::new(Archiver).with_root_context(Handed(store, entries.clone()));
    dom.rebuild_in_place();
    assert_eq!(entries.load(Ordering::SeqCst), 1, "one list, one undo");
}

#[tokio::test]
async fn the_list_does_not_read_a_list_header() {
    // The lookup reads a stored blob. The list draws rows; it may not reach it. Only the open reader does.
    dispatching();
    let (store, _dir) = seeded();
    let thread = put(&store, "weekly@rust.test", MAILTO, Held::Body);
    let looks = Arc::new(Looks::new());
    let mut dom = VirtualDom::new(App)
        .with_root_context(store.clone())
        .with_root_context(looks.clone());
    let _seen = rebuild_into(&mut dom);
    settle_for(&mut dom, &mut Seen::default(), Duration::from_millis(50)).await;
    assert_eq!(looks.reads(Read::List), 0, "the list read a list header");

    let (names, _, reader) = reader_looking(store, thread).await;
    assert!(reader.reads(Read::List) > 0 && names.iter().any(|name| name == UNSUBSCRIBE));
}

#[tokio::test]
#[ignore = "writes target/unsubscribe.html for a person or a headless browser to look at"]
async fn render_the_unsubscribe_to_a_file() {
    let (store, _dir) = seeded();
    let thread = put(&store, "weekly@rust.test", ONE_CLICK, Held::Body);
    let reader = reader_on(store, thread).await;
    let mut body = format!(
        "<section class=\"reader\" style=\"width:620px;height:230px;margin:16px\">{reader}</section>"
    );
    // Each popover as it opens, under a stand-in for the ⋯ the reader anchors it to.
    for headers in [ONE_CLICK, MAILTO, WEB] {
        let open = popover(headers);
        body.push_str(&format!(
            "<section class=\"reader\" style=\"width:620px;height:270px;margin:16px\">\
             <div class=\"reader-head\"><h2>This week in the list</h2><div class=\"reader-meta\">\
             <div class=\"reader-av\">N</div><div><div class=\"reader-from\">News</div></div>\
             <div class=\"leave\"><button type=\"button\" class=\"ds-button\" data-variant=\"mini\" \
             aria-label=\"Unsubscribe\" aria-expanded=\"true\"><span>Unsubscribe</span></button>{open}\
             </div></div></div></section>"
        ));
    }
    crate::ui::fixtures::dump("unsubscribe", &body);
}
