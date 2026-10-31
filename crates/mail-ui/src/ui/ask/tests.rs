//! Starting the window on a conversation, as a notification's click would.

use super::{
    Archive, Ask, Inbox, Mailto, Open, Raise, Reply, Thread, open_thread, parse_args, parse_uri,
};
use crate::ui::app::App;
use crate::ui::fixtures::{dispatching, realistic, rebuild_into, settle_quiet, thread_like};
use dioxus::dioxus_core::VirtualDom;
use mail_core::{SqliteStore, Store as _};
use mail_domain::ThreadId;
use mail_ui_kit::view::Shell;
use std::sync::Arc;

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// The URI of the ask `words` make: `None` is the CLI's, `Err` a bad `open`.
fn uri_of_args(words: &[&str]) -> Option<Result<String, ()>> {
    parse_args(&args(words)).map(|read| read.map(|ask| ask.uri()).map_err(|_| ()))
}

/// The window, started on `open`, painted until `marker` is on the page (or eight settles have
/// passed): what an ask does once shown is done after the first draw.
async fn started_on(store: &Arc<SqliteStore>, open: Open, marker: &str) -> String {
    dispatching();
    let mut dom = VirtualDom::new(App)
        .with_root_context(crate::ui::fixtures::client(store))
        .with_root_context(open);
    let _seen = rebuild_into(&mut dom);
    let mut page = String::new();
    for _ in 0..8 {
        settle_quiet(&mut dom).await;
        page = dioxus_ssr::render(&dom);
        if page.contains(marker) {
            break;
        }
    }
    page
}

/// A command line, and what the window reads from it: the ask's URI, an error, or nothing (the
/// command-line tool's).
type ArgsCase<'a> = (&'a [&'a str], Option<Result<String, ()>>);

#[test]
fn the_window_reads_no_arguments_open_and_a_lone_mailto_link_and_leaves_the_rest_to_the_cli() {
    let thread = ThreadId::generate();
    let id = thread.to_string();
    let account = mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(0xacc));
    let cases: &[ArgsCase] = &[
        (&[], Some(Ok(Raise.uri()))),
        (&["open", &id], Some(Ok(Thread(thread).uri()))),
        (&["open", "--reply", &id], Some(Ok(Reply(thread).uri()))),
        (&["open", "--archive", &id], Some(Ok(Archive(thread).uri()))),
        (
            &["open", "--inbox", account.as_str()],
            Some(Ok(Inbox(account.clone()).uri())),
        ),
        (&["open"], Some(Err(()))),
        (&["open", "not-a-thread"], Some(Err(()))),
        (&["open", &id, &id], Some(Err(()))),
        (&["open", "--reply"], Some(Err(()))),
        (&["open", "--reply", "not an id!"], Some(Err(()))),
        (&["open", "--archive"], Some(Err(()))),
        (&["open", "--archive", "not an id!"], Some(Err(()))),
        (&["open", "--inbox"], Some(Err(()))),
        (&["open", "--inbox", "not an id!"], Some(Err(()))),
        (
            &["mailto:someone@example.org"],
            Some(Ok("mailto:someone@example.org".to_owned())),
        ),
        (
            &["MAILTO:someone@example.org"],
            Some(Ok("MAILTO:someone@example.org".to_owned())),
        ),
        // A link beside other words is no desktop's command line.
        (&["mailto:someone@example.org", "list"], None),
        (&["open", "mailto:someone@example.org"], Some(Err(()))),
        (&["show", &id], None),
        (&["list"], None),
        (&["compose"], None),
        (&["someone@example.org"], None),
        (&["https://example.org/"], None),
        // A word that merely starts like it is not it.
        (&["opened", &id], None),
    ];
    for (words, want) in cases {
        assert_eq!(&uri_of_args(words), want, "{words:?}");
    }
}

