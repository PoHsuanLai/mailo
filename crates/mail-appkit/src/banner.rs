//! Banners: the notification centre's `NSUserNotification`, posted and heard in one place.
//!
//! One owner for both halves is the point. A banner is posted and forgotten (no thread waits on
//! it, nothing polls the centre for it to go), and the one delegate, installed once by
//! [`hear_banners`], hears every click on every banner this application has on screen, including
//! one a sibling process (`mailo watch`) posted, because what a click needs travels in the
//! banner's own `userInfo` and not in a table that process keeps. The `unsafe` is
//! `define_class!` and the untyped messages the deprecated API's bindings leave out.
// The crate denies `unsafe_code`; objc2 cannot expand its class macro without it. The
// notification centre API is deprecated in favour of `UserNotifications`, which needs a signed,
// bundled application and asynchronous authorisation; it still works, and is what mailo uses.
#![allow(unsafe_code, deprecated)]

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, ClassType, DefinedClass, define_class, msg_send};
use objc2_foundation::{
    NSArray, NSBundle, NSDictionary, NSObject, NSObjectProtocol, NSString, NSUserNotification,
    NSUserNotificationAction, NSUserNotificationActivationType, NSUserNotificationCenter,
    NSUserNotificationCenterDelegate, NSUserNotificationDefaultSoundName,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// The name by which a [`Banner`] asks for the system's own notification sound.
pub const DEFAULT_SOUND: &str = "NSUserNotificationDefaultSoundName";

/// A banner to post.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Banner {
    pub title: String,
    pub body: String,
    /// A system sound's name (or [`DEFAULT_SOUND`]); `None` for none.
    pub sound: Option<String>,
    /// The title of the banner's main button, if it has one.
    pub main_button: Option<String>,
    /// Further buttons, `(id, title)`, offered with the main one.
    pub more_buttons: Vec<(String, String)>,
    /// Text that comes back, as it was, with the click: how a click says what it was about.
    pub info: Vec<(String, String)>,
}

/// What the person pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pressed {
    /// The banner itself.
    Content,
    /// Its main button.
    Main,
    /// One of the further buttons, by its id.
    More(String),
    /// Anything else (an inline reply, or nothing the banner offered).
    Other,
}

/// A click on a banner: what was pressed, and the `info` the banner was posted with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    pub pressed: Pressed,
    pub info: HashMap<String, String>,
}

/// The notification centre, if this process has one: an application with no bundle identifier
/// (a bare binary run from a terminal) is not one the centre can name, and gets `None`.
fn center() -> Option<Retained<NSUserNotificationCenter>> {
    NSBundle::mainBundle().bundleIdentifier()?;
    unsafe {
        msg_send![
            NSUserNotificationCenter::class(),
            defaultUserNotificationCenter
        ]
    }
}

/// Post `banner`, and do not wait for it. Any thread. False when there is no centre to post to.
pub fn show(banner: &Banner) -> bool {
    let Some(center) = center() else {
        return false;
    };
    center.deliverNotification(&built(banner));
    true
}

