//! The composer pane.
//!
//! Split from `ui.rs` under `CONVENTIONS.md` §8 — one concept per file, and a file over ~400
//! lines wants splitting. Everything here is the one question "what is being written, and where
//! does it go when the user is done"; the decisions behind it live in `crate::view`, which is
//! free of Dioxus and tested without a window.

use crate::view::{Composing, Discarding, Shell};
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// Seconds between autosaves of an open composer.
///
/// Long enough not to write on every keystroke, short enough that what a crash costs is a
/// sentence rather than a letter.
const AUTOSAVE_EVERY: u64 = 3;

#[component]
pub(super) fn Composer(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    // Every hook first, before any early return. `use_hook` matches hooks between renders by
    // call order, so a component that returns before reaching one leaves every hook after it at
    // a different index on the next render. This function used to return above both hooks
    // below, which happened to be harmless — there was no third hook to shift — but "harmless
    // given the current body" is a property that the next hook added here would quietly end.
    //
    // Set by every field, cleared by a successful write. A flag rather than comparing against
    // the stored row on a timer: the comparison would read and parse the draft every few
    // seconds whether or not anyone had touched it.
    let mut dirty = use_signal(|| false);

    // The autosave. Close and Send both save, so what this covers is the window nothing else
    // does: the application going away while someone is still typing.
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(AUTOSAVE_EVERY)).await;
            if !dirty() {
                continue;
            }
            let store = consume_context::<Arc<SqliteStore>>();
            let current = shell.read().composing.clone();
            // Failures are swallowed on purpose. The usual one is a half-typed recipient, and a
            // timer that interrupts to complain about an address still being typed is worse than
            // one that waits. The text stays dirty and the next tick tries again.
            if persist(&store, current.as_ref()).is_ok() {
                dirty.set(false);
            }
        }
    });

    // Now the early return, below every hook. `App` only renders this when something is being
    // composed, but "only" is a claim about a caller, and the rules of hooks are not a matter
    // of who calls what.
    let Some(editing) = shell.read().composing.clone() else {
        return rsx! {};
    };

    rsx! {
        div { class: "composer",
            header { class: "composer-head",
                strong { "{editing.subject}" }
                button {
                    class: "ghost",
                    onclick: move |_| {
                        let store = consume_context::<Arc<SqliteStore>>();
                        let current = shell.read().composing.clone();
                        match persist(&store, current.as_ref()) {
                            Ok(_) => {
                                shell.write().close_composer();
                                revision += 1;
                            }
                            // Stay open rather than lose the text. A recipient that does not
                            // parse must not cost the user the paragraph they just wrote, and
                            // Discard is right there for anyone who meant to abandon it.
                            Err(why) => set_notice(&mut shell, Some(why)),
                        }
                    },
                    "Close"
                }
                button {
                    class: if editing.confirming_discard { "ghost danger" } else { "ghost" },
                    onclick: move |_| {
                        // First click asks, second deletes. `Discard` used to close the pane and
                        // nothing else, which for a draft that had never been saved is the same
                        // thing and for every other draft is not — a reply is written to the
                        // store the moment it is created, so the "discarded" draft was still in
                        // Drafts afterwards. It now removes the draft, which is why it asks.
                        // Decided and the borrow released before anything writes back: a
                        // `Signal` read held across a write is a panic at runtime, not a
                        // compile error, in the version of this that used `shell.read()` inline.
                        let decision = crate::view::discard_click(shell.read().composing.as_ref());
                        match decision {
                            None => {}
                            Some(Discarding::Confirm) => {
                                if let Some(c) = shell.write().composing.as_mut() {
                                    c.confirming_discard = true;
                                    c.notice = Some(
                                        "This deletes the draft. Discard again to confirm."
                                            .to_owned(),
                                    );
                                }
                            }
                            Some(Discarding::Delete(id)) => {
                                let store = consume_context::<Arc<SqliteStore>>();
                                match crate::compose::discard(&store, id) {
                                    Err(why) => set_notice(&mut shell, Some(why)),
                                    Ok(_) => {
                                        shell.write().close_composer();
                                        revision += 1;
                                    }
                                }
                            }
                        }
                    },
                    title: "Delete this draft",
                    if editing.confirming_discard { "Discard for good" } else { "Discard" }
                }
            }
            if let Some(notice) = editing.notice.clone() {
                p { class: "notice", "{notice}" }
            }
            // Only when there is a choice to make. One account is not a decision, and a row
            // that always says the same thing is furniture — the same rule the reader's "Load
            // remote images" offer had to learn.
            if shell.read().accounts.len() > 1 {
                label { "From"
                    select {
                        onchange: move |e: Event<FormData>| {
                            let Some(picked) = shell
                                .read()
                                .accounts
                                .iter()
                                .find(|(address, _)| *address == e.value())
                                .map(|(_, id)| *id)
                            else {
                                return;
                            };
                            let store = consume_context::<Arc<SqliteStore>>();
                            // Saved first. The account moves on the stored row, and anything in
                            // the boxes that has not been written yet would be read back over.
                            let current = shell.read().composing.clone();
                            let moved = persist(&store, current.as_ref()).and_then(|draft| {
                                crate::compose::move_draft_to(
                                    &store,
                                    draft.id,
                                    picked,
                                    chrono::Utc::now(),
                                )
                            });
                            match moved {
                                Ok(draft) => {
                                    if let Some(c) = shell.write().composing.as_mut() {
                                        c.from = draft.account;
                                        c.notice = None;
                                    }
                                    revision += 1;
                                }
                                Err(why) => set_notice(&mut shell, Some(why)),
                            }
                        },
                        for (address, id) in shell.read().accounts.clone() {
                            option {
                                key: "{id}",
                                value: "{address}",
                                selected: id == editing.from,
                                "{address}"
                            }
                        }
                    }
                }
            }
            label { "To"
                input {
                    value: "{editing.to}",
                    oninput: move |e| {
                        if let Some(c) = shell.write().composing.as_mut() {
                            c.to = e.value();
                        }
                        dirty.set(true);
                    },
                }
            }
            label { "Cc"
                input {
                    value: "{editing.cc}",
                    oninput: move |e| {
                        if let Some(c) = shell.write().composing.as_mut() {
                            c.cc = e.value();
                        }
                        dirty.set(true);
                    },
                }
            }
            label { "Subject"
                input {
                    value: "{editing.subject}",
                    oninput: move |e| {
                        if let Some(c) = shell.write().composing.as_mut() {
                            c.subject = e.value();
                        }
                        dirty.set(true);
                    },
                }
            }
            textarea {
                class: "composer-body",
                value: "{editing.body}",
                oninput: move |e| {
                    if let Some(c) = shell.write().composing.as_mut() {
                        c.body = e.value();
                    }
                    dirty.set(true);
                },
            }
            div { class: "composer-actions",
                button {
                    onclick: move |_| {
                        let store = consume_context::<Arc<SqliteStore>>();
                        // Cloned out of the signal in its own statement: the read guard ends
                        // here, so the handler can write a notice back afterwards. It also
                        // reads what is in the fields *now* rather than at last render.
                        let current = shell.read().composing.clone();
                        let saved = persist(&store, current.as_ref());
                        match saved {
                            Ok(_) => {
                                dirty.set(false);
                                set_notice(&mut shell, Some("Saved.".to_owned()));
                                revision += 1;
                            }
                            Err(why) => set_notice(&mut shell, Some(why)),
                        }
                    },
                    "Save"
                }
                button {
                    class: "primary",
                    onclick: move |_| {
                        let store = consume_context::<Arc<SqliteStore>>();
                        // Saved first, always. Sending what is in the widgets without writing
                        // it down means a failure between the two loses the user's edits.
                        let current = shell.read().composing.clone();
                        let sent = persist(&store, current.as_ref()).and_then(|draft| {
                            crate::compose::send(&store, draft.id, chrono::Utc::now())
                        });
                        match sent {
                            Ok(_) => {
                                shell.write().close_composer();
                                revision += 1;
                            }
                            Err(why) => set_notice(&mut shell, Some(why)),
                        }
                    },
                    "Send"
                }
                span { class: "hint", "Sending queues the message; the next sync delivers it." }
            }
        }
    }
}

/// Write the composer's fields back onto the stored draft.
///
/// Reads the draft from the store rather than keeping a copy in the widgets, so a field the
/// composer does not show — the identity, the Bcc list, what this replies to — is whatever the
/// store says and not whatever was true when the composer opened.
/// Write what the composer holds back to the store.
///
/// `pub(super)` because Escape closes the composer too, and closing must save whatever is in the
/// boxes — a second "close" that does not is how a keyboard loses a paragraph.
pub(super) fn persist(store: &SqliteStore, editing: Option<&Composing>) -> Result<Draft, String> {
    let editing = editing.ok_or_else(|| "nothing is being composed".to_owned())?;
    let base = store.draft(editing.draft).map_err(|e| e.to_string())?;
    let edited = editing.apply_to(&base, chrono::Utc::now())?;
    crate::compose::save(store, &edited)?;
    Ok(edited)
}

fn set_notice(shell: &mut Signal<Shell>, notice: Option<String>) {
    if let Some(c) = shell.write().composing.as_mut() {
        c.notice = notice;
    }
}
