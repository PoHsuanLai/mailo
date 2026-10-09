mod attachments;
pub(super) mod blocks;
mod cache;
mod fetch;
mod header;
mod images;
mod thumb;
mod tools;
mod viewer;

use super::press::on_primary;
use super::text::{attachment_rows, stamp};
use crate::ui::view::Shell;
use attachments::Attachments;
pub(in crate::ui) use cache::use_warming;
pub use cache::{rendered as render_message, warm};
use dioxus::prelude::*;
use ds::components::content::avatar::{
    AvatarFace, AvatarShape, AvatarSize, AvatarTone, person_hue,
};
use ds::components::content::label::LabelStyle;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::overlays::inline_banner::InlineBanner;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::render::Glyph;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::*;
use mail_mime::SanitizePolicy;
use mail_store::{SqliteStore, Store};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
pub(super) use viewer::{AttachmentViewer, viewer_key};

/// The first character of `name`, uppercased.
///
/// `chars`, not bytes: a CJK name's first byte is not a letter.
fn initial(name: &str) -> char {
    name.chars()
        .next()
        .and_then(|c| c.to_uppercase().next())
        .unwrap_or('?')
}

/// The sender's avatar: their first letter on the hue their address hashes to, as everywhere
/// else a person is drawn.
fn sender_face(message: &Message) -> AvatarFace {
    let named = message.from.name.as_deref().filter(|name| !name.is_empty());
    AvatarFace {
        initial: initial(named.unwrap_or(message.from.email.as_str())),
        size: AvatarSize::Size34,
        tone: AvatarTone::Person(person_hue(&message.from.email)),
        shape: AvatarShape::Round,
    }
}

/// When `message` came, as a list row writes it: the time today, a weekday this week, a date
/// beyond. The header's tooltip has the full date.
fn when_short(message: &Message, now: chrono::DateTime<chrono::Utc>) -> String {
    mail_core::when::listed(message.date, now, &chrono::Local)
}

/// The host a remote image would report the open to.
fn host_of(email: &str) -> &str {
    match email.rsplit_once('@') {
        Some((_, host)) if !host.is_empty() => host,
        _ => email,
    }
}

fn show_images() -> &'static str {
    "Show images"
}

/// The banner's second offer: this sender's images, from now on.
fn always_load_from(email: &str) -> String {
    format!("Always load from {email}")
}

/// Mute, in the head's tools: pressed while the conversation is muted, and a press mutes or
/// unmutes it through the same gesture as the row's button, so Ctrl Z and the toast take it back.
fn mute_tool(thread: ThreadId, mute: Mute, shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let (label, short, pressed) = match mute {
        Mute::Muted => ("Unmute this conversation", "Unmute", Check::On),
        Mute::Unmuted => ("Mute this conversation", "Mute", Check::Off),
    };
    let keys =
        crate::ui::keymap::action_keys(&shell.read().keymap, crate::ui::view::Shortcut::ToggleMute);
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            size: ControlSize::Large,
            image: ImagePosition::Only,
            icon: Some(IconSource::Glyph(Icon::BellOff)),
            label: label.to_owned(),
            title: Some(short.to_owned()),
            title_shortcut: keys,
            value: Some(pressed),
            onclick: move |_| {
                let store = consume_context::<Arc<SqliteStore>>();
                super::picks::mute_all(&store, shell, revision, &[thread]);
            },
        }
    }
}

/// The key [`super::unsubscribe::Leave`] is mounted under: the thread and what each of its
/// messages holds, so a body arriving asks again and another thread never shows this one's answer.
fn leave_key(thread: ThreadId, bodies: &super::unsubscribe::Bodies) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bodies.hash(&mut hasher);
    format!("{thread}-{:x}", hasher.finish())
}

/// Where a reader is drawn: beside the list, or alone in a conversation's own window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui) enum ReaderIn {
    /// The main window's reader pane, with its peek modes and its menu.
    #[default]
    Pane,
    /// A window of its own (`ui/window`): the window is the page, so no peek and no "Open in new
    /// window".
    Window,
}

/// What the reader displays for one message body in its sandboxed frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameBody {
    /// Headers only so far. Normal mid-sync, and not an empty message.
    NotFetched,
    /// The body rendered as sanitized HTML.
    Present {
        html: String,
        blocked_remote: bool,
        fetches: Vec<String>,
    },
}

