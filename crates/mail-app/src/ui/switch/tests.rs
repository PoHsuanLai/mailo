use super::{Slide, space_key, switch};
use crate::palette;
use crate::space::{self, PRESETS, Scope, Space, Spaces};
use crate::ui::app::App;
use crate::ui::fixtures::{INSIDE_THE_SHELL, Scripts, chord, dispatching, rebuild_into, work};
use crate::view::{Shell, Theme};
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{ElementId, VirtualDom};
use mail_domain::{AccountId, ThreadId};
use std::collections::BTreeMap;
use uuid::Uuid;

fn account(n: u128) -> AccountId {
    AccountId::from_uuid(Uuid::from_u128(n))
}

fn thread(n: u128) -> ThreadId {
    ThreadId::from_uuid(Uuid::from_u128(n))
}

fn two_spaces() -> Spaces {
    Spaces {
        spaces: vec![
            Space {
                name: "Work".to_owned(),
                scope: Scope::All,
                ..Space::default()
            },
            Space {
                name: "Home".to_owned(),
                scope: Scope::Accounts(vec![account(2)]),
                ..Space::default()
            },
        ],
        current: 0,
        recall: BTreeMap::new(),
    }
}

fn place(shell: &Shell) -> &str {
    &shell.places[shell.selected].name
}

fn select(shell: &mut Shell, name: &str) {
    let index = shell
        .places
        .iter()
        .position(|place| place.name == name)
        .unwrap_or_else(|| panic!("no place {name}"));
    shell.select(index);
}

#[test]
fn ctrl_and_a_digit_name_a_space() {
    const CASES: &[(&str, Option<usize>)] = &[
        ("1", Some(0)),
        ("2", Some(1)),
        ("9", Some(8)),
        ("0", None),
        ("10", None),
        ("a", None),
        ("", None),
    ];
    for &(key, want) in CASES {
        assert_eq!(space_key(key), want, "{key:?}");
    }
}

#[test]
fn each_space_gets_its_place_thread_and_tile_back() {
    let mut spaces = two_spaces();
    let mut shell = Shell::default();
    select(&mut shell, "Archive");
    shell.open(thread(7));
    shell.account = Some(account(1));
    shell.search = "from:dana".to_owned();

    assert_eq!(switch(&mut spaces, &mut shell, 1), Some(Slide::Right));
    assert_eq!(spaces.current, 1);
    assert_eq!(shell.scope, vec![account(2)], "the list is not Home's");
    assert_eq!(
        place(&shell),
        "Inbox",
        "a Space with no memory opens on the Inbox"
    );
    assert_eq!(shell.open, None);
    assert_eq!(
        shell.account, None,
        "Work's tile is not one of Home's accounts"
    );
    assert!(shell.search.is_empty(), "Work's search followed into Home");

    select(&mut shell, "Sent");
    shell.open(thread(9));
    shell.account = Some(account(2));

    assert_eq!(switch(&mut spaces, &mut shell, 0), Some(Slide::Left));
    assert_eq!(
        shell.scope,
        Vec::<AccountId>::new(),
        "Work is every account"
    );
    assert_eq!(place(&shell), "Archive");
    assert_eq!(shell.open, Some(thread(7)));
    assert_eq!(shell.account, Some(account(1)));

    assert_eq!(switch(&mut spaces, &mut shell, 1), Some(Slide::Right));
    assert_eq!(place(&shell), "Sent");
    assert_eq!(shell.open, Some(thread(9)));
    assert_eq!(shell.account, Some(account(2)));
}

#[test]
fn switching_to_where_you_are_or_nowhere_changes_nothing() {
    let mut spaces = two_spaces();
    let mut shell = Shell::default();
    select(&mut shell, "Archive");
    let (before_spaces, before_shell) = (spaces.clone(), shell.clone());
    assert_eq!(switch(&mut spaces, &mut shell, 0), None);
    assert_eq!(switch(&mut spaces, &mut shell, 2), None);
    assert_eq!(spaces, before_spaces);
    assert_eq!(shell, before_shell);
}

fn rows(page: &str) -> usize {
    page.matches("aria-label=\"Open ").count()
}

#[tokio::test]
async fn ctrl_2_repaints_the_frame_and_scopes_the_list() {
    dispatching();
    let built = work();
    let ids = crate::ui::data::accounts(&built.store);
    let mut stored = space::load(&built.dirs.config);
    let home = Space {
        name: "Solo".to_owned(),
        dots: PRESETS[4].to_vec(),
        theme: Theme::Light,
        scope: Scope::Accounts(vec![ids[0]]),
        ..Space::default()
    };
    stored.spaces.push(home.clone());
    space::save(&built.dirs.config, &stored).unwrap_or_else(|e| panic!("{e}"));

    let scripts = Scripts::default();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone())
        .with_root_context(scripts.document());
    let _ = rebuild_into(&mut dom);
    let before = rows(&dioxus_ssr::render(&dom));
    let evals_before = scripts.all().len();

    let _ = chord(
        &mut dom,
        "2",
        Modifiers::CONTROL,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    let page = dioxus_ssr::render(&dom);

    let gradient = palette::gradient(&palette::derive(&home.dots, false));
    let quoted = serde_json::to_string(&gradient).unwrap_or_default();
    let needle = format!("setProperty(\"--f-grad\", {quoted});");
    let new_scripts = &scripts.all()[evals_before..];
    assert!(
        new_scripts.iter().any(|script| script.contains(&needle)),
        "no script set Solo's gradient; ran {new_scripts:#?}"
    );
    let after = rows(&page);
    assert!(
        after < before && after > 0,
        "the list did not narrow to Solo's account: {before} rows before, {after} after"
    );
    assert!(
        page.contains("aria-label=\"Solo Space\"") && page.contains("Edit the Solo Space"),
        "the foot does not show Solo: {page}"
    );
    assert_eq!(
        space::load(&built.dirs.config).current,
        1,
        "the switch was not written to spaces.json"
    );
}
