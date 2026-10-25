#[cfg(feature = "anyview")]
mod anyview;
mod attachments;
pub(super) mod blocks;
mod cache;
mod column;
mod fetch;
pub(in crate::ui) mod find;
mod fit;
mod fold;
mod header;
mod images;
mod peek;
mod route;
mod thumb;
mod tools;

use super::press::on_primary;
use super::text::{attachment_rows, stamp};
use crate::ui::view::Shell;
use attachments::Attachments;
pub(in crate::ui) use cache::{frames, looks, use_frames, use_looks, use_warming};
pub(in crate::ui) use column::reading_column;
use dioxus::prelude::*;
use ds::components::content::avatar::{
    AvatarFace, AvatarShape, AvatarSize, AvatarTone, person_hue,
};
use ds::components::content::label::LabelStyle;
use ds::prelude::*;
use mail_core::message::{FrameBody, ON_THE_FRAME, Sent};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
pub(in crate::ui) use peek::sweep_left_open;
pub(super) use peek::{PeekView, peek_key};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

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
    AvatarFace {
        initial: initial(message.from.shown()),
        size: AvatarSize::Size34,
        tone: AvatarTone::Person(person_hue(&message.from.email)),
        shape: AvatarShape::Round,
    }
}

/// When `message` came, as a list row writes it: the time today, a weekday this week, a date
/// beyond. The header's tooltip has the full date.
fn when_short(
    message: &Message,
    now: chrono::DateTime<chrono::Utc>,
    clock: mail_core::when::Clock,
) -> String {
    mail_core::when::listed(message.date, now, &chrono::Local, clock)
}

