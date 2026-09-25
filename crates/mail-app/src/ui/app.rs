use super::command::CommandMenu;
use super::compose::{self, ComposerPage, PageKind, SendPill};
use super::data::{PAGE, accounts, count_badges, warm_the_first_screenful};
use super::frame;
use super::list::ThreadList;
use super::list_query::{ListView, use_list};
use super::ops::{Composes, apply_op, start_composing, start_new};
use super::reading::Reader;
use super::sidebar::Places;
use super::space_editor::SpaceEditor;
use super::style::STYLE;
use crate::space::{Space, Spaces};
use crate::view::{
    Appearance, Listing, PageMenu, Shell, Shortcut, Source, SyncState, badge_filter, folder_filter,
    nothing_to_show, places_with, synced,
};
use dioxus::prelude::*;
use ds::{Ds, Material};
use ds_settings::Environment;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

#[component]
pub(super) fn App() -> Element {
    // No store handle in the component body any more. Since phase 8c every read this component
    // makes goes through a `use_resource` that takes its own clone for a blocking thread, or
    // through the first-frame fallback beside it, which asks the context where it stands. A
    // handle held here is a handle that invites a query back onto the thread that draws.
    // The launched look, when `main` provided one. A test that builds `App` with only the
    // store keeps the first-run appearance.
    // Opened on a conversation when the window was started to show one (`mailo open`).
    let mut shell = use_signal(|| {
        let mut shell = Shell {
            appearance: try_consume_context::<Appearance>().unwrap_or_default(),
            ..Shell::default()
        };
        if let Some(super::Start::Thread(thread)) = try_consume_context::<super::Start>() {
            super::open_thread(&mut shell, thread);
        }
        shell
    });
    // Bumped after any write, to re-run the queries. Explicit rather than implicit so it is
    // obvious what causes a refresh.
    let mut revision = use_signal(|| 0u64);
    let boot = use_hook(frame::load_boot);
    let spaces = use_signal(|| boot.spaces.clone());
    let mut today_list = use_signal(|| boot.today.clone());
    let dirs = boot.dirs.clone();
    let mut side_hidden = use_signal(|| false);
    // The hidden sidebar, shown as a floating panel while the pointer is at the left edge.
    let mut side_peek = use_signal(|| false);
    // The Space editor's draft, while the sheet is open.
    let editing = use_signal(|| None::<crate::space::edit::Draft>);
    // Which way the sidebar's contents slid in on the last switch.
    let slide = use_signal(|| None::<super::switch::Slide>);
    let desk = compose::use_desk(today_list, spaces, dirs.clone(), side_hidden);
    let mut entering = use_signal(|| true);
    let mut just_added = use_signal(|| None::<mail_domain::ThreadId>);
    let mut seen_open = use_signal(|| None::<mail_domain::ThreadId>);
    let mut scoped = use_signal(|| false);
    let mut label_ids = use_signal(Vec::<mail_domain::LabelId>::new);
    // The server folders that are places, after the labels, as `view::places_with` orders them.
    let mut folder_refs = use_signal(Vec::<(String, MailboxRef)>::new);
    // Hover previews and the motion keyed to ops: shared state, provided once for the window.
    super::hover::use_hover();
    super::motion::use_motion();
    // The first frame cannot wait for the effect below: a chip's name is a lookup, and a
    // lookup against an empty list draws an empty chip.
    use_hook(|| {
        let store = consume_context::<Arc<SqliteStore>>();
        let known = crate::query::known_labels(&store);
        let ids: Vec<mail_domain::LabelId> = known.iter().map(|(_, id)| *id).collect();
        label_ids.set(ids);
        let folders = super::sidebar::folder_places(&store);
        let places = places_with(&known, &folders);
        folder_refs.set(folders);
        let mut write = shell.write();
        write.labels = known;
        write.places = places;
        write.accounts = crate::compose::sending_accounts(&store);
    });

    // How many pages of the list have been asked for. Reset whenever the list itself changes,
    // because "page 3" of the Inbox means nothing once the user is looking at Archive.
    let pages = use_signal(|| 1u32);
    let mut sync_state = use_signal(|| SyncState::Idle);
    // Opening a folder fetches it, and says so where a sync does.
    super::folder_open::use_fetching(sync_state);

    // One count per place, recomputed after any write. `Store::count` answers each in a single
    // indexed query, which is why the sidebar can afford to ask on every revision.
    // Resolved once. The sidebar's places are fixed after construction, so what each badge
    // counts never changes — only the answer does.
    // Render the first screenful before anybody asks for it — phase 8e.
    //
    // An ordinary `std::thread`, not a task: it writes into the render cache, which is a `Mutex`
    // and not a signal, so it needs nothing to poll it and nothing to notice when it finishes.
    // That is the whole reason this part of phase 8 works while F140 stands — every other way of
    // leaving the render thread has to find its way back onto one.
    //
    // `use_hook` runs its closure at mount, which is established: the note above the keyboard
    // says so, and it is `spawn` inside it that does not run.
    //
    // Opening a conversation then costs a hash lookup. Other clients parse, sanitize and embed
    // when you click; this has already done it.
    use_hook(|| {
        let store = consume_context::<Arc<SqliteStore>>();
        std::thread::spawn(move || {
            warm_the_first_screenful(&store);
        });
    });

    // Mailbox badges are fixed. Label badges join them when the label list changes, which is
    // a signal of its own so a keystroke — a shell change — does not recount.
    let badge_filters = use_memo(move || {
        let mut filters: Vec<Option<Filter>> = crate::view::default_places()
            .iter()
            .map(|place| badge_filter(&place.source))
            .collect();
        for id in label_ids.read().iter() {
            filters.push(Some(Filter::And(vec![
                Filter::HasLabel(*id),
                Filter::Read(ReadState::Unread),
            ])));
        }
        for (_, mailbox) in folder_refs.read().iter() {
            filters.push(badge_filter(&Source::Mail(folder_filter(mailbox))));
        }
        filters
    });

    // Depends on `revision` and the badge filters, not on `shell`. It used to read `shell`,
    // which subscribes a memo to *every* change of it — so each keystroke in the search box
    // re-ran one indexed count per place.
    let counted: Resource<Vec<Option<u64>>> = use_resource(move || {
        let _ = revision();
        let filters = badge_filters();
        let store = consume_context::<Arc<SqliteStore>>();
        async move {
            tokio::task::spawn_blocking(move || count_badges(&store, &filters))
                .await
                .unwrap_or_default()
        }
    });
    // And the answer for the very first frame, computed here because there is not one yet.
    //
    // This is what phase 8c got wrong the first time. A bare resource is empty until it
    // resolves, so the window opened on an empty mailbox — and under F140 it stays empty, since
    // nothing ever polls the task. Falling back to the synchronous answer costs one query on the
    // first frame and nothing afterwards: once the resource has a value it keeps it across
    // restarts, so a search never drops back to computing on this thread.
    let badges = use_memo(move || match counted.read().as_ref() {
        Some(counts) => counts.clone(),
        None => {
            let store = consume_context::<Arc<SqliteStore>>();
            count_badges(&store, &badge_filters())
        }
    });

    // The label names the search box can resolve. Re-read after every write, because a sync
    // that ingests a new Gmail label should make `label:` find it without a restart — and
    // written back only when it has actually changed, so an unrelated revision does not
    // invalidate the list below.
    use_effect(move || {
        let _ = revision();
        let store = consume_context::<Arc<SqliteStore>>();
        let known = crate::query::known_labels(&store);
        let ids: Vec<mail_domain::LabelId> = known.iter().map(|(_, id)| *id).collect();
        if label_ids.peek().as_slice() != ids.as_slice() {
            label_ids.set(ids);
        }
        if shell.peek().labels != known {
            shell.write().labels = known.clone();
        }
        // The same shape for the same reason: the From row needs the list, and an account added
        // in a terminal should reach the open window without a restart.
        let sending = crate::compose::sending_accounts(&store);
        if shell.peek().accounts != sending {
            shell.write().accounts = sending;
        }
        // Folders the same way: a folder made, renamed or listed for the first time is a place
        // at once.
        let folders = super::sidebar::folder_places(&store);
        if *folder_refs.peek() != folders {
            folder_refs.set(folders.clone());
        }
        let next = places_with(&known, &folders);
        if shell.peek().places != next {
            // The same place stays chosen when one is added before it: a new label moves every
            // folder down by one, and the list must not jump to the folder above.
            let selected = {
                let current = shell.peek();
                current
                    .places
                    .get(current.selected)
                    .and_then(|was| next.iter().position(|place| place.source == was.source))
                    .unwrap_or_else(|| current.selected.min(next.len().saturating_sub(1)))
            };
            let mut write = shell.write();
            write.places = next;
            write.selected = selected;
        }
    });

    // The Space's account list, once. A tile press is a shell change and must not reload it.
    use_effect(move || {
        if scoped() {
            return;
        }
        scoped.set(true);
        let scope = frame::scope_ids(&spaces.read().current_space());
        if shell.peek().scope != scope {
            shell.write().scope = scope;
        }
    });

    // Opening a thread is a shortcut in Today. Closing one is not a write to the mail.
    let today_dirs = dirs.clone();
    use_effect(move || {
        let open = shell.read().open;
        if open == seen_open() {
            return;
        }
        let already = open.is_some_and(|id| {
            today_list
                .peek()
                .live(spaces.peek().current, chrono::Utc::now())
                .contains(&id)
        });
        seen_open.set(open);
        let Some(id) = open else {
            return;
        };
        let index = spaces.peek().current;
        today_list.write().opened(index, id, chrono::Utc::now());
        if !already {
            just_added.set(Some(id));
        }
        if let Some(dirs) = today_dirs.clone() {
            let _ = crate::today::save(&dirs.state, &today_list.read());
        }
    });

    let mut list_watch = use_signal(|| (0usize, None::<mail_domain::AccountId>));
    use_effect(move || {
        let now = (shell.read().selected, shell.read().account);
        if now != list_watch() {
            list_watch.set(now);
            entering.set(true);
        }
    });

    // The list, by the same rule as the badges: computed here the first time, off the thread
    // afterwards, and for a search only once the box is still (`list_query`).
    let ListView {
        threads,
        top,
        marking,
    } = use_list(shell, pages, revision);

    let drafts = use_memo(move || {
        let _ = revision();
        if !matches!(shell.read().listing(PAGE), Listing::Drafts) {
            return Vec::new();
        }
        let store = consume_context::<Arc<SqliteStore>>();
        accounts(&store)
            .into_iter()
            .filter_map(|account| store.drafts(account).ok())
            .flatten()
            .collect::<Vec<_>>()
    });

    // One more row than asked for means there is another page. Asking the store for the count
    // would be a second query answering a question this one already answers.
    // Deliberately a growing limit rather than `Page::next`, which the store also returns.
    // `plan.md` asks for the cursor, and the cursor is the right answer for an accumulating
    // list — but accumulating means holding pages in a signal and rebuilding them after every
    // archive, star and ingest, and a mixture of pages fetched at different moments is exactly
    // the inconsistency keyset pagination exists to avoid. One query for the whole visible list
    // is always self-consistent, costs a few thousand indexed rows at the sizes this client is
    // for, and needs no invalidation logic at all. Revisit when a mailbox is large enough to
    // measure, at which point the cursor is already there.
    let more = use_memo(move || threads().len() as u32 >= PAGE * pages());

    // Why the pane is empty, when it is. Counted rather than assumed: the shell cannot add an
    // account, so the first run needs to name the command that can.
    let nothing = use_memo(move || {
        let _ = revision();
        let store = consume_context::<Arc<SqliteStore>>();
        nothing_to_show(accounts(&store).len(), &shell.read().search)
    });

    // The keyboard.
    //
    // A keydown targets the focused element and bubbles *up*, `body` is that element until
    // something focusable is clicked, and `body` is the app div's parent — so a handler on the
    // div is simply never reached. `tabindex` alone does not fix it and `autofocus` does not
    // either: that attribute is for form controls and WebKit ignores it on a div. Both compiled,
    // rendered, and did nothing, which nothing in this repository could have told me.
    //
    // So the div is focused from JavaScript, and refocused whenever focus falls back to `body` —
    // which is what happens after a click on anything that is not itself focusable. Written as a
    // fire-and-forget script rather than a task with a channel because a future spawned from a
    // component body is never polled here: `use_hook`'s closure runs, `spawn` inside it does not,
    // and `use_future` does not run at all. Spawning works from an event handler, which is where
    // the Sync button does it.
    // Whether a text box has focus. A letter is a shortcut while reading and a letter while
    // writing, and the client that confuses the two archives a conversation because someone
    // typed "e" into a reply. The composer counts wholesale: its fields are many and focus can
    // sit between them.
    let in_a_field = use_signal(|| false);

    let on_key = move |event: Event<KeyboardData>| {
        // `Key`'s Display is the DOM key name — "e", "ArrowDown", "Escape" — which is the
        // vocabulary `view::shortcut` is written against.
        let key = event.key().to_string();
        // The Contacts sheet owns it while it is open, over the Space editor when it was opened
        // from there: its filter takes letters, and Esc closes it and nothing else.
        if shell.read().contacts.is_some() {
            if key == "Escape" {
                super::contacts::close(shell);
            }
            return;
        }
        // The Import and Export sheets likewise: their fields take letters, Esc closes them.
        if shell.read().files.is_some() {
            if key == "Escape" {
                super::files::close(shell);
            }
            return;
        }
        // And the Add account sheet: its address and password take letters, Esc closes it.
        if shell.read().adding.is_some() {
            if key == "Escape" {
                super::add_account::close(shell);
            }
            return;
        }
        // The Rules sheet likewise: its fields take letters, Esc closes it.
        if shell.read().rules.is_some() {
            if key == "Escape" {
                super::rules::close(shell);
            }
            return;
        }
        // And the keys and certificates sheet: its fields take letters, Esc closes it.
        if shell.read().keys.is_some() {
            if key == "Escape" {
                super::pgp::keys::close(shell);
            }
            return;
        }
        // The Space editor owns the keyboard while it is open. Its name field takes letters,
        // its handles take the arrows, and Esc puts the Space back as the sheet found it.
        if editing.read().is_some() {
            if key == "Escape" {
                super::space_editor::cancel(editing, spaces);
            }
            return;
        }
        let typing_now = in_a_field() || shell.read().composing.is_some();
        if super::motion::key(&key, event.modifiers().ctrl(), typing_now, shell, revision) {
            return;
        }
        if key == "s" && event.modifiers().ctrl() {
            side_peek.set(false);
            side_hidden.set(!side_hidden());
            return;
        }
        if event.modifiers().ctrl()
            && let Some(index) = super::switch::space_key(&key)
        {
            super::switch::go(spaces, shell, pages, slide, index);
            return;
        }
        if (key == "f" || key == "F") && event.modifiers().ctrl() {
            super::reading::open_find(shell);
            return;
        }
        if (key == "p" || key == "P") && event.modifiers().ctrl() {
            // The open conversation, as one flow. Nothing open: nothing printed, nothing said.
            let open = shell.read().open;
            if let Some(job) = super::print::job_for(open) {
                super::print::print(job);
            }
            return;
        }
        if (key == "t" || key == "T") && event.modifiers().ctrl() {
            let open = shell.read().command.is_some();
            if open {
                shell.write().command = None;
                super::host::Host::focus_app();
            } else {
                shell.write().command = Some(String::new());
            }
            return;
        }
        // A menu is showing its own cursor. Shortcuts would archive a thread the user is
        // trying to filter for, and the menu's own handler already took the arrows.
        let menu_open = {
            let current = shell.read();
            current.command.is_some()
                || current.page_menu != PageMenu::Closed
                || current.snoozing.is_some()
                || current.labelling.is_some()
                || current.filing.is_some()
        };
        if menu_open {
            if key == "Escape" {
                let mut write = shell.write();
                write.command = None;
                write.page_menu = PageMenu::Closed;
                write.snoozing = None;
                write.labelling = None;
                write.filing = None;
                super::host::Host::focus_app();
            }
            return;
        }
        // Esc closes a find before it closes anything else, and clears its marks.
        if key == "Escape" && shell.read().find.is_some() {
            shell.write().find = None;
            return;
        }
        let typing = in_a_field() || shell.read().composing.is_some();
        // Esc closes a centre or full peek and revokes image consent. Side peek still falls
        // through to Shortcut::Back, and a composer still takes Esc.
        if key == "Escape" {
            let floating = {
                let current = shell.read();
                current.composing.is_none() && current.open.is_some() && current.peek.floats()
            };
            if floating {
                shell.write().close();
                return;
            }
        }
        let Some(action) = crate::view::shortcut(&key, typing) else {
            return;
        };
        let store = consume_context::<Arc<SqliteStore>>();
        let open = shell.read().open;
        match action {
            Shortcut::Next | Shortcut::Previous => {
                let ids: Vec<ThreadId> = threads().iter().map(|t| t.id).collect();
                if let Some(id) = crate::view::step(open, &ids, action == Shortcut::Next) {
                    shell.write().open(id);
                }
            }
            Shortcut::Back => {
                if shell.read().composing.is_some() {
                    // Parked, not closed: saved, and waiting in Today.
                    compose::park_current(desk, shell);
                    revision += 1;
                } else {
                    shell.write().open = None;
                }
            }
            Shortcut::TogglePin => {
                if let Some(id) = open
                    && apply_op(&store, id, OpKind::Pin)
                {
                    revision += 1;
                }
            }
            Shortcut::Compose => {
                // The only shortcut that does not consult the conversation under the cursor, and
                // so the only one that does anything in an empty mailbox.
                // Cloned out and the guard dropped before anything writes back. The same
                // hazard the composer's handlers document, caught here by the borrow checker
                // rather than at runtime.
                let known = shell.peek().accounts.clone();
                match start_new(&store, &known) {
                    Ok(draft) => {
                        shell.write().compose(&draft);
                        revision += 1;
                    }
                    // Nowhere to put it: the composer that would show a notice is what failed
                    // to open. Same bind as the reply buttons, and the same answer.
                    Err(why) => eprintln!("compose: {why}"),
                }
            }
            Shortcut::Reply | Shortcut::ReplyAll | Shortcut::Forward => {
                let what = match action {
                    Shortcut::Reply => Composes::Reply(ReplyScope::Sender),
                    Shortcut::ReplyAll => Composes::Reply(ReplyScope::All),
                    _ => Composes::Forward,
                };
                if let Some(id) = open
                    && let Ok(draft) = start_composing(&store, id, what)
                {
                    shell.write().compose(&draft);
                    revision += 1;
                }
            }
            _ => {
                // Resolved against the open thread's own summary, so the keyboard reaches
                // exactly what that row's buttons offer and nothing else.
                let Some(id) = open else { return };
                let Some(summary) = threads().iter().find(|t| t.id == id).cloned() else {
                    return;
                };
                if let Some(kind) = crate::view::op_for_shortcut(action, &summary) {
                    super::motion::act_kind(&store, shell, revision, id, kind);
                }
            }
        }
    };

    // Mail that only arrives when you press a button is mail you miss. `AccountEngine::watch`
    // has existed since phase 3 and nothing called it; this is the poll half of it, which is
    // what every account's `WatchMode::Poll` already asks for.
    //
    // The decision of *when* is `view::next_sync`, not here — in particular the rule that a
    // rejected credential stops the loop rather than slowing it. Five minutes is 288 attempts a
    // day, and 288 failed logins a day against the user's own mail server is how an account gets
    // locked.
    let _poll = use_future(move || async move {
        let mut failures = 0u32;
        let interval = {
            let store = consume_context::<Arc<SqliteStore>>();
            crate::sync::poll_interval(&store)
        };
        // A beat before the first pass, so opening the window is not also a network round trip
        // competing with the first paint.
        let mut wait = std::time::Duration::from_secs(2);
        // When each account's last timed pass started. The loop wakes at the shortest interval
        // any account asks for, and an account is synced only once its own has passed.
        let mut last = std::collections::HashMap::new();
        loop {
            tokio::time::sleep(wait).await;

            // Never two at once: a pass the user started is the same request, and two passes on
            // one account race each other's writes for the same rows.
            if !sync_state.read().may_start() {
                wait = interval;
                continue;
            }
            let store = consume_context::<Arc<SqliteStore>>();
            let intervals = crate::sync::due::intervals(&store);
            let started = std::time::Instant::now();
            let due = crate::sync::due::due(&intervals, &last, started);
            // With no account to sync at all, the pass still runs, to say why there is none.
            if due.is_empty() && !intervals.is_empty() {
                wait = interval;
                continue;
            }
            last.extend(due.iter().map(|account| (*account, started)));
            sync_state.set(SyncState::Running);
            let done = tokio::task::spawn_blocking(move || {
                crate::sync::due::run_due(store, chrono::Utc::now(), &due)
            })
            .await;

            // Order matters: a pass can both be refused and be told to slow down, and only one
            // of the two is worth stopping the loop for.
            let passed = match &done {
                Ok(Ok(ran)) if ran.rejected => crate::view::Passed::Rejected,
                Ok(Ok(ran)) => match ran.hold {
                    Some(wait) => crate::view::Passed::Throttled { wait },
                    None => crate::view::Passed::Fine,
                },
                // A pass that could not run at all, and a task that panicked, are both worth
                // trying again: a laptop lid is the usual cause of the first.
                Ok(Err(_)) | Err(_) => crate::view::Passed::Transient,
            };
            failures = match passed {
                // Being asked to wait is not a failure, and counting it as one would double a
                // wait the server had already named.
                crate::view::Passed::Fine | crate::view::Passed::Throttled { .. } => 0,
                _ => failures.saturating_add(1),
            };
            sync_state.set(match done {
                Ok(result) => synced(result.map(|ran| ran.text)),
                Err(e) => synced(Err(format!("the sync pass stopped: {e}"))),
            });
            revision += 1;

            match crate::view::next_sync(passed, failures, interval) {
                crate::view::NextSync::After(next) => wait = next,
                crate::view::NextSync::Wait(why) => {
                    // Said once and then nothing more. The Sync button still works, so a user
                    // who has fixed the credential is one click from finding out.
                    sync_state.set(SyncState::Failed(why));
                    revision += 1;
                    return;
                }
            }
        }
    });

    let peek = shell.read().peek.slug();
    let close_label = "Close";
    let frame_class = match (side_hidden(), side_peek()) {
        (false, _) => "app",
        (true, false) => "app no-side",
        (true, true) => "app no-side side-peek",
    };
    rsx! {
        Frame { spaces,
        style { {STYLE} }
        div { class: frame_class,
            tabindex: "0",
            onkeydown: on_key,
            onpointermove: move |event| {
                let at = event.client_coordinates();
                let held = !event.held_buttons().is_empty();
                super::motion::drag::moved((at.x, at.y), held);
            },
            onpointerup: move |_| super::motion::drag::release(shell, revision),
            "data-peek": "{peek}",
            if side_hidden() {
                ds::EdgeStrip { onenter: move |()| side_peek.set(true) }
            }
            if shell.read().open.is_some() && shell.read().peek.floats() {
                button {
                    class: "scrim",
                    r#type: "button",
                    aria_label: "{close_label}",
                    onclick: move |_| shell.write().close(),
                }
            }
            Places {
                shell, pages, badges, revision, spaces, today: today_list, dirs: dirs.clone(),
                side_hidden, side_peek, just_added, editing, slide,
            }
            SpaceEditor { spaces, editing, shell }
            super::hover::HoverLayer { site: super::hover::Site::Frame, shell, revision, spaces: Some(spaces) }
            if shell.read().command.is_some() {
                CommandMenu { shell, pages, revision, side_hidden, sync_state, spaces }
            }
            if shell.read().contacts.is_some() {
                super::contacts::ContactsSheet { shell }
            }
            if shell.read().files.is_some() {
                super::files::FilesSheet { shell, revision }
            }
            if shell.read().adding.is_some() {
                super::add_account::AddAccountSheet { shell, revision, spaces }
            }
            if shell.read().rules.is_some() {
                super::rules::RulesSheet { shell, revision }
            }
            if shell.read().keys.is_some() {
                super::pgp::keys::KeysSheet { shell }
            }
            div { class: "card",
            ThreadList {
                shell, pages, revision, in_a_field, threads, drafts, nothing, more,
                sync_state, entering, marking, top,
            }
            section { class: "reader",
                // A new message is a page in this column; a reply sits under its thread.
                match (compose::composing(&shell.read()), shell.read().open) {
                    (Some((draft, PageKind::Reply)), Some(thread)) => rsx! {
                        Reader { thread, shell, revision,
                            ComposerPage { key: "{draft}", draft, shell, revision }
                        }
                    },
                    (Some((draft, _)), _) => rsx! { ComposerPage { key: "{draft}", draft, shell, revision } },
                    (None, Some(thread)) => rsx! { Reader { thread, shell, revision } },
                    (None, None) => rsx! {
                        div { class: "reader-empty",
                            p { "Nothing open" }
                            p { class: "mono", "pick a thread" }
                        }
                    },
                }
                super::hover::LinkPill {}
            }
            SendPill { shell }
            }
        }
        }
    }
}