impl FrameBody {
    pub fn frame_html(&self) -> Option<&str> {
        match self {
            FrameBody::Present { html, .. } => Some(html),
            FrameBody::NotFetched => None,
        }
    }

    pub fn blocked_remote(&self) -> bool {
        match self {
            FrameBody::Present { blocked_remote, .. } => *blocked_remote,
            FrameBody::NotFetched => false,
        }
    }

    pub fn frame_fetches(&self) -> &[String] {
        match self {
            FrameBody::Present { fetches, .. } => fetches,
            FrameBody::NotFetched => &[],
        }
    }
}

/// Render a message body into its sandboxed frame representation, every time.
///
/// The reader goes through [`render_message`], which remembers the answer (`cache`); this is the
/// work itself, for the cache to call and for a measurement that must not hit it.
pub fn render_message_uncached(
    store: &SqliteStore,
    message: &Message,
    policy: SanitizePolicy,
) -> FrameBody {
    render(store, message, policy).0
}

/// What a rendering was made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Source {
    /// The stored body: the blob, or the text beside it.
    Stored,
    /// A body OpenPGP or S/MIME opened. A function of more than the blob, and of a key that may
    /// be locked again, so nothing keyed by the blob may keep it.
    Opened,
}

/// [`render_message_uncached`], saying what it rendered from — asked once, here, so a seal
/// opened while it renders cannot leave a cache believing the plaintext is the blob's.
pub(super) fn render(
    store: &SqliteStore,
    message: &Message,
    policy: SanitizePolicy,
) -> (FrameBody, Source) {
    match &message.body {
        Body::Absent => (FrameBody::NotFetched, Source::Stored),
        Body::Present { text, .. } => {
            let (parsed, source) = match super::pgp::parsed(message) {
                Some(opened) => (Some(opened), Source::Opened),
                None => (parse_body(store, message), Source::Stored),
            };
            let Some(parsed) = parsed else {
                return (plain_frame(text.as_deref().unwrap_or("")), source);
            };
            let body = if let Some(html) = parsed.html.as_deref() {
                html_frame(html, &parsed, policy)
            } else {
                let shown = parsed.text.as_deref().or(text.as_deref()).unwrap_or("");
                plain_frame(shown)
            };
            (body, source)
        }
    }
}

fn parse_body(store: &SqliteStore, message: &Message) -> Option<mail_mime::Parsed> {
    let raw = message.body.raw()?;
    // A reader, not the writer: a sync's ingest holds the writer for a whole batch.
    let bytes = store.blobs().get(&store.reader(), raw).ok()?;
    mail_mime::parse(&bytes).ok()
}

fn html_frame(html: &str, parsed: &mail_mime::Parsed, policy: SanitizePolicy) -> FrameBody {
    let safe = mail_mime::sanitize(html, policy);
    let blocked_remote = safe.blocked_remote() > 0;
    let fetches = safe.remote_fetches().to_vec();
    let embedded =
        mail_mime::embed_inline(safe.as_str(), &parsed.attachments, mail_mime::INLINE_BUDGET);
    FrameBody::Present {
        html: frame_document(&html_sheet(), safe.body_style(), &embedded),
        blocked_remote,
        fetches,
    }
}

/// The type a frame's document is set in, a mail client's own: the system's sans rather than
/// Blitz's serif.
const FRAME_FONT: &str =
    "-apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, Helvetica, Arial, sans-serif";

/// What an HTML message's document starts from before the sender's own sheets: a browser's
/// defaults, as a mail client's are, with quotes, code and headings drawn as the composer draws
/// them, so a message written here reads as it was written, with nothing wider than the frame where Blitz can say so
/// (a table's own percentage `max-width` does not hold a fixed-width table nested in an
/// auto-width one: a pane under 600 px still cuts a 600 px newsletter's right edge). An image
/// keeps the size its attributes give it, so a blocked one still holds its place. The sender's
/// `<style>` comes later in the document and its `<body>` colours and style sit on the body
/// itself, so either wins, a `padding: 0` included, and a full-bleed design stays one.
fn html_sheet() -> String {
    format!(
        ":root {{ color-scheme: light; }} html, body {{ margin: 0; }} \
         body {{ padding: 16px; font-family: {FRAME_FONT}; font-size: 14px; line-height: 1.5; \
         color: #1d1d1f; background: #fff; overflow-wrap: anywhere; }} \
         img {{ max-width: 100%; }} table {{ max-width: 100%; }} \
         blockquote {{ margin: 0 0 1em; padding-left: 12px; border-left: 3px solid #d2d2d7; color: #424245; }} \
         pre, code {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .9em; }} \
         pre {{ background: #f5f5f7; border-radius: 6px; padding: 10px 12px; white-space: pre-wrap; }} \
         h2 {{ font-size: 1.3em; line-height: 1.25; margin: .9em 0 .35em; }} \
         h3 {{ font-size: 1.15em; margin: .8em 0 .3em; }} h4 {{ font-size: 1em; margin: .7em 0 .3em; }} \
         a {{ color: #0066cc; }}"
    )
}

