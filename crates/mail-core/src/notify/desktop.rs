//! Notifications through the platform's own notification service: by `notify-rust`, the
//! freedesktop.org `org.freedesktop.Notifications` interface on the session bus on Linux and the
//! BSDs and toasts on Windows; by the app's `Posts`, the notification centre on macOS.
//!
//! Silent when there is nothing to talk to. A headless machine, an SSH session and a test run
//! have no session bus, and a watch that fetches mail must not stop, or even complain on every
//! pass, because there is nowhere to show a bubble.
//!
//! A click opens what the notification was about: `mailo open <thread>` for one
//! conversation, `mailo open --inbox <account>` for a summary's
//! account's inbox, started with the activation token the server minted
//! for the click so the compositor lets the window take focus. On Linux one listener for the whole
//! process hears the server's `ActivationToken`, `ActionInvoked` and `NotificationClosed`
//! ([`super::click`]); on macOS one delegate for the whole process hears every click, and what a
//! click needs rides in the banner itself ([`super::banner`]); on Windows each notification shown
//! is waited on by the thread that showed it, and what the service answers goes through
//! [`answer`], a pure function of the answer and what the notification opens. `mailo open` hands
//! the thread to a window already running, if there is one, before it opens one of its own.
//!
//! On Linux, what a click opens also rides along in the hints, for any other listener:
//! `x-mailo-thread` (a thread id, absent on a summary, which opens the inbox) and
//! `x-mailo-account`, beside `category` (`email.arrived`) and `desktop-entry` (`org.quire.Mail`).
//! The one action offered is `default`, the key a server invokes when the bubble itself is clicked.
//!
//! What each platform does not do:
//! - macOS: only the window hears a click (the app installs the delegate there and hands what it
//!   hears to [`heard`]): the click is delivered on the application's main run loop, which winit's is, and
//!   `mailo watch` is a command in a terminal with none. A banner is posted and forgotten: no
//!   thread waits on it and nothing polls the centre for it to go (waiting on each, as
//!   `notify-rust` does, costs a thread and a timer that reads every delivered banner twice a
//!   second for as long as one stays up, which for a banner nobody clears is for ever). A
//!   banner the watch shows opens its conversation when clicked all the same, if the window is
//!   running, which hears it. The banner names this application, so a program with no bundle
//!   identifier (a bare binary run from a terminal) shows none.
//! - Windows: toasts are shown as Windows PowerShell's. A toast names its sender by an
//!   application user model id, and one that no Start menu shortcut registers shows nothing at
//!   all, so the registered PowerShell id is the one that always works. No activation token: it
//!   is a Wayland and X11 notion.

use super::{Act, Notification, Notifier, Opens, Sound};
#[cfg(windows)]
use notify_rust::NotificationResponse;
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(all(unix, not(target_os = "macos")))]
use super::click::{Clicked, Clicks};

/// What hears a click: on Linux the one listener, on Windows a bound on the threads that wait. On
/// macOS the delegate the app holds, which nothing here keeps a handle to.
#[cfg(all(unix, not(target_os = "macos")))]
type Hearing = Clicks;
#[cfg(windows)]
type Hearing = Arc<Waiting>;

/// The platform's notification service, if there is one to reach.
#[derive(Debug, Clone)]
pub struct Desktop {
    reach: Reach,
    #[cfg(not(target_os = "macos"))]
    hearing: Hearing,
    /// macOS: what posts the banners, which is the app's.
    #[cfg(target_os = "macos")]
    banners: std::sync::Arc<dyn super::banner::Posts>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    Service,
    Nowhere,
}

impl Desktop {
    /// Find the notification service, or remember that there is none.
    #[cfg(not(target_os = "macos"))]
    pub fn connect() -> Self {
        let reach = reach();
        Desktop {
            reach,
            hearing: hearing(reach),
        }
    }

    /// The notification centre, posted to by `banners`. Whoever should hear the clicks installs
    /// the delegate for them and hands each to [`heard`]; a process with no run loop to deliver
    /// them on (`mailo watch`) posts all the same.
    #[cfg(target_os = "macos")]
    pub fn connect(banners: std::sync::Arc<dyn super::banner::Posts>) -> Self {
        Desktop {
            reach: reach(),
            banners,
        }
    }
}

