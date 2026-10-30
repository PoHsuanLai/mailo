//! macOS's [`MailHandler`]: `NSWorkspace`'s application for `mailto:`, compared with this app's
//! bundle by identifier, and `setDefaultApplicationAtURL:toOpenURLsWithScheme:` to change it,
//! which shows the system's own confirmation (macOS 12 and later) before anything changes.

use super::{Done, Handler, Made, MailHandler};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSBundle, NSError, NSFileManager, NSString, NSURL, ns_string};
use std::sync::Mutex;

/// The scheme mailo asks for.
const SCHEME: &str = "mailto";

/// This app's bundle, as the system knows it. `Send` and `Sync` because it holds only owned
/// Rust values; each call makes the Objective-C objects it needs.
#[derive(Debug)]
pub(super) struct Workspace {
    /// `io.github.PoHsuanLai.mailo`, from the bundle's Info.plist.
    identifier: String,
    /// Where `mailo.app` is.
    path: String,
}

impl Workspace {
    /// This process's bundle, when it is an application bundle with an identifier: `None` for a
    /// `mailo` binary run on its own, which the system could not open links with.
    pub(super) fn of_this_app() -> Option<Self> {
        let bundle = NSBundle::mainBundle();
        let identifier = bundle.bundleIdentifier()?.to_string();
        let path = bundle.bundleURL().path()?.to_string();
        path.ends_with(".app").then_some(Self { identifier, path })
    }
}

impl MailHandler for Workspace {
    fn current(&self) -> Handler {
        let Some(mailto) = NSURL::URLWithString(ns_string!("mailto:")) else {
            return Handler::Nobody;
        };
        let Some(app) = NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&mailto) else {
            return Handler::Nobody;
        };
        let identifier = NSBundle::bundleWithURL(&app).and_then(|found| found.bundleIdentifier());
        if identifier.is_some_and(|found| found.to_string() == self.identifier) {
            return Handler::Mailo;
        }
        Handler::Other(display_name(&app))
    }

    fn make_mailo(&self, done: Done) {
        let app = NSURL::fileURLWithPath(&NSString::from_str(&self.path));
        // The block is `Fn` and may be called on any queue; `done` is called once, the first time.
        let done = Mutex::new(Some(done));
        let block = RcBlock::new(move |error: *mut NSError| {
            let made = if error.is_null() {
                Made::Made
            } else {
                Made::Kept
            };
            let taken = done.lock().ok().and_then(|mut done| done.take());
            if let Some(done) = taken {
                done(made);
            }
        });
        NSWorkspace::sharedWorkspace()
            .setDefaultApplicationAtURL_toOpenURLsWithScheme_completionHandler(
                &app,
                &NSString::from_str(SCHEME),
                Some(&block),
            );
    }
}

/// The name Finder shows for the application at `app` ("Mail"), else its file name.
fn display_name(app: &NSURL) -> String {
    let path: Option<Retained<NSString>> = app.path();
    match path {
        Some(path) => NSFileManager::defaultManager()
            .displayNameAtPath(&path)
            .to_string(),
        None => app
            .lastPathComponent()
            .map(|name| name.to_string())
            .unwrap_or_default(),
    }
}
