mod attachments;
pub(super) mod blocks;
mod cache;
mod fetch;
mod thumb;
mod viewer;

use super::press::on_primary;
use super::text::{address, attachment_rows, from_name, stamp};
use crate::ui::view::{Peek, Shell};
use attachments::Attachments;
pub(in crate::ui) use cache::use_warming;
pub use cache::{rendered as render_message, warm};
use dioxus::prelude::*;
use ds::components::content::avatar::{
    AvatarFace, AvatarShape, AvatarSize, AvatarTone, person_hue,
};
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::overlays::inline_banner::InlineBanner;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::render::Glyph;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::*;
use mail_mime::SanitizePolicy;
use mail_store::{SqliteStore, Store};
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
        size: AvatarSize::Size28,
        tone: AvatarTone::Person(person_hue(&message.from.email)),
        shape: AvatarShape::Round,
    }
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

fn peek_tool(peek: Peek, current: Peek, icon: Icon, mut shell: Signal<Shell>) -> Element {
    let label = peek.label();
    let pressed = if current == peek {
        Check::On
    } else {
        Check::Off
    };
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            image: ImagePosition::Only,
            label: label.to_owned(),
            icon: Some(IconSource::Glyph(icon)),
            value: Some(pressed),
            onclick: move |_| shell.write().peek = peek,
        }
    }
}

/// Mute, in the head's tools: pressed while the conversation is muted, and a press mutes or
/// unmutes it through the same gesture as the row's button, so Ctrl Z and the toast take it back.
fn mute_tool(thread: ThreadId, mute: Mute, shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let (label, pressed) = match mute {
        Mute::Muted => ("Unmute this conversation", Check::On),
        Mute::Unmuted => ("Mute this conversation", Check::Off),
    };
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            image: ImagePosition::Only,
            icon: Some(IconSource::Glyph(Icon::BellOff)),
            label: label.to_owned(),
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

/// The reader's menu: what it does beyond its tools. "Open in new window" for now.
#[component]
fn ReaderMenu(thread: ThreadId) -> Element {
    let mut open = use_signal(|| false);
    let mut tool = use_signal(|| None::<ds::host::measure::MountedRef>);
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            image: ImagePosition::Only,
            icon: Some(IconSource::Glyph(Icon::Ellipsis)),
            label: "More".to_owned(),
            shown: Some(if open() {
                ds::prelude::Shown::Visible
            } else {
                ds::prelude::Shown::Hidden
            }),
            common: Common {
                mounted: Some(EventHandler::new(move |event: MountedEvent| {
                    tool.set(Some(ds::host::measure::MountedRef(event.data())));
                })),
                ..Common::default()
            },
            onclick: move |_| open.toggle(),
        }
        if open() {
            super::menu::Floating {
                anchor: tool(),
                title: String::new(),
                items: vec![super::window::menu_item()],
                on_pick: move |key: String| {
                    open.set(false);
                    if key == super::window::OPEN_KEY {
                        super::window::open_in_window(thread);
                    }
                },
                on_close: move |_| open.set(false),
            }
        }
    }
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
        html: embedded,
        blocked_remote,
        fetches,
    }
}