/// Listen for clicks when there is a server to click on.
#[cfg(all(unix, not(target_os = "macos")))]
fn hearing(reach: Reach) -> Hearing {
    match reach {
        Reach::Service => Clicks::start(|Clicked { opens, act, token }| {
            start(Launch { opens, act }, token.as_deref());
        }),
        Reach::Nowhere => Clicks::default(),
    }
}

#[cfg(windows)]
fn hearing(_: Reach) -> Hearing {
    Arc::default()
}

/// A click on a banner, from the one delegate: start what it asks for.
#[cfg(target_os = "macos")]
pub fn heard(click: super::banner::Click) {
    use super::banner::Pressed;
    let action = match &click.pressed {
        Pressed::Content | Pressed::Main => Some(super::banner::CONTENT),
        Pressed::More(id) => Some(id.as_str()),
        Pressed::Other => None,
    };
    if let Some(launch) = super::banner::launch(action, &click.info) {
        start(launch, None);
    }
}

/// On Linux, whether a notification server answers on the session bus. Asked once, off any
/// runtime the caller is driving, like every blocking zbus call (`mail_runtime::off_runtime`).
#[cfg(all(unix, not(target_os = "macos")))]
fn reach() -> Reach {
    match mail_runtime::off_runtime(notify_rust::get_server_information) {
        Ok(_) => Reach::Service,
        Err(_) => Reach::Nowhere,
    }
}

/// On macOS, the notification centre is always there.
#[cfg(target_os = "macos")]
fn reach() -> Reach {
    Reach::Service
}

#[cfg(windows)]
fn reach() -> Reach {
    Reach::Service
}

/// Wait for what the user does with `$shown`, and start the window if they clicked it.
///
/// A macro rather than a function because `notify-rust` does not export the toast handle's type
/// by name.
#[cfg(windows)]
macro_rules! wait_on {
    ($shown:expr, $notification:expr, $waiting:expr) => {{
        // Past the bound the notification is still shown; only its click opens nothing.
        if let Some(_place) = $waiting.enter() {
            let mut clicked = None;
            // A notification closed without an answer is a click nobody made.
            let _ = $shown.wait_for_response(|response: &NotificationResponse| {
                clicked = answer(response, &$notification.opens, $notification.reply);
            });
            if let Some(launch) = clicked {
                start(launch, None);
            }
        }
    }};
}

impl Notifier for Desktop {
    fn show(&self, notification: &Notification) {
        self.show_with(notification, Sound::default());
    }
}

impl Desktop {
    /// [`show`](Notifier::show) with `sound`: what a notifier that reads the person's settings as
    /// each is said ([`super::Interest`]) chooses.
    #[cfg(not(target_os = "macos"))]
    pub fn show_with(&self, notification: &Notification, sound: Sound) {
        if self.reach == Reach::Nowhere {
            return;
        }
        let notification = notification.clone();
        let hearing = self.hearing.clone();
        // On a thread of its own, because a notification service that is slow to answer — or
        // registered and wedged — holds the call for its whole timeout, and this is called
        // between passes of a loop that fetches mail for every account on one thread. On Windows
        // the same thread then waits for the click.
        std::thread::spawn(move || {
            let Ok(shown) = built(&notification, sound).show() else {
                return;
            };
            #[cfg(all(unix, not(target_os = "macos")))]
            hearing.show(shown.id(), notification.opens.clone(), notification.reply);
            #[cfg(windows)]
            wait_on!(shown, notification, hearing);
        });
    }

    /// [`show`](Notifier::show) with `sound`: what a notifier that reads the person's settings as
    /// each is said ([`super::Interest`]) chooses.
    #[cfg(target_os = "macos")]
    pub fn show_with(&self, notification: &Notification, sound: Sound) {
        if self.reach == Reach::Nowhere {
            return;
        }
        // Posted, not waited on: the click is heard by the delegate, from what the banner carries.
        // No bundle identifier means no centre to post to, and nothing to say about that.
        self.banners.post(&banner(notification, sound));
    }
}

