//! The menus a row opens when a click needs a payload.
//!
//! A label and a snooze time are not things a button can carry, so the row opens one of these
//! instead of performing the operation itself. Split from [`super::app`] (`CONVENTIONS.md` §8).

use super::ops::apply_label;
use crate::view::Shell;
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::SqliteStore;
use std::sync::Arc;

/// When to bring a conversation back.
#[component]
pub(super) fn SnoozeMenu(id: ThreadId, shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    rsx! {
        div { class: "labels",
            onclick: move |e: Event<MouseData>| e.stop_propagation(),
            for (says, phrase) in crate::view::snooze_choices() {
                button {
                    key: "{phrase}",
                    class: "label",
                    onclick: move |e: Event<MouseData>| {
                        e.stop_propagation();
                        let store =
                            consume_context::<Arc<SqliteStore>>();
                        match crate::snooze::snooze(
                            &store,
                            id,
                            phrase,
                            chrono::Utc::now(),
                        ) {
                            Ok(_) => {
                                shell.write().snoozing = None;
                                revision += 1;
                            }
                            // The vocabulary is fixed and the clock
                            // is the only other input, so this is
                            // "the year 262143 has no tomorrow".
                            Err(why) => eprintln!("snooze: {why}"),
                        }
                    },
                    "{says}"
                }
            }
        }
    }
}