fn built(banner: &Banner) -> Retained<NSUserNotification> {
    let note = NSUserNotification::new();
    note.setTitle(Some(&NSString::from_str(&banner.title)));
    note.setInformativeText(Some(&NSString::from_str(&banner.body)));
    match banner.sound.as_deref() {
        None | Some("") => {}
        Some(DEFAULT_SOUND) => {
            note.setSoundName(Some(unsafe { NSUserNotificationDefaultSoundName }))
        }
        Some(name) => note.setSoundName(Some(&NSString::from_str(name))),
    }
    match &banner.main_button {
        Some(title) => {
            note.setHasActionButton(true);
            note.setActionButtonTitle(&NSString::from_str(title));
        }
        None => note.setHasActionButton(false),
    }
    if !banner.more_buttons.is_empty() {
        let actions: Vec<Retained<NSUserNotificationAction>> = banner
            .more_buttons
            .iter()
            .map(|(id, title)| {
                NSUserNotificationAction::actionWithIdentifier_title(
                    Some(&NSString::from_str(id)),
                    Some(&NSString::from_str(title)),
                )
            })
            .collect();
        note.setAdditionalActions(Some(&NSArray::from_retained_slice(&actions)));
    }
    if !banner.info.is_empty() {
        let keys: Vec<Retained<NSString>> = banner
            .info
            .iter()
            .map(|(key, _)| NSString::from_str(key))
            .collect();
        let values: Vec<Retained<NSString>> = banner
            .info
            .iter()
            .map(|(_, value)| NSString::from_str(value))
            .collect();
        let key_refs: Vec<&NSString> = keys.iter().map(|key| &**key).collect();
        let info = NSDictionary::from_retained_objects(&key_refs, &values);
        // SAFETY: a dictionary of strings is a dictionary of objects; `userInfo` takes a property
        // list, which this is.
        unsafe {
            note.setUserInfo(Some(&*Retained::cast_unchecked::<
                NSDictionary<NSString, AnyObject>,
            >(info)));
        }
    }
    note
}

type Handler = Box<dyn Fn(Activation) + Send + Sync>;

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MailoBannerDelegate"]
    #[ivars = Handler]
    struct BannerDelegate;

    unsafe impl NSObjectProtocol for BannerDelegate {}

    unsafe impl NSUserNotificationCenterDelegate for BannerDelegate {
        #[unsafe(method(userNotificationCenter:didActivateNotification:))]
        fn did_activate(&self, center: &NSUserNotificationCenter, note: &NSUserNotification) {
            (self.ivars())(activation(note));
            // A banner that was answered has nothing left to say in the centre.
            center.removeDeliveredNotification(note);
        }

        #[unsafe(method(userNotificationCenter:shouldPresentNotification:))]
        fn should_present(
            &self,
            _center: &NSUserNotificationCenter,
            _note: &NSUserNotification,
        ) -> bool {
            // Banners show while mailo is frontmost too, as they always have.
            true
        }
    }
);

fn activation(note: &NSUserNotification) -> Activation {
    let pressed = match note.activationType() {
        NSUserNotificationActivationType::ContentsClicked => Pressed::Content,
        NSUserNotificationActivationType::ActionButtonClicked => Pressed::Main,
        NSUserNotificationActivationType::AdditionalActionClicked => note
            .additionalActivationAction()
            .and_then(|action| action.identifier())
            .map_or(Pressed::Other, |id| Pressed::More(id.to_string())),
        _ => Pressed::Other,
    };
    let mut info = HashMap::new();
    if let Some(dictionary) = note.userInfo() {
        for key in dictionary.allKeys().to_vec() {
            let value = dictionary.objectForKey(&key);
            // Only strings were put in (`built`); anything else is not ours and is skipped.
            let text = value.and_then(|value| value.downcast::<NSString>().ok());
            if let Some(text) = text {
                info.insert(key.to_string(), text.to_string());
            }
        }
    }
    Activation { pressed, info }
}

/// Make `on_click` hear every click on a banner of this application, on whichever thread the
/// centre delivers it. Once: false, changing nothing, when a delegate was installed already or
/// there is no centre. Call it only in a process that runs the main run loop, where a click is
/// delivered; elsewhere nothing ever calls back.
pub fn hear_banners(on_click: impl Fn(Activation) + Send + Sync + 'static) -> bool {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    let Some(center) = center() else {
        return false;
    };
    if INSTALLED.swap(true, Ordering::AcqRel) {
        return false;
    }
    let this = BannerDelegate::alloc().set_ivars(Box::new(on_click));
    let delegate: Retained<BannerDelegate> = unsafe { msg_send![super(this), init] };
    // SAFETY: the centre holds its delegate unretained, so it is kept alive for the process:
    // installed once, never replaced, and leaked on purpose.
    unsafe { center.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    std::mem::forget(delegate);
    true
}
