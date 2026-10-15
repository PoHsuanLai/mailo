//! The composer: a page in the reader column, on the Rust editor core.
//!
//! A new message is a page: the subject as its title, To, and the body at 66ch, over a foot of
//! Attach, ⋯ and Send ▾. Every other header row is drawn only while it says something that is not
//! the default (`props.rs`), and the choices that set them are in the foot's menus (`foot.rs`). A
//! reply is the same grid and foot under the thread it answers, with no bar and no subject. The body is one
//! root, quire's `EditSurface`, whose every edit goes through `mail_core::editor`; the page never edits
//! text itself. `adapt` turns what the surface hands over into editor events, and [`wire`] applies
//! them to the page.
//!
//! Esc parks the draft in Today; Send folds the page away and hands it to the outbox pill,
//! whose Undo brings back exactly what was sent.

// The adapter from quire's `EditSurface` to the editor core, and the surface: the body.
mod adapt;
mod attach;
mod body;
mod desk;
mod emoji;
mod float;
mod foot;
mod hand;
mod items;
mod later;
mod life;
mod opening;
mod page;
mod pill;
mod props;
mod protection;
mod receipt;
mod recipients;
mod remind;
mod render;
mod seal;
mod spell;
mod surface;
mod templates;
mod wire;

#[cfg(test)]
mod tests;

use crate::ui::actions::{self, Heard, Own};
use ds::components::controls::button_model::Bezel;
use ds::components::overlays::inline_banner::InlineBanner;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use std::sync::Arc;

use dioxus::prelude::*;
use mail_core::{SqliteStore, Store};
use mail_domain::{Change, ChangeId, DraftId, Patch};

use attach::Attach;
use body::Body;
use foot::{MoreButton, SendButton};
use life::{Anyway, Sent};
use page::{Fold, Guard, List, Phase, Saved};
use props::{Attached, Props};
use seal::{BarAct, SealBar, SealWarn, Sealed, seal_and_queue};

pub(in crate::ui) use desk::{Desk, park_current, reopen, show_queued, unpark, use_desk};
use hand::Handed;
pub use hand::Hub;
pub(in crate::ui) use hand::use_handed;
pub(in crate::ui) use later::{cancel_scheduled, scheduled, send_again};
pub(in crate::ui) use page::Page;
pub(in crate::ui) use page::PageKind;
pub(in crate::ui) use pill::SendPill;
pub use spell::Dictionaries;
pub(in crate::ui) use spell::{dictionaries, use_test_dictionaries};
pub(in crate::ui) use templates::{every as every_template, template_rows};

use super::press::on_primary;
use crate::ui::view::Shell;
use mail_core::password::Password;

/// The fold after Send: quire's timer for the fade, and what happens once it has
/// settled, which is that the page is taken away. Made by the page, so it belongs to it.
#[derive(Clone, Copy)]
struct Folding {
    timer: ds::motion::timer::MotionTimer,
    then: EventHandler<()>,
}

impl Folding {
    fn start(self) {
        self.timer.start(self.then);
    }
}

/// The draft being composed and where its page goes, or `None` when nothing is. A new message
/// that is not held in place has a window of its own ([`to_window`]), so it is drawn nowhere here.
pub(in crate::ui) fn composing(shell: &Shell) -> Option<(DraftId, PageKind)> {
    let open = shell.composing.as_ref()?;
    match kind_of(open.draft) {
        PageKind::New if !open.in_place && windows_open() => None,
        kind => Some((open.draft, kind)),
    }
}

/// The new message this window was asked to compose, which opens in a compose window of its own
/// rather than here: a new message or a forward, not yet brought into place.
pub(in crate::ui) fn to_window(shell: &Shell) -> Option<DraftId> {
    let open = shell.composing.as_ref()?;
    (!open.in_place && windows_open() && kind_of(open.draft) == PageKind::New).then_some(open.draft)
}