/// The labels on this conversation, and the ones it could wear.
#[component]
pub(super) fn LabelMenu(
    id: ThreadId,
    summary: ThreadSummary,
    shell: Signal<Shell>,
    revision: Signal<u64>,
) -> Element {
    rsx! {
        div { class: "labels",
            // Stops a click in the menu from also opening the
            // conversation underneath it.
            onclick: move |e: Event<MouseData>| e.stop_propagation(),
            if shell.read().labels.is_empty() {
                // Said rather than shown as an empty box: on a
                // fresh account there are no labels yet, and a menu
                // with nothing in it reads as something broken.
                p { class: "hint", "No labels yet. They arrive with the first sync." }
            }
            for choice in crate::view::label_menu(&shell.read().labels, &summary) {
                button {
                    key: "{choice.id}",
                    class: if choice.membership == Membership::In {
                        "label on"
                    } else {
                        "label"
                    },
                    onclick: {
                        let wanted = choice.toggled();
                        let which = choice.id;
                        move |e: Event<MouseData>| {
                            e.stop_propagation();
                            let store =
                                consume_context::<Arc<SqliteStore>>();
                            if apply_label(&store, id, which, wanted) {
                                revision += 1;
                            }
                        }
                    },
                    if choice.membership == Membership::In { "✓ " }
                    "{choice.name}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::app::App;
    use super::super::ops::apply_label;
    use crate::ui::fixtures::{ACCOUNT, dispatching, inbox_query, markup, realistic};
    use dioxus::prelude::*;
    use dioxus_core::{NoOpMutations, VirtualDom};
    use mail_domain::*;
    use mail_store::{SqliteStore, Store};

    /// Putting a conversation off, from the window — `plan.md` phase 7e.
    ///
    /// The Snoozed place has listed correctly since the place existed and the vocabulary has
    /// been parsed since the CLI learned it. Nothing in the window could snooze anything.
    mod putting_it_off {
        use super::*;

        #[tokio::test]
        async fn the_rows_offer_a_way_to_snooze() {
            let (store, _dir) = realistic();
            assert!(markup(store).contains(">Snooze<"));
        }

        #[test]
        fn snoozing_takes_it_out_of_the_inbox_and_the_snoozed_place_has_it() {
            let (store, _dir) = realistic();
            // The *place's* filter, not a bare `InMailbox`: hiding a snoozed conversation is
            // what `place_filter` is for, and asserting against the bare one would be asking
            // whether snoozing archives things, which it does not.
            let place = Query {
                filter: crate::view::place_filter(MailboxRole::Inbox),
                ..inbox_query()
            };
            let before = store.threads(&place, chrono::Utc::now()).unwrap();
            let thread = before.items[0].id;

            crate::snooze::snooze(&store, thread, "tomorrow", chrono::Utc::now())
                .expect("tomorrow is a time");

            let after = store.threads(&place, chrono::Utc::now()).unwrap();
            assert!(
                !after.items.iter().any(|t| t.id == thread),
                "a snoozed conversation is still in the inbox"
            );
            let asleep = store
                .threads(
                    &Query {
                        filter: crate::view::pending_snooze(),
                        ..inbox_query()
                    },
                    chrono::Utc::now(),
                )
                .unwrap();
            assert!(asleep.items.iter().any(|t| t.id == thread), "{asleep:?}");
        }

        #[test]
        fn every_phrase_the_menu_offers_is_one_the_parser_accepts() {
            // The menu's phrases are the command line's, so a button that said something the
            // parser had never heard of would be a button that does nothing. Checked rather
            // than assumed, because the two lists are written in different files.
            let now = chrono::Utc::now();
            for (says, phrase) in crate::view::snooze_choices() {
                let at = crate::view::snooze_until(phrase, now, &chrono::Local)
                    .unwrap_or_else(|why| panic!("{says:?} means {phrase:?}, which is not: {why}"));
                assert!(at > now, "{says:?} is not in the future");
            }
        }
    }

    /// Labels, in both directions — `plan.md` phase 7d.
    ///
    /// `label:` has searched since F131 and sync has ingested Gmail's labels since F136, and the
    /// row's own "Label" button opened nothing: `OpKind::AddLabel` has no `Op` because
    /// `Op::Label` carries a payload, and `op_for` correctly returned `None` for it. Correctly,
    /// and then nothing else happened.
    mod naming_a_conversation {
        use super::*;

        fn a_label(store: &SqliteStore, name: &str) -> LabelId {
            let id = LabelId::generate();
            store
                .connection()
                .execute(
                    "INSERT INTO labels (id, account, name, origin)
                     VALUES (?1, ?2, ?3, '\"provider\"')",
                    rusqlite::params![id.to_string(), ACCOUNT.to_string(), name],
                )
                .unwrap();
            id
        }

        #[test]
        fn a_label_can_be_put_on_and_taken_off_again() {
            let (store, _dir) = realistic();
            let travel = a_label(&store, "travel");
            let thread = store
                .threads(&inbox_query(), chrono::Utc::now())
                .unwrap()
                .items[0]
                .id;

            assert!(apply_label(&store, thread, travel, Membership::In));
            assert!(
                store
                    .thread(thread)
                    .unwrap()
                    .summary
                    .labels
                    .contains(&travel),
                "the label never landed"
            );

            assert!(apply_label(&store, thread, travel, Membership::Out));
            assert!(
                !store
                    .thread(thread)
                    .unwrap()
                    .summary
                    .labels
                    .contains(&travel),
                "the label would not come off"
            );
        }

        #[test]
        fn labelling_a_gmail_conversation_reaches_gmail() {
            // Under `ServerLabels::Supported` a label is the server's, not ours. The fixture's
            // capabilities are the real account's, so this is the path the user's mail takes.
            let (store, _dir) = realistic();
            let travel = a_label(&store, "travel");
            let thread = store
                .threads(&inbox_query(), chrono::Utc::now())
                .unwrap()
                .items[0]
                .id;

            apply_label(&store, thread, travel, Membership::In);

            let queued = store.outbox_due(ACCOUNT, chrono::Utc::now()).unwrap();
            assert!(
                queued.iter().any(|entry| matches!(
                    &entry.op,
                    ProtoOp::SetLabels { add, .. } if add.iter().any(|name| name == "travel")
                )),
                "the label stopped at this machine: {:?}",
                queued.iter().map(|e| &e.op).collect::<Vec<_>>()
            );
        }

        #[tokio::test]
        async fn the_menu_opens_from_the_row_and_lists_what_there_is() {
            dispatching();
            let (store, _dir) = realistic();
            a_label(&store, "travel");
            let mut dom = VirtualDom::new(App).with_root_context(store.clone());
            dom.rebuild_in_place();
            // The effect that fills `Shell::labels` runs on a revision; one render settles it.
            dom.render_immediate(&mut NoOpMutations);

            let page = dioxus_ssr::render(&dom);
            assert!(
                page.contains(">Label<"),
                "the rows offer no way to label anything:\n{page}"
            );
        }
    }
}