/// What a desktop notification's click launches, and what the window makes of those arguments:
/// each ask writes the command line that starts a window on it, and reads it back.
#[test]
fn every_ask_reads_back_from_the_command_line_it_writes() {
    let one = ThreadId::from_uuid(uuid::Uuid::from_u128(0x7001));
    let account = mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(0xacc));
    let asks: Vec<Box<dyn Ask>> = vec![
        Box::new(Thread(one)),
        Box::new(Reply(one)),
        Box::new(Archive(one)),
        Box::new(Inbox(account)),
        Box::new(Mailto::to("someone@example.org")),
        Box::new(Raise),
    ];
    for ask in &asks {
        let read = parse_args(&ask.args()).map(|read| read.map(|read| read.uri()));
        assert_eq!(read, Some(Ok(ask.uri())), "{ask:?}");
    }
    // The banner's buttons, as the notification code writes them.
    for (act, want) in [
        (mail_core::notify::Act::Open, Thread(one).args()),
        (mail_core::notify::Act::Reply, Reply(one).args()),
        (mail_core::notify::Act::Archive, Archive(one).args()),
    ] {
        let launch = mail_core::notify::desktop::Launch {
            opens: mail_core::notify::Opens::Thread(one),
            act,
        };
        assert_eq!(launch.args(), want);
    }
    let summary = mail_core::notify::desktop::Launch {
        opens: mail_core::notify::Opens::Inbox(mail_domain::id::account_id_from_uuid(
            uuid::Uuid::from_u128(0xacc),
        )),
        act: mail_core::notify::Act::Open,
    };
    assert_eq!(
        summary.args(),
        Inbox(mail_domain::id::account_id_from_uuid(
            uuid::Uuid::from_u128(0xacc)
        ))
        .args()
    );
}

/// Each ask is written as a URI of its own, which is not any other's, and anything else is not
/// one of ours.
#[test]
fn every_ask_round_trips_through_its_uri_and_nothing_else_is_ours() {
    let thread = ThreadId::from_uuid(uuid::Uuid::from_u128(0x1234));
    let account = mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(0xacc));
    let asks: Vec<Box<dyn Ask>> = vec![
        Box::new(Thread(thread)),
        Box::new(Reply(thread)),
        Box::new(Archive(thread)),
        Box::new(Inbox(account)),
        Box::new(Mailto::to("someone@example.org")),
        Box::new(Raise),
    ];
    for ask in &asks {
        let read = parse_uri(&ask.uri()).expect("one of ours");
        assert_eq!(read.uri(), ask.uri());
        assert_eq!(read.args(), ask.args());
    }
    assert!(Thread::from_uri(&Reply(thread).uri()).is_none());
    assert!(Thread::from_uri(&Archive(thread).uri()).is_none());
    assert!(Reply::from_uri(&Thread(thread).uri()).is_none());
    assert!(Reply::from_uri(&Archive(thread).uri()).is_none());
    assert!(Archive::from_uri(&Reply(thread).uri()).is_none());
    const NOT_OURS: &[&str] = &[
        "",
        "mailo:thread/",
        "mailo:thread/not-a-uuid",
        "https://example.test/thread/00000000-0000-0000-0000-000000001234",
        "mailo:thread",
        "mailo:reply",
        "mailo:reply/",
        "mailo:reply/not-a-uuid",
        "mailo:archive",
        "mailo:archive/",
        "mailo:archive/not-a-uuid",
        "mailo:inbox",
        "mailo:inbox/",
        "mailo:inbox/not an id!",
    ];
    for uri in NOT_OURS {
        assert!(parse_uri(uri).is_none(), "{uri:?}");
    }
}

#[test]
fn open_thread_selects_the_inbox_and_opens_the_conversation() {
    let thread = ThreadId::generate();
    let mut shell = Shell::default();
    shell.select(3);
    shell.show_remote_images = true;
    open_thread(&mut shell, thread);
    assert_eq!(shell.selected, 0, "not the inbox");
    assert_eq!(shell.places[shell.selected].name, "Inbox");
    assert_eq!(shell.open, Some(thread));
    assert!(!shell.show_remote_images, "consent carried over");
}

