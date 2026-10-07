//! The search bar: one field in the list's toolbar that is both the list's search and the
//! window's commands, with a panel of suggestions under it while it has the keyboard.
//!
//! What is typed is the list's search, so the list narrows as it always did. The panel is
//! quire's `Menu`, hung from the field, its highlight the field's (`MenuCursor::Controlled`):
//! the keyboard stays in the field, which moves the highlight, runs it and completes it
//! ([`super::keys`]). Its rows are one answer to one settled text: the search runs off the
//! thread that draws once the field has been still for [`QUIET`](super::super::debounce::QUIET),
//! and an answer to text a newer keystroke has replaced is dropped, so Return runs a row on
//! screen, not one from a query run afresh.

use super::super::menu::{anchor_at, menu_items};
use super::items::tokens;
use super::keys::{BarKey, Plain, Step, Typed, bar_key, step};
use super::panel::{Drawn, Fetch, World, panel_rows};
use super::sections::completion;
use crate::ui::common::classed;
use crate::ui::space::Spaces;
use crate::ui::view::{Bar, BarListing, BarOpen, Shell};
use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::base::vocab::Dismiss;
use ds::components::overlays::popover::Arrow;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_store::SqliteStore;
use std::sync::Arc;

/// What the bar is called, to a screen reader and to a test.
pub(in crate::ui) const LABEL: &str = "Search mail and commands";

/// The field: a Mac toolbar search field, a capsule at the toolbar's Large size.
#[component]
pub(in crate::ui) fn SearchBar(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    revision: Signal<u64>,
    in_a_field: Signal<bool>,
    side_hidden: Signal<bool>,
    spaces: Signal<Spaces>,
) -> Element {
    let store = use_hook(consume_context::<Arc<SqliteStore>>);
    let drawn = use_signal(Drawn::default);
    let mut frame = use_signal(|| None::<MountedRef>);
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
        let world = World {
            shell: &shell.read(),
            spaces: &spaces.read(),
            store: &store,
        };
        panel_rows(&world, &drawn.read(), &open.listing, sidebar)
    });
    let text = field_text(&shell.read());
    let chips = match open.as_ref().map(|open| &open.listing) {
        Some(BarListing::Templates(_)) => Vec::new(),
        _ => tokens(&text),
    };
    let active = open.as_ref().map_or(0, |open| open.active);
    let keyed = rows.clone();
    let on_key = move |event: KeyboardEvent| {
        let plain = if event.modifiers().is_empty() {
            Plain::Yes
        } else {
            Plain::No
        };
        let Some(key) = bar_key(&event.key().to_string(), plain) else {
            return;
        };
        if key == BarKey::Down && shell.peek().bar == Bar::Closed {
            event.prevent_default();
            arrive(shell);
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
        let done = step(key, active, keyed.len(), typed);
        if done != Step::Pass {
            // The window's own keys never hear a key the bar took: Escape is not also "close
            // the reader", Return not also "open in a window".
            event.prevent_default();
            event.stop_propagation();
        }
        let ctx = Ctx {
            shell,
            pages,
            revision,
            side_hidden,
            spaces,
            drawn,
        };
        match done {
            Step::Move(to) => set_active(shell, to),
            Step::Run(at) => {
                if let Some(row) = keyed.get(at) {
                    run_row(ctx, &row.key);
                }
            }
            Step::Complete(at) => {
                if let Some(row) = keyed.get(at) {
                    set_text(shell, pages, completion(row));
                }
            }
            Step::Clear => set_text(shell, pages, String::new()),
            Step::Leave => leave(shell),
            Step::Pass => {}
        }
    };
    let ctx = Ctx {
        shell,
        pages,
        revision,
        side_hidden,
        spaces,
        drawn,
    };
    rsx! {
        div { class: "bar",
            onmounted: move |event| frame.set(Some(MountedRef(event.data()))),
            // A press in the field is the field's, not a press outside the panel: the layer
            // stack would close the panel and keep the press from placing the caret.
            onpointerdown: move |event: PointerEvent| event.stop_propagation(),
            TextField {
                kind: FieldKind::Search,
                size: ControlSize::Large,
                label: LABEL.to_owned(),
                placeholder: "Search".to_owned(),
                value: text,
                tokens: chips,
                oninput: move |value: String| set_text(shell, pages, value),
                onkey: on_key,
                // A click into the field brings no panel: as the Mac's search field, it comes
                // with the first key typed, ⌘K or Down. (A menu that opens during the click's
                // press takes the keyboard from the field on Blitz.)
                onfocus: move |()| in_a_field.set(true),
                onblur: move |()| in_a_field.set(false),
                common: classed("search"),
            }
        }
        if open.is_some() {
            Fetch { shell, drawn }
            if !rows.is_empty() {
                // The panel is quire's popover under the field holding quire's menu rows. Only
                // its owner closes it (`Dismiss::Manual`): a press in the field must reach the
                // field, so a press elsewhere in the window closes it ([`press_elsewhere`]).
                Popover {
                    anchor: anchor_at(frame()),
                    placement: Placement::new(Side::Bottom, Align::Start),
                    gap: Px(4.0),
                    arrow: Arrow::None,
                    dismiss: Dismiss::Manual,
                    onclose: move |()| shell.write().bar = Bar::Closed,
                    div { class: "bar-rows",
                        Menu::<String> {
                            placement: MenuPlacement::Popup,
                            anchor: anchor_at(frame()),
                            flow: Flow::Inline,
                            items: menu_items("", &rows, false),
                            onpick: move |key: String| run_row(ctx, &key),
                            onclose: move |()| {},
                            active: MenuCursor::Controlled(Some(active.min(rows.len() - 1))),
                            on_active: move |to: Option<usize>| {
                                if let Some(to) = to {
                                    set_active(shell, to);
                                }
                            },
                        }
                    }
                }
            }
        }
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

/// The field holds `text` now: typed, completed or cleared. The panel comes up (or stays) with
/// its first row highlighted.
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

/// Down in the field with no panel: the panel comes up over the search there is now. The
/// templates' listing, once chosen, stays until Escape.
fn arrive(mut shell: Signal<Shell>) {
    let listing = matches!(
        &shell.peek().bar,
        Bar::Open(BarOpen {
            listing: BarListing::Templates(_),
            ..
        })
    );
    if !listing {
        let search = shell.peek().search.clone();
        shell.write().bar = Bar::Open(BarOpen::over(search));
    }
}

/// Escape in an empty field: back to the search from the templates, else out of the field and
/// back to the list.
fn leave(mut shell: Signal<Shell>) {
    let mut write = shell.write();
    if let Bar::Open(open) = &mut write.bar
        && matches!(open.listing, BarListing::Templates(_))
    {
        open.listing = BarListing::Search;
        open.active = 0;
        return;
    }
    write.bar = Bar::Closed;
    drop(write);
    crate::ui::host::Host::focus_app();
}

/// A press anywhere in the window but the bar: the panel goes, and the press does what it does.
pub(in crate::ui) fn press_elsewhere(mut shell: Signal<Shell>) {
    if shell.peek().bar != Bar::Closed {
        shell.write().bar = Bar::Closed;
    }
}

/// ⌘K: the keyboard to the bar, its text selected so the first key typed replaces it, and the
/// panel up over the search there is.
pub(in crate::ui) fn summon(mut shell: Signal<Shell>) {
    let search = shell.peek().search.clone();
    shell.write().bar = Bar::Open(BarOpen::over(search));
    crate::ui::host::Host::focus_all(".search input");
}
