#[cfg(feature = "anyview")]
mod anyview;
mod attachments;
pub(super) mod blocks;
mod cache;
mod code;
mod column;
mod fetch;
pub(in crate::ui) mod find;
mod fit;
mod fold;
mod header;
mod images;
mod list_post;
mod marking;
mod peek;
pub(in crate::ui) mod selection;
mod source;
mod thumb;
mod tools;
mod words;

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
use ds::components::overlays::empty_state::EmptyForm;
use ds::prelude::*;
use mail_client::Client;
use mail_client::Snapshot;
use mail_client::message::{INLINE_BYTES, Sent};
use mail_client::query::{Conversation, Messages};
use mail_domain::*;
use mail_reader::FrameBody;
use mail_ui_model::copy;
pub(in crate::ui) use peek::sweep_left_open;
pub(super) use peek::{PeekView, peek_key};
pub(in crate::ui) use source::{Raw, RawAsked, RawSheet, ask as ask_raw, close as close_raw};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
pub(in crate::ui) use tools::compose_on;

/// Whether what is shown is unread now, as the client has the conversation read: `message` when
/// it was opened alone, else any of `thread`. A conversation not read yet, or gone, is not
/// unread: nothing here reads the store.
fn is_unread(read: &Snapshot<Option<Messages>>, thread: ThreadId, only: Option<MessageId>) -> bool {
    let Snapshot::Ready { value, .. } = read else {
        return false;
    };
    let Some(loaded) = value.as_ref().as_ref().filter(|l| l.summary.id == thread) else {
        return false;
    };
    match only {
        Some(message) => loaded
            .messages
            .iter()
            .any(|alone| alone.id == message && alone.read == ReadState::Unread),
        None => loaded.summary.read == ReadState::Unread,
    }
}

/// Mark what is shown read as showing it does: on the server too, where the server keeps flags,
/// with no toast and no undo, and the list loses its dot. A message opened alone is the only one
/// read.
fn mark_read(client: &Client, thread: ThreadId, only: Option<MessageId>) {
    let (op, alone) = match only {
        Some(message) => (Some(Op::SetRead(ReadState::Read)), vec![(thread, message)]),
        None => (
            super::ops::resolve(client, thread, OpKind::MarkRead),
            Vec::new(),
        ),
    };
    let Some(op) = op else {
        return;
    };
    let ticket = super::live::client().act(mail_client::Perform {
        ops: vec![(thread, op)],
        alone,
        then_erase: false,
    });
    // Nothing waits on it: what it commits draws the list and the reader again.
    super::motion::when_done(ticket, |_| ());
}

/// How long the text size is left alone after a press before it is written to the settings: a
/// key held down or tapped on is one write, of where it ended.
const ZOOM_SETTLES: std::time::Duration = std::time::Duration::from_millis(300);

