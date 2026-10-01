mod attachments;
mod blocks;
mod find_bar;
mod found;
mod image;
mod remote;
mod source;
mod spans;
mod table;
mod thumb;
mod viewer;

use super::press::on_primary;
use super::text::{address, attachment_rows, from_name, stamp};
use crate::view::{Peek, Reading, Shell};
use attachments::Attachments;
use blocks::MessageView;
use dioxus::prelude::*;
use ds::components::content::avatar::{
    AvatarFace, AvatarShape, AvatarSize, AvatarTone, person_hue,
};
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::segmented::Tracking;
use ds::components::overlays::inline_banner::InlineBanner;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::render::Glyph;
use ds::style::tokens::control_size::ControlSize;
pub(super) use find_bar::open_find;
use find_bar::{FindBar, marking};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use source::{Showing, Shown, SourceView, Sources};
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

/// Reader, Original and Source: quire's segmented control, in a span so it can sit in the
/// header without becoming the iframe's parent. Original is offered only where there is a
/// frame; Source for every message, downloaded or not.
#[component]
fn ViewSwitch(
    message_id: MessageId,
    body: Option<BlobId>,
    frame: bool,
    mut showing: Signal<Showing>,
    sources: Signal<Sources>,
) -> Element {
    let now = source::shown(&showing.read(), message_id);
    let mut choices = vec![Choice::new(Shown::Reader, "Reader")];
    if frame {
        choices.push(Choice::new(Shown::Original, "Original"));
    }
    choices.push(Choice::new(Shown::Source, "Source"));
    rsx! {
        span { class: "view-switch",
            SegmentedControl::<Shown> {
                label: "How to show this message",
                choices,
                tracking: Tracking::SelectOne(now),
                size: ControlSize::Small,
                onchange: move |which: Shown| {
                    if which == Shown::Source {
                        source::open(message_id, body, showing, sources);
                    } else {
                        showing.write().insert(message_id, which);
                    }
                },
            }
        }
    }
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
    // Where the last attachment went, or why it did not. Cleared by opening another
    // conversation, because this component is rebuilt for each one.
    let mut saved = use_signal(|| None::<String>);
    // Which attachment is being fetched, if one is. That part's button stays disabled until
    // the fetch ends, so a second click cannot start a second download of it.
    let downloading = use_signal(|| None::<(MessageId, usize)>);
    // Which messages are showing their Original frame or their source. Keyed by message, so
    // opening another one does not carry the choice over. The frame itself is not created and
    // destroyed with this choice.
    let original = use_signal(Showing::new);
    // The sources read so far, by blob, so switching back and forth reads each once.
    let sources = use_signal(Sources::new);
    // Quotes the reader has unfolded, keyed by message and path.
    let quotes = use_signal(blocks::OpenQuotes::new);
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
    // The Reader view's remote images on Blitz, which mailo fetches itself: held to this
    // render's thread and consent, and fetched by `remote::Fetcher`'s effect (`remote.rs`).
    {
        let pictures = use_context_provider(remote::Pictures::new);
        pictures.hold(thread, shell.read().show_remote_images);
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
    // The HTML part is not a column: it lives inside the stored raw message, which is the only
    // copy that is byte-for-byte what the server sent. Parsed here, once per render of a thread,
    // rather than at ingest — storing the blocks would freeze today's limits into every row.
    // Images resolve in that walk. The frame, when there is one, is the sanitized markup.
    let shown: Vec<(Message, Reading, bool)> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .map(|message| {
            // A protected message opened already shows what it was opened to; everything else,
            // and one not opened yet, shows what is stored.
            let render = |policy| {
                super::pgp::reading(&message, policy)
                    .unwrap_or_else(|| crate::reader::render(&store, &message, policy))
            };
            let reading = render(policy);
            // Once the reader is allowed to fetch, the display pass blocks nothing, so it can
            // no longer say whether this message had a remote image. The blocked policy can:
            // that is the same question the offer was answered with.
            let remote = if reading.blocked_remote() {
                true
            } else if showing && reading.frame_html().is_some() {
                render(mail_mime::SanitizePolicy::CURRENT).blocked_remote()
            } else {
                false
            };
            (message, reading, remote)
        })
        .collect();

    // The consent, as the Original frames' network reads it (`ui/original`): written here, in the
    // render, so a frame that reloads with the consented markup finds it already granted, and
    // taken back by the same render that stops showing the images (open, select, close all
    // clear `show_remote_images`). Only the launched window, or a harness, provides one.
    if let Some((consent, holder)) = &consent {
        let allowed = showing.then(|| {
            shown
                .iter()
                .map(|(message, reading, _)| (message.id, reading.frame_fetches().to_vec()))
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
    // ⌘F's marks, or the list search's while no find is open. Blocks only: the frame is
    // never read and never marked.
    let (highlight, problem) = marking(&shell.read());
    let finding = shell.read().find.clone();
    let documents: Vec<_> = shown
        .iter()
        .map(|(_, reading, _)| reading.document())
        .collect();
    let (founds, total) = found::find_in(&documents, &highlight, finding.as_ref());
    // On Blitz, what of this render mailo fetches itself: the consented remote images it draws.
    let fetcher = {
        let wanted = if showing {
            remote::wanted(documents.iter().flatten().copied())
        } else {
            Vec::new()
        };
        rsx! { remote::Fetcher { thread, wanted } }
    };
    let invalid = problem.is_some();
    // A protected message opened to a body lists what is attached inside it; its stored parts
    // are its wrapping.
    let attached: Vec<_> = shown
        .iter()
        .map(|(message, _, _)| {
            super::pgp::attachments(message).unwrap_or_else(|| attachment_rows(message))
        })
        .collect();

    rsx! {
        {fetcher}
        div { class: "reader-head",
            div { class: "head-row",
                if finding.is_some() {
                    FindBar { shell, total, invalid }
                } else {
                    span { class: "spacer" }
                }
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
            if let Some(why) = problem {
                pre { class: "find-err mono", "{why}" }
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
                    // Its own template, so the key is that template's root key and a new one
                    // remounts it: rsx reads a key only on a template's root node, and one on a
                    // nested component is dropped, in a release build and a debug one alike.
                    {rsx! { super::unsubscribe::Leave { key: "{leave_key}", thread, bodies: bodies.clone(), revision } }}
                }
            }
            // Under the head, where a question about this message belongs. Keyed like Leave.
            {rsx! { super::receipt::Receipts { key: "{leave_key}", bodies } }}
        }
        div { class: "reader-body",
            div { class: "banners",
            if let Some(where_it_went) = saved() {
                // Where it went, named. A file saved somewhere the user cannot point at is a file
                // they have lost, and this pane's previous answer was to print a command to run.
                InlineBanner { severity: Severity::Ok, text: where_it_went, onclose: move |()| saved.set(None) }
            }
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
                                // The press takes the button away, and on Blitz the keyboard with it
                                // (quire focuses the pressed button a frame later, gone or not): it is
                                // handed back to the window, as a closing panel hands it back.
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
            for (((message, reading, _), found), attached) in shown.into_iter().zip(founds).zip(attached) {
                article { key: "{message.id}", class: "frame",
                    header {
                        Label { text: from_name(&message), style: LabelStyle::Headline }
                        Label { text: address(&message), role: LabelRole::Secondary, style: LabelStyle::Footnote }
                        time { Label { text: stamp(&message), role: LabelRole::Tertiary, style: LabelStyle::Footnote } }
                        // Original wherever there is a frame; Source for every message. A span,
                        // not a div: a div between the article and its iframe is a new parent,
                        // and a new parent reloads the frame.
                        ViewSwitch {
                            message_id: message.id,
                            body: message.body.raw(),
                            frame: reading.frame_html().is_some(),
                            showing: original,
                            sources,
                        }
                    }
                    // What the message's OpenPGP or S/MIME says, and its passphrase field. A
                    // sibling before the frame, like the invitation under it, for the same reason.
                    super::pgp::Seal {
                        key: "{message.id}-{message.body.raw():?}",
                        message: message.id,
                        body: message.body.raw(),
                        landed,
                    }
                    // A calendar invitation, drawn by this window and never inside the sender's
                    // HTML. A sibling before the frame, not its parent: it lands after the first
                    // paint, and inserting a sibling does not move the iframe. Keyed on the
                    // message and its body, so a body arriving asks again.
                    super::invite::Invitation {
                        key: "{message.id}-{message.body.raw():?}",
                        message: message.id,
                        body: message.body.raw(),
                    }
                    // What is attached, if anything: what was sent, or what a protected
                    // message was opened to holds. Keyed like the seal, so a body arriving asks
                    // again.
                    if !attached.is_empty() {
                        Attachments {
                            key: "{message.id}-{message.body.raw():?}",
                            message: message.id,
                            body: message.body.raw(),
                            rows: attached,
                            saved,
                            downloading,
                            shell,
                        }
                    }
                    // The iframe, when this message has one, is the first element MessageView
                    // draws, and it is drawn on every render. Toggling Reader / Original changes
                    // a class. Conditionally rendering the iframe would reload it: a new parent,
                    // or a frame that was not in the tree, re-runs the document, loses scroll,
                    // and re-fetches anything just consented to.
                    if matches!(reading, Reading::NotFetched) {
                        p { class: "pending", "Not downloaded" }
                    } else {
                        MessageView {
                            holder: consent.as_ref().map(|(_, holder)| *holder),
                            message_id: message.id,
                            reading: reading.clone(),
                            original,
                            quotes,
                            shell,
                            found,
                        }
                    }
                    // The source, when it is how this message is shown: after the body, so the
                    // frame keeps its parent.
                    SourceView {
                        message: message.id,
                        body: message.body.raw(),
                        showing: original,
                        sources,
                        shell,
                        revision,
                        said: saved,
                    }
                }
            }
            // An inline reply, after every frame so no iframe gains a new parent.
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
                blocks::Sandbox { html, concealed: false }
            }
        }
    }
}

#[cfg(test)]
mod find_tests;
#[cfg(test)]
mod tests;