fn plain_frame(text: &str) -> FrameBody {
    let escaped = escape_html(text);
    let html = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><style>\
         :root {{ color-scheme: light dark; }}\
         body {{ margin: 16px; font-family: -apple-system, BlinkMacSystemFont, \
         \"Segoe UI\", Roboto, Helvetica, Arial, sans-serif; font-size: 14px; \
         line-height: 1.6; white-space: pre-wrap; overflow-wrap: anywhere; }}\
         </style></head><body>{escaped}</body></html>"
    );
    FrameBody::Present {
        html,
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
    let policy = shell.read().policy();
    let showing = shell.read().show_remote_images;
    let peek = shell.read().peek;
    // What this reader sent to be rendered off the thread, and the count that moves when some of
    // it lands (`cache::render_later`).
    let sent = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(cache::Sent::default())));
    let rendered_later = use_signal(|| 0u64);
    let _ = rendered_later();
    let mut room = cache::ON_THE_FRAME;
    let mut later = Vec::new();
    // Newest first, so the message the conversation opens on is the one the frame's room goes
    // to; drawn oldest first as before.
    let mut shown: Vec<(Message, Option<FrameBody>, bool)> = loaded
        .messages
        .iter()
        .rev()
        .filter_map(|id| store.message(*id).ok())
        .map(|message| {
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
                    frame(mail_mime::SanitizePolicy::CURRENT).is_some_and(|it| it.blocked_remote())
                }
                _ => false,
            };
            (message, body, remote)
        })
        .collect();
    shown.reverse();
    if !later.is_empty() {
        cache::render_later(store.clone(), later, sent.clone(), rendered_later);
    }

    // The consent, as the Original frames' network reads it (`ui/original`): written here, in the
    // render, so a frame that reloads with the consented markup finds it already granted, and
    // taken back by the same render that stops showing the images (open, select, close all
    // clear `show_remote_images`). Only the launched window, or a harness, provides one.
    if let Some((consent, holder)) = &consent {
        let allowed = showing.then(|| {
            shown
                .iter()
                .map(|(message, body, _)| {
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
        .map(|(message, _, _)| (message.id, message.body.raw()))
        .collect();
    let leave_key = leave_key(thread, &bodies);
    // An encrypted message's subject travels inside it; the outside says `...`.
    let subject = shown
        .iter()
        .find(|(message, _, _)| message.subject == loaded.summary.subject)
        .and_then(|(message, _, _)| super::pgp::subject(message))
        .unwrap_or_else(|| loaded.summary.subject.clone());
    let meta = shown.last().map(|(message, _, _)| {
        (
            sender_face(message),
            from_name(message),
            address(message),
            stamp(message),
        )
    });
    // Whose word the sender's checks are on: the message the head names.
    let checked = shown
        .last()
        .map(|(message, _, _)| (message.id, message.body.raw()));
    let from_host = shown
        .iter()
        .rev()
        .find(|(_, _, remote)| *remote)
        .map(|(message, _, _)| host_of(&message.from.email).to_owned());
    // A protected message opened to a body lists what is attached inside it; its stored parts
    // are its wrapping.
    let attached: Vec<_> = shown
        .iter()
        .map(|(message, _, _)| {
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
                        {peek_tool(Peek::Side, peek, Icon::Panel, shell)}
                        {peek_tool(Peek::CENTER, peek, Icon::Square, shell)}
                        {peek_tool(Peek::FULL, peek, Icon::Maximize, shell)}
                        ReaderMenu { thread }
                    }
                }
            }
            h2 { Label { text: subject, style: LabelStyle::Title } }
            if loaded.summary.mute == Mute::Muted {
                div { class: "muted-note", role: "status",
                    Glyph { icon: Icon::BellOff, size: IconSize::Micro }
                    span { "Muted — new replies arrive read and skip the inbox" }
                }
            }
            super::follow_up::FollowUpNote { follow_up: loaded.summary.follow_up }
            if let Some((face, from, addr, when)) = meta {
                div { class: "reader-meta",
                    if let Some((id, raw)) = checked {
                        {rsx! { super::brand::ReaderAvatar { key: "{id}-{raw:?}", message: id, body: raw, from: addr.clone(), initial: face.initial.to_string() } }}
                    } else {
                        Avatar { initial: face.initial, size: face.size, tone: face.tone }
                    }
                    div { class: "reader-who",
                        Label { text: from, style: LabelStyle::Headline }
                        Label { text: addr, role: LabelRole::Secondary, style: LabelStyle::Footnote }
                        if let Some((id, raw)) = checked {
                            {rsx! { super::checks::SenderChecks { key: "{id}-{raw:?}", message: id, body: raw } }}
                        }
                        Label { text: when, role: LabelRole::Tertiary, style: LabelStyle::Footnote }
                    }
                    {rsx! { super::unsubscribe::Leave { key: "{leave_key}", thread, bodies: bodies.clone(), revision } }}
                }
            }
            {rsx! { super::receipt::Receipts { key: "{leave_key}", bodies } }}
        }
        div { class: "reader-body",
            div { class: "banners",
            if let Some(host) = from_host {
                if showing {
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
                                size: ControlSize::Small,
                                label: show_images(),
                                common: Common { aria_label: Some(show_images().to_owned()), ..Common::default() },
                                onclick: on_primary(move || {
                                    shell.write().show_remote_images = true;
                                    super::host::Host::focus_app();
                                }),
                            }
                        },
                    }
                }
            }
            }
            for ((message, body, _), attached) in shown.into_iter().zip(attached) {
                article { key: "{message.id}", class: "frame",
                    header {
                        Label { text: from_name(&message), style: LabelStyle::Headline }
                        Label { text: address(&message), role: LabelRole::Secondary, style: LabelStyle::Footnote }
                        time { Label { text: stamp(&message), role: LabelRole::Tertiary, style: LabelStyle::Footnote } }
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
            {children}
        }
    }
}

/// The Original frame as the reader draws it, in mailo's stylesheet, and nothing else: for the
/// guarantee tests on Blitz (`tests/native_frame.rs`), which put markup in it that the sanitizer
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