/// What the window resolves its look from: the live settings and desktop preferences `launch`
/// provides, else a fixed value a test provides, else the first run.
///
/// A test never reaches `ds_settings::use_environment`, so it never reads or watches the real
/// config directory.
fn environment() -> Environment {
    if let Some(live) = try_consume_context::<ReadSignal<Environment>>() {
        return live();
    }
    try_consume_context::<Environment>().unwrap_or_default()
}

/// The quire root the window draws inside, wearing the current Space.
///
/// Its own component so a Space change re-renders only the root's attributes and frame
/// layers, not `App`: the children are `App`'s, unchanged. The Space's look is the frame; its
/// motion is the root's motion level, since quire's `SpaceLook` has none (reported to quire);
/// the rest of the appearance is `appearance.toml`'s. A switch or an edit only writes the
/// Spaces, and `Ds` cross-fades the frame's layers itself.
#[component]
fn Frame(spaces: Signal<Spaces>, children: Element) -> Element {
    let environment = environment();
    let space = spaces.read().current_space();
    let appearance = window_appearance(&environment, &space);
    rsx! {
        Ds {
            appearance,
            system: environment.system,
            look: space.look,
            material: Material::Window,
            tint_alpha: Some(environment.tint_alpha()),
            {children}
        }
    }
}

