//! The Apple event a URL arrives as, taken before the run loop starts. The one `unsafe` here is
//! `define_class!` and the untyped message to `NSAppleEventManager`.
#![allow(unsafe_code)]

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSObject};
use std::cell::RefCell;

type Handler = Box<dyn Fn(String)>;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MailoUrlHandler"]
    #[ivars = Handler]
    struct UrlHandler;

    impl UrlHandler {
        /// A URL the system opens with this application: `open mailto:…`, a click on a link while
        /// mailo is the mail handler. AppKit would hand it to `application:openURLs:` for a
        /// delegate that had one at launch; taking the event itself works before any delegate does.
        #[unsafe(method(handleGetURLEvent:withReplyEvent:))]
        fn handle_get_url_event(
            &self,
            event: &NSAppleEventDescriptor,
            _reply: &NSAppleEventDescriptor,
        ) {
            let url: Option<Retained<NSAppleEventDescriptor>> =
                unsafe { msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT] };
            if let Some(url) = url.and_then(|descriptor| descriptor.stringValue()) {
                (self.ivars())(url.to_string());
            }
        }
    }
);

/// `kInternetEventClass` and `kAEGetURL`, both `'GURL'`; `keyDirectObject`, `'----'`.
const INTERNET_EVENT_CLASS: u32 = u32::from_be_bytes(*b"GURL");
const GET_URL: u32 = u32::from_be_bytes(*b"GURL");
const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

thread_local! {
    /// The event manager keeps its handler unretained; the main thread keeps it alive here.
    static HANDLER: RefCell<Option<Retained<UrlHandler>>> = const { RefCell::new(None) };
}

/// Make `on_url` hear each URL the system opens with this application, as an absolute string, on
/// the main thread's run loop. Call it before the event loop is made: a URL that launched the
/// application is delivered when the loop first runs, to whoever has registered by then. False,
/// changing nothing, off the main thread or when one is registered already.
pub fn hear_urls(on_url: impl Fn(String) + 'static) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    if HANDLER.with(|slot| slot.borrow().is_some()) {
        return false;
    }
    let this = UrlHandler::alloc(mtm).set_ivars(Box::new(on_url));
    let handler: Retained<UrlHandler> = unsafe { msg_send![super(this), init] };
    let manager = NSAppleEventManager::sharedAppleEventManager();
    unsafe {
        let _: () = msg_send![
            &*manager,
            setEventHandler: &*handler,
            andSelector: sel!(handleGetURLEvent:withReplyEvent:),
            forEventClass: INTERNET_EVENT_CLASS,
            andEventID: GET_URL,
        ];
    }
    HANDLER.with(|slot| *slot.borrow_mut() = Some(handler));
    true
}
