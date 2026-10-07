//! The Settings window beside the lane's main window: the same store, the same file dialog and
//! save directory, and the same revision, so what it changes the main window hears.

use ds_blitz::{FocusFallback, PrintOutcome, RootContexts};
use ds_harness::{Clock, Harness, HarnessConfig, Query, Viewport};

use super::hands::{click, until};

/// The size the Settings window opens at.
const VIEW: Viewport = Viewport {
    width: 780,
    height: 720,
    scale_percent: 100,
};

/// The Settings window over `contexts`, on `page` (a sidebar entry's name).
pub fn open_at(contexts: RootContexts, page: &str) -> Harness {
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let config = HarnessConfig::new(VIEW)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_net(ds_blitz::NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts.with(printer));
    let mut harness = Harness::new(mail_app::ui::native::settings_root, config);
    let entry = format!(".ds-sidebar [*|aria-label=\"{page}\"]");
    until(&mut harness, "Settings draws its sidebar", |h| {
        h.count(&entry) == 1
    });
    click(&mut harness, &entry);
    let shown = format!(".settings-page[*|data-page=\"{page}\"]");
    let what = format!("Settings shows {page}");
    until(&mut harness, &what, |h| h.count(&shown) == 1);
    harness
}