/// At most this many notifications are waited on for a click at once, one thread each. A service
/// should say when each one closes, but one that never does must not make a week-long watch grow
/// a thread for every notification it ever showed; past the bound a notification is still shown,
/// and a click on it opens nothing until an earlier one closes.
#[cfg(windows)]
pub const REMEMBERED: usize = 64;

/// A count of the notifications being waited on, bounded by [`REMEMBERED`].
#[cfg(windows)]
#[derive(Debug, Default)]
pub struct Waiting(AtomicUsize);

/// One place among the [`Waiting`], given back when it is dropped.
#[cfg(windows)]
#[derive(Debug)]
pub struct Place<'a>(&'a Waiting);

#[cfg(windows)]
impl Waiting {
    /// A place to wait in, or `None` when [`REMEMBERED`] are already waiting.
    pub fn enter(&self) -> Option<Place<'_>> {
        let mut held = self.0.load(Ordering::Acquire);
        loop {
            if held >= REMEMBERED {
                return None;
            }
            match self
                .0
                .compare_exchange_weak(held, held + 1, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Some(Place(self)),
                Err(now) => held = now,
            }
        }
    }

    pub fn len(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(windows)]
impl Drop for Place<'_> {
    fn drop(&mut self) {
        self.0.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A program to start for a click, and what it does: a window where it opens, or a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub opens: Opens,
    pub act: Act,
}

impl Launch {
    /// The arguments that start the program: `open <thread>`, or `open --inbox <account>` for a
    /// summary's account's inbox, `open --archive <thread>` for the Archive button, and
    /// `open --reply <thread>` for Reply: each button runs in the window, where its result is
    /// seen and an Undo is offered.
    pub fn args(&self) -> Vec<String> {
        match (&self.act, &self.opens) {
            // The archive the window does, with its toast and Undo; a failure is the window's to say.
            (Act::Archive, Opens::Thread(thread)) => {
                vec![
                    "open".to_owned(),
                    "--archive".to_owned(),
                    thread.to_string(),
                ]
            }
            // The reply to the conversation the banner is about, in the account it is in: a
            // `mailto:` link names neither.
            (Act::Reply, Opens::Thread(thread)) => {
                vec!["open".to_owned(), "--reply".to_owned(), thread.to_string()]
            }
            (Act::Open, Opens::Thread(thread)) => {
                vec!["open".to_owned(), thread.to_string()]
            }
            (Act::Open, Opens::Inbox(account)) => vec![
                "open".to_owned(),
                "--inbox".to_owned(),
                account.as_str().to_owned(),
            ],
            (Act::Archive | Act::Reply, Opens::Inbox(_)) => Vec::new(),
        }
    }
}

/// What follows from the service's answer about a notification that opens `opens` and offers
/// a Reply button when `reply`: a program to start for a click on the notification or on a button
/// it offered, and nothing for a close or for any other action.
#[cfg(windows)]
pub fn answer(response: &NotificationResponse, opens: &Opens, reply: bool) -> Option<Launch> {
    let act = match response {
        NotificationResponse::Default => Some(Act::Open),
        NotificationResponse::Action(action) => super::act_for(action, opens, reply),
        NotificationResponse::Reply(_) | NotificationResponse::Closed(_) => None,
    }?;
    Some(Launch {
        opens: opens.clone(),
        act,
    })
}