/// A plain-text message's sheet: its lines as written, in the scheme the desktop is in.
fn plain_sheet() -> String {
    format!(
        ":root {{ color-scheme: light dark; }} \
         body {{ margin: 16px; font-family: {FRAME_FONT}; font-size: 14px; line-height: 1.6; \
         white-space: pre-wrap; overflow-wrap: anywhere; }}"
    )
}

/// A frame's whole document: `sheet`, then `body` (markup already safe to show) in a `<body>`
/// carrying `body_style`, the sender's own body colours and style, when there are any.
fn frame_document(sheet: &str, body_style: Option<&str>, body: &str) -> String {
    let styled = body_style
        .map(|style| format!(" style=\"{}\"", escape_html(style)))
        .unwrap_or_default();
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><style>{sheet}</style></head>\
         <body{styled}>{body}</body></html>"
    )
}

fn plain_frame(text: &str) -> FrameBody {
    FrameBody::Present {
        html: frame_document(&plain_sheet(), None, &escape_html(text)),
        blocked_remote: false,
        fetches: Vec::new(),
    }
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\n' | '\t' => out.push(ch),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// `revision` is the window's, moved when leaving a list queues a message, and whenever any
/// window moves the store (`ui/revisions`), which draws the reader again; a reader drawn on its
/// own has none.
#[component]
pub(super) fn Reader(
    thread: ThreadId,
    shell: Signal<Shell>,
    revision: Option<Signal<u64>>,
    #[props(default)] place: ReaderIn,
    children: Element,
) -> Element {
    let store = use_context::<Arc<SqliteStore>>();
    // The store may have moved under this conversation, here or in another window.
    if let Some(revision) = revision {
        let _ = revision();
    }
    // Moved when an OpenPGP or S/MIME message has been opened and has a body of its own to show,
    // so the messages are drawn again with it. The opening itself happens in `pgp::Seal`, off
    // this thread; here it is only looked up.
    let landed = use_signal(|| 0u64);
    let _ = landed();
    // The window's settings, for whose images load without asking. Not `prefs::use_settings`: a
    // reader drawn with no window around it (a test's) asks about every image rather than read
    // the person's own `settings.toml`, which is the safe way for a stray reader to be wrong.
    let settings = use_hook(try_consume_context::<Signal<crate::settings::MailSettings>>);
    // The link a right click in a message's frame was on, which the window's frames report
    // (`original::FrameMenus`); a reader with no window around it has none. And the Copy Link
    // menu it opens.
    let frame_menus = use_signal(try_consume_context::<super::original::FrameMenus>);
    let mut link_menu = use_signal(|| None::<super::original::LinkAsked>);
    // The window's consent to remote images, and this reader's claim on it; closing the reader
    // takes back what it granted.
    let consent = use_hook(|| {
        try_consume_context::<super::original::Consent>().map(|consent| {
            let holder = consent.holder();
            (consent, holder)
        })
    });
    {
        let consent = consent.clone();
        use_drop(move || {
            if let Some((consent, holder)) = consent {
                consent.release(holder);
            }
        });
    }
    let Ok(loaded) = store.thread(thread) else {
        return rsx! {
            div { class: "reader-empty",
                EmptyState { title: "Conversation gone" }
            }
        };
    };
    // Read here, in the render, so adding a trusted sender or changing the mode draws the reader
    // again with the images it now allows.
    let reading = settings
        .map(|settings| settings.read().reading.clone())
        .unwrap_or_default();
    let pressed = shell.read().show_remote_images;
    let peek = shell.read().peek;
    // What this reader sent to be rendered off the thread, and the count that moves when some of
    // it lands (`cache::render_later`).
    let sent = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(cache::Sent::default())));
    let rendered_later = use_signal(|| 0u64);
    let _ = rendered_later();
    // The same for senders' checks, on which whether a message's images load can turn
    // (`images::look_later`).
    let looked = use_hook(|| Rc::new(RefCell::new(images::Checked::default())));
    let checked_later = use_signal(|| 0u64);
    let _ = checked_later();
    let mut wanted = Vec::new();
    let mut room = cache::ON_THE_FRAME;
    let mut later = Vec::new();
    // Newest first, so the message the conversation opens on is the one the frame's room goes
    // to; drawn oldest first as before.
    //
    // Whether images load is decided per message, not per conversation: "Show images" allows
    // every message in it, and the settings ([`images::auto_allow`]) allow each message on its
    // own sender's standing. A thread where a trusted sender's newsletter was answered by a
    // stranger loads the newsletter's images and asks about the stranger's, and a forged message
    // dropped into a trusted sender's thread gains nothing from the thread it is in. The render
    // and its cache entry are already per message and keyed by the policy (`cache`), and the
    // Original frames' consent lists each message's fetches apart (`ui/original`), so nothing
    // has to be shared between messages that would let one's allowance reach another.
    let mut shown: Vec<(Message, Option<FrameBody>, bool, bool)> = loaded
        .messages
        .iter()
        .rev()
        .filter_map(|id| store.message(*id).ok())
        .map(|message| {
            let showing = pressed
                || images::auto_allow_message(&reading, &looked.borrow(), &message, &mut wanted);
            let policy = shell.read().policy_with(showing);
            let mut frame = |policy| {
                cache::on_the_frame(
                    &store,
                    &message,
                    policy,
                    &mut room,
                    &sent.borrow(),
                    &mut later,
                )
            };
            // Both renderings are remembered (`cache`), under their own policies, so a message
            // whose banner needs the blocked count while the images are shown parses once, ever.
            let body = frame(policy);
            let remote = match &body {
                Some(body) if body.blocked_remote() => true,
                Some(body) if showing && body.frame_html().is_some() => {
                    frame(mail_mime::SanitizePolicy::FRAME).is_some_and(|it| it.blocked_remote())
                }
                _ => false,
            };
            (message, body, remote, showing)
        })
        .collect();
    shown.reverse();
    if !later.is_empty() {
        cache::render_later(store.clone(), later, sent.clone(), rendered_later);
    }

    // The consent, as the Original frames' network reads it (`ui/original`): written here, in the
    // render, so a frame that reloads with the consented markup finds it already granted, and
    // taken back by the same render that stops showing the images (open, select, close all
    // clear `show_remote_images`; a trusted sender taken off the list, or the mode set back to
    // Ask, draws the reader again without them). Only the launched window, or a harness, provides
    // one.
    if let Some((consent, holder)) = &consent {
        // Only the messages whose images are shown: one the settings left blocked in a thread
        // where another loads keeps its frame's requests refused.
        let allowed = shown.iter().any(|(.., showing)| *showing).then(|| {
            shown
                .iter()
                .filter(|(.., showing)| *showing)
                .map(|(message, body, ..)| {
                    let fetches = body.as_ref().map(FrameBody::frame_fetches);
                    (message.id, fetches.unwrap_or_default().to_vec())
                })
                .collect()
        });
        consent.hold(*holder, thread, allowed);
    }

    // What the list lookup is a function of. Ids and blob ids only: nothing here reads a blob.
    let bodies: super::unsubscribe::Bodies = shown
        .iter()
        .map(|(message, ..)| (message.id, message.body.raw()))
        .collect();
    let leave_key = leave_key(thread, &bodies);
    // An encrypted message's subject travels inside it; the outside says `...`.
    let subject = shown
        .iter()
        .find(|(message, ..)| message.subject == loaded.summary.subject)
        .and_then(|(message, ..)| super::pgp::subject(message))
        .unwrap_or_else(|| loaded.summary.subject.clone());
    // How many messages there are, and the moment their dates are written against: the newest
    // says everything in its header, the rest who and when.
    let count = shown.len();
    let now = super::clock::now();
    // The banner: what is still blocked comes first, since that is what it asks about; else the
    // host whose images are showing. Both name the newest such message's sender.
    let blocked = shown
        .iter()
        .rev()
        .find(|(_, _, remote, showing)| *remote && !showing)
        .map(|(message, ..)| message.clone());
    let from_host = shown
        .iter()
        .rev()
        .find(|(_, _, remote, _)| *remote)
        .map(|(message, ..)| host_of(&message.from.email).to_owned());
    // "Always load from" the blocked sender: offered where it could ever load their images
    // (not under `Always`, which already would; not in Junk, which is always asked about), and
    // never to a sender who is not provably who the address says: trusting a forged address would
    // trust everyone who forges it next.
    // Until the sender's checks are known it is not offered; it appears when they land.
    let offer = blocked.as_ref().and_then(|message| {
        let worth = reading.remote_images != crate::settings::LoadRemoteImages::Always
            && message.mailbox != MailboxRole::Spam
            && !message.from.email.trim().is_empty();
        if !worth {
            return None;
        }
        let passed = images::dmarc_known(&looked.borrow(), message, &mut wanted)?;
        (!images::suspicious(message.from.name.as_deref(), &message.from.email, passed))
            .then(|| message.from.email.trim().to_lowercase())
    });
    if !wanted.is_empty() {
        let messages = shown.iter().map(|(message, ..)| message.clone()).collect();
        images::look_later(
            store.clone(),
            messages,
            wanted,
            looked.clone(),
            checked_later,
        );
    }
    // A protected message opened to a body lists what is attached inside it; its stored parts
    // are its wrapping.
    let attached: Vec<_> = shown
        .iter()
        .map(|(message, ..)| {
            super::pgp::attachments(message).unwrap_or_else(|| attachment_rows(message))
        })
        .collect();

    rsx! {
        div { class: "reader-head",
            div { class: "head-row",
                span { class: "spacer" }
                div { class: "bar-tools",
                    if let Some(revision) = revision {
                        super::move_to::MoveTool { thread, shell, revision }
                        {mute_tool(thread, loaded.summary.mute, shell, revision)}
                        super::follow_up::FollowUpTool {
                            thread,
                            current: loaded.summary.follow_up,
                            shell,
                            revision,
                        }
                    }
                    super::print::PrintTool { thread }
                    if place == ReaderIn::Pane {
                        tools::ViewMenu { thread, peek, shell }
                    }
                    if let Some(revision) = revision {
                        tools::ReaderMore { summary: loaded.summary.clone(), shell, revision }
                    }
                }
            }
            h2 { Label { text: subject, style: LabelStyle::Title } }
            if loaded.summary.mute == Mute::Muted {
                div { class: "muted-note", role: "status",
                    Glyph { icon: Icon::BellOff, size: IconSize::Compact }
                    span { "Muted — new replies arrive read and skip the inbox" }
                }
            }
            super::follow_up::FollowUpNote { follow_up: loaded.summary.follow_up }
            {rsx! { super::receipt::Receipts { key: "{leave_key}", bodies: bodies.clone() } }}
        }
        div { class: "reader-body",
            div { class: "banners",
            if let Some(host) = from_host {
                if blocked.is_none() {
                    InlineBanner {
                        severity: Severity::Info,
                        icon: Some(Icon::Image),
                        text: format!("Showing remote images from {host}"),
                    }
                } else {
                    InlineBanner {
                        severity: Severity::Warn,
                        icon: Some(Icon::Image),
                        text: "Remote images blocked",
                        actions: rsx! {
                            Button {
                                label: show_images(),
                                common: Common { aria_label: Some(show_images().to_owned()), ..Common::default() },
                                onclick: on_primary(move || {
                                    shell.write().show_remote_images = true;
                                    super::host::Host::focus_app();
                                }),
                            }
                            if let Some(email) = offer {
                                Button {
                                    label: always_load_from(&email),
                                    common: Common { aria_label: Some(always_load_from(&email)), ..Common::default() },
                                    onclick: on_primary(move || {
                                        // The sender joins the list, and under Ask the mode
                                        // becomes Trusted (`ReadingSettings::trust_images_from`);
                                        // written to settings.toml and told to the other windows.
                                        let email = email.clone();
                                        if let Err(why) = crate::ui::prefs::change(|settings| {
                                            settings.reading.trust_images_from(&email);
                                        }) {
                                            eprintln!("remote images: {why}");
                                        }
                                        // This conversation's images now, whatever the write did.
                                        shell.write().show_remote_images = true;
                                        super::host::Host::focus_app();
                                    }),
                                }
                            }
                        },
                    }
                }
            }
            }
            for (at, ((message, body, ..), attached)) in shown.into_iter().zip(attached).enumerate() {
                article { key: "{message.id}", class: "frame",
                    oncontextmenu: move |event: MouseEvent| {
                        let asked = frame_menus.peek().as_ref().and_then(|menus| menus.take());
                        if let Some(asked) = asked {
                            event.prevent_default();
                            event.stop_propagation();
                            link_menu.set(Some(asked));
                        }
                    },
                    header::MessageHead {
                        detail: header::detail_at(at, count),
                        when: (when_short(&message, now), stamp(&message)),
                        extras: (header::detail_at(at, count) == header::Detail::Full).then(|| header::Extras {
                            checked: (message.id, message.body.raw()),
                            thread,
                            leave_key: leave_key.clone(),
                            bodies: bodies.clone(),
                            revision,
                        }),
                        message: message.clone(),
                    }
                    super::pgp::Seal {
                        key: "{message.id}-{message.body.raw():?}",
                        message: message.id,
                        body: message.body.raw(),
                        landed,
                    }
                    super::invite::Invitation {
                        key: "{message.id}-{message.body.raw():?}",
                        message: message.id,
                        body: message.body.raw(),
                    }
                    if !attached.is_empty() {
                        Attachments {
                            key: "{message.id}-{message.body.raw():?}",
                            message: message.id,
                            body: message.body.raw(),
                            rows: attached,
                            shell,
                        }
                    }
                    match body {
                        // Being rendered off the thread: the frame's own box, so nothing moves
                        // when it lands.
                        None => rsx! {
                            div { key: "{message.id}-rendering", class: "html", aria_busy: "true" }
                        },
                        Some(FrameBody::NotFetched) => rsx! {
                            fetch::BodyPane {
                                key: "{message.id}-pane",
                                message: message.id,
                                account: message.account,
                                landed,
                            }
                        },
                        Some(FrameBody::Present { html, .. }) => rsx! {
                            blocks::MessageView {
                                holder: consent.as_ref().map(|(_, holder)| *holder),
                                message_id: message.id,
                                html,
                            }
                        },
                    }
                }
            }
            if let Some(asked) = link_menu() {
                LinkMenu { asked, menus: frame_menus, open: link_menu }
            }
            {children}
        }
    }
}

