//! The launch question and Settings › General's row over a fake handler, so no test changes what
//! the real system opens email links with.

use super::{Asked, DefaultMailQuestion, DefaultMailRow, load, name_of, save};
use crate::ui::appearance::WindowDirs;
use crate::ui::fixtures::{acct_account, click, dispatching, drain_seen, rebuild_into};
use crate::ui::view::Shell;
use dioxus::dioxus_core::VirtualDom;
use dioxus::prelude::*;
use ds::prelude::*;
use mail_os::mail_handler::{Done, Handler, Made, MailHandler, MailHandlers};
use mail_ui_model::copy;
use std::sync::{Arc, Mutex};

/// A handler that answers `current` from memory, and on a request records it and makes mailo
/// the handler when `agrees`, as the system's dialog does when the person picks mailo.
struct Fake {
    current: Mutex<Handler>,
    asked: Mutex<u32>,
    agrees: Made,
}

impl Fake {
    fn new(current: Handler, agrees: Made) -> Arc<Self> {
        Arc::new(Self {
            current: Mutex::new(current),
            asked: Mutex::new(0),
            agrees,
        })
    }

    fn asked(&self) -> u32 {
        *self.asked.lock().unwrap()
    }
}

impl MailHandler for Fake {
    fn current(&self) -> Handler {
        self.current.lock().unwrap().clone()
    }

    fn make_mailo(&self, done: Done) {
        *self.asked.lock().unwrap() += 1;
        if self.agrees == Made::Made {
            *self.current.lock().unwrap() = Handler::Mailo;
        }
        done(self.agrees);
    }
}

fn dirs(root: &tempfile::TempDir) -> WindowDirs {
    WindowDirs {
        config: root.path().join("config"),
        state: root.path().join("state"),
    }
}

/// The question, in a window with an account.
#[component]
fn Asking() -> Element {
    let shell = use_signal(|| Shell {
        accounts: vec![("ada@example.test".to_owned(), acct_account())],
        ..Shell::default()
    });
    rsx! {
        Ds { appearance: Appearance::default(), material: Material::Window,
            DefaultMailQuestion { shell }
        }
    }
}

/// The question, in a window with no account yet.
#[component]
fn FirstRun() -> Element {
    let shell = use_signal(Shell::default);
    rsx! {
        Ds { appearance: Appearance::default(), material: Material::Window,
            DefaultMailQuestion { shell }
        }
    }
}

fn opened(root: fn() -> Element, fake: &Arc<Fake>, dirs: &WindowDirs) -> (VirtualDom, String) {
    dispatching();
    let handlers = MailHandlers(fake.clone());
    let mut dom = VirtualDom::new(root)
        .with_root_context(handlers)
        .with_root_context(dirs.clone());
    let _ = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    let page = dioxus_ssr::render(&dom);
    (dom, page)
}

#[test]
fn the_question_is_asked_while_another_app_opens_email_links() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::new(Handler::Other("Mail".to_owned()), Made::Made);
    let (_, page) = opened(Asking, &fake, &dirs(&root));
    assert!(page.contains(copy::confirm::MAKE_DEFAULT_MAIL), "{page}");
    assert_eq!(
        fake.asked(),
        0,
        "asked the system before the person answered"
    );
}

#[test]
fn make_default_asks_the_system_and_the_question_is_not_asked_again() {
    let root = tempfile::tempdir().unwrap();
    let dirs = dirs(&root);
    let fake = Fake::new(Handler::Other("Mail".to_owned()), Made::Made);
    dispatching();
    let mut dom = VirtualDom::new(Asking)
        .with_root_context(MailHandlers(fake.clone()))
        .with_root_context(dirs.clone());
    let seen = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    click(&mut dom, seen.one("id", super::ASK_MAKE_ID));
    let _ = drain_seen(&mut dom);
    assert_eq!(fake.asked(), 1, "the system was not asked");
    assert!(
        !dioxus_ssr::render(&dom).contains(copy::confirm::MAKE_DEFAULT_MAIL),
        "the question stayed"
    );
    assert_eq!(load(&dirs.state), Asked::Answered);
}