/// The key [`super::unsubscribe::Lookout`] and [`super::receipt::Receipts`] are mounted under:
/// the thread and what each of its messages holds, so a body arriving asks again and another
/// thread never shows this one's answer.
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
    let frames = use_frames();
    let looks = use_looks();
    // ⌘F's frames, which every message's frame below joins (`find`).
    let found = find::use_find(shell);
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
    // The earlier messages a press opened, and those that were unread when the conversation was
    // opened (`fold`): an earlier message is one line unless it is in either.
    let unfolded = use_signal(HashSet::<MessageId>::new);
    let unread = use_hook(|| Rc::new(RefCell::new(HashSet::<MessageId>::new())));
    // The conversation's way out of its list, as the lookout found it, under the key it was
    // found for (`leave_key`): the ⋯ offers it only under this conversation's key.
    let way_out = use_signal(|| None::<(String, super::unsubscribe::Offer)>);
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
    // The images each shown message's frame embeds, as last drawn, for a click on one of them in
    // the frame (`original::FrameImages`) to open in Peek.
    let embedded = use_hook(|| Rc::new(RefCell::new(Embedded::new())));
    use_frame_images(
        shell,
        consent.as_ref().map(|(_, holder)| *holder),
        embedded.clone(),
        store.clone(),
    );
    // Opening a conversation reads it: an unread one is marked read once it is drawn (on the
    // server too, where the server keeps flags), with no toast and no undo, and the list loses
    // its dot. The messages that were unread as it opened stay unfolded: the render gathered
    // them into `unread` before this runs.
    {
        let store = store.clone();
        use_effect(use_reactive!(|thread| {
            let unread = store
                .thread(thread)
                .is_ok_and(|loaded| loaded.summary.read == ReadState::Unread);
            if unread
                && super::ops::resolve(&store, thread, OpKind::MarkRead).is_some_and(|op| {
                    super::ops::perform(&store, thread, op, super::clock::hours()).is_some()
                })
                && let Some(mut revision) = revision
            {
                revision += 1;
            }
        }));
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
    let sent = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(Sent::default())));
    let rendered_later = use_signal(|| 0u64);
    let _ = rendered_later();
    // The same for senders' checks, on which whether a message's images load can turn
    // (`images::look_later`).
    let looked = use_hook(|| Rc::new(RefCell::new(images::Checked::default())));
    let checked_later = use_signal(|| 0u64);
    let _ = checked_later();
    let mut wanted = Vec::new();
    let mut room = ON_THE_FRAME;
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
    let mut shown: Vec<(Message, Option<Arc<FrameBody>>, bool, bool)> = loaded
        .messages
        .iter()
        .rev()
        .filter_map(|id| store.message(*id).ok())
        .map(|message| {
            // A body decrypted without proof it is unchanged never loads remote content, and is
            // offered no button to: the seal's line says why (`Looks::opened_may_load_remote`).
            let may = looks.opened_may_load_remote(&message);
            let showing = may
                && (pressed
                    || images::auto_allow_message(
                        &reading,
                        &looks,
                        &looked.borrow(),
                        &message,
                        &mut wanted,
                    ));
            let policy = shell.read().policy_with(showing);
            let mut frame = |policy| {
                frames.on_the_frame(
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
                _ if !may => false,
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
    found.order(shown.iter().map(|(message, ..)| message.id).collect());
    *embedded.borrow_mut() = shown
        .iter()
        .map(|(message, body, ..)| {
            let images = body.as_deref().map(FrameBody::embedded).unwrap_or_default();
            (message.id, images.to_vec())
        })
        .collect();
    if !later.is_empty() {
        cache::render_later(
            frames.clone(),
            store.clone(),
            later,
            sent.clone(),
            rendered_later,
        );
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
                    let fetches = body.as_deref().map(FrameBody::frame_fetches);
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
    let leave = way_out
        .read()
        .as_ref()
        .filter(|(found, _)| *found == leave_key)
        .map(|(_, offer)| offer.clone());
    // An encrypted message's subject travels inside it; the outside says `...`.
    let subject = shown
        .iter()
        .find(|(message, ..)| message.subject == loaded.summary.subject)
        .and_then(|(message, ..)| looks.opened_subject(message))
        .unwrap_or_else(|| loaded.summary.subject.clone());
    // How many messages there are, and the moment their dates are written against: the newest
    // says everything in its header, the rest who and when.
    let count = shown.len();
    unread.borrow_mut().extend(
        shown
            .iter()
            .filter(|(message, ..)| message.read == ReadState::Unread)
            .map(|(message, ..)| message.id),
    );
    let folds: Vec<fold::Fold> = shown
        .iter()
        .enumerate()
        .map(|(at, (message, ..))| {
            let held =
                unread.borrow().contains(&message.id) || unfolded.read().contains(&message.id);
            fold::fold_at(
                at,
                count,
                if held {
                    fold::Hold::Open
                } else {
                    fold::Hold::Nothing
                },
            )
        })
        .collect();
    let now = super::clock::now();
    let hours = super::clock::hours();
    // The images line, only while images are held back: it is drawn on the newest such message
    // and asks about its sender. Images once shown say nothing more.
    let blocked = shown
        .iter()
        .zip(&folds)
        .rev()
        .find(|((_, _, remote, showing), fold)| **fold == fold::Fold::Open && *remote && !showing)
        .map(|((message, ..), _)| message.clone());
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
        let passed = images::dmarc_known(&looks, &looked.borrow(), message, &mut wanted)?;
        (!images::suspicious(message.from.name.as_deref(), &message.from.email, passed))
            .then(|| message.from.email.trim().to_lowercase())
    });
    if !wanted.is_empty() {
        let messages = shown.iter().map(|(message, ..)| message.clone()).collect();
        images::look_later(
            looks.clone(),
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
            super::pgp::attachments(&looks, message).unwrap_or_else(|| attachment_rows(message))
        })
        .collect();

    // The person's own addresses, for the header to leave out a To line of only them.
    let yours: Vec<String> = super::data::account_rows(&store)
        .into_iter()
        .map(|row| row.address.trim().to_lowercase())
        .collect();
    let toolbar = tools::toolbar_ops(&mail_core::view::hover_actions(&loaded.summary));
    rsx! {
        div { class: "reader-head",
            div { class: "head-row",
                if shell.read().find.is_some() {
                    find::FindBar { shell }
                }
                span { class: "spacer" }
                div { class: "bar-tools",
                    if let Some(revision) = revision {
                        tools::ReaderTools { summary: loaded.summary.clone(), ops: toolbar.clone(), shell, revision }
                        tools::ReaderMore {
                            summary: loaded.summary.clone(),
                            toolbar: toolbar.clone(),
                            peek: (place == ReaderIn::Pane).then_some(peek),
                            shell,
                            revision,
                            leave,
                        }
                    }
                }
            }
            h2 { Label { text: subject, style: LabelStyle::Title, common: crate::ui::common::classed("reader-subject") } }
            super::receipt::Receipts { key: "{leave_key}", bodies: bodies.clone() }
            super::unsubscribe::Lookout { key: "{leave_key}", thread, bodies: bodies.clone(), tag: leave_key.clone(), found: way_out }
        }
        div { class: "reader-body",
            for (at, (((message, body, ..), attached), fold)) in shown.into_iter().zip(attached).zip(folds).enumerate() {
                article { key: "{message.id}", class: "frame",
                    oncontextmenu: move |event: MouseEvent| {
                        let asked = frame_menus.peek().as_ref().and_then(|menus| menus.take());
                        if let Some(asked) = asked {
                            event.prevent_default();
                            event.stop_propagation();
                            link_menu.set(Some(asked));
                        }
                    },
                    if fold == fold::Fold::Folded {
                        fold::FoldedLine {
                            message: message.clone(),
                            when: (when_short(&message, now, hours), stamp(&message, hours)),
                            unfolded,
                        }
                    }
                    if fold != fold::Fold::Folded {
                        header::MessageHead {
                            detail: header::detail_at(at, count),
                            when: (when_short(&message, now, hours), stamp(&message, hours)),
                            extras: (header::detail_at(at, count) == header::Detail::Full).then(|| header::Extras {
                                checked: (message.id, message.body.raw()),
                                landed,
                            }),
                            message: message.clone(),
                            yours: yours.clone(),
                        }
                    }
                    if fold != fold::Fold::Folded && blocked.as_ref().is_some_and(|blocked| blocked.id == message.id) {
                            ImagesHidden { shell, offer: offer.clone() }
                    }
                    if fold != fold::Fold::Folded {
                        super::pgp::Seal {
                            key: "{message.id}-{message.body.raw():?}",
                            message: message.id,
                            body: message.body.raw(),
                            landed,
                        }
                    }
                    if fold != fold::Fold::Folded {
                        super::invite::Invitation {
                            key: "{message.id}-{message.body.raw():?}",
                            message: message.id,
                            body: message.body.raw(),
                        }
                    }
                    if fold != fold::Fold::Folded {
                        match body.as_deref() {
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
                            Some(FrameBody::Present { html, plain, .. }) => rsx! {
                                blocks::MessageView {
                                    holder: consent.as_ref().map(|(_, holder)| *holder),
                                    message_id: message.id,
                                    html: html.clone(),
                                    plain: *plain,
                                    fit: at + 1 < count,
                                }
                            },
                        }
                    }
                    // After the body, where Mail shows a message's files: Tab passes through the
                    // frame to them (Blitz 4ca21b12).
                    if fold != fold::Fold::Folded && !attached.is_empty() {
                            Attachments {
                                key: "{message.id}-{message.body.raw():?}",
                                message: message.id,
                                body: message.body.raw(),
                                rows: attached,
                                shell,
                            }
                    }
                }
            }
            if let Some(asked) = link_menu() {
                LinkMenu { asked, menus: frame_menus, open: link_menu, shell }
            }
            {children}
        }
    }
}

/// Each shown message and the images its frame embeds (`FrameBody::embedded`).
type Embedded = Vec<(MessageId, Vec<mail_mime::EmbeddedImage>)>;

/// On Blitz, a click on an image in an Original frame is reported by quire outside any component
/// (`ui/original/links.rs`). One in a frame this reader drew (its tag names `holder`), drawn from
/// one of that message's own parts, opens the part in Peek, as its tile in the strip does. Any
/// other image, a remote one among them, does nothing.
fn use_frame_images(
    shell: Signal<Shell>,
    holder: Option<super::original::Holder>,
    embedded: Rc<RefCell<Embedded>>,
    store: Arc<SqliteStore>,
) {
    let images = use_hook(|| {
        try_consume_context::<super::original::FrameImages>().map(|images| images.listen())
    });
    use_future(move || {
        let images = images.clone();
        let embedded = embedded.clone();
        let store = store.clone();
        async move {
            let (Some(mut images), Some(holder)) = (images, holder) else {
                return;
            };
            while let Some(clicked) = images.next().await {
                let Some((by, message)) = super::original::Holder::of_tag(&clicked.tag) else {
                    continue;
                };
                if by != holder {
                    continue;
                }
                let cid = embedded
                    .borrow()
                    .iter()
                    .find(|(id, _)| *id == message)
                    .and_then(|(_, images)| mail_mime::drawn_from(&clicked.src, images));
                if let Some(cid) = cid {
                    peek::open_inline(shell, &store, message, &cid);
                }
            }
        }
    });
}

/// One quiet line over the newest message whose remote images are held back: they are hidden,
/// and Load Images shows this conversation's. `offer` is the sender whose images could load from
/// now on, where that is safe to offer (Always Load from). Not a banner: nothing is wrong.
#[component]
fn ImagesHidden(shell: Signal<Shell>, offer: Option<String>) -> Element {
    use super::copy::{controls, status};
    use ds::components::controls::button_model::Bezel;
    use ds::root::common::Common;
    rsx! {
        p { class: "images-hidden", role: "status",
            Label { text: status::IMAGES_HIDDEN, role: ds::components::content::label::LabelRole::Tertiary, style: LabelStyle::Footnote }
            Button {
                bezel: Bezel::Inline,
                label: controls::LOAD_IMAGES,
                common: Common { aria_label: Some(controls::LOAD_IMAGES.to_owned()), ..Common::default() },
                onclick: on_primary(move || {
                    shell.write().show_remote_images = true;
                    super::host::Host::focus_app();
                }),
            }
            if let Some(email) = offer {
                Button {
                    bezel: Bezel::Inline,
                    label: controls::always_load_from(&email),
                    common: Common { aria_label: Some(controls::always_load_from(&email)), ..Common::default() },
                    onclick: on_primary(move || {
                        // The sender joins the list, and under Ask the mode becomes Trusted
                        // (`ReadingSettings::trust_images_from`); written to settings.toml and told
                        // to the other windows.
                        let email = email.clone();
                        if let Err(why) = crate::ui::prefs::change(|settings| {
                            settings.reading.trust_images_from(&email);
                        }) {
                            log::warn!("remote images: {why}");
                        }
                        // This conversation's images now, whatever the write did.
                        shell.write().show_remote_images = true;
                        super::host::Host::focus_app();
                    }),
                }
            }
        }
    }
}

/// Open Link, Preview Link and Copy Link, at the pointer, for a link a right click in a message's
/// frame was on. Preview Link shows where it goes in Peek (`peek::open_link`).
#[component]
fn LinkMenu(
    asked: super::original::LinkAsked,
    menus: Signal<Option<super::original::FrameMenus>>,
    open: Signal<Option<super::original::LinkAsked>>,
    shell: Signal<Shell>,
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
    let text = asked.text.clone();
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
                    PREVIEW_LINK => peek::open_link(
                        shell,
                        crate::ui::view::LinkCard { text: text.clone(), href: href.clone() },
                    ),
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
const PREVIEW_LINK: &str = "preview-link";
const COPY_LINK: &str = "copy-link";

/// The link menu's rows, in the Mac's order.
fn link_items() -> Vec<super::menu::MenuItem> {
    [
        (OPEN_LINK, super::copy::menus::OPEN_LINK, Icon::Link),
        (PREVIEW_LINK, super::copy::menus::PREVIEW_LINK, Icon::Globe),
        (COPY_LINK, super::copy::menus::COPY_LINK, Icon::Copy),
    ]
    .into_iter()
    .map(|(key, name, icon)| super::menu::MenuItem::icon(key, icon, name))
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
