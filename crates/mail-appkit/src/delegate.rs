//! `NSApplicationDelegate` for the one thing AppKit sends it that winit does not: the Dock icon
//! clicked while mailo runs. The `unsafe` is `define_class!`, which objc2 cannot expand without.
// The crate denies `unsafe_code`; objc2 cannot expand its class and URL macros without it.
#![allow(unsafe_code)]

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSApplicationDelegate};
use objc2_foundation::{NSObject, NSObjectProtocol};
use std::cell::RefCell;

type Handler = Box<dyn Fn()>;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MailoApplicationDelegate"]
    #[ivars = Handler]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn application_should_handle_reopen(
            &self,
            _application: &NSApplication,
            _has_visible_windows: bool,
        ) -> bool {
            (self.ivars())();
            // The event is ours: the callback raises the window, so AppKit need not.
            false
        }
    }
);

thread_local! {
    /// `NSApplication` keeps its delegate weakly; the main thread keeps it alive here.
    static DELEGATE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
}

/// Make `on_reopen` hear the Dock icon clicked, or the application opened again, while it runs.
/// False, changing nothing, off the main thread or when another delegate is set already. Call it
/// once the event loop exists: winit creates `NSApplication` itself, and asking for the shared one
/// earlier is not supported.
pub fn hear_reopen(on_reopen: impl Fn() + 'static) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    if app.delegate().is_some() {
        return false;
    }
    let this = Delegate::alloc(mtm).set_ivars(Box::new(on_reopen));
    let delegate: Retained<Delegate> = unsafe { msg_send![super(this), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    DELEGATE.with(|slot| *slot.borrow_mut() = Some(delegate));
    true
}
