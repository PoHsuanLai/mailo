//! The search panel: Spotlight inside the window. ⌘K, or a press on the toolbar's search,
//! brings up a card at the top centre of the window holding a large search field, and under it
//! the mail, commands, places and people the text names.
//!
//! What is typed is the list's search, so the list behind the card narrows as it always did.
//! The card is quire's `SearchField` in its card form (the field and its rows as one surface,
//! floated from the line at the top; a press outside it closes it, with no scrim). The rows show
//! at once (`Panel::Shown`) and the arrows stop at the ends (`Ends::Stop`); the keyboard stays in
//! the field, which moves the highlight and runs it, and mailo's [`super::keys`] completes it and
//! leaves. The rows are one answer to one settled
//! text: the search runs off the thread that draws once the field has been still for
//! [`QUIET`](super::super::debounce::QUIET), and an answer to text a newer keystroke has replaced
//! is dropped, so Return runs a row on screen, not one from a query run afresh.

use super::items::tokens;
use super::keys::{BarKey, Plain, Step, Typed, bar_key, step};
use super::panel::{Drawn, Fetch, World, panel_rows};
use super::sections::completion;
use super::suggest::suggestions;
use crate::ui::actions::{self, Heard, Own};
use crate::ui::menu::anchor_at;
use crate::ui::space::Spaces;
use crate::ui::view::{Bar, BarListing, BarOpen, Shell};
use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_core::SqliteStore;
use std::sync::Arc;

/// What the panel's field is called, to a screen reader and to a test.
pub(in crate::ui) const LABEL: &str = "Search mail and commands";

/// The panel's field, for the host to put the keyboard in.
pub(in crate::ui) const FIELD: &str = ".ds-search-card input";

/// The panel, while it is up; and, always, the line across the top of the window it hangs from.
#[component]
pub(in crate::ui) fn Spotlight(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    revision: Signal<u64>,
    in_a_field: Signal<bool>,
    side_hidden: Signal<bool>,
    spaces: Signal<Spaces>,
) -> Element {
    let store = use_hook(consume_context::<Arc<SqliteStore>>);
    let keys = use_keys();
    let drawn = use_signal(Drawn::default);
    let mut top = use_signal(|| None::<MountedRef>);
    let open = match &shell.read().bar {
        Bar::Open(open) => Some(open.clone()),
        Bar::Closed => None,
    };
    let sidebar = if side_hidden() {
        Shown::Hidden
    } else {
        Shown::Visible
    };
    let rows = open.as_ref().map_or_else(Vec::new, |open| {
        keys.with_keymap(|keymap| {
            let world = World {
                shell: &shell.read(),
                spaces: &spaces.read(),
                store: &store,
                keymap,
            };
            panel_rows(&world, &drawn.read(), &open.listing, sidebar)
        })
    });
    let ctx = Ctx {
        shell,
        pages,
        revision,
        side_hidden,
        spaces,
        drawn,
    };
    let line = rsx! {
        // The line the card hangs from, centred under it: the width of the window, a little
        // way down from its top, as Spotlight sits on the screen.
        div {
            class: "spotlight-line",
            onmounted: move |event| top.set(Some(MountedRef(event.data()))),
        }
    };
    let Some(open) = open else {
        return line;
    };
    let text = field_text(&shell.read());
    let chips = match &open.listing {
        BarListing::Templates(_) => Vec::new(),
        BarListing::Search => tokens(&text),
    };
    // The highlighted row, by its key: quire's cursor names a row by what picking it hands back.
    let lit = rows
        .get(open.active.min(rows.len().saturating_sub(1)))
        .map(|row| row.key.clone());
    let keyed = rows.clone();
    let keyed_on = rows.clone();
    let items = suggestions(&rows);
    rsx! {
        {line}
        Fetch { shell, drawn }
        SearchField::<String> {
            label: LABEL.to_owned(),
            placeholder: "Search mail, commands, places and people".to_owned(),
            size: ControlSize::ExtraLarge,
            value: text,
            tokens: chips,
            suggestions: vec![SuggestionSection::bare(items)],
            // The rows are the person's to run before a key is typed, and Escape on an empty field
            // is ours: it leaves the panel (or the templates), the rows staying until then.
            open: Panel::Shown,
            // Arrows stop at the first and last row, as `keys::step` has it.
            ends: Ends::Stop,
            present: SuggestionsPresent::Card,
            // The highlight is the shell's (`BarOpen::active`), so a click, a key and the text
            // changing all move the one.
            cursor: SearchCursor::Is(lit),
            on_highlight: move |key: Option<String>| {
                if let Some(at) = key.and_then(|key| keyed.iter().position(|row| row.key == key)) {
                    set_active(shell, at);
                }
            },
            // The card hangs from the line across the top of the window; a press outside it
            // closes it, and the search stays what it was.
            place: Some(CardPlace {
                anchor: anchor_at(top()),
                placement: Placement::new(Side::Bottom, Align::Center),
            }),
            ondismiss: move |()| leave(shell),
            oninput: move |value: String| set_text(shell, pages, value),
            onpick: move |key: String| run_row(ctx, &key),
            onkey: move |event: KeyboardEvent| on_key(ctx, keys, &keyed_on, event),
            onfocus: move |()| in_a_field.set(true),
            onblur: move |()| in_a_field.set(false),
        }
    }
}