/// Start this same binary as the window, and leave it running.
///
/// `token` is the activation token the server minted for the click: handed on in the
/// environment under both names a toolkit reads (`XDG_ACTIVATION_TOKEN`, and the older
/// `DESKTOP_STARTUP_ID`), it is how the compositor knows the new window was asked for by a click
/// and may take focus. The child is waited for on a thread of its own, so it is neither held up
/// nor left a zombie. A window that cannot be started is a click that did nothing, with nobody to
/// tell but the watch's terminal.
fn start(launch: Launch, token: Option<&str>) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            log::warn!("notification click: cannot find this program to start the window: {e}");
            return;
        }
    };
    let mut command = std::process::Command::new(exe);
    command
        .args(launch.args())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(token) = token {
        command
            .env("XDG_ACTIVATION_TOKEN", token)
            .env("DESKTOP_STARTUP_ID", token);
    }
    match command.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                // Reaped so it leaves no zombie; how the opened program exits is its own affair.
                let _ = child.wait();
            });
        }
        Err(e) => log::warn!("notification click: cannot start the window: {e}"),
    }
}

/// The notification as this platform's service is handed it.
#[cfg(not(target_os = "macos"))]
fn built(n: &Notification, sound: Sound) -> notify_rust::Notification {
    let mut out = notify_rust::Notification::new();
    out.appname("mailo").summary(&n.summary);
    platform(&mut out, n, sound);
    out
}

/// Linux: the freedesktop hints, the icon, the bubble's own action and the buttons of a banner
/// about one conversation, the sound (the freedesktop sound theme's name for it, as the one hint
/// that says so: two would leave the server to take either), and a body escaped for a server that reads it as markup.
#[cfg(all(unix, not(target_os = "macos")))]
fn platform(out: &mut notify_rust::Notification, n: &Notification, sound: Sound) {
    use notify_rust::Hint;
    out.body(&escape(&n.body))
        .icon("mailo")
        .action(super::DEFAULT_ACTION, "Open")
        // The freedesktop category for this event, which servers use to pick a sound and to
        // group.
        .hint(Hint::Category("email.arrived".to_owned()))
        .hint(Hint::DesktopEntry("org.quire.Mail".to_owned()))
        .hint(Hint::Custom(
            "x-mailo-account".to_owned(),
            n.account.to_string(),
        ));
    buttons(out, n);
    match sound_name(sound) {
        Some(name) => out.sound_name(name),
        None => out.hint(Hint::SuppressSound(true)),
    };
    if let Opens::Thread(thread) = n.opens {
        out.hint(Hint::Custom(
            "x-mailo-thread".to_owned(),
            thread.to_string(),
        ));
    }
}

/// Windows shows the body as text. A toast would draw each button as one of its own, so it has
/// none, and a click on a toast is its own activation.
#[cfg(windows)]
fn platform(out: &mut notify_rust::Notification, n: &Notification, sound: Sound) {
    out.body(&n.body);
    if let Some(name) = sound_name(sound) {
        out.sound_name(name);
    }
}

/// The banner as the notification centre is handed it: the text as text, and a banner about one
/// conversation offers Open as its main button, with Archive and Reply among its further ones. What
/// a click needs to know comes back with it ([`super::banner::info`]).
#[cfg(target_os = "macos")]
fn banner(n: &Notification, sound: Sound) -> super::banner::Banner {
    let mut out = super::banner::Banner {
        title: n.summary.clone(),
        body: n.body.clone(),
        sound: sound_name(sound).map(str::to_owned),
        info: super::banner::info(n),
        ..Default::default()
    };
    if matches!(n.opens, Opens::Thread(_)) {
        out.main_button = Some("Open".to_owned());
        out.more_buttons
            .push((super::ARCHIVE_ACTION.to_owned(), "Archive".to_owned()));
        if n.reply {
            out.more_buttons
                .push((super::REPLY_ACTION.to_owned(), "Reply".to_owned()));
        }
    }
    out
}

/// The Archive and Reply buttons a banner about one conversation offers.
#[cfg(all(unix, not(target_os = "macos")))]
fn buttons(out: &mut notify_rust::Notification, n: &Notification) {
    if matches!(n.opens, Opens::Thread(_)) {
        out.action(super::ARCHIVE_ACTION, "Archive");
    }
    if n.reply {
        out.action(super::REPLY_ACTION, "Reply");
    }
}