/// Whether this window can open another: it was launched (quire hands every launched window its
/// `WindowHandle`) or handed a `ComposeWindows`. Where it cannot (a snapshot, quire's harness), a
/// new message is drawn in place from its first frame.
fn windows_open() -> bool {
    try_consume_context::<crate::ui::window::ComposeWindows>().is_some()
        || try_consume_context::<ds_blitz::WindowHandle>().is_some()
}

/// Whether `draft` answers a message (a reply, under its thread) or stands alone.
fn kind_of(draft: DraftId) -> PageKind {
    let store = consume_context::<Arc<SqliteStore>>();
    match store.draft(draft) {
        Ok(stored) if stored.in_reply_to.is_some() => PageKind::Reply,
        _ => PageKind::New,
    }
}

/// The page for `draft`. Loads it from where it was parked, else from the store.
#[component]
pub(in crate::ui) fn ComposerPage(
    draft: DraftId,
    shell: Signal<Shell>,
    revision: Signal<u64>,
) -> Element {
    let desk = use_context::<Desk>();
    let loaded = use_hook(|| {
        let store = consume_context::<Arc<SqliteStore>>();
        let mut parked = desk.parked;
        let page = life::load(&store, draft, &mut parked.write());
        desk::unpark(desk, draft);
        page
    });
    match loaded {
        Some(initial) => rsx! { PageView { initial, shell, revision } },
        None => rsx! {
            div { class: "reader-empty",
                p { "That draft is gone." }
            }
        },
    }
}

/// Where the keyboard is when a page opens, so what is typed next is written: r, a, f, c and
/// every other way in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Opening {
    /// A new message nobody is in yet: its To.
    To,
    /// A reply, or a message already addressed: its body.
    Body,
}

/// Where `page` takes the keyboard as it opens.
pub(super) fn opening(page: &Page) -> Opening {
    match (page.kind, page.to.as_slice()) {
        (PageKind::New, []) => Opening::To,
        (PageKind::Reply, _) | (PageKind::New, [_, ..]) => Opening::Body,
    }
}

