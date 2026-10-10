//! The search panel drawn alone for a test: open over a text, with the signals a test reads
//! back, and beside a label menu for the stylesheet test.

use super::*;
use crate::ui::view::Shell;
use chrono::Utc;
use ds::prelude::*;
use mail_domain::Filter;
use std::cell::Cell;

thread_local! {
    static HELD: Cell<Option<Held>> = const { Cell::new(None) };
}

/// The signals a test reads back from [`BarAlone`].
#[derive(Clone, Copy)]
pub(in crate::ui) struct Held {
    pub shell: Signal<Shell>,
    pub side_hidden: Signal<bool>,
}

/// What the last [`BarAlone`] drawn holds.
pub(in crate::ui) fn held() -> Held {
    HELD.with(Cell::get)
        .unwrap_or_else(|| panic!("no search panel was drawn"))
}

/// The search panel alone, up over `typed`, inside a quire root as the window has it: the panel
/// floats in the root's overlay.
#[component]
pub(in crate::ui) fn BarAlone(typed: String) -> Element {
    // The window's keymap with mailo's actions in it, as `App` makes it: ⌘K is one of them.
    let _keys = crate::ui::actions::use_registered();
    let shell = use_signal(|| Shell {
        search: typed.clone(),
        bar: Bar::Open(crate::ui::view::BarOpen::over(String::new())),
        ..Shell::default()
    });
    let side_hidden = use_signal(|| false);
    HELD.with(|slot| slot.set(Some(Held { shell, side_hidden })));
    let pages = use_signal(|| 1u32);
    let revision = use_signal(|| 0u64);
    let in_a_field = use_signal(|| false);
    let spaces = use_signal(|| crate::ui::space::first_run(&[]));
    rsx! {
        Ds {
            appearance: Appearance::default(),
            material: Material::Window,
            stylesheet: ds::assembly::ds::Inject::Host,
            Spotlight { shell, pages, revision, in_a_field, side_hidden, spaces }
        }
    }
}

/// The search panel open on `dana`, and a label menu, so the stylesheet test sees those
/// classes.
#[component]
pub(in crate::ui) fn OpenMenus() -> Element {
    let store = use_hook(consume_context::<Arc<SqliteStore>>);
    let known = mail_core::query::known_labels(&store);
    let shell = use_signal(|| Shell {
        labels: known,
        ..Shell::default()
    });
    let revision = use_signal(|| 0u64);
    let summary = mail_core::search::Source::listed(
        store.as_ref(),
        &Filter::All,
        mail_core::search::first(1),
        Utc::now(),
    )
    .into_iter()
    .next();
    rsx! {
        BarAlone { typed: "dana".to_owned() }
        Ds {
            appearance: Appearance::default(),
            material: Material::Window,
            stylesheet: ds::assembly::ds::Inject::Host,
            // quire's menu, floating in the root's overlay.
            if let Some(summary) = summary {
                {
                    let id = summary.id;
                    rsx! { super::super::menus::LabelMenu { id, summary, shell, revision, anchor: None } }
                }
            }
        }
    }
}
