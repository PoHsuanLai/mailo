//! The window under test, opened one way in every native test.
//!
//! Each test file used to build its own `HarnessConfig` over `mail_ui::ui::native::contexts`,
//! and the copies differed in a handful of choices: the viewport, the root component, the
//! directories, the clock, what else the window is handed, and what the first frame waits for.
//! Those are the fields of [`Open`]; the rest is written here once.

use crate::settle::settle_until;
use dioxus::prelude::Element;
use ds_blitz::{FocusFallback, NetPolicy, PrintOutcome, RootContexts};
use ds_harness::{Clock, Harness, HarnessConfig, Viewport};
use mail_core::SqliteStore;
use mail_ui_kit::appearance::WindowDirs;
use std::sync::Arc;
use std::time::Duration;

/// The viewport most tests draw in.
pub const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// What [`open_window`] waits for before it hands the window back.
#[derive(Clone, Copy)]
pub enum Wait {
    /// The entrances quire runs on a frame's wait after mount.
    Entrances,
    /// The entrances, then at least one conversation.
    SomeList,
}

/// How a window is opened. The defaults are the main window over a store, on the virtual clock,
/// with its first conversations drawn.
pub struct Open<'a> {
    pub view: Viewport,
    pub root: fn() -> Element,
    pub spaces: Option<mail_ui_kit::space::Spaces>,
    pub dirs: Option<WindowDirs>,
    pub start: mail_ui::ui::ask::Open,
    pub fallback: FocusFallback,
    pub clock: Clock,
    /// Handed to the window besides the client: a recorder, say.
    pub with: Option<Box<dyn FnOnce(RootContexts) -> RootContexts + 'a>>,
    /// The window's print dialog; `None` is one that cancels, so no test opens the system's.
    pub printer: Option<mail_ui::ui::native::Printer>,
    pub wait: Wait,
}

impl Default for Open<'_> {
    fn default() -> Self {
        Open {
            view: VIEW,
            root: mail_ui::ui::native::root,
            spaces: None,
            dirs: None,
            start: mail_ui::ui::ask::Open::raise(),
            fallback: FocusFallback::default(),
            clock: Clock::Virtual,
            with: None,
            printer: None,
            wait: Wait::SomeList,
        }
    }
}

/// A window over `store`, as `how` says, its first frame drawn.
pub fn open_window(store: &Arc<SqliteStore>, how: Open) -> Harness {
    let printer = how.printer.unwrap_or_else(|| {
        mail_ui::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled))
    });
    let mut contexts = mail_ui::ui::native::contexts(
        mail_client::Client::open(Arc::clone(store), mail_client::ClientConfig::default()),
        mail_ui_kit::view::Appearance::default(),
        how.spaces,
        how.dirs,
        how.start,
    )
    .with(printer);
    if let Some(with) = how.with {
        contexts = with(contexts);
    }
    let config = HarnessConfig::new(how.view)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(how.fallback)
        .with_clock(how.clock)
        .with_contexts(contexts);
    let mut harness = Harness::new(how.root, config);
    match how.wait {
        Wait::Entrances => harness.advance(Duration::from_millis(300)),
        Wait::SomeList => {
            harness.advance(Duration::from_millis(300));
            settle_until(&mut harness, |h| h.count(".list .ds-thread") > 0);
        }
    }
    harness
}
