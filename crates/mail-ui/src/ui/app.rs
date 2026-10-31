use super::actions::{self, Heard};
use super::ask::{self, Open, Window};
use super::compose::{self, SendPill};
use super::data::PAGE;
use super::frame;
use super::intent::{Intent, Scope, intent, on_thread};
use super::list::ThreadList;
use super::list_query::{ListView, use_list};
use super::live::use_live;
use super::ops::{Writes, open_composer};
use super::reading::{ReaderIn, reading_column};
use super::sidebar::Places;
use super::style::STYLE;
use super::window_core::{use_window_core, use_window_desk};
use crate::ui::space::Spaces;
use crate::ui::view::{
    Appearance, Listing, PageMenu, Peek, Shell, Shortcut, nothing_to_show, places_with,
    scoped_unread_filter,
};
use chordkit::StandardAction;
use dioxus::prelude::*;
use ds::components::app::spaces::{Showing, SwitchChord, Switched, use_spaces};
use ds::components::chrome::split_view::model::{Collapsing, PaneSize, PaneSpec, SplitPane};
use ds::components::chrome::split_view::view::SplitView;
use ds::prelude::*;
use ds::style::kit::Styles;
use ds_settings::Environment;
use mail_client::profile::ProfileId;
use mail_domain::*;
use mail_ui_model::selection::Toward;

/// The Spaces the window opens on, `first` as stored: a window started from a click (`mailo
/// open`, its conversation already in the shell) opens on the Space [`ask::space_for`] chooses
/// for it, written so that is where the window is. Shown without a switch: nothing was on
/// screen to remember.
fn arrived_at(mut first: Spaces) -> Spaces {
    let Some(open) = try_consume_context::<Open>() else {
        return first;
    };
    let client = crate::ui::live::client();
    if let Some(id) = ask::space_for(&first, &client, &*open.ask)
        && id != first.current().id
    {
        first.select(id);
        frame::keep(&first);
    }
    first
}