#[component]
fn PageView(initial: Page, shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let mut desk = use_context::<Desk>();
    let keys = use_keys();
    let mut page = use_signal(|| initial.clone());
    let folding = Folding {
        timer: use_motion_timer(Anim::Fade),
        then: use_callback(move |()| close(page, shell, desk)),
    };
    let opened_on = use_hook(|| shell.peek().open);
    use_hook(|| desk.current.set(Some(page)));
    // Leaving without Esc, say for another draft, still keeps this one.
    use_drop(move || {
        let writing = page.try_peek().map(|page| page.phase == Phase::Writing);
        if writing == Ok(true) {
            desk::keep(desk, page);
        }
        if let Ok(mut current) = desk.current.try_write()
            && *current == Some(page)
        {
            *current = None;
        }
    });
    // Autosave: once the page has been still for a moment, through the draft save path.
    use_future(move || async move {
        let mut last = u64::MAX;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            let (edits, dirty) = {
                let read = page.peek();
                (
                    read.edits,
                    read.saved == Saved::Dirty && read.phase == Phase::Writing,
                )
            };
            if dirty && edits == last {
                let store = consume_context::<Arc<SqliteStore>>();
                let saved = life::save(&store, &mut page.write(), chrono::Utc::now());
                if let Err(why) = saved {
                    page.write().notice = Some(why);
                }
            }
            last = edits;
        }
    });
    // A reply opens under the thread, which may run past the pane: brought into view, so its
    // bar and Send are not left under the window's edge.
    use_hook(|| {
        if page.peek().kind == PageKind::Reply {
            super::host::Host::scroll_into_view(".inline-reply .c-foot");
        }
    });
    // Files dropped on the page from a file manager are attached as picked ones are.
    let target = attach::use_drop_target(page);
    // Opening another conversation would redraw the reader: park first.
    if shell.read().open != opened_on && page.peek().phase == Phase::Writing {
        desk::park(desk, page, shell);
        return rsx! {};
    }

    let read = page.read();
    let reply = read.kind == PageKind::Reply;
    let root = match (reply, read.phase) {
        (true, Phase::Folding) => "inline-reply parking",
        (true, _) => "inline-reply",
        (false, Phase::Folding) => "cpage sending",
        (false, _) => "cpage",
    };
    let subject = read.subject.clone();
    let notice = read.notice.clone();
    let warn = read.guard == Guard::Warn;
    let seal_bar = read.seal_bar.clone();
    let anyway_label = "Send anyway";
    // The plain text is written out only while it is shown: not on every key.
    let flowed = match read.plain {
        Fold::Open => Some(mail_core::editor::to_flowed(&read.session.doc)),
        Fold::Folded => None,
    };
    drop(read);

    let send = move |anyway: Anyway| send_page(page, desk, revision, folding, anyway, None);
    // Made here, so a send or a lookup the bar starts belongs to the page and not to the bar,
    // which goes away as it starts.
    let on_seal = use_callback(move |act: BarAct| seal_act(page, desk, revision, folding, act));

    rsx! {
        div {
            class: "{root}",
            "data-drop": target.drop_attr(),
            onmounted: move |event| target.mounted(event),
            onkeydown: move |event: KeyboardEvent| {
                let key = event.key().to_string();
                // The page is a text surface: the chords that reach it are the ones a field leaves.
                let heard = actions::heard(keys, &event, true);
                if heard == Some(Heard::Own(Own::Send)) {
                    event.prevent_default();
                    event.stop_propagation();
                    send(Anyway::No);
                } else if heard == Some(Heard::Own(Own::AddCc)) {
                    event.prevent_default();
                    event.stop_propagation();
                    props::reveal(page, List::Cc);
                } else if heard == Some(Heard::Own(Own::AddBcc)) {
                    event.prevent_default();
                    event.stop_propagation();
                    props::reveal(page, List::Bcc);
                } else if key == "Escape" {
                    event.prevent_default();
                    event.stop_propagation();
                    escape(page, shell, desk);
                }
            },
            // No bar: autosave is silent, keeping or deleting the draft is the foot's ⋯ (and
            // Esc), and a window of its own is made larger by its frame, not by a mode.
            div { class: "c-scroll",
                // The head reads as a message's (`reader.css`): the subject where its title is,
                // at its size, and the header lines under it, one hairline under them all.
                if !reply {
                    div { class: "c-head",
                        TextField {
                            label: "Subject".to_owned(),
                            bezel: FieldBezel::Plain,
                            placeholder: "Subject".to_owned(),
                            value: subject,
                            oninput: move |value: String| {
                                let mut write = page.write();
                                write.subject = value;
                                write.touch();
                            },
                            common: Common { extra_class: ExtraClass::parse("c-title").ok(), ..Common::default() },
                        }
                    }
                }
                Props { page, shell }
                div { class: "c-main",
                    if let Some(notice) = notice {
                        InlineBanner {
                            severity: Severity::Info,
                            text: notice,
                            onclose: move |()| page.write().notice = None,
                        }
                    }
                    // `/` › Attachment opens the picker the foot's Attach does.
                    Body { page, shell, on_attach: move |_| attach::choose(page) }
                    if let Some(flowed) = flowed {
                        div { class: "plain", "{flowed}" }
                    }
                    Attached { page }
                }
            }
            div { class: "c-foot",
                if warn {
                    InlineBanner {
                        severity: Severity::Warn,
                        icon: Some(Icon::Paperclip),
                        text: "No file attached",
                        actions: rsx! {
                            Attach { page, label: "Attach a file", bezel: Bezel::Push }
                            Button {
                                label: "Send anyway",
                                onclick: on_primary(move || send(Anyway::Yes)),
                                common: Common { aria_label: Some(anyway_label.to_owned()), ..Common::default() },
                            }
                        },
                    }
                }
                SealWarn { bar: seal_bar, on_act: on_seal }
                Attach { page, label: "Attach", bezel: Bezel::Toolbar }
                MoreButton {
                    page,
                    windowed: desk.hand.read().is_some(),
                    on_main: move |()| to_main(page, shell, desk),
                    on_keep: move |()| desk::park(desk, page, shell),
                    on_delete: move |()| delete_draft(page, shell, desk, revision),
                }
                span { class: "grow" }
                SendButton { page, on_send: move |()| send(Anyway::No) }
            }
        }
    }
}