/// Copy Link and Open Link, at the pointer, for a link a right click in a message's frame was on.
#[component]
fn LinkMenu(
    asked: super::original::LinkAsked,
    menus: Signal<Option<super::original::FrameMenus>>,
    open: Signal<Option<super::original::LinkAsked>>,
) -> Element {
    let mut open = open;
    let at = Rect {
        origin: asked.at,
        size: Size {
            width: Px(0.0),
            height: Px(0.0),
        },
    };
    let href = asked.href.clone();
    rsx! {
        super::menu::Floating {
            anchor: None,
            placed: Some(at),
            title: String::new(),
            items: link_items(),
            on_pick: move |key: String| {
                open.set(None);
                match key.as_str() {
                    COPY_LINK => super::host::Host::copy(&href),
                    OPEN_LINK => {
                        if let Some(menus) = menus.peek().as_ref() {
                            menus.open(&href);
                        }
                    }
                    _ => {}
                }
            },
            on_close: move |_| open.set(None),
        }
    }
}

const OPEN_LINK: &str = "open-link";
const COPY_LINK: &str = "copy-link";

/// The link menu's rows, in the Mac's order.
fn link_items() -> Vec<super::menu::MenuItem> {
    [
        (OPEN_LINK, "Open Link", Icon::Link),
        (COPY_LINK, "Copy Link", Icon::Copy),
    ]
    .into_iter()
    .map(|(key, name, icon)| super::menu::MenuItem {
        key: key.to_owned(),
        tile: super::menu::Tile::Icon(icon),
        name: name.to_owned(),
        help: None,
        right: super::menu::Right::None,
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    })
    .collect()
}

/// The Original frame as the reader draws it, in mailo's stylesheet, and nothing else: for the
/// guarantee tests on Blitz (`tests/app/native_frame.rs`), which put markup in it that the sanitizer
/// would never have let through. A test binary of its own, because a Blitz document replaces the
/// process's event converter, which the `VirtualDom` fixtures in this crate's unit tests rely on.
#[component]
pub fn OriginalFrame(html: String) -> Element {
    rsx! {
        style { {ds::stylesheet()} }
        style { {super::style::STYLE} }
        div { class: "reader-body",
            article { class: "frame",
                blocks::Sandbox { html }
            }
        }
    }
}

#[cfg(test)]
mod tests;