/// The name this platform's service knows `sound` by, or `None` for no sound at all.
///
/// The names are the platform's own: the sound theme's on Linux, the system sounds' on macOS,
/// and the few a Windows toast can play, so a sound with no match there is the nearest one.
fn sound_name(sound: Sound) -> Option<&'static str> {
    let names = match sound {
        Sound::Default => (
            "message-new-email",
            "NSUserNotificationDefaultSoundName",
            "Mail",
        ),
        Sound::Ping => ("message", "Ping", "SMS"),
        Sound::Glass => ("complete", "Glass", "IM"),
        Sound::Pop => ("bell", "Pop", "Reminder"),
        Sound::Submarine => ("dialog-information", "Submarine", "Default"),
        Sound::Silent => return None,
    };
    Some(if cfg!(target_os = "macos") {
        names.1
    } else if cfg!(windows) {
        names.2
    } else {
        names.0
    })
}

/// A body is markup to a server that advertises `body-markup`, and a subject is a stranger's text:
/// `<b>` in one must reach the screen as those three characters.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    use std::sync::Arc;

    /// On macOS the banner goes to what the app handed in, and to nothing else: a test posts to
    /// a recorder and never to the notification centre.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_banner_is_posted_by_the_poster_the_app_handed_in() {
        use super::super::banner::{Banner, Posts};
        use std::sync::Mutex;

        #[derive(Debug, Default)]
        struct Recorder(Mutex<Vec<Banner>>);
        impl Posts for Recorder {
            fn post(&self, banner: &Banner) {
                if let Ok(mut posted) = self.0.lock() {
                    posted.push(banner.clone());
                }
            }
        }

        let recorder = Arc::new(Recorder::default());
        let desktop = Desktop::connect(recorder.clone());
        desktop.show_with(
            &Notification {
                account: mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(1)),
                summary: "Ada".to_owned(),
                body: "lunch".to_owned(),
                opens: Opens::Thread(mail_domain::ThreadId::from_uuid(uuid::Uuid::from_u128(2))),
                reply: true,
            },
            Sound::Silent,
        );
        let posted = recorder.0.lock().map(|p| p.clone()).unwrap_or_default();
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0].title, "Ada");
        assert_eq!(posted[0].main_button.as_deref(), Some("Open"));
        assert_eq!(posted[0].sound, None);
    }

    // Only Windows reads a response (Linux listens in `click`, macOS in `banner`), so the click
    // test, the one user of this id, runs there.
    #[cfg(windows)]
    fn thread(n: u128) -> Opens {
        Opens::Thread(mail_domain::ThreadId::from_uuid(uuid::Uuid::from_u128(n)))
    }

    #[cfg(windows)]
    fn inbox() -> Opens {
        Opens::Inbox(mail_domain::id::account_id_from_uuid(
            uuid::Uuid::from_u128(0xacc),
        ))
    }

    #[test]
    fn each_act_starts_the_program_that_does_it() {
        let one = mail_domain::ThreadId::from_uuid(uuid::Uuid::from_u128(1));
        let acc = mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(0xacc));
        let cases = [
            (
                "a conversation",
                Opens::Thread(one),
                Act::Open,
                vec!["open", "00000000-0000-0000-0000-000000000001"],
            ),
            (
                "the inbox",
                Opens::Inbox(acc.clone()),
                Act::Open,
                vec!["open", "--inbox", acc.as_str()],
            ),
            (
                "archive",
                Opens::Thread(one),
                Act::Archive,
                vec!["open", "--archive", "00000000-0000-0000-0000-000000000001"],
            ),
            (
                "reply",
                Opens::Thread(one),
                Act::Reply,
                vec!["open", "--reply", "00000000-0000-0000-0000-000000000001"],
            ),
        ];
        for (name, opens, act, args) in cases {
            assert_eq!(Launch { opens, act }.args(), args, "{name}");
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_banner_asks_the_server_for_the_one_sound_chosen_or_for_none() {
        use notify_rust::Hint;
        let note = Notification {
            account: mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(0xacc)),
            summary: "Ada".to_owned(),
            body: "lunch".to_owned(),
            opens: Opens::Thread(mail_domain::ThreadId::from_uuid(uuid::Uuid::from_u128(1))),
            reply: true,
        };
        let names = |sound| -> Vec<String> {
            built(&note, sound)
                .hints
                .iter()
                .filter_map(|hint| match hint {
                    Hint::SoundName(name) => Some(name.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(names(Sound::Ping), vec!["message".to_owned()]);
        assert_eq!(names(Sound::Default), vec!["message-new-email".to_owned()]);
        assert!(names(Sound::Silent).is_empty());
        assert!(
            built(&note, Sound::Silent)
                .hints
                .contains(&Hint::SuppressSound(true))
        );
    }

    #[test]
    fn a_silent_banner_has_no_sound_and_every_other_has_a_name() {
        assert_eq!(sound_name(Sound::Silent), None);
        for sound in [
            Sound::Default,
            Sound::Ping,
            Sound::Glass,
            Sound::Pop,
            Sound::Submarine,
        ] {
            assert!(
                sound_name(sound).is_some_and(|name| !name.is_empty()),
                "{sound:?}"
            );
        }
    }

    #[test]
    fn a_subject_cannot_become_markup() {
        assert_eq!(escape("<b>R&D</b>"), "&lt;b&gt;R&amp;D&lt;/b&gt;");
        assert_eq!(escape("lunch on friday"), "lunch on friday");
    }

    #[cfg(windows)]
    #[test]
    fn a_click_on_the_notification_opens_what_it_was_about_and_nothing_else_does() {
        use notify_rust::CloseReason;
        let cases: [(&str, NotificationResponse, Option<Launch>); 7] = [
            (
                "the bubble clicked",
                NotificationResponse::Default,
                Some(Launch {
                    opens: thread(1),
                    act: Act::Open,
                }),
            ),
            (
                "the default action named as an action",
                NotificationResponse::Action("default".to_owned()),
                Some(Launch {
                    opens: thread(1),
                    act: Act::Open,
                }),
            ),
            (
                "an action we never offered",
                NotificationResponse::Action("delete".to_owned()),
                None,
            ),
            (
                "the archive button",
                NotificationResponse::Action("archive".to_owned()),
                Some(Launch {
                    opens: thread(1),
                    act: Act::Archive,
                }),
            ),
            (
                "an inline reply",
                NotificationResponse::Reply("ok".to_owned()),
                None,
            ),
            (
                "dismissed",
                NotificationResponse::Closed(CloseReason::Dismissed),
                None,
            ),
            (
                "expired",
                NotificationResponse::Closed(CloseReason::Expired),
                None,
            ),
        ];
        for (name, response, expect) in cases {
            assert_eq!(answer(&response, &thread(1), false), expect, "{name}");
        }
        assert_eq!(
            answer(&NotificationResponse::Default, &inbox(), false),
            Some(Launch {
                opens: inbox(),
                act: Act::Open
            }),
            "a summary opens the inbox"
        );
        assert_eq!(
            answer(
                &NotificationResponse::Action("reply".to_owned()),
                &thread(1),
                true
            )
            .map(|launch| launch.act),
            Some(Act::Reply),
            "the reply button, when it was offered"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_service_that_never_says_closed_cannot_grow_the_waiting_without_end() {
        let waiting = Waiting::default();
        let mut places: Vec<_> = (0..REMEMBERED).map_while(|_| waiting.enter()).collect();
        assert_eq!(places.len(), REMEMBERED);
        assert!(waiting.enter().is_none(), "one past the bound");
        assert_eq!(waiting.len(), REMEMBERED);
        // A notification that closes gives its place back, and the next one may wait.
        drop(places.pop());
        assert_eq!(waiting.len(), REMEMBERED - 1);
        let next = waiting.enter();
        assert!(next.is_some());
        assert_eq!(waiting.len(), REMEMBERED);
    }

    #[cfg(windows)]
    #[test]
    fn every_place_given_back_leaves_nothing_waiting() {
        let waiting = Waiting::default();
        {
            let _one = waiting.enter();
            let _two = waiting.enter();
            assert_eq!(waiting.len(), 2);
        }
        assert!(waiting.is_empty());
    }
}