/// Send, through the guards. Queued, the page folds away and the pill takes over. A draft to be
/// signed or encrypted is sealed off the thread that draws, with `passphrase` for its key when
/// one was typed; it is dropped once the send is done.
fn send_page(
    mut page: Signal<Page>,
    desk: Desk,
    revision: Signal<u64>,
    folding: Folding,
    anyway: Anyway,
    passphrase: Option<Password>,
) {
    let store = consume_context::<Arc<SqliteStore>>();
    let secrets = super::pgp::seams().secrets;
    let sent = life::send(
        &store,
        secrets.as_ref(),
        &mut page.write(),
        anyway,
        // The window's clock, which a reminder asked for here is later measured against.
        super::clock::now(),
        &chrono::Local,
    );
    match sent {
        Ok(Sent::Sealing { leaves, remind }) => {
            let draft = page.peek().draft;
            // Spawned from a press, which is where a task is polled (F140).
            spawn(async move {
                match seal_and_queue(store, draft, leaves, remind, passphrase).await {
                    Ok(due) => {
                        let sent = life::folded(&mut page.write(), due);
                        queued(page, desk, revision, folding, sent);
                    }
                    Err(Sealed::Locked { key, tried }) => {
                        page.write().seal_bar = SealBar::Locked { key, tried };
                    }
                    Err(Sealed::Refused(why)) => {
                        let mut write = page.write();
                        write.seal_bar = SealBar::Clear;
                        write.notice = Some(why);
                    }
                }
            });
        }
        Ok(sent) => queued(page, desk, revision, folding, sent),
        Err(why) => page.write().notice = Some(why),
    }
}

/// What the signing and encrypting bar's buttons do.
fn seal_act(
    mut page: Signal<Page>,
    desk: Desk,
    revision: Signal<u64>,
    folding: Folding,
    act: BarAct,
) {
    // The bar is only ever reached past the other guards, so its sends go as "anyway".
    match act {
        BarAct::WithoutEncryption | BarAct::Plain => {
            let mut write = page.write();
            write.protection = match act {
                BarAct::Plain => protection::Protection::None,
                _ => write.protection.without_encryption(),
            };
            write.seal_bar = SealBar::Clear;
            write.touch();
            drop(write);
            send_page(page, desk, revision, folding, Anyway::Yes, None);
        }
        BarAct::Unlock(passphrase) => {
            send_page(page, desk, revision, folding, Anyway::Yes, Some(passphrase));
        }
        // Making a key or importing a certificate is the sending account's page's, in Settings.
        BarAct::OpenKeys => {
            let store = consume_context::<Arc<SqliteStore>>();
            match store.draft(page.peek().draft) {
                Ok(stored) => super::settings_window::open_account(stored.account),
                Err(_) => super::settings_window::open_at(crate::ui::view::SettingsPage::Accounts),
            }
        }
        BarAct::LookUp(addresses) => {
            page.write().seal_bar = SealBar::Looking(addresses.clone());
            let store = consume_context::<Arc<SqliteStore>>();
            let lookup = super::pgp::seams().lookup;
            let draft = page.peek().draft;
            spawn(async move {
                let done = tokio::task::spawn_blocking(move || {
                    let stored = store.draft(draft).map_err(|e| e.to_string())?;
                    seal::look_up(
                        &store,
                        lookup.as_ref(),
                        &stored,
                        &addresses,
                        chrono::Utc::now(),
                    )
                })
                .await
                .unwrap_or_else(|error| Err(format!("The lookup stopped: {error}")));
                let mut write = page.write();
                match done {
                    Ok((bar, said)) => {
                        write.seal_bar = bar;
                        write.notice = Some(said);
                    }
                    Err(why) => {
                        write.seal_bar = SealBar::Clear;
                        write.notice = Some(why);
                    }
                }
            });
        }
    }
}