#[test]
fn not_now_is_an_answer_and_the_next_launch_does_not_ask() {
    let root = tempfile::tempdir().unwrap();
    let dirs = dirs(&root);
    let fake = Fake::new(Handler::Other("Mail".to_owned()), Made::Made);
    dispatching();
    let mut dom = VirtualDom::new(Asking)
        .with_root_context(MailHandlers(fake.clone()))
        .with_root_context(dirs.clone());
    let seen = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    click(&mut dom, seen.one("id", super::ASK_LATER_ID));
    let _ = drain_seen(&mut dom);
    assert_eq!(fake.asked(), 0, "Not Now asked the system");
    assert_eq!(load(&dirs.state), Asked::Answered);

    let (_, again) = opened(Asking, &fake, &dirs);
    assert!(
        !again.contains(copy::confirm::MAKE_DEFAULT_MAIL),
        "asked again: {again}"
    );
}

#[test]
fn nothing_is_asked_when_mailo_opens_them_or_before_there_is_an_account() {
    let root = tempfile::tempdir().unwrap();
    let dirs = dirs(&root);
    let already = Fake::new(Handler::Mailo, Made::Made);
    let (_, page) = opened(Asking, &already, &dirs);
    assert!(!page.contains(copy::confirm::MAKE_DEFAULT_MAIL), "{page}");

    let other = Fake::new(Handler::Other("Mail".to_owned()), Made::Made);
    let (_, page) = opened(FirstRun, &other, &dirs);
    assert!(!page.contains(copy::confirm::MAKE_DEFAULT_MAIL), "{page}");
    assert_eq!(
        load(&dirs.state),
        Asked::NotYet,
        "a first run spent the question"
    );
}

#[test]
fn the_question_is_not_asked_once_answered() {
    let root = tempfile::tempdir().unwrap();
    let dirs = dirs(&root);
    save(&dirs.state, Asked::Answered).unwrap();
    let fake = Fake::new(Handler::Other("Mail".to_owned()), Made::Made);
    let (_, page) = opened(Asking, &fake, &dirs);
    assert!(!page.contains(copy::confirm::MAKE_DEFAULT_MAIL), "{page}");
}

#[component]
fn Row() -> Element {
    rsx! { DefaultMailRow {} }
}

#[test]
fn the_row_names_the_app_and_make_default_turns_it_to_mailo() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::new(Handler::Other("Mail".to_owned()), Made::Made);
    dispatching();
    let mut dom = VirtualDom::new(Row)
        .with_root_context(MailHandlers(fake.clone()))
        .with_root_context(dirs(&root));
    let seen = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains(copy::fields::DEFAULT_MAIL_APP), "{page}");
    assert!(page.contains(">Mail<"), "the other app's name: {page}");
    click(&mut dom, seen.one("id", super::MAKE_DEFAULT_ID));
    let _ = drain_seen(&mut dom);
    assert_eq!(fake.asked(), 1);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(&format!(">{}<", copy::fields::THIS_APP)),
        "{page}"
    );
    assert!(!page.contains(copy::controls::MAKE_DEFAULT), "{page}");
}

#[test]
fn a_declined_request_leaves_the_row_offering_it() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::new(Handler::Other("Mail".to_owned()), Made::Kept);
    dispatching();
    let mut dom = VirtualDom::new(Row)
        .with_root_context(MailHandlers(fake.clone()))
        .with_root_context(dirs(&root));
    let seen = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    click(&mut dom, seen.one("id", super::MAKE_DEFAULT_ID));
    let _ = drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains(">Mail<"), "{page}");
    assert!(page.contains(copy::controls::MAKE_DEFAULT), "{page}");
}

#[test]
fn without_a_handler_there_is_no_row() {
    let mut dom = VirtualDom::new(Row);
    dom.rebuild_in_place();
    let page = dioxus_ssr::render(&dom);
    assert!(!page.contains(copy::fields::DEFAULT_MAIL_APP), "{page}");
}

#[test]
fn the_row_says_none_when_no_app_opens_them() {
    assert_eq!(name_of(&Handler::Nobody), copy::fields::NO_MAIL_APP);
    assert_eq!(name_of(&Handler::Mailo), copy::fields::THIS_APP);
}