#[component]
pub(super) fn App() -> Element {
    // The places, labels and badges are values the client keeps read (`use_live`), never a read
    // on the thread that draws.
    // The launched look, when `main` provided one. A test that builds `App` with only the
    // client keeps the first-run appearance.
    // Opened on a conversation when the window was started to show one (`mailo open`), or on
    // the composer when it was started from a `mailto:` link.
    let mut shell = use_signal(|| {
        let mut shell = Shell {
            appearance: try_consume_context::<Appearance>().unwrap_or_default(),
            // The user's keys, read once. A window handed no directories reads no file.
            keymap: try_consume_context::<crate::ui::appearance::WindowDirs>()
                .map(|dirs| crate::ui::keymap::load(&dirs.config))
                .unwrap_or_default(),
            // The orders chosen for the places, where the window's root already read them; the
            // effect after `use_prefs` below reads them where it did not.
            sorts: mail_ui_model::settings::sorts_of(&crate::ui::prefs::current().window),
            ..Shell::default()
        };
        // What the window was started to show (`mailo open`, a banner's click): its conversation
        // or inbox, shown here. Its Space is chosen with the Spaces, below, and what it does
        // once shown is done after the first draw.
        if let Some(open) = try_consume_context::<Open>() {
            open.ask.show(&mut shell);
        }
        shell
    });
    // The window's keymap, with mailo's actions in it.
    let core = use_window_core();
    let keys = core.keys;
    // The conversations opened in windows of their own, to raise one asked for again.
    super::window::use_opened();
    super::settings_window::use_settings_opened();
    let boot = use_hook(frame::load_boot);
    // quire's Spaces: the list, switching, the look and the menu. Where a Space was left is the
    // shell's place and open thread; arriving shows where the Space was left.
    // How many pages of the list have been asked for. Reset whenever the list itself changes,
    // because "page 3" of the Inbox means nothing once the user is looking at Archive.
    let mut pages = use_signal(|| 1u32);
    let handle = {
        let first = boot.spaces.clone();
        use_spaces(
            move || arrived_at(first),
            |spaces: &crate::ui::space::Spaces| frame::keep(spaces),
            move || super::switch::recall_of(&shell.peek()),
            move |arrived: Switched<crate::ui::space::Recall>| {
                super::switch::restore(&mut shell.write(), &arrived.restore);
                pages.set(1);
            },
        )
    };
    // For what switches Space from elsewhere: ⌘K's Spaces.
    use_context_provider(|| handle);
    // What a running window is asked from outside: a notification's click, `mailo open`. A
    // conversation opens here, where the person reads, in a Space that shows its account, and
    // the window is raised with the click's token.
    super::handoff::use_handoff(shell, handle);
    // A window started on an ask does what it asks once it is shown: a banner's Archive archives
    // the conversation as the window's own Archive does (the toast with Undo, a failure said in
    // it), a Reply opens the composer, a `mailto:` link saves its draft and opens it.
    // It runs as an effect, after the first draw: the toast host is mounted with the list, and a
    // toast said before it is would only be kept as words, never shown.
    let mut launched = use_hook(|| CopyValue::new(try_consume_context::<Open>()));
    use_effect(move || {
        let asked = launched.write().take();
        if let Some(open) = asked {
            open.ask.run(Window {
                shell,
                spaces: handle,
            });
        }
    });
    let spaces = handle.spaces();
    let mut today_list = use_signal(|| boot.today.clone());
    // ⌘K's Recent reads it: what was opened and put aside in each Space.
    use_context_provider(|| today_list);
    let dirs = boot.dirs.clone();
    // A Space deleted in another window (Settings › Spaces) takes its Today with it here, as one
    // deleted from the Space's menu does.
    let mut known = use_hook(|| CopyValue::new(space_ids(&spaces.peek())));
    use_effect({
        let dirs = dirs.clone();
        move || {
            let now = space_ids(&spaces.read());
            let gone: Vec<crate::ui::space::SpaceId> = known
                .peek()
                .iter()
                .filter(|id| !now.contains(id))
                .copied()
                .collect();
            known.set(now);
            if gone.is_empty() {
                return;
            }
            for id in gone {
                today_list.write().drop_space(id);
            }
            if let Some(dirs) = &dirs {
                let _ = crate::ui::today::save(&dirs.state, &today_list.peek());
            }
        }
    });
    // The files this window saves, behind the sidebar's Downloads button.
    super::downloads::use_provide_shelf(dirs.as_ref());
    let mut side_hidden = use_signal(|| false);
    let prefs = super::prefs::use_prefs(dirs.as_ref());
    // The places' orders follow `settings.toml`, so one chosen in another window sorts this one.
    use_effect(move || {
        let sorts = mail_ui_model::settings::sorts_of(&prefs.read().window);
        if shell.peek().sorts != sorts {
            shell.write().sorts = sorts;
        }
    });
    // Another window (Settings) may have written a key binding or the Spaces: the desk's hook
    // reads them again. `settings.toml` needs nothing here: the window's root watches it.
    let desk = use_window_desk(core, shell, spaces, Some(handle), today_list, dirs.clone());
    let mut seen_open = use_signal(|| None::<mail_domain::ThreadId>);
    // Every account with its folders and labels, and the saved views, kept read by the client:
    // the places are made of them.
    let every = use_live(|| mail_client::query::Accounts);
    let views = use_live(|| mail_client::query::Views);
    // The VIPs, whom the VIPs place lists and its badge counts.
    let vips_live = use_live(|| mail_client::query::VipList);
    let vip_list = use_memo(move || {
        vips_live
            .read()
            .value()
            .map(|vips| vips.as_ref().clone())
            .unwrap_or_default()
    });
    // The labels, folders and saved views the places are made of, as they stand. Equal values
    // are not news, so a reading that changed nothing redraws nothing.
    let known = use_memo(move || {
        every
            .read()
            .value()
            .map(|accounts| super::sidebar::known_labels(accounts))
            .unwrap_or_default()
    });
    let folder_refs = use_memo(move || {
        every
            .read()
            .value()
            .map(|accounts| super::sidebar::folder_places(accounts))
            .unwrap_or_default()
    });
    let saved_views = use_memo(move || {
        views
            .read()
            .value()
            .map(|views| views.as_ref().clone())
            .unwrap_or_default()
    });
    // The first frame has the places when the client already has the accounts read (the
    // program reads them before the window opens): a chip's name is a lookup, and a lookup
    // against an empty list draws an empty chip.
    use_hook(|| {
        let client = crate::ui::live::client();
        let mut write = shell.write();
        if every.peek().value().is_some() {
            write.labels = known.peek().clone();
            write.vips = vip_list.peek().clone();
            write.places = places_with(
                &known.peek(),
                &folder_refs.peek(),
                &saved_views.peek(),
                &vip_list.peek(),
            );
        }
        write.accounts = client.compose().sending_accounts();
    });

    // The window follows the store's moves from its first frame: a client that reads on the thread
    // that draws (a test's window) acts on its commits from then on.
    use_hook(crate::ui::live::moved_peek);

    // Mail that only arrives when you press a button is mail you miss: every account's link, the
    // timers that poll them and the loop that runs their passes (`fetching`).
    super::fetching::use_fetching();
    // Whom to ask to sign in again, once the links have all said how they stand.
    super::reauth::use_reauth();
    // The unread count on the dock or the Dash, when the window was launched with one.
    super::launcher_count::use_launcher_count(shell);
    // Trash and Spam erased after the time Settings gives them, and Delete's under "deletes now".
    super::erase::use_eraser(shell);

    // The accounts the list is showing. A memo of its own, so only a change of Space reaches the
    // counts, never a keystroke.
    let badge_scope = use_memo(move || shell.read().account_filter());
    // What each place's badge counts, in the order of the places drawn. A memo, so a keystroke
    // (a shell change that leaves the places as they are) asks nothing again.
    let badge_filters = use_memo(move || {
        let scope = badge_scope();
        shell
            .read()
            .places
            .iter()
            .map(|place| scoped_unread_filter(&place.source, scope.clone()))
            .collect::<Vec<Option<Filter>>>()
    });
    // The counts, kept by the client and read again as mail arrives or changes, off the thread
    // that draws.
    let counted = use_live(move || {
        mail_client::query::Counts(badge_filters().into_iter().flatten().collect())
    });
    // Each place's badge: the count of its own filter, found by the filter, so an answer read
    // for another list of places never puts a count on the wrong row. None until counted, and
    // none for nothing unread.
    let badges = use_memo(move || {
        let counted = counted.read();
        let counted = counted.value();
        badge_filters()
            .iter()
            .map(|filter| {
                let filter = filter.as_ref()?;
                mail_client::query::Counts::of(counted?, filter).filter(|n| *n > 0)
            })
            .collect::<Vec<Option<u64>>>()
    });

    // The label names the search box can resolve, as the client reads them: a sync that ingests
    // a new Gmail label makes `label:` find it without a restart. Written back only when it has
    // actually changed, so an unrelated reading does not invalidate the list below.
    use_effect(move || {
        // Not before both are read: the places of an account list not read yet are not "no
        // labels", and the chosen place must not be lost to them.
        if every.read().value().is_none() || views.read().value().is_none() {
            return;
        }
        let known = known();
        let folders = folder_refs();
        let views = saved_views();
        if shell.peek().labels != known {
            shell.write().labels = known.clone();
        }
        // And the VIPs: one marked in the reader, a row's menu or Settings is listed at once.
        let vips = vip_list();
        if shell.peek().vips != vips {
            shell.write().vips = vips.clone();
        }
        let next = places_with(&known, &folders, &views, &vips);
        if shell.peek().places != next {
            // The same place stays chosen when one is added before it: a new label moves every
            // folder down by one, and the list must not jump to the folder above.
            let selected = {
                let current = shell.peek();
                super::view::selected_after(&current.places, current.selected, &next, &vips)
            };
            let mut write = shell.write();
            write.places = next;
            write.selected = selected;
        }
    });

    // The accounts the From row can send from: an account added in a terminal reaches the open
    // window without a restart.
    use_effect(move || {
        let _ = crate::ui::live::moved_for(
            mail_client::Topics::ACCOUNTS | mail_client::Topics::IDENTITIES,
        );
        let client = crate::ui::live::client();
        let sending = client.compose().sending_accounts();
        if shell.peek().accounts != sending {
            shell.write().accounts = sending;
        }
    });

    // The Space's account list, whenever the Spaces change: the Space's menu (Accounts) edits
    // it. Read through `peek`, so a shell change (a tile pressed) does not run this. Another
    // Space, or a pressed tile whose account left the Space, goes back to every account.
    let mut on_space = use_hook(|| CopyValue::new(None::<crate::ui::space::SpaceId>));
    use_effect(move || {
        let (id, scope) = {
            let all = spaces.read();
            (all.current().id, all.current().payload.scope())
        };
        let moved = on_space.replace(Some(id)).is_some_and(|was| was != id);
        let pressed = shell.peek().account.clone();
        let left = pressed.is_some_and(|account| moved || !scope.shows(account));
        if shell.peek().scope != scope || left {
            let mut write = shell.write();
            write.scope = scope;
            if left {
                write.account = None;
            }
        }
    });

    // How the list is drawn is kept (`window.snippets`, `window.label_chips`,
    // `window.group_by_date`): the list's ⋯ menu writes it, and detent or another window may.
    // Memoised, so a change to any other setting does not run the effect.
    let drawn = use_memo(move || {
        let window = prefs.read().window.clone();
        (
            super::view::PageParts::of(&window),
            super::view::PageGroup::from(window.group_by_date),
        )
    });
    use_effect(move || {
        let (parts, group) = drawn();
        if shell.peek().parts != parts || shell.peek().group != group {
            let mut write = shell.write();
            write.parts = parts;
            write.group = group;
        }
    });

    // The places kept a message to a row (`window.single_messages`): the list's ⋯ writes them,
    // and another window may.
    use_effect(move || {
        let single = prefs.read().window.single_messages.clone();
        if shell.peek().single != single {
            shell.write().single = single;
        }
    });

    // Provider marks are a setting (`window.provider_marks`), which detent may change while the
    // window is open.
    use_effect(move || {
        let marks = crate::ui::view::Marks::from(prefs.read().window.provider_marks);
        if shell.peek().appearance.marks != marks {
            shell.write().appearance.marks = marks;
        }
    });

    // An account removed, here or from a terminal, leaves every Space. One added stays where it
    // was put, which may be no Space. Not while a part of the Space's menu is open: its close
    // writes the Spaces. Run again when the part closes, which also takes the account out of the
    // Space it kept.
    // The ids as the client has read them; none while that read has not happened or failed,
    // which must not look like every account gone. Equal ids are not news.
    let account_ids = use_memo(move || {
        every.read().value().map(|accounts| {
            accounts
                .iter()
                .map(|account| account.id.clone())
                .collect::<Vec<_>>()
        })
    });
    use_effect(move || {
        let known = account_ids();
        if handle.editing().is_some() {
            return;
        }
        let Some(known) = known else {
            return;
        };
        let mut next = spaces.peek().clone();
        // The window opens the default Profile until it can open others.
        let profile = ProfileId::default();
        if crate::ui::space::forget_unknown(&mut next, &profile, &known)
            == crate::ui::space::Forgot::Changed
        {
            frame::keep(&next);
            let scope = next.current().payload.scope();
            let mut spaces = spaces;
            spaces.set(next);
            shell.write().scope = scope;
        }
    });

    // Opening a thread is a shortcut in Today and a line in History. Closing one is not a write
    // to the mail.
    let today_dirs = dirs.clone();
    use_effect(move || {
        let open = shell.read().open;
        if open == seen_open() {
            return;
        }
        seen_open.set(open);
        let Some(id) = open else {
            return;
        };
        let space = spaces.peek().current().id;
        let now = chrono::Utc::now();
        // Off the thread that draws: the store's writer may be held by a sync.
        let client = crate::ui::live::client();
        drop(tokio::task::spawn_blocking(move || {
            client.history().record(id, now)
        }));
        today_list
            .write()
            .opened(space, id, crate::ui::today::at(now));
        if let Some(dirs) = today_dirs.clone() {
            let _ = crate::ui::today::save(&dirs.state, &today_list.read());
        }
    });

    // The list, by the same rule as the badges: computed here the first time, off the thread
    // afterwards, and for a search only once the box is still (`list_query`).
    let ListView {
        threads,
        messages,
        top,
        marking,
        paging,
        asked,
        failed,
    } = use_list(shell, pages);
    // Render what the person is about to open before they open it — the top of the list, and
    // the neighbours of the open conversation — so that opening it is a lookup.
    super::reading::use_warming(shell, threads);

    // Every account's drafts, kept read, while Drafts or Scheduled is shown; nothing is read
    // for any other place.
    let held = crate::ui::live::use_live(move || {
        let listing = shell.read().listing(PAGE);
        mail_client::query::DraftsOf(match listing {
            Listing::Drafts => every
                .read()
                .value()
                .map(|accounts| accounts.iter().map(|account| account.id.clone()).collect())
                .unwrap_or_default(),
            Listing::Scheduled => {
                let sending: Vec<(String, porter_core::AccountId)> = every
                    .read()
                    .value()
                    .map(|accounts| {
                        accounts
                            .iter()
                            .filter(|account| !account.local)
                            .map(|account| (account.address.clone(), account.id.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                super::compose::sending_in(&sending, &shell.read().scope)
            }
            _ => Vec::new(),
        })
    });
    let drafts = use_memo(move || {
        let found = held
            .read()
            .value()
            .map(|drafts| drafts.to_vec())
            .unwrap_or_default();
        match shell.read().listing(PAGE) {
            Listing::Drafts => found,
            Listing::Scheduled => super::compose::scheduled_in(found),
            _ => Vec::new(),
        }
    });

    // One more row than asked for means there is another page. Asking the client for the count
    // would be a second query answering a question this one already answers.
    // Deliberately a growing limit rather than `Page::next`, which the client also returns. A
    // cursor is the usual answer for an accumulating list — but accumulating means holding
    // pages in a signal and rebuilding them after every archive, star and ingest, and a mixture
    // of pages fetched at different moments is exactly the inconsistency keyset pagination
    // exists to avoid. One query for the whole visible list is always self-consistent, costs a
    // few thousand indexed rows at the sizes this client is
    // for, and needs no invalidation logic at all. Revisit when a mailbox is large enough to
    // measure, at which point the cursor is already there.
    let more = use_memo(move || threads().len() as u32 >= PAGE * pages());

    // Why the pane is empty, when it is. Counted rather than assumed: the shell cannot add an
    // account, so the first run needs to name the command that can.
    let nothing = use_memo(move || {
        // Before the client has read the accounts there is no telling there are none, so the
        // pane does not say "No account" for the moment the first read takes.
        let count = every.read().value().map_or(1, |accounts| accounts.len());
        let shell = shell.read();
        nothing_to_show(count, &shell.search, &shell.quick)
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
        // `Key`'s Display is the DOM key name: "e", "ArrowDown", "Escape".
        let key = event.key().to_string();
        // Peek owns it while it is open: Esc closes it, the arrows move between attachments and
        // turn a PDF's pages, and nothing reaches the conversation behind it.
        if shell.read().preview.is_some() {
            super::reading::peek_key(shell, keys, &event);
            return;
        }
        // The Import and Export sheets likewise: their fields take letters, Esc closes them.
        if shell.read().files.is_some() {
            if key == "Escape" {
                super::files::close(shell);
            }
            return;
        }
        // The sign-in question: Esc is Later, and no other key does what it would to the rows
        // behind it.
        if let Some(reauth) = try_consume_context::<super::reauth::Reauth>()
            && reauth.asking()
        {
            if key == "Escape" {
                reauth.later();
            }
            return;
        }
        // The view editor likewise: its fields take letters, Esc closes it.
        if shell.read().view_editor.is_some() {
            if key == "Escape" {
                super::views::close(shell);
            }
            return;
        }
        // The Delete Forever alert: Esc cancels it, and no other key does anything, least of all
        // what it would have done to the rows behind it. Only its button deletes.
        if shell.read().destroying.is_some() {
            if key == "Escape" {
                super::destroy::close(shell);
            }
            return;
        }
        // Tidy Up likewise: Esc cancels it, and only its button archives.
        if shell.read().applying_rules.is_some() {
            if key == "Escape" {
                super::rules::apply::close(shell);
            }
            return;
        }
        if shell.read().tidying.is_some() {
            if key == "Escape" {
                super::tidy::close(shell);
            }
            return;
        }
        // A message's headers or source likewise: Esc closes the sheet, and no key reaches the
        // rows behind it.
        if shell.read().raw.is_some() {
            if key == "Escape" {
                super::reading::close_raw(shell);
            }
            return;
        }
        // A Space's menu, or the part it opened, owns the keyboard while it shows: the name
        // field takes letters, the colour field's handles the arrows, and quire's layer stack
        // closes it on Escape. A part that holds no focus (the Delete question) never sees the
        // key, so an Escape that reaches the window closes it here. Edit Space… opens Settings
        // instead of the kit's sheet (`space_menu`), so `Edit` never shows here.
        if let Some(open) = handle.open() {
            if key == "Escape" && !matches!(open.showing, Showing::Menu | Showing::Edit) {
                handle.close_menu();
            }
            return;
        }
        // Escape closes ⌘F's bar before it closes anything behind it (its field closes it
        // itself, and keeps the key).
        if key == "Escape" && shell.read().find.is_some() && shell.read().composing.is_none() {
            super::reading::find::close(shell);
            return;
        }
        let typing_now = in_a_field() || shell.read().composing.is_some();
        // What the press asks, from chordkit's keymap with the user's keys over it: a standard
        // action (⌘F, ⌘P, ⌘Z), one of mailo's chords (⌘K, ⌘1), or a mail key (j, e, #).
        let heard = actions::heard(keys, &event, typing_now);
        let scope = if typing_now {
            Scope::Typing
        } else {
            Scope::List
        };
        let intent = intent(heard, scope);
        if super::motion::key(
            &key,
            heard == Some(Heard::Standard(StandardAction::Undo)),
            typing_now,
            shell,
        ) {
            return;
        }
        // ⌘1 to ⌘9 are Space n: quire's kit says which, on the platform's primary key.
        if let Some(number) =
            SwitchChord::Primary.pressed(keys.platform(), &event.key(), event.modifiers())
        {
            let _ = handle.switch_index(usize::from(number.get()) - 1);
            return;
        }
        match intent {
            Some(Intent::ToggleSidebar) => {
                side_hidden.set(!side_hidden());
                return;
            }
            Some(Intent::Print) => {
                // The open conversation, as one flow. Nothing open: nothing printed, nothing said.
                let open = shell.read().open;
                if let Some(job) = super::print::job_for(open) {
                    super::print::print(job);
                }
                return;
            }
            Some(Intent::Search) => {
                super::command::summon(shell);
                return;
            }
            Some(Intent::Find) => {
                // With no conversation open there is nothing to find in: it is the search.
                if !super::reading::find::open(shell) {
                    super::command::summon(shell);
                }
                return;
            }
            Some(Intent::FindStep(step)) => {
                super::reading::find::step(shell, step);
                return;
            }
            Some(Intent::Settings) => {
                super::settings_window::open();
                return;
            }
            Some(Intent::Zoom(step)) => {
                super::reading::zoom(step);
                return;
            }
            Some(Intent::Raw(raw)) => {
                let open = shell.read().open;
                if let Some(open) = open {
                    super::reading::ask_raw(shell, open, raw);
                }
                return;
            }
            _ => {}
        }
        // A menu is showing its own cursor. Shortcuts would archive a thread the user is
        // trying to filter for, and the menu's own handler already took the arrows.
        let menu_open = {
            let current = shell.read();
            current.page_menu != PageMenu::Closed || current.further.is_some()
        };
        if menu_open {
            if key == "Escape" {
                let mut write = shell.write();
                write.page_menu = PageMenu::Closed;
                write.further = None;
                super::host::Host::focus_app();
            }
            return;
        }
        // Filtering the list is the list's, so a menu that is open takes the key first.
        if intent == Some(Intent::FilterUnread) {
            super::list::quick::toggle(shell, pages, super::quick::Toggle::Unread);
            return;
        }
        let listed = || -> Vec<ThreadId> { threads().iter().map(|t| t.id).collect() };
        // ⌘A picks every listed conversation. In a field it is the field's select-all.
        if intent == Some(Intent::SelectAll) {
            shell.write().pick_all(&listed());
            return;
        }
        // Esc lets go of a selection before it closes anything behind it.
        if key == "Escape" && !typing_now && shell.read().picked.any(&listed()) {
            shell.write().unpick(&listed());
            return;
        }
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
        // Shift+Enter opens the open conversation in a window of its own (a focused row's own
        // handler takes it first, for that row).
        if intent == Some(Intent::OpenInWindow) {
            let open = shell.read().open;
            if let Some(open) = open {
                super::window::open_in_window(open);
            }
            return;
        }
        // ⌘N writes a message wherever the caret is, Move to Trash trashes while reading. No bare letter is
        // a shortcut with ⌘ held: ⌘C is copy, and ⌘A was select-all above. Esc still closes.
        let Some(Intent::Mail(action)) = intent else {
            return;
        };
        let client = crate::ui::live::client();
        let open = shell.read().open;
        match action {
            Shortcut::Next | Shortcut::Previous => {
                let forward = action == Shortcut::Next;
                let singles = messages();
                let rows = threads();
                if !singles.is_empty() && singles.len() == rows.len() {
                    // A message to a row: the cursor moves by message, and opens each alone.
                    let here = shell.read().focus.filter(|_| open.is_some());
                    if let Some(next) = mail_client::conversation::step(here, &singles, forward)
                        && let Some(at) = singles.iter().position(|m| *m == next)
                    {
                        shell.write().open_message(rows[at].id, next, at);
                    }
                } else {
                    let ids: Vec<ThreadId> = rows.iter().map(|t| t.id).collect();
                    if let Some(id) = mail_client::conversation::step(open, &ids, forward) {
                        shell.write().open(id);
                    }
                }
            }
            Shortcut::ExtendNext | Shortcut::ExtendPrevious => {
                let toward = if action == Shortcut::ExtendNext {
                    Toward::Next
                } else {
                    Toward::Previous
                };
                let singles = messages();
                if singles.is_empty() || singles.len() != threads().len() {
                    shell.write().extend(toward, &listed());
                } else {
                    // A message to a row: the range moves by row, as a conversation can have several.
                    shell.write().extend_rows(toward, &listed());
                }
            }
            Shortcut::Back => {
                if shell.read().composing.is_some() {
                    // Parked, not closed: saved, and waiting in Today.
                    compose::park_current(desk, shell);
                } else if open.is_some() {
                    let mut write = shell.write();
                    write.open = None;
                    write.focus = None;
                } else if matches!(
                    super::command::affordance(&shell.read().search, &shell.read().bar),
                    super::command::Affordance::Searching(_)
                ) {
                    // With nothing open, Esc lets go of the search narrowing the list, as the
                    // × at the end of its header does.
                    super::command::clear(shell, pages);
                }
            }
            Shortcut::TogglePin | Shortcut::Reply | Shortcut::ReplyAll | Shortcut::Forward => {
                if let Some(id) = open {
                    on_thread(&client, shell, id, action);
                }
            }
            Shortcut::ToggleMute => {
                super::picks::mute_picked(&client, shell, &threads());
            }
            Shortcut::Compose => {
                // The only shortcut that does not consult the conversation under the cursor, and
                // so the only one that does anything in an empty mailbox.
                // Cloned out and the guard dropped before anything writes back. The same
                // hazard the composer's handlers document, caught here by the borrow checker
                // rather than at runtime.
                // A refusal is the composer's to say (`open_composer` logs it).
                let _ = open_composer(&client, shell, Writes::New);
            }
            _ => {
                // The picked conversations, or with none picked the open one. Resolved against
                // their own summaries, so the keyboard reaches exactly what their rows offer and
                // nothing else, and all of it is one gesture with one undo.
                super::picks::act_on_picked(&client, shell, action, &threads());
            }
        }
    };

    // Follow-up reminders: swept at launch, when the next comes due, and whenever the store moves,
    // so a conversation nobody answered comes back to the top of the inbox
    // (`mail_client::follow_up`).
    super::follow_up::use_reminders();
    super::follow_up::use_snooze_wake();
    // The copies Open wrote, removed once their message is deleted when the setting says so.
    super::downloads::unedited::use_remove_unedited();

    let peek = shell.read().peek;
    let frame_class = if side_hidden() { "app no-side" } else { "app" };
    // The open thread or the composer: a column of the card, or, floating, quire's `Peek` over it.
    let reading = rsx! {
        {reading_column(shell.read().open, shell, ReaderIn::Pane)}
    };
    let floating = shell.read().open.is_some() || compose::composing(&shell.read()).is_some();
    let pinned = if side_hidden() {
        Shown::Hidden
    } else {
        Shown::Visible
    };
    let side_body = if side_hidden() {
        rsx! {}
    } else {
        rsx! {
            Places {
                shell, pages, badges, handle,
                side_hidden,
            }
        }
    };
    // The card: the list in a pane of its own (resizable, with a hairline between it and the
    // reader), then the reader, which takes what is left.
    let card = rsx! {
        div { class: "card",
            SplitView {
                label: "List and reader",
                panes: vec![SplitPane::new(LIST, rsx! {
                    ThreadList {
                        shell, pages, threads, messages, drafts, nothing, more,
                        marking, top, paging, side_hidden, question: asked, failed,
                    }
                })],
                section { class: "reader",
                    match (peek, floating) {
                        (Peek::Float(mode), true) => rsx! {
                            ds::components::app::peek::Peek {
                                mode,
                                label: "Reader",
                                onclose: move |()| shell.write().close(),
                                {reading}
                            }
                        },
                        _ => reading,
                    }
                    super::hover::LinkPill {}
                }
            }
            SendPill { shell }
        }
    };
    rsx! {
        Frame { spaces,
        AppStyle { css: STYLE }
        div { class: frame_class,
            style: "--side-w: {SIDEBAR_WIDTH}px",
            tabindex: "0",
            onmounted: super::host::Host::app_mounted,
            onkeydown: on_key,
            onpointermove: move |event| {
                let at = event.client_coordinates();
                let held = !event.held_buttons().is_empty();
                super::motion::drag::moved((at.x, at.y), held);
            },
            onpointerup: move |_| {
                super::motion::drag::release(shell);
            },
            "data-peek": peek.slug(),
            // The window is quire's `SplitView`: the sidebar pane, resizable and folding away past
            // half its least, then the card. Pinned, the sidebar is the pane's body; hidden, its
            // `EdgePeek` moves out of the pane (which clips what floats over it) into a
            // zero-width host at the window's edge.
            SplitView {
                label: "Mail",
                panes: vec![SplitPane::new(SIDEBAR, side_body).shown(pinned)],
                on_shown: move |(_, shown): (usize, Shown)| side_hidden.set(shown == Shown::Hidden),
                {card}
            }
            if side_hidden() {
                div { class: "edge-host",
                    Places {
                        shell, pages, badges, handle,
                        side_hidden,
                    }
                }
            }
            super::space_menu::MailSpaceMenu { handle, today: today_list }
            // A sign-in straight in the browser: run here, and said here when it ends.
            super::add_account::DirectSignInWatch { spaces }
            // The search panel, at the top centre of the window while it is up.
            super::command::Spotlight { shell, pages, in_a_field, side_hidden, spaces }
            if shell.read().files.is_some() {
                super::files::FilesSheet { shell }
            }
            if shell.read().view_editor.is_some() {
                super::views::ViewSheet { shell, pages }
            }
            if shell.read().preview.is_some() {
                super::reading::PeekView { shell }
            }
            if shell.read().destroying.is_some() {
                super::destroy::DestroyAlert { shell }
            }
            if shell.read().applying_rules.is_some() {
                super::rules::apply::ApplyRulesAlert { shell }
            }
            if shell.read().tidying.is_some() {
                super::tidy::TidySheet { shell }
            }
            if shell.read().raw.is_some() {
                super::reading::RawSheet { shell }
            }
            if shell.read().no_account {
                super::no_account::NoAccountAlert { shell }
            }
            super::reauth::ReauthPrompt {}
            // Once, at a launch with an account: make mailo the application email links open.
            super::default_mail::DefaultMailQuestion { shell }
        }
        }
    }
}

/// The sidebar's preferred width. The stylesheet reads it as `--side-w` (set on the window's root),
/// since the send pill is centred over what is right of the sidebar.
pub(super) const SIDEBAR_WIDTH: f32 = 232.0;

/// The sidebar pane: 232 wide, 212 to 320, and it folds away past half of its least. 212 is
/// what the sidebar's rows need; the foot (Downloads, the Space dots, one chevron) needs less.
const SIDEBAR: PaneSpec = PaneSpec {
    preferred: PaneSize::Fixed(Px(SIDEBAR_WIDTH)),
    min: PaneSize::Fixed(Px(212.0)),
    max: PaneSize::Fixed(Px(320.0)),
    collapsing: Collapsing::Snaps,
};

/// The list pane: 400 wide, 280 to 640, and it does not fold.
const LIST: PaneSpec = PaneSpec {
    preferred: PaneSize::Fixed(Px(400.0)),
    min: PaneSize::Fixed(Px(280.0)),
    max: PaneSize::Fixed(Px(640.0)),
    collapsing: Collapsing::Never,
};

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

/// The person's own stylesheets, live when `launch` watches the config directory, else none.
fn styles() -> ReadSignal<Styles> {
    match try_consume_context::<ReadSignal<Styles>>() {
        Some(live) => live,
        None => ReadSignal::new(use_signal(Styles::default)),
    }
}

/// The quire root the window draws inside, wearing the current Space.
///
/// Its own component so a Space change re-renders only the root's attributes and frame
/// layers, not `App`: the children are `App`'s, unchanged. The Space's look is the frame (the
/// sidebar sits on its colour, the card on paper); its motion is the root's motion level, since
/// quire's `SpaceLook` has none (reported to quire); the rest of the appearance, the type
/// included, is `appearance.toml`'s. A switch or an edit only writes the Spaces, and `Ds`
/// cross-fades the frame's layers itself. `sheet` is the design system's stylesheet when a window
/// needs more than `ds::stylesheet()`: the add-account window's parts are `ds-shell`'s.
#[component]
pub(super) fn Frame(
    spaces: Signal<Spaces>,
    #[props(default)] sheet: Option<&'static str>,
    children: Element,
) -> Element {
    let environment = environment();
    let look = spaces.read().current().look.clone();
    let appearance = window_appearance(&environment);
    rsx! {
        Ds {
            appearance,
            system: environment.system,
            look,
            material: Material::Window,
            styles: styles(),
            custom_styles: environment.settings.appearance.custom_styles,
            surface: Some("window"),
            sheet,
            {children}
        }
    }
}

/// `appearance.toml`'s appearance. How much the window moves is the person's motion setting and
/// the desktop's reduced-motion preference (`Ds { system }`), never a Space's.
pub(super) fn window_appearance(environment: &Environment) -> ds::prelude::Appearance {
    environment.settings.appearance.appearance()
}

/// The ids of `spaces`, in order.
fn space_ids(spaces: &crate::ui::space::Spaces) -> Vec<crate::ui::space::SpaceId> {
    spaces.list().iter().map(|space| space.id).collect()
}

#[cfg(test)]
mod tests {
    use super::App;
    use crate::ui::fixtures::{
        FakeKey, INSIDE_THE_SHELL, acct_account, dispatching, drain, empty, inbox_query, markup,
        press, realistic, seeded,
    };
    use dioxus::prelude::*;
    use dioxus_core::{NoOpMutations, VirtualDom};
    use mail_core::Store;

    #[tokio::test]
    async fn the_first_run_offers_a_way_to_add_an_account() {
        // A database with no account looked exactly like an empty mailbox: six folders, a Sync
        // button and "Nothing here." A new person needs a way out, and it is a button in the
        // window, not a command to type in a terminal.
        let (client, _dir) = empty();
        let markup = markup(client);

        assert!(markup.contains("No account"), "{markup}");
        assert!(
            markup.contains(mail_ui_model::copy::controls::ADD_ACCOUNT),
            "no way to add an account:\n{markup}"
        );
        assert!(
            !markup.contains("mailo account add"),
            "a command line is not the way out of the window:\n{markup}"
        );
        let shown = without_styles(&markup);
        assert!(
            !shown.contains("Nothing here"),
            "it still says the thing that told a new user nothing:\n{shown}"
        );
        // No command text anywhere in the frame, whatever it is named in.
        assert!(
            !shown.to_lowercase().contains("mailo "),
            "the first run tells of a command line:\n{shown}"
        );
    }

    #[tokio::test]
    async fn compose_with_no_account_says_so_and_offers_to_add_one() {
        // The pencil, ⌘N and `c` did nothing at all with no account to send from: the reason
        // went to the terminal. The window says it, and offers the way out.
        dispatching();
        let (store, _dir) = empty();
        let mut dom = VirtualDom::new(App).with_root_context(crate::ui::fixtures::client(&store));
        dom.rebuild_in_place();
        assert!(
            !dioxus_ssr::render(&dom).contains("No account to send from"),
            "the alert is up before anything asked for a message"
        );

        press(&mut dom, "c", INSIDE_THE_SHELL);
        // The alert is a quire sheet: placed in the root's overlay by an effect, drawn after.
        drain(&mut dom);

        let page = dioxus_ssr::render(&dom);
        assert!(page.contains("No account to send from"), "{page}");
        assert!(
            page.contains("Add an account to write a message."),
            "{page}"
        );
    }

    #[tokio::test]
    async fn a_configured_account_with_an_empty_folder_is_not_told_to_add_an_account() {
        // The other direction. `seeded()` has one account and mail in the inbox, so nothing on
        // the page should be setup advice: not the "No account" the first run shows.
        let (client, _dir) = seeded();
        let shown = without_styles(&markup(client));
        assert!(!shown.contains("No account"), "{shown}");
    }

    /// Whether a task spawned from inside an event handler ever runs.
    ///
    /// A future spawned from a *component body* is never polled here. The Sync button spawns
    /// from a click handler, which is a different path, and whether it runs decides whether the
    /// one button that fetches mail works at all, so it is worth a component that exists only to
    /// ask it.
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
        // A future started at mount runs, so a periodic sync can be an ordinary `use_future`.
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
        crate::ui::fixtures::deliver(
            &dom,
            "keydown",
            std::rc::Rc::new(PlatformEventData::new(Box::new(FakeKey("j")))),
            ElementId(1),
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
        // Whether a keydown delivered to the shell reaches the handler, the decision, and the
        // database.
        // `j` opens the first conversation and `e` archives it.
        dispatching();
        let (store, _dir) = realistic();
        let mut dom = VirtualDom::new(App).with_root_context(crate::ui::fixtures::client(&store));
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
        let mut dom = VirtualDom::new(App).with_root_context(crate::ui::fixtures::client(&store));
        dom.rebuild_in_place();
        let before = store.drafts(acct_account()).unwrap().len();

        press(&mut dom, "j", INSIDE_THE_SHELL);
        press(&mut dom, "f", INSIDE_THE_SHELL);

        let drafts = store.drafts(acct_account()).unwrap();
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
        // A new message through the real tree. Everything else the keyboard does acts on the
        // conversation under the cursor; this one has no conversation, which is why it is also
        // the only shortcut that does anything at all in an empty mailbox.
        dispatching();
        let (store, _dir) = realistic();
        let mut dom = VirtualDom::new(App).with_root_context(crate::ui::fixtures::client(&store));
        dom.rebuild_in_place();
        let before = store.drafts(acct_account()).unwrap().len();

        // No `j` first, deliberately: nothing is open and nothing needs to be.
        press(&mut dom, "c", INSIDE_THE_SHELL);

        let drafts = store.drafts(acct_account()).unwrap();
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
        // Attaching from the window: what this asserts is the existence of the control, a button
        // that opens the native dialog (`ui::pick`); `compose::tests::life` presses it.
        dispatching();
        let (store, _dir) = realistic();
        let mut dom = VirtualDom::new(App).with_root_context(crate::ui::fixtures::client(&store));
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
        let (client, _dir) = realistic();
        assert!(markup(client).contains(&format!(
            "aria-label=\"{}\"",
            mail_ui_model::copy::aria::NEW_MESSAGE
        )));
    }

    #[tokio::test]
    async fn a_letter_typed_into_a_reply_is_not_a_shortcut() {
        // The failure the whole `typing` guard exists for, through the real tree rather than
        // only against the pure function: open a conversation, start a reply, and then "e" is a
        // letter someone is writing rather than Archive.
        dispatching();
        let (store, _dir) = realistic();
        let mut dom = VirtualDom::new(App).with_root_context(crate::ui::fixtures::client(&store));
        dom.rebuild_in_place();

        let drafts_before = store.drafts(acct_account()).unwrap().len();
        press(&mut dom, "j", INSIDE_THE_SHELL);
        press(&mut dom, "r", INSIDE_THE_SHELL);
        // Counted across the keystroke, not merely "more than none": `realistic()` seeds a
        // draft of its own, so `drafts > 0` would have been true whatever `r` did.
        assert_eq!(
            store.drafts(acct_account()).unwrap().len(),
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
        // can carry; the other half is the host's, which gives `.app` the keyboard when it
        // mounts and after a press that left it nowhere (`ui/host/native.rs`).
        let (client, _dir) = seeded();
        let markup = markup(client);
        let root = shell_markup(&markup);
        let tag = &root[..root.find('>').unwrap_or(root.len())];
        assert!(
            tag.contains(r#"tabindex="0""#),
            "the app root is not focusable:\n{tag}"
        );
    }

    #[tokio::test]
    async fn the_shell_draws_the_mail_it_holds() {
        // `rebuild_in_place` only proved the components run. This is the first assertion in the
        // project about what they actually put on the page.
        let (client, _dir) = realistic();
        let markup = markup(client);

        for expected in [
            "Inbox",
            "Waiting",
            "Starred",
            "Archive",
            "Sent",
            "Pinned",
            "History",
            crate::ui::command::BOX_LABEL,
            "GitHub",
            "dinner?",
        ] {
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
        let mut dom = VirtualDom::new(App).with_root_context(crate::ui::fixtures::client(&store));
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

    /// How many of quire's `Peek` panels are up: the reader over the card.
    fn peeks_up(page: &str) -> usize {
        page.matches(r#"class="ds-peek""#).count()
    }

    /// `(what to do, the data-peek value, whether quire's peek panel is up)`.
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

    /// A press on the reader's ⋯, its View row, then `label` in the view menu, once quire's
    /// blink has passed and the pick has acted: what the window drew.
    async fn pick_view(
        dom: &mut VirtualDom,
        seen: crate::ui::fixtures::Seen,
        label: &str,
    ) -> crate::ui::fixtures::Seen {
        use crate::ui::fixtures::{click, drain_seen, pick_named};
        let more = *seen
            .all("aria-label", mail_ui_model::copy::aria::MORE)
            .last()
            .expect("no reader ⋯");
        click(dom, more);
        // Only the rows this menu drew: what was seen before may hold an earlier menu's.
        let drawn = drain_seen(dom);
        let viewing = pick_named(dom, &drawn, mail_ui_model::copy::menus::VIEW).await;
        let viewing = viewing.merge(drain_seen(dom));
        let after = pick_named(dom, &viewing, label).await;
        seen.merge(drawn).merge(viewing).merge(after)
    }

    #[tokio::test]
    async fn each_peek_writes_data_peek_and_the_panel_only_while_a_thread_is_open() {
        use crate::ui::fixtures::{click, key};

        dispatching();
        let (store, _dir) = realistic();
        let mut dom = VirtualDom::new(App).with_root_context(crate::ui::fixtures::client(&store));
        dom.rebuild_in_place();
        let mut seen = crate::ui::fixtures::Seen::default();

        let mut failures = Vec::new();
        for (action, slug, scrim) in PEEK_STEPS {
            match *action {
                "start" => {}
                "open" | "escape" => {
                    let key_name = if *action == "open" { "j" } else { "Escape" };
                    seen = key(&mut dom, key_name);
                    // A panel quire floats is drawn on the render after the one that asked.
                    let later = crate::ui::fixtures::drain_seen(&mut dom);
                    seen = seen.merge(later);
                }
                "scrim" => {
                    // The panel has two ways to close it, the scrim and its own tool; the scrim
                    // is the first drawn.
                    let id = *seen
                        .all("aria-label", "Close peek")
                        .first()
                        .expect("the panel draws no way to close it");
                    click(&mut dom, id);
                    crate::ui::fixtures::drain(&mut dom);
                }
                label => {
                    seen = pick_view(&mut dom, std::mem::take(&mut seen), label).await;
                }
            }
            let page = dioxus_ssr::render(&dom);
            let got = peek_attr(&page);
            let panels = peeks_up(&page);
            let want_panels = if *scrim { 1 } else { 0 };
            if got != *slug || panels != want_panels {
                failures.push(format!(
                    "{action}: data-peek {got:?} (want {slug:?}), {panels} peek panels (want {want_panels})"
                ));
            }
            if *action == "start" {
                if !page.contains(mail_ui_model::copy::empty::NO_MESSAGE_SELECTED) {
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

    /// `page` without its `<style>` elements.
    fn without_styles(page: &str) -> String {
        let mut shown = String::new();
        let mut rest = page;
        while let Some(open) = rest.find("<style>") {
            shown.push_str(&rest[..open]);
            rest = rest[open..]
                .find("</style>")
                .map_or("", |close| &rest[open + close + "</style>".len()..]);
        }
        shown.push_str(rest);
        shown
    }
}