/// A key in the panel's field: what [`step`] decides, done.
fn on_key(ctx: Ctx, keys: Keys, rows: &[crate::ui::menu::MenuItem], event: KeyboardEvent) {
    let shell = ctx.shell;
    // ⌘K again: the panel's field is in quire's overlay, outside the window's own keys, so it
    // answers the chord itself, selecting what is typed so the next key replaces it.
    let heard = actions::heard(keys, &event, true);
    if heard == Some(Heard::Own(Own::Search)) {
        event.prevent_default();
        event.stop_propagation();
        crate::ui::host::Host::focus_all(FIELD);
        return;
    }
    let plain = if event.modifiers().is_empty() {
        Plain::Yes
    } else {
        Plain::No
    };
    let Some(key) = bar_key(&event.key().to_string(), plain) else {
        return;
    };
    // Return runs the highlighted row through the field's own pick, which quire makes before it
    // lets this hear the key, so this only keeps the key from the window. The arrows are
    // quire's too: they reach here only with no rows to move over, where they do nothing.
    if key == BarKey::Return {
        if !rows.is_empty() {
            event.prevent_default();
            event.stop_propagation();
        }
        return;
    }
    let typed = if field_text(&shell.peek()).is_empty() {
        Typed::Empty
    } else {
        Typed::Some
    };
    let active = match &shell.peek().bar {
        Bar::Open(open) => open.active,
        Bar::Closed => 0,
    };
    let done = step(key, active, rows.len(), typed);
    if done != Step::Pass || key == BarKey::Escape {
        // The window's own keys never hear a key the panel took: Escape is not also "close the
        // reader", Return not also "open in a window".
        event.prevent_default();
        event.stop_propagation();
    }
    match done {
        Step::Move(to) => set_active(shell, to),
        Step::Run(at) => {
            if let Some(row) = rows.get(at) {
                run_row(ctx, &row.key);
            }
        }
        Step::Complete(at) => {
            if let Some(row) = rows.get(at) {
                set_text(shell, ctx.pages, completion(row));
            }
        }
        Step::Clear => set_text(shell, ctx.pages, String::new()),
        Step::Leave => back(shell),
        Step::Pass => {}
    }
}

/// The signals a row's pick acts on.
#[derive(Clone, Copy)]
pub(super) struct Ctx {
    pub shell: Signal<Shell>,
    pub pages: Signal<u32>,
    pub revision: Signal<u64>,
    pub side_hidden: Signal<bool>,
    pub spaces: Signal<Spaces>,
    pub drawn: Signal<Drawn>,
}

/// Run the row keyed `key`: a template while the panel lists them, else what the search's row
/// does.
fn run_row(ctx: Ctx, key: &str) {
    let listing = match &ctx.shell.peek().bar {
        Bar::Open(open) => open.listing.clone(),
        Bar::Closed => BarListing::Search,
    };
    match listing {
        BarListing::Templates(_) => super::templates::run(ctx.shell, ctx.revision, key),
        BarListing::Search => {
            let choice = super::sections::choice_of(&ctx.drawn.peek().results, key);
            super::act(ctx, choice);
        }
    }
}

/// What the field shows: the templates' filter while the panel lists them, else the search.
fn field_text(shell: &Shell) -> String {
    match &shell.bar {
        Bar::Open(BarOpen {
            listing: BarListing::Templates(typed),
            ..
        }) => typed.clone(),
        Bar::Open(_) | Bar::Closed => shell.search.clone(),
    }
}

/// The field holds `text` now: typed, completed or cleared. The first row is highlighted again.
fn set_text(mut shell: Signal<Shell>, mut pages: Signal<u32>, text: String) {
    let mut write = shell.write();
    match &mut write.bar {
        Bar::Open(open) => {
            open.active = 0;
            if let BarListing::Templates(typed) = &mut open.listing {
                *typed = text;
                return;
            }
        }
        Bar::Closed => {
            let before = write.search.clone();
            write.bar = Bar::Open(BarOpen::over(before));
        }
    }
    write.search = text;
    drop(write);
    pages.set(1);
}

fn set_active(mut shell: Signal<Shell>, to: usize) {
    if let Bar::Open(open) = &mut shell.write().bar {
        open.active = to;
    }
}

/// Escape in an empty field: back to the search from the templates, else the panel goes and
/// the keyboard goes back to the window.
fn back(shell: Signal<Shell>) {
    let mut shell = shell;
    if let Bar::Open(open) = &mut shell.write().bar
        && matches!(open.listing, BarListing::Templates(_))
    {
        open.listing = BarListing::Search;
        open.active = 0;
        return;
    }
    leave(shell);
}

/// The panel goes, whatever it listed, the search staying what was typed, and the keyboard goes
/// back to the window.
fn leave(mut shell: Signal<Shell>) {
    if shell.peek().bar != Bar::Closed {
        shell.write().bar = Bar::Closed;
    }
    crate::ui::host::Host::focus_app();
}

/// ⌘K, or a press on the toolbar's search: the panel up over the search there is, and the
/// keyboard in its field with the text selected, so the first key typed replaces it.
pub(in crate::ui) fn summon(mut shell: Signal<Shell>) {
    if shell.peek().bar == Bar::Closed {
        let search = shell.peek().search.clone();
        shell.write().bar = Bar::Open(BarOpen::over(search));
    }
    crate::ui::host::Host::focus_all(FIELD);
}
