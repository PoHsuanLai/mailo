//! The count on the macOS Dock tile: `NSApp.dockTile.badgeLabel`.
//!
//! AppKit may only be touched on the main thread. The window's document runs there (winit's
//! loop, which quire's `launch` runs, is the main thread's run loop on macOS), and [`Badge::show`]
//! is called from it; when it is not, `MainThreadMarker::new` says so and nothing is drawn rather
//! than anything unsafe being done. Zero clears the badge. The Dock drops it when mailo quits.

use super::{Badge, Unread};
use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use objc2_foundation::NSString;

/// The Dock tile as a [`Badge`].
pub(super) struct Dock;

impl Badge for Dock {
    fn show(&self, unread: Unread) {
        let Some(main) = MainThreadMarker::new() else {
            return;
        };
        let label = super::label(unread).map(|text| NSString::from_str(&text));
        NSApplication::sharedApplication(main)
            .dockTile()
            .setBadgeLabel(label.as_deref());
    }
}
