//! Starting the window on a conversation, as a notification's click would.

use super::{Start, mailto_of, open_thread, start_mailto, start_of};
use crate::ui::app::App;
use crate::ui::fixtures::{realistic, thread_like};
use crate::ui::view::Shell;
use dioxus::dioxus_core::VirtualDom;
use mail_core::Store as _;
use mail_domain::ThreadId;

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// Arguments, and what the window makes of them: `None` is the CLI's, `Err` a bad `open`.
type Case<'a> = (&'a [&'a str], Option<Result<Start, ()>>);

#[test]
fn the_window_reads_no_arguments_and_open_and_leaves_the_rest_to_the_cli() {
    let thread = ThreadId::generate();
    let id = thread.to_string();
    let cases: &[Case] = &[
        (&[], Some(Ok(Start::Inbox))),
        (&["open", &id], Some(Ok(Start::Thread(thread)))),
        (&["open"], Some(Err(()))),
        (&["open", "not-a-thread"], Some(Err(()))),
        (&["open", &id, &id], Some(Err(()))),
        (&["show", &id], None),
        (&["list"], None),
        // A word that merely starts like it is not it.
        (&["opened", &id], None),
    ];
    for (words, want) in cases {
        let got = start_of(&args(words)).map(|read| read.map_err(|_| ()));
        assert_eq!(&got, want, "{words:?}");
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
    let paint = |start: Option<Start>| {
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
    let inbox = paint(Some(Start::Inbox));
    assert!(!inbox.contains(heading), "{inbox}");

    let opened = paint(Some(Start::Thread(thread)));
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
    let spaces = crate::ui::space::built(
        vec![
            (
                "Elsewhere".to_owned(),
                ds::prelude::SpaceLook::default(),
                crate::ui::space::Mail::of(Vec::new()),
            ),
            (
                "Supervision".to_owned(),
                ds::prelude::SpaceLook::default(),
                crate::ui::space::Mail::of(vec![account]),
            ),
        ],
        0,
    );
    let paint = |start: Start| {
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

    let plain = paint(Start::Inbox);
    assert!(head(&plain).contains("Elsewhere"), "{}", head(&plain));
    let opened = paint(Start::Thread(thread));
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

#[test]
fn a_lone_mailto_link_is_the_windows_and_anything_else_is_not() {
    const CASES: &[(&[&str], Option<&str>)] = &[
        (&["mailto:someone@example.org"], Some("someone@example.org")),
        (&["MAILTO:someone@example.org"], Some("someone@example.org")),
        // What the desktop runs when `%u` has no link: no argument, the inbox.
        (&[], None),
        // A link beside other words is no desktop's command line.
        (&["mailto:someone@example.org", "list"], None),
        (&["open", "mailto:someone@example.org"], None),
        (&["compose"], None),
        (&["someone@example.org"], None),
        (&["https://example.org/"], None),
    ];
    for (words, want) in CASES {
        let got = mailto_of(&args(words));
        let got = got.as_ref().map(|link| {
            link.to
                .iter()
                .map(|to| to.email.as_str())
                .collect::<Vec<_>>()
                .join(",")
        });
        assert_eq!(got.as_deref(), *want, "{words:?}");
    }
}

#[test]
fn a_window_started_from_a_mailto_link_opens_the_composer_on_its_draft() {
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

    let Some(link) = mailto_of(&args(&[
        "mailto:someone@example.org?cc=other@example.org&subject=Lunch%20on%20Friday&body=Free%3F",
    ])) else {
        panic!("not read as a mailto link");
    };
    let now = chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap_or_default();
    let start = match start_mailto(&crate::ui::fixtures::client(&store), &link, now) {
        Ok(start) => start,
        Err(why) => panic!("no draft: {why}"),
    };
    let Start::Compose(id) = start else {
        panic!("not the composer: {start:?}");
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

    let paint = |start: Start| {
        let mut dom = VirtualDom::new(App)
            .with_root_context(crate::ui::fixtures::client(&store))
            .with_root_context(start);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    };
    // The composer's subject field holding the link's subject, and its recipient chip.
    let subject = r#"aria-label="Subject" aria-placeholder="Subject" autocomplete="off" value="Lunch on Friday""#;
    let chip = r#"aria-label="Remove someone""#;
    let inbox = paint(Start::Inbox);
    assert!(
        !inbox.contains(subject) && !inbox.contains(chip),
        "a composer opened unasked:\n{inbox}"
    );
    let composing = paint(start);
    assert!(
        composing.contains(subject) && composing.contains(chip),
        "the composer is not on the link's draft:\n{composing}"
    );
}

/// A banner's Reply launches the window on a reply to that conversation, not on a `mailto:` link
/// that names neither the conversation nor its account.
#[test]
fn a_banners_reply_starts_the_window_on_a_reply_to_its_conversation() {
    let one = ThreadId::from_uuid(uuid::Uuid::from_u128(0x7001));
    let args_of = mail_core::notify::desktop::Launch {
        opens: mail_core::notify::Opens::Thread(one),
        act: mail_core::notify::Act::Reply("mailto:ada%40example.test".to_owned()),
    }
    .args();
    assert_eq!(
        args_of,
        ["open".to_owned(), "--reply".to_owned(), one.to_string()]
    );
    assert_eq!(
        super::start_of(&args_of),
        Some(Ok(super::Start::Reply(one)))
    );
    assert!(matches!(
        super::start_of(&args(&["open", "--reply"])),
        Some(Err(_))
    ));
    assert!(matches!(
        super::start_of(&args(&["open", "--reply", "not an id!"])),
        Some(Err(_))
    ));
}

/// What a desktop notification's click launches, and what the window makes of those arguments.
#[test]
fn a_click_starts_the_window_on_the_conversation_or_the_accounts_inbox() {
    let one = ThreadId::from_uuid(uuid::Uuid::from_u128(0x7001));
    let opens = mail_core::notify::desktop::Launch {
        opens: mail_core::notify::Opens::Thread(one),
        act: mail_core::notify::Act::Open,
    }
    .args();
    assert_eq!(opens, ["open".to_owned(), one.to_string()]);
    // What the window's own reading of the command line makes of them.
    assert_eq!(super::start_of(&opens), Some(Ok(super::Start::Thread(one))));
    // A summary's: its account's inbox, through the same reading.
    let account = mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(0xacc));
    let inbox = mail_core::notify::desktop::Launch {
        opens: mail_core::notify::Opens::Inbox(account.clone()),
        act: mail_core::notify::Act::Open,
    }
    .args();
    assert_eq!(
        inbox,
        ["open".to_owned(), "--inbox".to_owned(), account.to_string()]
    );
    assert_eq!(
        super::start_of(&inbox),
        Some(Ok(super::Start::InboxOf(account)))
    );
    assert!(matches!(
        super::start_of(&args(&["open", "--inbox"])),
        Some(Err(_))
    ));
    assert!(matches!(
        super::start_of(&args(&["open", "--inbox", "not an id!"])),
        Some(Err(_))
    ));
}

/// A link's `bcc` is a recipient nobody else will see, chosen by whoever wrote the link: the
/// composer opens with that row shown and the address in it, never folded away.
#[test]
fn a_mailto_links_hidden_recipient_is_shown_in_the_composer() {
    let (store, _dir) = realistic();
    let Some(link) = mailto_of(&args(&[
        "mailto:someone@example.org?bcc=hidden@example.org",
    ])) else {
        panic!("not read as a mailto link");
    };
    let now = chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap_or_default();
    let start = match start_mailto(&crate::ui::fixtures::client(&store), &link, now) {
        Ok(start) => start,
        Err(why) => panic!("no draft: {why}"),
    };
    let mut dom = VirtualDom::new(App)
        .with_root_context(crate::ui::fixtures::client(&store))
        .with_root_context(start);
    dom.rebuild_in_place();
    let composing = dioxus_ssr::render(&dom);
    assert!(
        composing.contains(r#"data-row="bcc""#)
            && composing.contains(r#"aria-label="Remove hidden""#),
        "the link's Bcc is not shown:\n{composing}"
    );
}