/// Bigger, Smaller or Actual Size: the reader's text size a step along, drawn at once in this
/// window, and written to the settings once the presses stop (the other windows and the next
/// launch then draw messages at it).
pub(in crate::ui) fn zoom(step: mail_ui_model::settings::ZoomStep) {
    static PRESSES: crate::ui::prefs::Latest = crate::ui::prefs::Latest::new();
    let Some(mut shown) = try_consume_context::<Signal<mail_ui_model::settings::MailSettings>>()
    else {
        log::warn!("text size: the window has no settings to change");
        return;
    };
    let next = shown.peek().reading.text_zoom.stepped(step);
    shown.write().reading.text_zoom = next;
    let mine = PRESSES.press();
    let write = crate::ui::prefs::change_later(move |settings| settings.reading.text_zoom = next);
    dioxus::dioxus_core::spawn_forever(async move {
        ds::base::time::clock::sleep(ZOOM_SETTLES).await;
        if !PRESSES.is_last(mine) {
            return;
        }
        if let Err(why) = write.await {
            log::warn!("text size: {why}");
        }
    });
}

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
    clock: mail_client::when::Clock,
) -> String {
    mail_client::when::listed(message.date, now, &chrono::Local, clock)
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

/// A conversation that is no longer there.
fn gone() -> Element {
    rsx! {
        div { class: "reader-empty",
            EmptyState { title: "Conversation gone" }
        }
    }
}

/// A conversation the client could not read: said, with the reason to try again, not drawn as a
/// conversation that is gone.
fn cannot_load() -> Element {
    rsx! {
        div { class: "reader-empty",
            EmptyState {
                form: EmptyForm::Failure,
                title: copy::status::CONVERSATION_CANNOT_LOAD,
                description: Some(copy::status::NOT_READ.to_owned().into()),
            }
        }
    }
}

/// Outline lines where the conversation will be, while the client reads it for the first time:
/// the pane reads as "coming", not as empty.
fn loading() -> Element {
    rsx! {
        div { class: "reader-loading", "aria-busy": "true",
            for n in 0..3 {
                SkeletonRow { key: "{n}", lines: SkeletonLines::Two }
            }
        }
    }
}

/// With `tools`, the reader has its toolbar, as a window's does; a reader drawn on its own has
/// none. `only` is the one message to show, when it was opened from a list of single messages
/// (`Shell::focus`); one that is not the conversation's shows the conversation whole.
#[component]
pub(super) fn Reader(
    thread: ThreadId,
    shell: Signal<Shell>,
    #[props(default)] tools: bool,
    #[props(default)] place: ReaderIn,
    #[props(default)] only: Option<MessageId>,
    children: Element,
) -> Element {
    let client = super::live::client();
    // Where the sender's name marks a VIP: this window's shell.
    use_context_provider(|| crate::ui::vip::Marking { shell });
    // The read receipts the thread's messages ask about, which their headers draw a mark for.
    use_context_provider(|| super::receipt::Asks(Signal::new(Vec::new())));
    let read = super::live::use_live(use_reactive!(|thread| Conversation(thread)));
    let frames = use_frames();
    let looks = use_looks();
    // The person's own addresses, as the client keeps the accounts read: no store is asked.
    let accounts = super::data::use_account_rows();
    // ⌘F's frames, which every message's frame below joins (`find`).
    let found = find::use_find(shell);
    // The client may have moved under this conversation, here or in another window.
    let _ = crate::ui::live::moved_for(
        mail_client::Topics::SUMMARIES
            | mail_client::Topics::LABELS
            | mail_client::Topics::ACCOUNTS
            | mail_client::Topics::IDENTITIES
            | mail_client::Topics::CONTACTS
            | mail_client::Topics::KEYS,
    );
    // Moved when an OpenPGP or S/MIME message has been opened and has a body of its own to show,
    // so the messages are drawn again with it. The opening itself happens in `pgp::Seal`, off
    // this thread; here it is only looked up.
    let landed = use_signal(|| 0u64);
    let _ = landed();
    // The window's settings, for whose images load without asking. Not `prefs::use_settings`: a
    // reader drawn with no window around it (a test's) asks about every image rather than read
    // the person's own `settings.toml`, which is the safe way for a stray reader to be wrong.
    let settings = use_hook(try_consume_context::<Signal<mail_ui_model::settings::MailSettings>>);
    // The link a right click in a message's frame was on, which the window's frames report
    // (`original::FrameMenus`); a reader with no window around it has none. And the Copy Link
    // menu it opens.
    let frame_menus = use_signal(try_consume_context::<super::original::FrameMenus>);
    let mut link_menu = use_signal(|| None::<super::original::LinkAsked>);
    // The words selected in a message a right click was on (`blocks::Sandbox`), and the Copy menu
    // they open where no link was.
    let mut words_asked = use_signal(|| None::<String>);
    // A message's own menu (Show All Headers, View Source), opened by a right click on it that
    // is not on a link: which message, and where.
    let mut message_menu = use_signal(|| None::<MessageAsked>);
    // The earlier messages a press opened, and those that were unread when the conversation was
    // opened (`fold`): an earlier message is one line unless it is in either.
    let unfolded = use_signal(HashSet::<MessageId>::new);
    let unread = use_hook(|| Rc::new(RefCell::new(HashSet::<MessageId>::new())));
    // The conversation's way out of its list, as the lookout found it, under the key it was
    // found for (`leave_key`): the ⋯ offers it only under this conversation's key.
    let way_out = use_signal(|| None::<(String, super::unsubscribe::Offer)>);
    // Whether the newest message names its list's address, as the key it was found under: the ⋯
    // offers Reply to List only under this conversation's key.
    let list_post = use_signal(|| None::<String>);
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
        client.clone(),
    );
    // Showing a conversation reads it: an unread one is marked read once it is drawn, after
    // the wait `reading.mark_read` asks for, or never (`marking`). The messages that were unread
    // as it opened stay unfolded: the render gathered them into `unread` before this runs.
    let pending = use_hook(|| Rc::new(RefCell::new(marking::Pending::default())));
    {
        let pending = pending.clone();
        use_drop(move || pending.borrow_mut().left());
    }
    // Decided once per conversation shown, when the client has read it: what is unread cannot be
    // told before, and a reading that follows a commit (the one this effect's own mark makes)
    // decides nothing again.
    let decided = use_hook(|| Rc::new(std::cell::Cell::new(None::<(ThreadId, Option<MessageId>)>)));
    {
        let client = client.clone();
        use_effect(use_reactive!(|thread, only| {
            if decided.get() == Some((thread, only)) {
                return;
            }
            let now: Snapshot<Option<Messages>> = (*read.read()).clone();
            if !matches!(&now, Snapshot::Ready { value, .. }
                if value.as_ref().as_ref().is_some_and(|loaded| loaded.summary.id == thread))
            {
                return;
            }
            decided.set(Some((thread, only)));
            let reading = settings
                .map(|settings| settings.peek().reading.clone())
                // A reader with no window around it (a test's) marks read on open, as ever.
                .unwrap_or_default();
            let unread = is_unread(&now, thread, only);
            match marking::on_show(reading.mark_read, reading.mark_read_delay, unread) {
                marking::Mark::Now => {
                    pending.borrow_mut().left();
                    mark_read(&client, thread, only);
                }
                marking::Mark::Leave => pending.borrow_mut().left(),
                marking::Mark::After(wait) => {
                    pending
                        .borrow_mut()
                        .shown(thread, ds::base::time::clock::now(), wait);
                    let pending = pending.clone();
                    let client = client.clone();
                    spawn(async move {
                        ds::base::time::clock::sleep(wait).await;
                        let due = pending
                            .borrow_mut()
                            .due(thread, ds::base::time::clock::now());
                        if due && is_unread(&read.peek(), thread, only) {
                            mark_read(&client, thread, only);
                        }
                    });
                }
            }
        }));
    }
    // The conversation as the client keeps it read. A value for another conversation (the last
    // one's, shown while this one is read) is never drawn under this one.
    let snapshot = read();
    let held = snapshot.value().cloned();
    let loaded = match held.as_deref() {
        Some(Some(loaded)) if loaded.summary.id == thread => loaded,
        Some(None) if matches!(snapshot, Snapshot::Ready { .. }) => return gone(),
        _ if matches!(snapshot, Snapshot::Failed { .. }) => return cannot_load(),
        _ => return loading(),
    };
    let only = only.filter(|message| loaded.messages.iter().any(|held| held.id == *message));
    // Read here, in the render, so adding a trusted sender or changing the mode draws the reader
    // again with the images it now allows.
    let reading = settings
        .map(|settings| settings.read().reading.clone())
        .unwrap_or_default();
    // A message with both bodies is rendered, and looked up, as the setting prefers.
    frames.prefer(reading.prefer_plain_text.into());
    // "Load remote images: Never": nothing remote loads, by press or by setting, and nothing
    // offers to load it.
    let block_all = reading.remote_images == mail_ui_model::settings::LoadRemoteImages::Never;
    // Images that cannot be fetched the way Privacy says are not fetched another way: the line
    // over the message says so, where it would offer Load Images.
    let route_problem = crate::ui::prefs::route_problem();
    let pressed = shell.read().show_remote_images && !block_all;
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
    let mut room = INLINE_BYTES;
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
        .filter(|message| only.is_none_or(|alone| alone == message.id))
        .cloned()
        .map(|message| {
            // A body decrypted without proof it is unchanged never loads remote content, and is
            // offered no button to: the seal's line says why (`Looks::opened_may_load_remote`).
            let may = looks.opened_may_load_remote(&message) && !block_all;
            let showing = may
                && (pressed
                    || images::auto_allow_message(
                        &reading,
                        &looks,
                        &looked.borrow(),
                        &message,
                        &mut wanted,
                    ));
            let policy = if block_all {
                Shell::policy_blocked()
            } else {
                shell.read().policy_with(showing)
            };
            let mut frame = |policy| {
                client.framing(&frames).render_inline(
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
                    frame(mail_reader::SanitizePolicy::FRAME).is_some_and(|it| it.blocked_remote())
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
            client.clone(),
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
    let list = list_post.read().as_deref() == Some(leave_key.as_str());
    // An encrypted message's subject travels inside it; the outside says `...`. A message shown
    // alone is titled with its own.
    let titled = match only {
        Some(_) => shown.first().map(|(message, ..)| message.subject.clone()),
        None => None,
    }
    .unwrap_or_else(|| loaded.summary.subject.clone());
    let subject = shown
        .iter()
        .find(|(message, ..)| message.subject == titled)
        .and_then(|(message, ..)| looks.opened_subject(message))
        .unwrap_or(titled);
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
        .find(|((_, _, remote, showing), fold)| {
            // With no usable route even images that would load are held, and the line says why.
            !block_all
                && **fold == fold::Fold::Open
                && *remote
                && (route_problem.is_some() || !showing)
        })
        .map(|((message, ..), _)| message.clone());
    // "Always load from" the blocked sender: offered where it could ever load their images
    // (not under `Always`, which already would; not in Junk, which is always asked about), and
    // never to a sender who is not provably who the address says: trusting a forged address would
    // trust everyone who forges it next.
    // Until the sender's checks are known it is not offered; it appears when they land.
    let offer = blocked.as_ref().and_then(|message| {
        let worth = route_problem.is_none()
            && reading.remote_images != mail_ui_model::settings::LoadRemoteImages::Always
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
            client.clone(),
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

    // The message the ⋯ › Save Attachments… means, among those whose files are drawn.
    let files = attachments::files_meant(
        &shown
            .iter()
            .zip(&attached)
            .zip(&folds)
            .filter(|(_, fold)| **fold != fold::Fold::Folded)
            .map(|(((message, ..), rows), _)| (message.id, rows.len()))
            .collect::<Vec<_>>(),
        only,
    );
    // The person's own addresses, for the header to leave out a To line of only them.
    let yours: Vec<String> = accounts
        .read()
        .iter()
        .map(|row| row.address.trim().to_lowercase())
        .collect();
    let toolbar = tools::toolbar_ops(&mail_client::conversation::allowed_actions(&loaded.summary));
    rsx! {
        div { class: "reader-head",
            div { class: "head-row",
                if shell.read().find.is_some() {
                    find::FindBar { shell }
                }
                span { class: "spacer" }
                div { class: "bar-tools",
                    if tools {
                        tools::ReaderTools { summary: loaded.summary.clone(), ops: toolbar.clone(), shell }
                        tools::ReaderMore {
                            summary: loaded.summary.clone(),
                            toolbar: toolbar.clone(),
                            peek: (place == ReaderIn::Pane).then_some(peek),
                            shell,
                            leave,
                            list,
                            files,
                        }
                    }
                }
            }
            h2 {
                if loaded.summary.star == Star::Starred {
                    span { class: "flag-mark", "data-flag": loaded.summary.flag.as_str(),
                        role: "img", aria_label: mail_ui_model::copy::aria::flagged(loaded.summary.flag),
                        ds::style::icon::render::Glyph { icon: Icon::Star, size: IconSize::Compact }
                    }
                }
                Label { text: subject, style: LabelStyle::Title, common: crate::ui::common::classed("reader-subject") }
            }
            super::receipt::Receipts { key: "{leave_key}", bodies: bodies.clone(), settled: !is_unread(&snapshot, thread, only) }
            super::unsubscribe::Lookout { key: "{leave_key}", thread, bodies: bodies.clone(), tag: leave_key.clone(), found: way_out }
            list_post::PostLookout { key: "{leave_key}", thread, bodies: bodies.clone(), tag: leave_key.clone(), found: list_post }
        }
        div { class: "reader-body",
            for (at, (((message, body, ..), attached), fold)) in shown.into_iter().zip(attached).zip(folds).enumerate() {
                article { key: "{message.id}", class: "frame",
                    oncontextmenu: {
                        let id = message.id;
                        move |event: MouseEvent| {
                            let asked = frame_menus.peek().as_ref().and_then(|menus| menus.take());
                            let words = words_asked.take();
                            event.prevent_default();
                            event.stop_propagation();
                            match asked {
                                Some(asked) => link_menu.set(Some(asked)),
                                None => {
                                    let at = event.client_coordinates();
                                    message_menu.set(Some(MessageAsked {
                                        message: id,
                                        text: words,
                                        at: Point { x: Px(at.x as f32), y: Px(at.y as f32) },
                                    }));
                                }
                            }
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
                    if fold != fold::Fold::Folded {
                        code::CodeLine {
                            key: "{message.id}-{message.body.raw():?}-code",
                            message: message.id,
                            body: message.body.raw(),
                        }
                    }
                    if fold != fold::Fold::Folded && blocked.as_ref().is_some_and(|blocked| blocked.id == message.id) {
                            ImagesHidden { shell, offer: offer.clone(), problem: route_problem }
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
                                    account: message.account.clone(),
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
                                    asked: words_asked,
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
                                account: accounts
                                    .read()
                                    .iter()
                                    .find(|row| row.id == message.account)
                                    .map_or_else(|| copy::errors::THIS_ACCOUNT.to_owned(), |row| row.address.clone()),
                                shell,
                            }
                    }
                }
            }
            if let Some(asked) = link_menu() {
                LinkMenu { asked, menus: frame_menus, open: link_menu, shell }
            }
            if let Some(asked) = message_menu() {
                MessageMenu { asked, open: message_menu, shell }
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
    client: Client,
) {
    let images = use_hook(|| {
        try_consume_context::<super::original::FrameImages>().map(|images| images.listen())
    });
    use_future(move || {
        let images = images.clone();
        let embedded = embedded.clone();
        let client = client.clone();
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
                    peek::open_inline(shell, &client, message, &cid);
                }
            }
        }
    });
}

/// One quiet line over the newest message whose remote images are held back: they are hidden,
/// and Load Images shows this conversation's. `offer` is the sender whose images could load from
/// now on, where that is safe to offer (Always Load from). Not a banner: nothing is wrong. When
/// the relay or proxy chosen in Privacy cannot be used (`problem`), the line says so instead and
/// offers nothing: the images are not fetched straight to make up for it.
#[component]
fn ImagesHidden(
    shell: Signal<Shell>,
    offer: Option<String>,
    problem: Option<mail_ui_model::settings::RouteProblem>,
) -> Element {
    use ds::components::controls::button_model::Bezel;
    use ds::root::common::Common;
    use mail_ui_model::copy::{controls, status};
    if let Some(problem) = problem {
        return rsx! {
            p { class: "images-hidden", role: "status",
                Label { text: status::images_not_routed(problem), role: ds::components::content::label::LabelRole::Tertiary, style: LabelStyle::Footnote }
            }
        };
    }
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

/// A right click on a message, not on a link: which message, the words selected in it (if any),
/// and where the pointer is, in the window's coordinates.
#[derive(Debug, Clone, PartialEq)]
struct MessageAsked {
    message: MessageId,
    text: Option<String>,
    at: Point,
}

/// Copy (for the words selected, which go on the clipboard as ⌘C would put them), Show All
/// Headers and View Source, at the pointer, for the message a right click was on.
#[component]
fn MessageMenu(
    asked: MessageAsked,
    open: Signal<Option<MessageAsked>>,
    shell: Signal<Shell>,
) -> Element {
    let mut open = open;
    let keys = use_keys();
    let placed = Rect {
        origin: asked.at,
        size: Size {
            width: Px(0.0),
            height: Px(0.0),
        },
    };
    let message = asked.message;
    let text = asked.text.clone();
    // A message that asks for a read receipt offers the two answers here as well as in its mark.
    let question = super::receipt::question_of(message);
    rsx! {
        super::menu::Floating {
            anchor: None,
            placed: Some(placed),
            title: String::new(),
            items: keys.with_keymap(|keymap| message_items(keymap, text.is_some(), question.is_some())),
            on_pick: move |key: String| {
                open.set(None);
                if let Some(answer) = super::receipt::answer_of(&key) {
                    super::receipt::give(message, answer);
                    return;
                }
                let raw = match key.as_str() {
                    COPY => {
                        if let Some(text) = &asked.text {
                            super::host::Host::copy(text);
                        }
                        return;
                    }
                    SHOW_HEADERS => Raw::Headers,
                    VIEW_SOURCE => Raw::Source,
                    _ => return,
                };
                source::ask_message(shell, message, raw);
            },
            on_close: move |_| open.set(None),
        }
    }
}

const COPY: &str = "copy";
const SHOW_HEADERS: &str = "show-headers";
const VIEW_SOURCE: &str = "view-source";

/// A message's own menu's rows, each with its chord as `keymap` has it.
fn message_items(
    keymap: &chordkit::Keymap,
    copy: bool,
    receipt: bool,
) -> Vec<super::menu::MenuItem> {
    use super::actions::{Own, hint_own};
    use super::menu::{MenuItem, Right};
    let rows: Vec<MenuItem> = [
        (
            SHOW_HEADERS,
            mail_ui_model::copy::menus::SHOW_ALL_HEADERS,
            Icon::ListTree,
            Own::ShowHeaders,
        ),
        (
            VIEW_SOURCE,
            mail_ui_model::copy::menus::VIEW_SOURCE,
            Icon::FileCode,
            Own::ViewSource,
        ),
    ]
    .into_iter()
    .map(|(key, name, icon, own)| MenuItem {
        right: hint_own(keymap, own).map_or(Right::None, Right::Shortcut),
        ..MenuItem::icon(key, icon, name)
    })
    .collect();
    let copy = copy.then(|| MenuItem::icon(COPY, Icon::Copy, mail_ui_model::copy::menus::COPY));
    let receipt = receipt.then(super::receipt::items).unwrap_or_default();
    copy.into_iter().chain(rows).chain(receipt).collect()
}

const OPEN_LINK: &str = "open-link";
const PREVIEW_LINK: &str = "preview-link";
const COPY_LINK: &str = "copy-link";

/// The link menu's rows, in the Mac's order.
fn link_items() -> Vec<super::menu::MenuItem> {
    [
        (OPEN_LINK, mail_ui_model::copy::menus::OPEN_LINK, Icon::Link),
        (
            PREVIEW_LINK,
            mail_ui_model::copy::menus::PREVIEW_LINK,
            Icon::Globe,
        ),
        (COPY_LINK, mail_ui_model::copy::menus::COPY_LINK, Icon::Copy),
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
