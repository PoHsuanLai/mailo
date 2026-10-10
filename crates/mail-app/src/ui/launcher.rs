//! The unread count on the app's launcher: the Dash or dock on Linux, the Dock on macOS (item 20).
//!
//! Two questions, kept apart. *What* is counted is [`unread`], a query over the store: the
//! conversations in the inbox that are unread and not snoozed away, in the accounts of the Space
//! the window shows. *Where* it goes is a seam ([`Badge`]): the platform's launcher in the window
//! ([`platform`]), a recorder in the tests, which must never reach a real desktop.
//!
//! Why the Space and not every account: a Space is the user saying which accounts they are
//! attending to now, and the window's inbox lists exactly those. A launcher that counted an
//! account the Space leaves out would announce mail the window, opened, would not show. A Space
//! of every account (the default) counts every account. A pressed account tile is not a Space; it
//! is a look at one account for a moment, and the count does not follow it.
//!
//! Why conversations and not messages: the list, the sidebar's badges and the notifications all
//! speak in conversations, and a badge of 7 that opens on three unread rows reads as a bug.
//!
//! What each platform shows:
//! - Linux and the BSDs: the `com.canonical.Unity.LauncherEntry` signal on the session bus
//!   ([`unity`]), which GNOME's Dash to Dock and Ubuntu Dock, KDE's task manager and Plank read.
//!   GNOME's own Dash draws no count.
//! - macOS: the Dock tile's badge (`dock`).
//! - Windows: nothing yet. The taskbar's overlay icon is `ITaskbarList3::SetOverlayIcon`, a COM
//!   call on the window's handle, which only `unsafe` code can make and which the window's owner
//!   (quire's launch loop) holds.

pub mod unity;

#[cfg(not(any(target_os = "macos", windows)))]
mod session;

#[cfg(target_os = "macos")]
mod dock;

use super::view::{Source, badge_filter};
use chrono::{DateTime, Utc};
use mail_core::Store;
use mail_core::place::place_filter;
use mail_domain::{Filter, MailboxRole};
use std::sync::Arc;

/// How many unread conversations the inbox holds, in the accounts being counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Unread(pub u64);

/// The conversations the launcher counts, in the accounts of `scope`.
///
/// The inbox's own badge filter (`view::badge_filter` of the inbox place), so the launcher and the
/// sidebar's Inbox badge cannot disagree about what "unread in the inbox" means: snoozed
/// conversations are away, and read ones are read.
pub fn filter(scope: &crate::ui::space::Scope) -> Filter {
    let inbox = Source::Mail(place_filter(MailboxRole::Inbox));
    // `badge_filter` answers `None` only for Drafts, which the inbox is not.
    let unread = badge_filter(&inbox).unwrap_or(Filter::Nothing);
    match scope.filter() {
        None => unread,
        Some(accounts) => Filter::And(vec![accounts, unread]),
    }
}

/// Count them. One indexed query (`Store::count`), made off the thread that draws.
pub fn unread<S: Store + ?Sized>(
    store: &S,
    scope: &crate::ui::space::Scope,
    now: DateTime<Utc>,
) -> Result<Unread, mail_core::StoreError> {
    store.count(&filter(scope), now).map(Unread)
}

/// The text of a badge that draws its own label (the macOS Dock's): the number, or nothing for
/// zero, since an empty inbox is not news.
pub fn label(unread: Unread) -> Option<String> {
    (unread.0 > 0).then(|| unread.0.to_string())
}

/// Where the count is shown.
///
/// A seam with two sides: the platform's launcher, and a recorder in the tests. Infallible on
/// purpose, like `notify::Notifier`: a dock that cannot be reached is not the window's problem,
/// and there is nobody to tell but the terminal. Called on the thread that draws, so an
/// implementation that does I/O hands it to a thread of its own.
pub trait Badge: Send + Sync {
    fn show(&self, unread: Unread);
}

/// The window's launcher, a root context. A window without one (every test that does not hand
/// it a recorder) counts nothing and shows nothing.
#[derive(Clone)]
pub struct Launcher(pub Arc<dyn Badge>);

impl std::fmt::Debug for Launcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Launcher(..)")
    }
}

/// This platform's launcher, or `None` where mailo cannot show a count.
///
/// Starts nothing that can fail: on Linux the session bus is reached on a thread of its own when
/// there is first something to say, and a desktop with no bus, or no dock listening, costs one
/// line on the terminal.
pub fn platform() -> Option<Launcher> {
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let id = std::env::var("FLATPAK_ID").ok();
        let desktop = unity::desktop_id(id.as_deref()).to_owned();
        Some(Launcher(Arc::new(unity::Unity::new(
            desktop,
            session::Session::start(),
        ))))
    }
    #[cfg(target_os = "macos")]
    {
        Some(Launcher(Arc::new(dock::Dock)))
    }
    #[cfg(windows)]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_is_the_number_and_zero_is_none() {
        const CASES: &[(u64, Option<&str>)] = &[(0, None), (1, Some("1")), (1234, Some("1234"))];
        for &(unread, expect) in CASES {
            assert_eq!(label(Unread(unread)).as_deref(), expect, "{unread} unread");
        }
    }
}
