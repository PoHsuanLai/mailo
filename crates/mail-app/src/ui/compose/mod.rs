//! The composer: a page in the reader column, on the Rust editor core.
//!
//! A new message is a page: the subject as its title, property rows, and the body at 66ch. A
//! reply is the same page, compact, under the thread it answers. The body is one
//! `contenteditable` root whose every edit goes through `crate::editor`; the page never edits
//! text itself. See [`wire`] for how the browser's events reach Rust.
//!
//! Esc parks the draft in Today; Send folds the page away and hands it to the outbox pill,
//! whose Undo brings back exactly what was sent.

mod body;
mod desk;
mod float;
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
mod render;
mod seal;
mod templates;
mod wire;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use dioxus::prelude::*;
use mail_domain::DraftId;
use mail_store::{SqliteStore, Store};

use super::field::{Field, FieldKind};
use super::icon::{Glyph, Icon};
use body::Body;
use life::{Anyway, Sent};
use page::{Focus, Fold, Guard, Page, Phase, Saved, When};
use props::Props;
use seal::{BarAct, SealBar, SealWarn, Sealed, seal_and_queue};

pub(in crate::ui) use desk::{Desk, ParkedDrafts, park_current, show_queued, use_desk};
pub(in crate::ui) use later::ScheduledDrafts;
pub(in crate::ui) use page::PageKind;
pub(in crate::ui) use pill::SendPill;
pub(in crate::ui) use templates::{
    every as every_template, forget as forget_template, template_rows,
};
pub(in crate::ui) use wire::GLUE;

use crate::password::Password;
use crate::view::Shell;

/// How long the fold after Send runs before the page is taken away, if no animation says so.
const FOLD_MS: u64 = 650;