/// `appearance.toml`'s appearance, moving as `space` says unless the desktop asks for less.
pub(super) fn window_appearance(environment: &Environment, space: &Space) -> ds::Appearance {
    ds::Appearance {
        motion: space.motion.with_desktop(environment.system),
        ..environment.settings.appearance.appearance()
    }
}

#[cfg(test)]
mod tests {
    use super::super::launch::KEEP_FOCUS;
    use super::App;
    use crate::ui::fixtures::{
        ACCOUNT, FakeKey, INSIDE_THE_SHELL, dispatching, empty, inbox_query, markup, press,
        realistic, seeded,
    };
    use dioxus::prelude::*;
    use dioxus_core::{NoOpMutations, VirtualDom};
    use mail_store::Store;

    #[tokio::test]
    async fn the_whole_app_renders() {
        // Catches what compiling cannot: a missing context, a panic inside `rsx!`, a query that
        // blows up on a real database. Until this test the components had never been executed
        // at all — every other test stops at `view.rs`.
        let (store, _dir) = seeded();
        let mut dom = VirtualDom::new(App).with_root_context(store);
        dom.rebuild_in_place();
    }

    #[tokio::test]
    async fn the_first_run_names_the_command_that_gets_you_out_of_it() {
        // A database with no account looked exactly like an empty mailbox: six folders, a Sync
        // button and "Nothing here." The shell cannot add an account, so that was the end of the
        // road rather than a state with a way out.
        let (store, _dir) = empty();
        let markup = markup(store);

        assert!(markup.contains("No account yet"), "{markup}");
        // The angle brackets come back escaped, which is the renderer doing its job; asserting
        // on one spelling of the escape would be asserting on dioxus rather than on the shell.
        assert!(
            markup.contains("mailo account add"),
            "the command is not on the page:\n{markup}"
        );
        assert!(
            markup.contains("class=\"command\""),
            "it is not set apart from the prose, so it reads as italic advice:\n{markup}"
        );
        assert!(
            !markup.contains("Nothing here"),
            "it still says the thing that told a new user nothing:\n{markup}"
        );
    }