#[test]
fn a_window_started_on_a_thread_opens_it_in_the_reader() {
    let (store, _dir) = realistic();
    let thread = thread_like(&store, "supervision meeting");
    let paint = |start: Option<Open>| {
        let mut dom = VirtualDom::new(App).with_root_context(crate::ui::fixtures::client(&store));
        if let Some(start) = start {
            dom = dom.with_root_context(start);
        }
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    };
    let heading =
        "data-style=\"title\">Re: Re: Re: Fwd: supervision meeting — moved to Thursday</span></h2>";

    let plain = paint(None);
    assert!(
        !plain.contains(heading),
        "a reader opened unasked:\n{plain}"
    );
    let inbox = paint(Some(Open::raise()));
    assert!(!inbox.contains(heading), "{inbox}");

    let opened = paint(Some(Open::new(Thread(thread), None)));
    assert!(
        opened.contains(heading),
        "the reader is not on it:\n{opened}"
    );
}

/// A window started from a click (`mailo open`, nothing running to hand it to) opens on the Space
/// that holds the conversation's account, not the one it was left in.
#[test]
fn a_window_started_on_a_thread_opens_in_the_space_that_holds_its_account() {
    let (store, _dir) = realistic();
    let thread = thread_like(&store, "supervision meeting");
    let Ok(account) = store.thread(thread).map(|thread| thread.summary.account) else {
        panic!("the fixture's conversation");
    };
    let spaces = mail_ui_kit::space::built(
        vec![
            (
                "Elsewhere".to_owned(),
                ds::prelude::SpaceLook::default(),
                mail_ui_kit::space::Mail::of(Vec::new()),
            ),
            (
                "Supervision".to_owned(),
                ds::prelude::SpaceLook::default(),
                mail_ui_kit::space::Mail::of(vec![account]),
            ),
        ],
        0,
    );
    let paint = |start: Open| {
        let mut dom = VirtualDom::new(App)
            .with_root_context(crate::ui::fixtures::client(&store))
            .with_root_context(spaces.clone())
            .with_root_context(start);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    };
    let head = |html: &str| {
        html.split("class=\"ds-space-head")
            .nth(1)
            .map(|rest| rest.chars().take(400).collect::<String>())
            .unwrap_or_default()
    };

    let plain = paint(Open::raise());
    assert!(head(&plain).contains("Elsewhere"), "{}", head(&plain));
    let opened = paint(Open::new(Thread(thread), None));
    assert!(
        head(&opened).contains("Supervision"),
        "not in the Space that holds it: {}",
        head(&opened)
    );
    assert!(
        opened.contains("supervision meeting — moved to Thursday</span></h2>"),
        "the reader is not on it"
    );
}

/// A `mailto:` link saves the draft it asks for, from the first sending account, and sends
/// nothing.
#[test]
fn a_mailto_link_saves_the_draft_it_asks_for() {
    let (store, _dir) = realistic();
    let accounts = mail_core::compose::sending_accounts(&store);
    let Some(account) = accounts.first().map(|(_, id)| id.clone()) else {
        panic!("the fixture has no sending account");
    };
    let drafts = || {
        store
            .drafts(account.clone())
            .map(|d| d.len())
            .unwrap_or_default()
    };
    let drafts_before = drafts();

    let Some(link) = Mailto::parse(
        "mailto:someone@example.org?cc=other@example.org&subject=Lunch%20on%20Friday&body=Free%3F",
    ) else {
        panic!("not read as a mailto link");
    };
    let now = chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap_or_default();
    let id = match link.draft(&crate::ui::fixtures::client(&store), now) {
        Ok(id) => id,
        Err(why) => panic!("no draft: {why}"),
    };

    let draft = match store.draft(id) {
        Ok(draft) => draft,
        Err(why) => panic!("the draft was not saved: {why}"),
    };
    assert_eq!(draft.account, account);
    let emails = |list: &[mail_domain::Address]| -> Vec<String> {
        list.iter().map(|a| a.email.clone()).collect()
    };
    assert_eq!(emails(&draft.to), ["someone@example.org"]);
    assert_eq!(emails(&draft.cc), ["other@example.org"]);
    assert!(draft.bcc.is_empty());
    assert_eq!(draft.subject, "Lunch on Friday");
    assert!(draft.text.starts_with("Free?"), "{:?}", draft.text);
    assert_eq!(drafts(), drafts_before + 1);
}

