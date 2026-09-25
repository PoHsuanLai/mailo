//! A link clicked inside the Original frame on Blitz: quire's `FrameLinks::Intercept`.
//!
//! The frame never navigates (ds-native gives every frame document a navigation provider that
//! only reports the click), so a stranger's page is never loaded into the reader. The click
//! reaches mailo, which opens the address in the system browser, the way a link in the Reader
//! view opens: `http`, `https` and `mailto` only, through the same check the Reader view's links
//! are built with ([`mail_mime::SafeUrl`]: no other scheme, no control or bidi characters).
//! Anything else (`file:`, `javascript:`, a `data:` document) does nothing.

use ds_native::{FrameLink, FrameLinks};
use mail_mime::SafeUrl;
use std::sync::Arc;

/// Whatever opens a link: the system browser in the window, a recorder in a test.
pub trait Browse: Send + Sync + 'static {
    /// Open `url`, already checked.
    fn open(&self, url: &str);
}

/// The system browser, as the Add account sheet's sign-in opens it.
pub(crate) struct System;

impl Browse for System {
    fn open(&self, url: &str) {
        // Best effort, as a click on a link in a browser is: nothing waits on it.
        let _ = webbrowser::open(url);
    }
}

/// What a click in any frame does: `browse` opens it, if it is a link a reader may follow.
pub(crate) fn frame_links(browse: Arc<dyn Browse>) -> FrameLinks {
    FrameLinks::intercept(move |link: FrameLink| {
        if let Some(url) = SafeUrl::parse(&link.href) {
            browse.open(url.as_str());
        }
    })
}