    #[tokio::test]
    async fn a_configured_account_with_an_empty_folder_is_not_told_to_add_an_account() {
        // The other direction. `seeded()` has one account and mail in the inbox, so nothing on
        // the page should be setup advice.
        let (store, _dir) = seeded();
        let markup = markup(store);
        assert!(!markup.contains("No account yet"), "{markup}");
    }

    /// Whether a task spawned from inside an event handler ever runs.
    ///
    /// F103 established that a future spawned from a *component body* is never polled here, and
    /// left the other half open: the Sync button spawns from a click handler, which is a
    /// different path. Left open it is a question about whether the one button that fetches mail
    /// works at all, so it is worth a component that exists only to ask it.
    static SPAWN_RAN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    #[component]
    fn SpawnProbe() -> Element {
        rsx! {
            div {
                onkeydown: move |_| {
                    spawn(async {
                        SPAWN_RAN.store(true, std::sync::atomic::Ordering::SeqCst);
                    });
                },
            }
        }
    }

    static MOUNTED_SPAWN_RAN: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    #[component]
    fn MountedSpawnProbe() -> Element {
        let _ = use_future(move || async move {
            MOUNTED_SPAWN_RAN.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        rsx! { div {} }
    }

    #[tokio::test]
    async fn a_future_started_when_a_component_mounts_does_run() {
        // F103 concluded it does not, from prints taken under the broken launch F107 found. If
        // that conclusion was an artefact then a periodic sync can be an ordinary `use_future`,
        // and if it was not then it has to be driven some other way. Worth knowing before
        // building on either answer.
        MOUNTED_SPAWN_RAN.store(false, std::sync::atomic::Ordering::SeqCst);
        let mut dom = VirtualDom::new(MountedSpawnProbe);
        dom.rebuild_in_place();
        tokio::time::timeout(std::time::Duration::from_millis(500), dom.wait_for_work())
            .await
            .ok();
        dom.render_immediate(&mut NoOpMutations);
        assert!(
            MOUNTED_SPAWN_RAN.load(std::sync::atomic::Ordering::SeqCst),
            "a future started at mount never ran"
        );
    }

    #[tokio::test]
    async fn a_task_spawned_from_an_event_handler_does_run() {
        use dioxus_core::ElementId;

        dispatching();
        SPAWN_RAN.store(false, std::sync::atomic::Ordering::SeqCst);
        let mut dom = VirtualDom::new(SpawnProbe);
        dom.rebuild_in_place();
        #[allow(deprecated)]
        dom.handle_event(
            "keydown",
            std::rc::Rc::new(PlatformEventData::new(Box::new(FakeKey("j")))),
            ElementId(1),
            true,
        );
        // One turn of the loop, which is what a spawned task needs to be picked up.
        tokio::time::timeout(std::time::Duration::from_millis(500), dom.wait_for_work())
            .await
            .ok();
        dom.render_immediate(&mut NoOpMutations);

        assert!(
            SPAWN_RAN.load(std::sync::atomic::Ordering::SeqCst),
            "a task spawned from a click handler never ran — which is how the Sync button works"
        );
    }

    #[tokio::test]
    async fn a_keystroke_reaches_the_store() {
        // The half of F103 that could not be checked by pressing keys at the window: whether a
        // keydown delivered to the shell reaches the handler, the decision, and the database.
        // `j` opens the first conversation and `e` archives it.
        dispatching();
        let (store, _dir) = realistic();
        let mut dom = VirtualDom::new(App).with_root_context(store.clone());
        dom.rebuild_in_place();

        let before = store
            .threads(&inbox_query(), chrono::Utc::now())
            .unwrap()
            .items
            .len();
        assert!(before > 1, "the fixture should have something to move");

        press(&mut dom, "j", INSIDE_THE_SHELL);
        press(&mut dom, "e", INSIDE_THE_SHELL);

        let after = store
            .threads(&inbox_query(), chrono::Utc::now())
            .unwrap()
            .items
            .len();
        assert_eq!(
            after,
            before - 1,
            "a keystroke did not reach the store: {before} conversations before, {after} after"
        );
    }

    #[tokio::test]
    async fn f_opens_a_forward_of_the_open_conversation() {
        // Forwarding was modelled in `mail-domain` and reachable from no surface at all. This is
        // the shell's half: `j` to open a conversation, `f` to carry it somewhere else.
        dispatching();
        let (store, _dir) = realistic();
        let mut dom = VirtualDom::new(App).with_root_context(store.clone());
        dom.rebuild_in_place();
        let before = store.drafts(ACCOUNT).unwrap().len();

        press(&mut dom, "j", INSIDE_THE_SHELL);
        press(&mut dom, "f", INSIDE_THE_SHELL);

        let drafts = store.drafts(ACCOUNT).unwrap();
        assert_eq!(drafts.len(), before + 1, "`f` opened nothing");
        let made = drafts
            .iter()
            .find(|d| d.forward_of.is_some())
            .expect("a forward, not a reply");
        assert!(made.subject.starts_with("Fwd: "), "{}", made.subject);
        assert!(
            made.to.is_empty(),
            "a forward starts with nobody on it; the composer is where they are named"
        );
        assert!(
            made.text
                .contains("---------- Forwarded message ----------"),
            "the message being carried is not in it:\n{}",
            made.text
        );
    }

    #[tokio::test]
    async fn c_writes_to_someone_who_has_not_written_first() {
        // Phase 7a through the real tree. Everything else the keyboard does acts on the
        // conversation under the cursor; this one has no conversation, which is why it is also
        // the only shortcut that does anything at all in an empty mailbox.
        dispatching();
        let (store, _dir) = realistic();
        let mut dom = VirtualDom::new(App).with_root_context(store.clone());
        dom.rebuild_in_place();
        let before = store.drafts(ACCOUNT).unwrap().len();

        // No `j` first, deliberately: nothing is open and nothing needs to be.
        press(&mut dom, "c", INSIDE_THE_SHELL);

        let drafts = store.drafts(ACCOUNT).unwrap();
        assert_eq!(drafts.len(), before + 1, "`c` opened nothing");
        let made = drafts
            .iter()
            .max_by_key(|d| d.updated)
            .expect("the draft just written");
        assert_eq!(
            made.in_reply_to, None,
            "a new message must not answer anything"
        );
        assert_eq!(made.forward_of, None, "nor carry anything");
        assert!(
            made.to.is_empty(),
            "there is no original to take a recipient from, and a guess is one the sender has \
             to notice and undo"
        );
        assert!(made.subject.is_empty(), "{:?}", made.subject);
    }

    #[tokio::test]
    async fn the_composer_offers_a_way_to_attach_a_file() {
        // Phase 7b's window half. `PendingAttachment` was modelled, persisted and assembled into
        // multipart, and no surface could make one — so what this asserts is the existence of
        // the control, which is the whole of what was missing. The control is a button that
        // opens the native dialog (`ui::pick`); `compose::tests::life` presses it.
        dispatching();
        let (store, _dir) = realistic();
        let mut dom = VirtualDom::new(App).with_root_context(store.clone());
        dom.rebuild_in_place();
        press(&mut dom, "c", INSIDE_THE_SHELL);

        let page = dioxus_ssr::render(&dom);
        assert!(
            page.contains(r#"aria-label="Attach""#),
            "the composer has no way to attach anything:\n{page}"
        );
    }

    #[tokio::test]
    async fn the_sidebar_offers_a_way_to_write() {
        // The button, for anyone who does not know the key.
        let (store, _dir) = realistic();
        assert!(markup(store).contains(">Compose<"));
    }

    #[tokio::test]
    async fn the_rows_offer_a_way_to_forward() {
        // The button, for anyone who does not know the key.
        let (store, _dir) = realistic();
        assert!(markup(store).contains("Forward"));
    }

    #[tokio::test]
    async fn a_letter_typed_into_a_reply_is_not_a_shortcut() {
        // The failure the whole `typing` guard exists for, through the real tree rather than
        // only against the pure function: open a conversation, start a reply, and then "e" is a
        // letter someone is writing rather than Archive.
        dispatching();
        let (store, _dir) = realistic();
        let mut dom = VirtualDom::new(App).with_root_context(store.clone());
        dom.rebuild_in_place();

        let drafts_before = store.drafts(ACCOUNT).unwrap().len();
        press(&mut dom, "j", INSIDE_THE_SHELL);
        press(&mut dom, "r", INSIDE_THE_SHELL);
        // Counted across the keystroke, not merely "more than none": `realistic()` seeds a
        // draft of its own, so `drafts > 0` would have been true whatever `r` did.
        assert_eq!(
            store.drafts(ACCOUNT).unwrap().len(),
            drafts_before + 1,
            "`r` did not open a reply, so the rest of this proves nothing"
        );

        let before = store
            .threads(&inbox_query(), chrono::Utc::now())
            .unwrap()
            .items
            .len();
        press(&mut dom, "e", INSIDE_THE_SHELL);

        assert_eq!(
            store
                .threads(&inbox_query(), chrono::Utc::now())
                .unwrap()
                .items
                .len(),
            before,
            "an \"e\" typed into a reply archived the conversation behind it"
        );
    }

    #[tokio::test]
    async fn the_root_can_hold_focus_so_the_keyboard_has_somewhere_to_land() {
        // A keydown targets the focused element and bubbles up, so a handler on an element that
        // can never hold focus is never called. This asserts the one half of that which markup
        // can carry; the other half is `KEEP_FOCUS`, injected into the page head.
        let (store, _dir) = seeded();
        let markup = markup(store);
        assert!(
            markup.contains(r#"tabindex="0""#),
            "the app root is not focusable:\n{markup}"
        );
    }

    #[tokio::test]
    async fn the_focus_script_targets_the_element_that_carries_the_handler() {
        // Two halves of one mechanism in two files: the script focuses `.app`, and `.app` is the
        // class on the div the key handler is attached to. If either is renamed without the
        // other the keyboard stops working silently.
        assert!(KEEP_FOCUS.contains(".app"), "{KEEP_FOCUS}");
        let (store, _dir) = seeded();
        assert!(markup(store).contains(r#"class="app""#));
    }

    #[tokio::test]
    async fn the_shell_draws_the_mail_it_holds() {
        // `rebuild_in_place` only proved the components run. This is the first assertion in the
        // project about what they actually put on the page.
        let (store, _dir) = realistic();
        let markup = markup(store);

        for expected in ["Inbox", "Drafts", "Search all mail", "GitHub", "dinner?"] {
            assert!(markup.contains(expected), "no {expected:?} in:\n{markup}");
        }
        // Long subjects are ellipsised by CSS, not truncated in the markup — the full text has
        // to be there for the browser to do it and for a wider window to show more.
        assert!(
            markup.contains("stabilisation (#53667)"),
            "the subject was cut short before it reached the page:\n{markup}"
        );
    }

    #[tokio::test]
    async fn re_rendering_the_app_is_stable() {
        // A second render is where hook-order mistakes surface: hooks are matched between
        // renders by call order, so a component whose hook count changes corrupts every index
        // after it, and the first render alone would never show it.
        let (store, _dir) = seeded();
        let mut dom = VirtualDom::new(App).with_root_context(store);
        dom.rebuild_in_place();
        for _ in 0..3 {
            dom.mark_dirty(dioxus_core::ScopeId::APP);
            dom.render_immediate(&mut NoOpMutations);
        }
    }

    /// The markup after the stylesheet. `data-peek` is also a selector in the CSS, and a search
    /// of the whole page would find that selector whether or not the shell wrote the attribute.
    fn shell_markup(page: &str) -> &str {
        let Some(class_at) = page.find(r#"class="app""#) else {
            panic!("no app root:\n{page}");
        };
        let start = page[..class_at].rfind('<').unwrap_or(class_at);
        &page[start..]
    }

    fn peek_attr(page: &str) -> String {
        let shell = shell_markup(page);
        let key = "data-peek=\"";
        let Some(at) = shell.find(key) else {
            panic!("the shell has no data-peek:\n{shell}");
        };
        let rest = &shell[at + key.len()..];
        rest.split('"').next().unwrap_or("").to_owned()
    }

    fn scrim_count(page: &str) -> usize {
        shell_markup(page).matches(r#"class="scrim""#).count()
    }

    fn iframe_srcdoc(page: &str) -> String {
        let shell = shell_markup(page);
        let Some(at) = shell.find("<iframe") else {
            panic!("no iframe:\n{shell}");
        };
        let tag = &shell[at..];
        let Some(end) = tag.find('>') else {
            panic!("iframe tag was not closed:\n{tag}");
        };
        let open = &tag[..end];
        let key = "srcdoc=\"";
        let Some(start) = open.find(key) else {
            panic!("iframe has no srcdoc: {open}");
        };
        open[start + key.len()..]
            .split('"')
            .next()
            .unwrap_or("")
            .to_owned()
    }

    /// `(what to do, the data-peek value, whether the scrim is up)`.
    ///
    /// The slugs are literals. The shell writes `Peek::slug()`; if this table called `slug()`
    /// too, the two would agree whatever the function returned.
    const PEEK_STEPS: &[(&str, &str, bool)] = &[
        ("start", "side", false),
        ("open", "side", false),
        ("Centre peek", "center", true),
        ("scrim", "center", false),
        ("open", "center", true),
        ("Full page", "full", true),
        ("escape", "full", false),
        ("open", "full", true),
        ("Side peek", "side", false),
    ];

    #[tokio::test]
    async fn each_peek_writes_data_peek_and_the_scrim_only_while_a_thread_is_open() {
        use crate::ui::fixtures::{click, key};

        dispatching();
        let (store, _dir) = realistic();
        let mut dom = VirtualDom::new(App).with_root_context(store);
        dom.rebuild_in_place();
        let mut seen = crate::ui::fixtures::Seen::default();

        let mut failures = Vec::new();
        for (action, slug, scrim) in PEEK_STEPS {
            match *action {
                "start" => {}
                "open" | "escape" => {
                    let key_name = if *action == "open" { "j" } else { "Escape" };
                    seen = key(&mut dom, key_name);
                }
                "scrim" => {
                    let id = seen.one("aria-label", "Close");
                    seen = click(&mut dom, id);
                }
                label => {
                    let id = seen.one("aria-label", label);
                    seen = click(&mut dom, id);
                }
            }
            let page = dioxus_ssr::render(&dom);
            let got = peek_attr(&page);
            let scrims = scrim_count(&page);
            let want_scrims = if *scrim { 1 } else { 0 };
            if got != *slug || scrims != want_scrims {
                failures.push(format!(
                    "{action}: data-peek {got:?} (want {slug:?}), {scrims} scrims (want {want_scrims})"
                ));
            }
            if *action == "start" {
                if !page.contains("Nothing open") || !page.contains("pick a thread") {
                    failures.push(format!(
                        "the empty reader does not say what is open:\n{page}"
                    ));
                }
                if page.contains("Select a conversation.") {
                    failures.push("the empty reader still says Select a conversation.".to_owned());
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[tokio::test]
    async fn changing_the_peek_does_not_remount_the_reader() {
        use crate::ui::fixtures::{click, key};
        use crate::ui::reading::{reader_mounts, reset_reader_mounts};

        dispatching();
        reset_reader_mounts();
        let (store, _dir) = realistic();
        let mut dom = VirtualDom::new(App).with_root_context(store);
        dom.rebuild_in_place();

        // The newest rows are plain text. Walk until the open thread is one with a frame,
        // which is the node a remount would reload. The peek buttons are created on the first
        // open and not rewritten while only the message changes, so their ids come from that
        // first render.
        let mut centre = None;
        let mut full = None;
        let mut page = String::new();
        for _ in 0..8 {
            let seen = key(&mut dom, "j");
            centre = centre.or_else(|| seen.get("aria-label", "Centre peek"));
            full = full.or_else(|| seen.get("aria-label", "Full page"));
            page = dioxus_ssr::render(&dom);
            if shell_markup(&page).contains("<iframe") {
                break;
            }
        }
        assert!(
            shell_markup(&page).contains("<iframe"),
            "no HTML thread in the fixture:\n{page}"
        );
        let mounted = reader_mounts();
        assert!(mounted >= 1, "the reader never mounted");
        let srcdoc = iframe_srcdoc(&page);
        let centre = centre.expect("the reader never drew Centre peek");
        let full = full.expect("the reader never drew Full page");

        click(&mut dom, centre);
        click(&mut dom, full);
        let after = dioxus_ssr::render(&dom);
        assert_eq!(
            reader_mounts(),
            mounted,
            "peek changed the reader's mount count, so the iframe was recreated: {mounted} before, {} after",
            reader_mounts()
        );
        assert_eq!(
            peek_attr(&after),
            "full",
            "the Full page click did not change the peek, so this test never moved the reader"
        );
        assert_eq!(
            iframe_srcdoc(&after),
            srcdoc,
            "peek replaced the iframe's document"
        );
    }
}

#[cfg(test)]
mod reactivity_tests {
    //! What a keystroke costs.
    //!
    //! A memo that reads a signal is subscribed to *every* change of it, so reading `shell` to
    //! get something that never changes — the sidebar's places — makes an unrelated write
    //! recompute it. The list must re-query when the search box changes; the badges must not.

    use super::*;
    use dioxus_core::{NoOpMutations, VirtualDom};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// How many times the badge memo has run.
    static BADGE_RUNS: AtomicUsize = AtomicUsize::new(0);
    /// The shell signal, published so the test can write it the way a keystroke does.
    static TYPE_NOW: AtomicUsize = AtomicUsize::new(0);

    /// A stand-in for `App`'s badge memo: depends on `revision`, not on `shell`.
    #[component]
    fn Badges() -> Element {
        let mut shell = use_signal(Shell::default);
        let revision = use_signal(|| 0u64);

        let filters: Vec<Option<Filter>> = use_hook(|| {
            crate::view::default_places()
                .iter()
                .map(|place| badge_filter(&place.source))
                .collect()
        });
        let badges = use_memo(move || {
            let _ = revision();
            BADGE_RUNS.fetch_add(1, Ordering::SeqCst);
            filters.len()
        });

        // A write to `shell`, driven from the test: this is the keystroke. Done in an effect
        // rather than during render, because writing a signal while rendering is not what a
        // key press does and not what is being measured.
        use_effect(move || {
            if TYPE_NOW.swap(0, Ordering::SeqCst) > 0 {
                shell.write().search.push('x');
            }
        });

        let typed = shell.read().search.len();
        rsx! { div { "{badges()} {typed}" } }
    }

    #[tokio::test]
    async fn typing_in_the_search_box_does_not_recount_every_badge() {
        BADGE_RUNS.store(0, Ordering::SeqCst);
        TYPE_NOW.store(0, Ordering::SeqCst);
        let mut dom = VirtualDom::new(Badges);
        dom.rebuild_in_place();
        let after_first = BADGE_RUNS.load(Ordering::SeqCst);

        for _ in 0..5 {
            // A keystroke: write `shell`, then let the dom settle.
            TYPE_NOW.store(1, Ordering::SeqCst);
            dom.mark_dirty(dioxus_core::ScopeId::APP);
            dom.render_immediate(&mut NoOpMutations);
            dom.render_immediate(&mut NoOpMutations);
        }

        let runs = BADGE_RUNS.load(Ordering::SeqCst);
        assert_eq!(
            runs,
            after_first,
            "the badge memo re-ran {} times for keystrokes that changed no count; each run is \
             one indexed query per place",
            runs - after_first
        );
    }
}