/// A window started from a `mailto:` link saves its draft and opens the composer on it.
#[tokio::test]
async fn a_window_started_from_a_mailto_link_opens_the_composer_on_its_draft() {
    let (store, _dir) = realistic();
    // The composer's subject field holding the link's subject, and its recipient chip.
    let subject = r#"aria-label="Subject" aria-placeholder="Subject" autocomplete="off" value="Lunch on Friday""#;
    let chip = r#"aria-label="Remove someone""#;
    let inbox = started_on(&store, Open::raise(), subject).await;
    assert!(
        !inbox.contains(subject) && !inbox.contains(chip),
        "a composer opened unasked:\n{inbox}"
    );
    let Some(link) = Mailto::parse(
        "mailto:someone@example.org?cc=other@example.org&subject=Lunch%20on%20Friday&body=Free%3F",
    ) else {
        panic!("not read as a mailto link");
    };
    let composing = started_on(&store, Open::new(link, None), subject).await;
    assert!(
        composing.contains(subject) && composing.contains(chip),
        "the composer is not on the link's draft:\n{composing}"
    );
}

/// A window started on an archive files the conversation, says so with an Undo, and the Undo puts
/// it back: the same path as the window's Archive key.
#[tokio::test]
async fn a_window_started_on_an_archive_archives_it_and_offers_undo() {
    use crate::ui::fixtures::{
        INSIDE_THE_SHELL, PRIMARY, chord, dispatching, rebuild_into, settle_quiet, work,
    };
    use mail_domain::MailboxRole;
    dispatching();
    let built = work();
    let in_inbox = |store: &mail_core::SqliteStore| {
        store
            .thread(built.dana)
            .unwrap()
            .summary
            .mailboxes
            .contains(MailboxRole::Inbox)
    };
    assert!(in_inbox(&built.store));
    let mut dom = VirtualDom::new(App)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone())
        .with_root_context(Open::new(Archive(built.dana), None));
    let _seen = rebuild_into(&mut dom);
    let mut page = String::new();
    for _ in 0..8 {
        settle_quiet(&mut dom).await;
        page = dioxus_ssr::render(&dom);
        if !in_inbox(&built.store) && page.contains("ds-toast-action") {
            break;
        }
    }
    assert!(
        !in_inbox(&built.store),
        "the banner's Archive archived nothing"
    );
    assert!(
        page.contains("class=\"ds-toast-action\""),
        "no toast with an Undo:\n{page}"
    );
    chord(
        &mut dom,
        "z",
        PRIMARY,
        dioxus::dioxus_core::ElementId(INSIDE_THE_SHELL as usize),
    );
    assert!(in_inbox(&built.store), "Undo did not put it back");
}

/// A link's `bcc` is a recipient nobody else will see, chosen by whoever wrote the link: the
/// composer opens with that row shown and the address in it, never folded away.
#[tokio::test]
async fn a_mailto_links_hidden_recipient_is_shown_in_the_composer() {
    let (store, _dir) = realistic();
    let Some(link) = Mailto::parse("mailto:someone@example.org?bcc=hidden@example.org") else {
        panic!("not read as a mailto link");
    };
    let chip = r#"aria-label="Remove hidden""#;
    let composing = started_on(&store, Open::new(link, None), chip).await;
    assert!(
        composing.contains(r#"data-row="bcc""#) && composing.contains(chip),
        "the link's Bcc is not shown:\n{composing}"
    );
}