/// A press of Send's outcome: queued, the page folds away and the pill takes over.
fn queued(
    page: Signal<Page>,
    mut desk: Desk,
    mut revision: Signal<u64>,
    folding: Folding,
    sent: Sent,
) {
    match sent {
        Sent::Queued { draft, due, when } => {
            let mut back = page.peek().clone();
            back.phase = Phase::Writing;
            back.guard = Guard::Clear;
            let outgoing = desk::Outgoing {
                draft,
                due,
                when,
                page: back,
                refused: None,
            };
            // A compose window hands the send to the main window, whose pill takes it (and its
            // Undo) while this window closes.
            match desk.hand.peek().clone() {
                Some(hub) => hub.hand(Handed::Queued(outgoing)),
                None => {
                    desk.outbox.set(Some(outgoing));
                    desk::unpark(desk, draft);
                }
            }
            revision += 1;
            // The page folds away on quire's clock, and goes once the fold has settled.
            folding.start();
        }
        // Sealing is answered by the send that started it; a stop has already said why.
        Sent::Stopped | Sent::Sealing { .. } => {}
    }
}

/// Esc: a menu first, then the draft is parked.
fn escape(mut page: Signal<Page>, shell: Signal<Shell>, desk: Desk) {
    if page.peek().float != page::Float::Closed {
        page.write().float = page::Float::Closed;
    } else {
        desk::park(desk, page, shell);
    }
}

/// ⋯ › Open in Main Window, in a compose window: the page, saved and exactly as it is, goes to the
/// main window's reader column, and this window's composer closes (so the window does).
fn to_main(mut page: Signal<Page>, mut shell: Signal<Shell>, desk: Desk) {
    let Some(hub) = desk.hand.peek().clone() else {
        return;
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let saved = life::save(&store, &mut page.write(), chrono::Utc::now());
    if let Err(why) = saved {
        page.write().notice = Some(why);
        return;
    }
    let mut moved = page.peek().clone();
    moved.float = page::Float::Closed;
    moved.selection = None;
    hub.hand(Handed::ToMain(moved));
    page.write().phase = Phase::Closed;
    shell.write().close_composer();
}

/// The fold has run: the page goes, the pill stays.
fn close(mut page: Signal<Page>, mut shell: Signal<Shell>, _desk: Desk) {
    if page.peek().phase != Phase::Folding {
        return;
    }
    page.write().phase = Phase::Closed;
    let draft = page.peek().draft;
    if shell.peek().composing.as_ref().map(|open| open.draft) == Some(draft) {
        shell.write().close_composer();
    }
}

/// Delete the draft, as ⋯ › Delete Draft does, and offer it back. Saved first, so the undo
/// brings back what was on the page; the undo is the window's (the toast's tab and ⌘Z), and puts
/// the draft back in Drafts. A draft already on its way is refused, as before (334b758).
fn delete_draft(
    mut page: Signal<Page>,
    mut shell: Signal<Shell>,
    desk: Desk,
    mut revision: Signal<u64>,
) {
    let store = consume_context::<Arc<SqliteStore>>();
    let draft = page.peek().draft;
    let kept = life::save(&store, &mut page.write(), chrono::Utc::now());
    let stored = match kept {
        Ok(stored) => stored,
        Err(why) => {
            page.write().notice = Some(why);
            return;
        }
    };
    match mail_core::compose::discard(&store, draft) {
        Ok(_) => {
            page.write().phase = Phase::Closed;
            desk::unpark(desk, draft);
            let account = stored.account.clone();
            let undo = mail_core::undo::Undo {
                said: "Draft deleted".to_owned(),
                thread: None,
                account,
                forward: Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::DraftDelete(draft)],
                },
                inverse: Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::DraftUpsert(Box::new(stored))],
                },
                remote: None,
                resend: None,
            };
            shell.write().close_composer();
            revision += 1;
            // A compose window closes as the draft goes: the main window holds the undo.
            match desk.hand.peek().clone() {
                Some(hub) => hub.hand(Handed::Undo(undo)),
                None => desk::offer_undo(shell, undo),
            }
        }
        Err(why) => page.write().notice = Some(crate::ui::remedy::told(why)),
    }
}
