//! The count on the macOS Dock tile: `NSApp.dockTile.badgeLabel`.
//!
//! AppKit may only be touched on the main thread. A count said there (a window's document runs on
//! it: winit's loop, which quire's `launch` runs, is the main thread's run loop on macOS) is drawn
//! at once; one said on another thread (mailo.app's own count, kept by a thread of its own while
//! no window is open) is handed to the main queue, which that run loop drains, and drawn there.
//! Nothing unsafe is done either way. Zero clears the badge. The Dock drops it when mailo quits.

use super::{Badge, Unread};
use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use objc2_foundation::NSString;

/// The Dock tile as a [`Badge`].
pub(super) struct Dock;

impl Badge for Dock {
    fn show(&self, unread: Unread) {
        let label = super::label(unread);
        match MainThreadMarker::new() {
            Some(main) => draw(main, label.as_deref()),
            None => dispatch2::DispatchQueue::main().exec_async(move || {
                if let Some(main) = MainThreadMarker::new() {
                    draw(main, label.as_deref());
                }
            }),
        }
    }
}

/// Put `label` on the Dock tile, or clear it.
fn draw(main: MainThreadMarker, label: Option<&str>) {
    let label = label.map(NSString::from_str);
    NSApplication::sharedApplication(main)
        .dockTile()
        .setBadgeLabel(label.as_deref());
}
