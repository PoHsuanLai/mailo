//! The composer: a page in the reader column, on the Rust editor core.
//!
//! A new message is a page: the subject as its title, To, and the body at 66ch, over a foot of
//! Attach, ⋯ and Send ▾. Every other header row is drawn only while it says something that is not
//! the default (`props.rs`), and the choices that set them are in the foot's menus (`foot.rs`). A
//! reply is the same grid and foot under the thread it answers, with no bar and no subject. The body is one
//! root, quire's `EditSurface`, whose every edit goes through `mail_editor`; the page never edits
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
mod held;
mod items;
mod later;
mod life;
mod page;
mod pill;
mod props;
mod protection;
mod receipt;
mod recipients;
mod remind;
mod render;
mod seal;
mod signoff;
mod sizing;
mod spell;
mod styling;
mod surface;
mod templates;
mod wire;

#[cfg(test)]
pub(in crate::ui) mod tests;

use crate::ui::actions::{self, Heard, Own};
use ds::components::controls::button_model::Bezel;
use ds::components::overlays::inline_banner::InlineBanner;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;

use dioxus::prelude::*;
use mail_client::Client;
use mail_domain::{Change, ChangeId, Draft, DraftId, Patch, SendState};

use attach::Attach;
use foot::{MoreButton, SendButton};
use life::{Anyway, Sent};
pub(in crate::ui) use page::List;
use page::{Fold, Guard, Phase, Saved};
use props::{Attached, Props};
use seal::{BarAct, SealBar, SealWarn, Sealed, seal_and_queue};

/// The body, on its own: Settings writes a signature in it.
pub(in crate::ui) use body::Body;
pub(in crate::ui) use desk::{Desk, park_current, reopen, show_queued, use_desk};
pub(in crate::ui) use later::{cancel_scheduled, scheduled, scheduled_in, send_again, sending_in};
/// Discarding a draft, for every caller in this process: the window's and the intents'.
pub use life::discarding as discarding_draft;
pub(in crate::ui) use page::Page;
pub(in crate::ui) use page::PageKind;
pub(in crate::ui) use page::When;
pub(in crate::ui) use pill::SendPill;
pub(in crate::ui) use props::{Line, list_label};
pub use spell::Dictionaries;
pub(in crate::ui) use spell::use_test_dictionaries;
pub(in crate::ui) use templates::{every as every_template, template_rows};

use super::press::on_primary;
use crate::ui::copy;
use crate::ui::view::Shell;
use mail_client::password::Password;

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
/// is written in the reader column, as a reply is under its thread.
pub(in crate::ui) fn composing(shell: &Shell) -> Option<(DraftId, PageKind)> {
    let open = shell.composing.as_ref()?;
    Some((open.draft, kind_of(open.draft)))
}

/// Whether `draft` answers a message (a reply, under its thread) or stands alone.
fn kind_of(draft: DraftId) -> PageKind {
    let client = crate::ui::live::client();
    match client.stored().draft(draft) {
        Ok(stored) if stored.in_reply_to.is_some() => PageKind::Reply,
        _ => PageKind::New,
    }
}

/// The page for `draft`: held for its time, or being written.
#[component]
pub(in crate::ui) fn ComposerPage(draft: DraftId, shell: Signal<Shell>) -> Element {
    // One held to leave at a time when it is opened is read, not written: the outbox has it as
    // it was sealed. Cancel Sending makes it a draft again, and the page then writes it here.
    // Asked once: a page scheduled while it is open folds away as it did.
    let mut held = use_signal(|| {
        let client = crate::ui::live::client();
        client
            .stored()
            .draft(draft)
            .ok()
            .and_then(|stored| match stored.state {
                SendState::Scheduled { at } => Some((stored, at)),
                _ => None,
            })
    });
    if let Some((stored, at)) = held() {
        return rsx! {
            held::HeldPage {
                draft: stored,
                at,
                shell,
                on_cancelled: move |()| held.set(None),
            }
        };
    }
    rsx! { Writing { draft, shell } }
}

