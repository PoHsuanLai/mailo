//! Starting the window on a conversation, as a notification's click would.

use super::{Start, mailto_of, open_thread, start_mailto, start_of};
use crate::ui::app::App;
use crate::ui::fixtures::{realistic, thread_like};
use crate::view::Shell;
use dioxus::dioxus_core::VirtualDom;
use mail_domain::ThreadId;
use mail_store::Store as _;

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
        let mut dom = VirtualDom::new(App).with_root_context(store.clone());
        if let Some(start) = start {
            dom = dom.with_root_context(start);
        }
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    };
    let heading = "<h2>Re: Re: Re: Fwd: supervision meeting — moved to Thursday</h2>";

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
    let accounts = crate::compose::sending_accounts(&store);
    let Some(account) = accounts.first().map(|(_, id)| *id) else {
        panic!("the fixture has no sending account");
    };
    let drafts = || store.drafts(account).map(|d| d.len()).unwrap_or_default();
    let drafts_before = drafts();

    let Some(link) = mailto_of(&args(&[
        "mailto:someone@example.org?cc=other@example.org&subject=Lunch%20on%20Friday&body=Free%3F",
    ])) else {
        panic!("not read as a mailto link");
    };
    let now = chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap_or_default();
    let start = match start_mailto(&store, &link, now) {
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
            .with_root_context(store.clone())
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
