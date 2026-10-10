use super::{recall_of, restore};
use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Scripts, Work, chord, dispatching, rebuild_into, root_attr, work,
};
use crate::ui::space::{self, Mail, Scope, SpaceId};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use dioxus_core::{ElementId, VirtualDom};
use ds::prelude::{Scheme, SpaceLook, Theme, Word};
use ds::style::space::frame_vars::FrameVars;
use ds::style::space::look::CardAccent;
use ds::style::space::palette::{Dot, derive, gradient};
use ds::style::space::presets::PRESETS;
use mail_domain::ThreadId;
use mail_domain::id::account_id_from_uuid;
use porter_core::AccountId;
use uuid::Uuid;

fn account(n: u128) -> AccountId {
    account_id_from_uuid(Uuid::from_u128(n))
}

fn thread(n: u128) -> ThreadId {
    ThreadId::from_uuid(Uuid::from_u128(n))
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

/// The neutral look with `dots`.
fn dotted(dots: &[Dot]) -> SpaceLook {
    SpaceLook {
        dots: dots.to_vec(),
        ..SpaceLook::default()
    }
}

/// quire's kit hands back what [`recall_of`] took when the Space was left; [`restore`] shows it
/// again, and what belonged to the Space left (a search, a selection) does not follow.
#[test]
fn a_space_gets_its_place_thread_and_tile_back_and_nothing_else() {
    let mut shell = Shell::default();
    select(&mut shell, "Archive");
    shell.open(thread(7));
    shell.account = Some(account(1));
    let left = recall_of(&shell);

    let mut elsewhere = Shell::default();
    select(&mut elsewhere, "Sent");
    elsewhere.open(thread(9));
    elsewhere.search = "from:dana".to_owned();
    restore(&mut elsewhere, &left);
    assert_eq!(place(&elsewhere), "Archive");
    assert_eq!(elsewhere.open, Some(thread(7)));
    assert_eq!(elsewhere.account, Some(account(1)));
    assert!(
        elsewhere.search.is_empty(),
        "the search followed into the Space"
    );

    // A Space never left opens on the first place, with nothing open and every account.
    restore(&mut elsewhere, &space::Recall::default());
    assert_eq!(place(&elsewhere), "Inbox");
    assert_eq!(elsewhere.open, None);
    assert_eq!(elsewhere.account, None);
}

fn rows(page: &str) -> usize {
    page.matches("aria-label=\"Open ").count()
}

/// Render `dom` as work lands until `done` holds of the page, for at most quire's settle bound
/// of wall clock; the page `done` last saw.
async fn drawn_when(dom: &mut VirtualDom, done: impl Fn(&str) -> bool) -> String {
    let until = tokio::time::Instant::now() + ds_harness::harness::SETTLE_BOUND;
    loop {
        dom.render_immediate(&mut dioxus_core::NoOpMutations);
        let page = dioxus_ssr::render(dom);
        let now = tokio::time::Instant::now();
        if done(&page) || now >= until {
            return page;
        }
        let _ = tokio::time::timeout(until - now, dom.wait_for_work()).await;
    }
}

#[tokio::test]
async fn ctrl_2_repaints_the_frame_and_scopes_the_list() {
    dispatching();
    let built = work();
    let ids = crate::ui::data::accounts(&built.store);
    let home = SpaceLook {
        theme: Theme::Light,
        ..dotted(PRESETS[4].dots)
    };
    let solo = with_second(
        &built,
        "Solo",
        home.clone(),
        Mail::over(Scope::Accounts(vec![ids[0].clone()])),
    );

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
        crate::ui::fixtures::PRIMARY,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    // The list keeps what it drew until the new Space's query, on its blocking thread, lands:
    // wait for that rather than reading the page once, which raced the thread.
    let page = drawn_when(&mut dom, |page| rows(page) < before).await;

    // The frame is the root's own to paint now: no script, the Space's gradient on `.ds`.
    let gradient = gradient(&derive(&home.dots, Scheme::Light));
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
        page.contains("aria-label=\"Solo Space\"") && page.contains("The Solo Space"),
        "the foot does not show Solo: {page}"
    );
    assert_eq!(
        space::load(&built.dirs).current().id,
        solo,
        "the switch was not written to spaces.json"
    );
}