/// The draft being composed and where its page goes, or `None` when nothing is.
pub(in crate::ui) fn composing(shell: &Shell) -> Option<(DraftId, PageKind)> {
    let draft = shell.composing.as_ref()?.draft;
    let store = consume_context::<Arc<SqliteStore>>();
    let kind = match store.draft(draft) {
        Ok(stored) if stored.in_reply_to.is_some() => PageKind::Reply,
        _ => PageKind::New,
    };
    Some((draft, kind))
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

#[component]
fn PageView(initial: Page, shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let mut desk = use_context::<Desk>();
    let mut page = use_signal(|| initial.clone());
    let mut plain = use_signal(|| Fold::Folded);
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
    // Opening another conversation would redraw the reader: park first.
    if shell.read().open != opened_on && page.peek().phase == Phase::Writing {
        desk::park(desk, page, shell);
        return rsx! {};
    }

    let read = page.read();
    let reply = read.kind == PageKind::Reply;
    let root = match (reply, read.phase, read.focus) {
        (true, Phase::Folding, _) => "inline-reply parking",
        (true, _, _) => "inline-reply",
        (false, Phase::Folding, _) => "cpage sending",
        (false, _, Focus::On) => "cpage focus",
        (false, _, Focus::Off) => "cpage",
    };
    let status = match read.saved {
        Saved::Opened => "Draft · saved",
        Saved::Clean => "Draft · saved just now",
        Saved::Dirty => "Draft · saving…",
    };
    let dirty = read.saved == Saved::Dirty;
    let subject = read.subject.clone();
    let notice = read.notice.clone();
    let warn = read.guard == Guard::Warn;
    let seal_bar = read.seal_bar.clone();
    let scheduled = read.when != When::Now;
    let send_label = if scheduled { "Schedule" } else { "Send" };
    let anyway_label = "Send anyway";
    let flowed = crate::editor::to_flowed(&read.session.doc);
    drop(read);

    let send = move |anyway: Anyway| send_page(page, shell, desk, revision, anyway, None);
    // Made here, so a send or a lookup the bar starts belongs to the page and not to the bar,
    // which goes away as it starts.
    let on_seal = use_callback(move |act: BarAct| seal_act(page, shell, desk, revision, act));

    rsx! {
        div {
            class: "{root}",
            onanimationend: move |_| {
                if page.peek().phase == Phase::Folding {
                    close(page, shell, desk);
                }
            },
            onkeydown: move |event: KeyboardEvent| {
                let key = event.key().to_string();
                let modifiers = event.modifiers();
                let ctrl = modifiers.ctrl() || modifiers.meta();
                if ctrl && key == "Enter" {
                    event.prevent_default();
                    event.stop_propagation();
                    send(Anyway::No);
                } else if ctrl && modifiers.shift() && key.eq_ignore_ascii_case("f") {
                    event.prevent_default();
                    event.stop_propagation();
                    toggle_focus(page, desk);
                } else if key == "Escape" {
                    event.prevent_default();
                    event.stop_propagation();
                    escape(page, shell, desk);
                }
            },
            if !reply {
                div { class: "c-top",
                    span { class: if dirty { "state dirty" } else { "state" },
                        span { class: "pip" }
                        span { "{status}" }
                    }
                    span { class: "grow" }
                    button { class: "tool", r#type: "button", title: "Focus (Ctrl Shift F)",
                        onclick: move |_| toggle_focus(page, desk),
                        Glyph { icon: Icon::Maximize, class: None }
                    }
                    button { class: "tool", r#type: "button", title: "Keep for later: it waits in Today (Esc)",
                        onclick: move |_| desk::park(desk, page, shell),
                        Glyph { icon: Icon::Archive, class: None }
                    }
                    button { class: "tool", r#type: "button", title: "Discard",
                        onclick: move |_| discard(page, shell, desk),
                        Glyph { icon: Icon::Trash, class: None }
                    }
                }
            }
            div { class: "c-scroll",
                if !reply {
                    Field {
                        kind: FieldKind::Inline,
                        value: subject,
                        placeholder: "Subject".to_owned(),
                        extra: Some("c-title".to_owned()),
                        on_input: move |value: String| {
                            let mut write = page.write();
                            write.subject = value;
                            write.touch();
                        },
                        on_focus: |_| {},
                        on_blur: |_| {},
                    }
                }
                if let Some(notice) = notice {
                    p { class: "notice", "{notice}" }
                }
                Props { page, shell }
                Body { page, shell, on_attach: move |_| page.write().notice = Some("Pick the file with Attach, below.".to_owned()) }
                if !reply {
                    div { class: "c-hint",
                        span { kbd { "/" } " headings, lists, images…" }
                        span { "select text to style it" }
                        span { kbd { "@" } " mention" }
                        span { kbd { "Ctrl" } kbd { "Enter" } " send" }
                        span { kbd { "Esc" } " keep for later" }
                    }
                }
                if plain() == Fold::Open {
                    div { class: "plain",
                        span { class: "cap", "text/plain, format=flowed — sent alongside the HTML, from the same document" }
                        "{flowed}"
                    }
                }
            }
            div { class: "c-foot",
                if warn {
                    div { class: "c-warn",
                        Glyph { icon: Icon::Paperclip, class: None }
                        span { "You wrote about an attachment, and nothing is attached." }
                        Attach { page, label: "Attach a file" }
                        button { class: "mini", r#type: "button", aria_label: "{anyway_label}", onclick: move |_| send(Anyway::Yes), "Send anyway" }
                    }
                }
                SealWarn { bar: seal_bar, on_act: on_seal }
                button {
                    class: "mini",
                    r#type: "button",
                    aria_pressed: if plain() == Fold::Open { "true" } else { "false" },
                    onclick: move |_| plain.set(if plain() == Fold::Open { Fold::Folded } else { Fold::Open }),
                    "Plain text"
                }
                Attach { page, label: "Attach" }
                span { class: "grow" }
                button { class: "btn", r#type: "button", aria_label: "{send_label}", onclick: move |_| send(Anyway::No),
                    if scheduled {
                        Glyph { icon: Icon::Clock, class: None }
                        "Schedule"
                    } else {
                        Glyph { icon: Icon::Send, class: None }
                        "Send"
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
    shell: Signal<Shell>,
    desk: Desk,
    revision: Signal<u64>,
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
        chrono::Utc::now(),
        &chrono::Local,
    );
    match sent {
        Ok(Sent::Sealing { leaves }) => {
            let draft = page.peek().draft;
            // Spawned from a press, which is where a task is polled (F140).
            spawn(async move {
                match seal_and_queue(store, draft, leaves, passphrase).await {
                    Ok(due) => {
                        let sent = life::folded(&mut page.write(), due);
                        queued(page, shell, desk, revision, sent);
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
        Ok(sent) => queued(page, shell, desk, revision, sent),
        Err(why) => page.write().notice = Some(why),
    }
}

/// What the signing and encrypting bar's buttons do.
fn seal_act(
    mut page: Signal<Page>,
    shell: Signal<Shell>,
    desk: Desk,
    revision: Signal<u64>,
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
            send_page(page, shell, desk, revision, Anyway::Yes, None);
        }
        BarAct::Unlock(passphrase) => {
            send_page(page, shell, desk, revision, Anyway::Yes, Some(passphrase));
        }
        BarAct::OpenSheet => super::pgp::keys::open(shell),
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
    shell: Signal<Shell>,
    mut desk: Desk,
    mut revision: Signal<u64>,
    sent: Sent,
) {
    match sent {
        Sent::Queued { draft, due, when } => {
            let mut back = page.peek().clone();
            back.phase = Phase::Writing;
            back.guard = Guard::Clear;
            desk.outbox.set(Some(desk::Outgoing {
                draft,
                due,
                when,
                page: back,
                refused: None,
            }));
            desk::unpark(desk, draft);
            revision += 1;
            spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(FOLD_MS)).await;
                close(page, shell, desk);
            });
        }
        // Sealing is answered by the send that started it; a stop has already said why.
        Sent::Stopped | Sent::Sealing { .. } => {}
    }
}

/// Esc: a menu first, then focus mode, then the draft is parked.
fn escape(mut page: Signal<Page>, shell: Signal<Shell>, desk: Desk) {
    let (menu, focus) = {
        let read = page.peek();
        (read.float != page::Float::Closed, read.focus)
    };
    if menu {
        page.write().float = page::Float::Closed;
    } else if focus == Focus::On {
        toggle_focus(page, desk);
    } else {
        desk::park(desk, page, shell);
    }
}

fn toggle_focus(mut page: Signal<Page>, mut desk: Desk) {
    let on = page.peek().focus == Focus::Off && page.peek().kind == PageKind::New;
    page.write().focus = if on { Focus::On } else { Focus::Off };
    desk.side_hidden.set(on);
}

/// The fold has run: the page goes, the pill stays.
fn close(mut page: Signal<Page>, mut shell: Signal<Shell>, mut desk: Desk) {
    if page.peek().phase != Phase::Folding {
        return;
    }
    page.write().phase = Phase::Closed;
    if page.peek().focus == Focus::On {
        desk.side_hidden.set(false);
    }
    let draft = page.peek().draft;
    if shell.peek().composing.as_ref().map(|open| open.draft) == Some(draft) {
        shell.write().close_composer();
    }
}

/// Delete the draft. Its queued send, if any, goes with it (334b758).
fn discard(mut page: Signal<Page>, mut shell: Signal<Shell>, desk: Desk) {
    let store = consume_context::<Arc<SqliteStore>>();
    let draft = page.peek().draft;
    match crate::compose::discard(&store, draft) {
        Ok(_) => {
            page.write().phase = Phase::Closed;
            desk::unpark(desk, draft);
            shell.write().close_composer();
        }
        Err(why) => page.write().notice = Some(why),
    }
}

/// A file picker, as a button. The file lands on the draft and in the Attached row.
#[component]
fn Attach(page: Signal<Page>, label: &'static str) -> Element {
    rsx! {
        label { class: "mini attach",
            Glyph { icon: Icon::Paperclip, class: None }
            "{label}"
            input {
                class: "inp c-file",
                r#type: "file",
                multiple: true,
                onchange: move |event: Event<FormData>| {
                    let files = event.files();
                    spawn(async move {
                        for file in files {
                            let name = file.name();
                            if file.size() > crate::compose::ATTACHMENT_BUDGET {
                                page.write().notice = Some(format!("{name} is too large to send"));
                                continue;
                            }
                            let Ok(bytes) = file.read_bytes().await else {
                                page.write().notice = Some(format!("cannot read {name}"));
                                continue;
                            };
                            let store = consume_context::<Arc<SqliteStore>>();
                            let draft = page.peek().draft;
                            let now = chrono::Utc::now();
                            let saved = life::save(&store, &mut page.write(), now).and_then(|_| {
                                crate::compose::attach_bytes(&store, draft, &name, &bytes, now)
                            });
                            let mut write = page.write();
                            match saved {
                                Ok(stored) => {
                                    write.attached = crate::compose::attached_to(&store, &stored);
                                    write.guard = Guard::Clear;
                                }
                                Err(why) => write.notice = Some(why),
                            }
                        }
                    });
                },
            }
        }
    }
}
