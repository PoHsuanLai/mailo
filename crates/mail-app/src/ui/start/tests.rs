//! Starting the window on a conversation, as a notification's click would.

use super::{Start, open_thread, start_of};
use crate::ui::app::App;
use crate::ui::fixtures::{realistic, thread_like};
use crate::view::Shell;
use dioxus::dioxus_core::VirtualDom;
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
