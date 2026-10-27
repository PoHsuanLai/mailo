//! A copy that clipboard history and cloud sync do not keep: a one-time code.
//!
//! quire's clipboard (`ds_blitz::clipboard`) writes plain text, which a clipboard manager, the
//! desktop's history and a synced clipboard all keep, so a code copied there outlives its use
//! (and is shown to anyone who opens the history). The window's Copy Code goes through here
//! instead, to the system clipboard through `arboard` with the platform's own exclusion set:
//! `exclude_from_history` on Linux and macOS, and on Windows also `exclude_from_cloud` and
//! `exclude_from_monitoring`. gap(quire): a `write_sensitive` on the clipboard would make this
//! one write.
//!
//! What copies is a [`SensitiveCopy`] seam, like [`ClipboardImage`](super::pasted_image): a test
//! hands the window its own (a root context, built with [`SensitiveCopy::with_sink`] from outside
//! the crate), and in this crate's tests the real one copies nothing, so no test writes the
//! desktop's clipboard.

use dioxus::prelude::*;
use std::sync::Arc;

/// What puts a code on the clipboard.
type Sink = dyn Fn(&str) + Send + Sync;

/// Whatever copies a sensitive text, as a root context.
#[derive(Clone)]
pub struct SensitiveCopy(Arc<Sink>);

impl std::fmt::Debug for SensitiveCopy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SensitiveCopy")
    }
}

impl SensitiveCopy {
    /// A copy that hands the text to `sink`. For tests: no real clipboard.
    pub fn with_sink(sink: impl Fn(&str) + Send + Sync + 'static) -> Self {
        SensitiveCopy(Arc::new(sink))
    }

    /// The desktop's clipboard. In this crate's tests, one that copies nothing.
    fn real() -> Self {
        if cfg!(test) {
            return SensitiveCopy(Arc::new(|_| {}));
        }
        SensitiveCopy(Arc::new(copy_system))
    }
}

/// Put `text` on the clipboard as something history and sync leave out.
pub(in crate::ui) fn copy(text: &str) {
    let copier = try_consume_context::<SensitiveCopy>().unwrap_or_else(SensitiveCopy::real);
    (copier.0)(text);
}

/// Copy on a thread of its own. On Linux the clipboard is served by the program that set it, for
/// as long as it holds it, so the thread stays until another program takes the clipboard (a
/// later copy among them); elsewhere the system keeps what was set and the thread ends at once.
/// A clipboard that cannot be reached is said in the log.
fn copy_system(text: &str) {
    let text = text.to_owned();
    let spawned = std::thread::Builder::new()
        .name("mailo-clipboard".to_owned())
        .spawn(move || {
            if let Err(e) = put(text) {
                log::warn!("a code could not be copied: {e}");
            }
        });
    if let Err(e) = spawned {
        log::warn!("a code could not be copied: {e}");
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn put(text: String) -> Result<(), arboard::Error> {
    use arboard::SetExtLinux as _;
    arboard::Clipboard::new()?
        .set()
        .exclude_from_history()
        .wait()
        .text(text)
}

#[cfg(target_os = "macos")]
fn put(text: String) -> Result<(), arboard::Error> {
    use arboard::SetExtApple as _;
    arboard::Clipboard::new()?
        .set()
        .exclude_from_history()
        .text(text)
}

#[cfg(windows)]
fn put(text: String) -> Result<(), arboard::Error> {
    use arboard::SetExtWindows as _;
    arboard::Clipboard::new()?
        .set()
        .exclude_from_history()
        .exclude_from_cloud()
        .exclude_from_monitoring()
        .text(text)
}
