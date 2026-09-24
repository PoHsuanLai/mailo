use super::{Slide, space_key, switch};
use crate::space::{self, PRESETS, Scope, Space, Spaces};
use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Scripts, Work, chord, dispatching, rebuild_into, root_attr, work,
};
use crate::view::{Motion, Shell, Theme};
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{ElementId, VirtualDom};
use ds::{CardAccent, FrameVars, Scheme, SpaceLook};
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
        look: SpaceLook {
            dots: PRESETS[4].to_vec(),
            theme: Theme::Light,
            ..Space::default().look
        },
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

    let _ = chord(
        &mut dom,
        "2",
        Modifiers::CONTROL,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    let page = dioxus_ssr::render(&dom);

    // The frame is the root's own to paint now: no script, the Space's gradient on `.ds`.
    let gradient = ds::gradient(&ds::derive(&home.look.dots, Scheme::Light));
    let style = root_attr(&page, "style").unwrap_or_default();
    assert!(
        style.contains(&format!("--f-grad:{gradient};")),
        "the frame does not wear Solo's gradient: {style}"
    );
    assert!(
        scripts
            .all()
            .iter()
            .all(|script| !script.contains("--f-grad")),
        "a script still paints the frame: {:#?}",
        scripts.all()
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

/// `built`'s Work Space, then `extra`, written to its `spaces.json`, with Work current.
fn with_second(built: &Work, extra: Space) {
    let mut stored = space::load(&built.dirs.config);
    stored.spaces.push(extra);
    stored.current = 0;
    space::save(&built.dirs.config, &stored).unwrap_or_else(|e| panic!("{e}"));
}

/// `App` on `built`, first frame rendered.
fn app_on(built: &Work) -> VirtualDom {
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let _ = rebuild_into(&mut dom);
    dom
}

/// Ctrl+`digit`, as a person switches Space.
fn switch_to(dom: &mut VirtualDom, digit: &'static str) -> String {
    let _ = chord(
        dom,
        digit,
        Modifiers::CONTROL,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    dioxus_ssr::render(dom)
}

/// Each `div.ds-layer`'s `data-layer` (`None` for the front one) and `--f-grad`, in order.
fn layers(page: &str) -> Vec<(Option<String>, String)> {
    page.split("<div class=\"ds-layer\"")
        .skip(1)
        .map(|rest| {
            let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
            let attr = |name: &str| {
                let needle = format!(" {name}=\"");
                tag.find(&needle).map(|at| {
                    let value = &tag[at + needle.len()..];
                    value[..value.find('"').unwrap_or(value.len())].to_owned()
                })
            };
            let style = attr("style").unwrap_or_default();
            let gradient = style
                .strip_prefix("--f-grad:")
                .unwrap_or(&style)
                .trim_end_matches(';')
                .to_owned();
            (attr("data-layer"), gradient)
        })
        .collect()
}

/// What `paint.rs` guaranteed, carried to the `Ds` root: the first frame and a switch paint a
/// Space from the same values. There they were the same `setProperty` pairs in the head script
/// and the switch script; here they are `ds::FrameVars` of the Space in the resolved scheme,
/// whether the window opened on the Space or switched to it.
#[tokio::test]
async fn the_first_frame_and_a_switch_paint_a_space_the_same() {
    dispatching();
    let cases = [
        (
            "dark postmark",
            SpaceLook {
                dots: PRESETS[1].to_vec(),
                theme: Theme::Dark,
                card_accent: CardAccent::Postmark,
                grain: ds::Grain(70),
            },
            Motion::Standard,
            Scheme::Dark,
        ),
        (
            "light hint, calm",
            SpaceLook {
                dots: PRESETS[3].to_vec(),
                theme: Theme::Light,
                ..Space::default().look
            },
            Motion::Calm,
            Scheme::Light,
        ),
        (
            "system postmark",
            SpaceLook {
                // Pine, the retired accent preset, as a Space made from it keeps it.
                dots: vec![ds::Dot {
                    hue: 164.066_35,
                    chroma: 0.7,
                }],
                card_accent: CardAccent::Postmark,
                ..Space::default().look
            },
            Motion::Extra,
            // No desktop preference in a test: System is light.
            Scheme::Light,
        ),
    ];
    for (name, look, motion, scheme) in cases {
        let want = FrameVars::of(&look, scheme).style_attr();
        let other = Space {
            name: "Other".to_owned(),
            look: look.clone(),
            motion,
            ..Space::default()
        };

        let built = work();
        with_second(&built, other.clone());
        let mut dom = app_on(&built);
        let switched = switch_to(&mut dom, "2");

        let opened = work();
        let mut stored = space::load(&opened.dirs.config);
        stored.spaces.push(other);
        stored.current = 1;
        space::save(&opened.dirs.config, &stored).unwrap_or_else(|e| panic!("{e}"));
        let first = dioxus_ssr::render(&app_on(&opened));

        for (how, page) in [("switched to", &switched), ("opened on", &first)] {
            let style = root_attr(page, "style").unwrap_or_default();
            assert!(
                style.starts_with(&want),
                "{name}, {how}: {style}\nwant {want}"
            );
            assert_eq!(
                root_attr(page, "data-theme").as_deref(),
                Some(scheme.slug()),
                "{name}, {how}"
            );
            assert_eq!(
                root_attr(page, "data-motion").as_deref(),
                Some(ds::Motion::from(motion).slug()),
                "{name}, {how}"
            );
        }
    }
}

/// `a_crossfade_freezes_the_old_layer_and_fades_the_other_in`, carried to `Ds`: after a
/// switch the old gradient is still on a layer, now the hidden one the stylesheet fades out,
/// and the new one is on the front layer, which fades in. quire's own `FrameLayers` tests
/// cover the swap itself; this is the window doing it on a real switch.
#[tokio::test]
async fn a_switch_keeps_the_old_gradient_behind_the_new_one() {
    dispatching();
    let built = work();
    let work_look = space::load(&built.dirs.config).current_space().look;
    let home = Space {
        name: "Home".to_owned(),
        look: SpaceLook {
            dots: PRESETS[2].to_vec(),
            ..Space::default().look
        },
        ..Space::default()
    };
    with_second(&built, home.clone());
    let mut dom = app_on(&built);
    let old = ds::gradient(&ds::derive(&work_look.dots, Scheme::Light));
    let new = ds::gradient(&ds::derive(&home.look.dots, Scheme::Light));
    let before = layers(&dioxus_ssr::render(&dom));
    assert!(
        before.iter().any(|(slot, g)| slot.is_none() && *g == old),
        "Work is not on the front layer: {before:?}"
    );

    let after = layers(&switch_to(&mut dom, "2"));
    assert_eq!(after.len(), 2, "{after:?}");
    assert!(
        after.iter().any(|(slot, g)| slot.is_none() && *g == new),
        "Home is not on the front layer: {after:?}"
    );
    assert!(
        after
            .iter()
            .any(|(slot, g)| slot.as_deref() == Some("back") && *g == old),
        "Work's gradient is not held on the hidden layer to fade from: {after:?}"
    );
}

/// `postmark_writes_no_accent_and_clears_the_system_rule`, carried to `Ds`: a Space that lends
/// the card its hue writes `--accent` on the root, a Postmark Space writes none, and switching
/// from one to the other takes it away rather than leaving the last Space's behind.
#[tokio::test]
async fn postmark_writes_no_accent_and_a_switch_to_it_clears_the_hue() {
    dispatching();
    let built = work();
    let postmark = Space {
        name: "Plain".to_owned(),
        look: SpaceLook {
            card_accent: CardAccent::Postmark,
            ..Space::default().look
        },
        ..Space::default()
    };
    with_second(&built, postmark);
    let mut dom = app_on(&built);
    let hue = root_attr(&dioxus_ssr::render(&dom), "style").unwrap_or_default();
    assert!(hue.contains("--accent:"), "Work lends its hue: {hue}");
    let plain = root_attr(&switch_to(&mut dom, "2"), "style").unwrap_or_default();
    assert!(!plain.contains("--accent"), "Postmark kept a hue: {plain}");
}