/// `built`'s Work Space, then one called `name` with `look` over `mail`, written to its
/// `spaces.json`, with Work current. The new Space's id is returned.
fn with_second(built: &Work, name: &str, look: SpaceLook, mail: Mail) -> SpaceId {
    let mut stored = space::load(&built.dirs);
    let first = stored.current().id;
    let made = stored.add(mail);
    stored.edit(made, |space| {
        space.name = name.to_owned();
        space.look = look;
    });
    stored.select(first);
    space::save(Some(&built.dirs), &stored).unwrap_or_else(|e| panic!("{e}"));
    made
}

/// `App` on `built`, first frame rendered.
fn app_on(built: &Work) -> VirtualDom {
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let _ = rebuild_into(&mut dom);
    dom
}

/// ⌘`digit`, as a person switches Space.
fn switch_to(dom: &mut VirtualDom, digit: &'static str) -> String {
    let _ = chord(
        dom,
        digit,
        crate::ui::fixtures::PRIMARY,
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
            "dark, the card follows the Space",
            SpaceLook {
                grain: ds::prelude::Grain(35),
                dots: PRESETS[1].dots.to_vec(),
                theme: Theme::Dark,
                card_accent: CardAccent::SpaceHue,
            },
            Scheme::Dark,
        ),
        (
            "light, the chosen accent",
            SpaceLook {
                theme: Theme::Light,
                ..dotted(PRESETS[3].dots)
            },
            Scheme::Light,
        ),
        (
            "system, a hand-made dot",
            SpaceLook {
                grain: ds::prelude::Grain(35),
                ..dotted(&[Dot {
                    hue: 164.066_35,
                    chroma: 0.7,
                }])
            },
            // No desktop preference in a test: System is light.
            Scheme::Light,
        ),
    ];
    for (name, look, scheme) in cases {
        let want = FrameVars::of(&look, scheme).style_attr();
        let built = work();
        with_second(&built, "Other", look.clone(), Mail::default());
        let mut dom = app_on(&built);
        let switched = switch_to(&mut dom, "2");

        let opened = work();
        let other = with_second(&opened, "Other", look.clone(), Mail::default());
        let mut stored = space::load(&opened.dirs);
        stored.select(other);
        space::save(Some(&opened.dirs), &stored).unwrap_or_else(|e| panic!("{e}"));
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
    let work_look = space::load(&built.dirs).current().look.clone();
    let home = dotted(PRESETS[2].dots);
    with_second(&built, "Home", home.clone(), Mail::default());
    let mut dom = app_on(&built);
    let old = gradient(&derive(&work_look.dots, Scheme::Light));
    let new = gradient(&derive(&home.dots, Scheme::Light));
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

/// A Space whose card follows its hue writes `--accent` on the root, a Space that keeps the
/// chosen accent writes none, and switching from one to the other takes it away rather than
/// leaving the last Space's behind.
#[tokio::test]
async fn the_chosen_accent_writes_none_and_a_switch_to_it_clears_the_hue() {
    dispatching();
    let built = work();
    let mut stored = space::load(&built.dirs);
    let first = stored.current().id;
    stored.edit(first, |space| space.look.card_accent = CardAccent::SpaceHue);
    space::save(Some(&built.dirs), &stored).unwrap_or_else(|e| panic!("{e}"));
    let chosen = SpaceLook {
        card_accent: CardAccent::Chosen,
        ..SpaceLook::default()
    };
    with_second(&built, "Plain", chosen, Mail::default());
    let mut dom = app_on(&built);
    let hue = root_attr(&dioxus_ssr::render(&dom), "style").unwrap_or_default();
    assert!(hue.contains("--accent:"), "Work lends its hue: {hue}");
    let plain = root_attr(&switch_to(&mut dom, "2"), "style").unwrap_or_default();
    assert!(
        !plain.contains("--accent"),
        "the chosen accent kept a hue: {plain}"
    );
}