/// The composer on `draft`, loaded from where it was parked, else from the client.
#[component]
fn Writing(draft: DraftId, shell: Signal<Shell>) -> Element {
    let desk = use_context::<Desk>();
    let loaded = use_hook(|| {
        let client = crate::ui::live::client();
        let mut parked = desk.parked;
        let compose = crate::ui::prefs::current().compose;
        let page = life::load(&client, draft, &mut parked.write(), &compose);
        desk::unpark(desk, draft);
        page
    });
    match loaded {
        Some(initial) => rsx! { PageView { initial, shell } },
        None => rsx! {
            div { class: "reader-empty",
                EmptyState { title: "That draft is gone." }
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
fn PageView(initial: Page, shell: Signal<Shell>) -> Element {
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
    // Autosave: once the page has been still for a moment, through the draft save path, with a
    // copy kept on the server now and then.
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
                // Taken here and written on a blocking thread, so a slow disk never holds a
                // frame. The next save is not asked for until this one is done, and a save on
                // close or Send meanwhile is a later page, which this one does not overwrite.
                let client = crate::ui::live::client();
                let (taken, stamp) = (page.peek().clone(), life::stamp());
                let now = chrono::Utc::now();
                let saved = tokio::task::spawn_blocking(move || {
                    let saved = life::save_stamped(&client, &taken, stamp, now)?;
                    // And a copy for the server's Drafts, at most every `KEEP_EVERY`. The save
                    // stands whatever becomes of it.
                    if saved.is_some()
                        && let Err(why) =
                            life::keep_on_server(&client, taken.draft, now, life::Keeping::WhenDue)
                    {
                        log::warn!("the draft's copy on the server was not queued: {why}");
                    }
                    Ok(saved)
                })
                .await
                .unwrap_or_else(|why| Err(why.to_string()));
                match saved {
                    // Clean only when nothing was typed while it was written.
                    Ok(Some(_)) if page.peek().edits == edits => page.write().saved = Saved::Clean,
                    Ok(_) => {}
                    Err(why) => page.write().notice = Some(why),
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
    // What the files come to, against what the account sends.
    sizing::use_weight(page);
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
    let over = sizing::over(&read).is_some();
    let seal_bar = read.seal_bar.clone();
    let anyway_label = "Send anyway";
    // The plain text is written out only while it is shown: not on every key.
    let flowed = match read.plain {
        Fold::Open => Some(mail_editor::to_flowed(&read.session.doc)),
        Fold::Folded => None,
    };
    drop(read);

    let send = move |anyway: Anyway| send_page(page, desk, folding, anyway, None);
    // Made here, so a send or a lookup the bar starts belongs to the page and not to the bar,
    // which goes away as it starts.
    let on_seal = use_callback(move |act: BarAct| seal_act(page, desk, folding, act));

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
            div { class: "c-scroll",
                // The head is a message's in the reader (`reader.css`): the reader's bar, holding
                // what a message can do (Attach, ⋯, Send ▾ at its end), the subject where its title
                // is, at its size, and the header lines under it, one hairline under them all. A
                // reply, under its thread, has the thread's bar above it and keeps its own foot.
                if !reply {
                    div { class: "c-head",
                        div { class: "head-row",
                            div { class: "bar-tools",
                                Attach { page, label: crate::ui::copy::controls::ATTACH, bezel: Bezel::Toolbar }
                                MoreButton {
                                    page,
                                    on_keep: move |()| desk::park(desk, page, shell),
                                    on_delete: move |()| delete_draft(page, shell, desk),
                                }
                                SendButton { page, on_send: move |()| send(Anyway::No) }
                            }
                        }
                        TextField {
                            label: copy::fields::SUBJECT.to_owned(),
                            bezel: FieldBezel::Plain,
                            placeholder: copy::fields::SUBJECT.to_owned(),
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
            // What stands between the page and its sending, said under it; a reply's foot is its
            // bar as well.
            if reply || warn || over || seal_bar != SealBar::Clear {
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
                sizing::TooLarge { page, on_anyway: move |()| send(Anyway::Yes) }
                SealWarn { bar: seal_bar, on_act: on_seal }
                if reply {
                    Attach { page, label: crate::ui::copy::controls::ATTACH, bezel: Bezel::Toolbar }
                    MoreButton {
                        page,
                        on_keep: move |()| desk::park(desk, page, shell),
                        on_delete: move |()| delete_draft(page, shell, desk),
                    }
                    span { class: "grow" }
                    SendButton { page, on_send: move |()| send(Anyway::No) }
                }
            }
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
    folding: Folding,
    anyway: Anyway,
    passphrase: Option<Password>,
) {
    let client = crate::ui::live::client();
    let secrets = super::pgp::seams().secrets;
    let sent = life::send(
        &client,
        secrets.as_ref(),
        &mut page.write(),
        anyway,
        life::grace(),
        life::Dated {
            // The window's clock, which a reminder asked for here is later measured against.
            now: super::clock::now(),
            zone: &chrono::Local,
            clock: super::clock::hours(),
        },
    );
    match sent {
        Ok(Sent::Sealing { going, remind }) => {
            let draft = page.peek().draft;
            // Spawned from a press, which is where a task is polled.
            spawn(async move {
                match seal_and_queue(client, draft, going, remind, passphrase).await {
                    Ok(due) => {
                        let sent = life::folded(&mut page.write(), due, going.grace);
                        queued(page, desk, folding, sent);
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
        Ok(sent) => queued(page, desk, folding, sent),
        Err(why) => page.write().notice = Some(why),
    }
}

/// What the signing and encrypting bar's buttons do.
fn seal_act(mut page: Signal<Page>, desk: Desk, folding: Folding, act: BarAct) {
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
            send_page(page, desk, folding, Anyway::Yes, None);
        }
        BarAct::Unlock(passphrase) => {
            send_page(page, desk, folding, Anyway::Yes, Some(passphrase));
        }
        // Making a key or importing a certificate is the sending account's page's, in Settings.
        BarAct::OpenKeys => {
            let client = crate::ui::live::client();
            match client.stored().draft(page.peek().draft) {
                Ok(stored) => super::settings_window::open_account(stored.account),
                Err(_) => super::settings_window::open_at(crate::ui::view::PageId::ACCOUNTS),
            }
        }
        BarAct::LookUp(addresses) => {
            page.write().seal_bar = SealBar::Looking(addresses.clone());
            let client = crate::ui::live::client();
            let lookup = super::pgp::seams().lookup;
            let draft = page.peek().draft;
            spawn(async move {
                let done = tokio::task::spawn_blocking(move || {
                    let stored = client.stored().draft(draft).map_err(|e| e.to_string())?;
                    seal::look_up(
                        &client,
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
fn queued(page: Signal<Page>, mut desk: Desk, folding: Folding, sent: Sent) {
    match sent {
        Sent::Queued {
            draft,
            due,
            when,
            grace,
        } => {
            let mut back = page.peek().clone();
            back.phase = Phase::Writing;
            back.guard = Guard::Clear;
            let outgoing = desk::Outgoing {
                draft,
                due,
                when,
                grace,
                page: back,
                refused: None,
            };
            desk.outbox.set(Some(outgoing));
            desk::unpark(desk, draft);
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
fn delete_draft(mut page: Signal<Page>, shell: Signal<Shell>, desk: Desk) {
    let client = crate::ui::live::client();
    let draft = page.peek().draft;
    let kept = life::save(&client, &mut page.write(), chrono::Utc::now());
    let stored = match kept {
        Ok(stored) => stored,
        Err(why) => {
            page.write().notice = Some(why);
            return;
        }
    };
    match deleted(&client, stored) {
        Ok(undo) => {
            page.write().phase = Phase::Closed;
            forget(shell, desk, draft, undo);
        }
        Err(why) => page.write().notice = Some(crate::ui::remedy::told(why)),
    }
}

/// Discard `stored` and the undo that puts it back in Drafts.
fn deleted(
    client: &Client,
    stored: Draft,
) -> Result<mail_logic::undo::Undo, mail_client::prelude::CoreError> {
    life::discard(client, stored.id)?;
    Ok(mail_logic::undo::Undo {
        thread: None,
        account: stored.account.clone(),
        forward: Patch {
            id: ChangeId::generate(),
            changes: vec![Change::DraftDelete(stored.id)],
        },
        inverse: Patch {
            id: ChangeId::generate(),
            changes: vec![Change::DraftUpsert(Box::new(stored))],
        },
        remote: None,
        resend: None,
    })
}

/// The one teardown of a deleted draft, from the foot's ⋯, the row menu or anywhere else: it
/// leaves Today and the parked pages, its composer closes if it is the one open, the lists are
/// read again, and its undo is kept and offered.
fn forget(mut shell: Signal<Shell>, desk: Desk, draft: DraftId, undo: mail_logic::undo::Undo) {
    desk.parked
        .clone()
        .write()
        .retain(|page| page.draft != draft);
    desk::unpark(desk, draft);
    if shell.peek().composing.as_ref().map(|open| open.draft) == Some(draft) {
        shell.write().close_composer();
    }
    crate::ui::motion::record(
        shell,
        vec![undo],
        crate::ui::copy::toasts::DRAFT_DELETED.to_owned(),
    );
}

/// Delete the stored draft `draft`, as Drafts' row menu does, and offer it back. Its composer
/// closes if it is the one open, and Today lists it no more.
pub(in crate::ui) fn delete_stored_draft(shell: Signal<Shell>, draft: DraftId) {
    let client = crate::ui::live::client();
    let desk = consume_context::<Desk>();
    let outcome = client
        .stored()
        .draft(draft)
        .map_err(mail_client::prelude::CoreError::from)
        .and_then(|stored| deleted(&client, stored));
    match outcome {
        Ok(undo) => forget(shell, desk, draft, undo),
        Err(why) => crate::ui::motion::tell(
            crate::ui::remedy::told(why),
            crate::ui::motion::Follow::Nothing,
        ),
    }
}
