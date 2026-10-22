//! Lane 13: Spaces put in order in Settings › Spaces by dragging a row's grip, as a person does:
//! pressed on the grip, moved down past the other rows, released. The row lifts and follows the
//! pointer, the rows it passes make room, and the order on disk is the new one. A press on the row
//! itself still opens the Space's page; a Space alone has nothing to be dragged past.

use ds::prelude::{Point, Px};
use ds_harness::{Harness, Input};
use mail_app::ui::appearance::WindowDirs;
use mail_app::ui::space;

use super::hands::{centre, click, until};
use super::window::{Window, ms};

const NAMES: [&str; 3] = ["Home", "Work", "Travel"];

/// The `n`th row (from 1) of the Spaces' own list, the one that can be reordered.
fn row(n: usize) -> String {
    format!("[*|data-reorder=\"grip\"] > .ds-list-item:nth-child({n})")
}

fn names(dirs: &WindowDirs) -> Vec<String> {
    space::stored(dirs)
        .expect("the Spaces were written")
        .list()
        .iter()
        .map(|one| one.name.clone())
        .collect()
}

/// Press `from`, move to `to` a frame at a time, and release there; `during` looks at the
/// window while the row is held over `to`.
fn drag(settings: &mut Harness, from: Point, to: Point, during: impl FnOnce(&mut Harness)) {
    settings.send(Input::pointer_move(from));
    settings.send(Input::pointer_down(from));
    settings.advance(ms(50));
    for step in 1..=10 {
        let part = step as f32 / 10.0;
        settings.send(Input::pointer_move(Point {
            x: Px(from.x.0 + (to.x.0 - from.x.0) * part),
            y: Px(from.y.0 + (to.y.0 - from.y.0) * part),
        }));
        settings.advance(ms(16));
    }
    during(settings);
    settings.send(Input::pointer_up(to));
    settings.advance(ms(100));
}

#[test]
fn a_space_dragged_by_its_grip_takes_its_new_place() {
    let window = Window::open(|_| {});
    let state = window.state();
    let dirs = WindowDirs {
        config: state
            .parent()
            .expect("the window's directory")
            .join("config"),
        state,
    };
    let three = space::built(
        NAMES
            .iter()
            .map(|name| {
                let look = ds::prelude::SpaceLook::default();
                ((*name).to_owned(), look, space::Mail::default())
            })
            .collect(),
        0,
    );
    space::save(Some(&dirs), &three).expect("the Spaces were written");
    let mut window = window;

    window.in_settings("Spaces", |settings| {
        until(settings, "each Space's row has its grip", |h| {
            h.count(".ds-list-grip") == 3
        });
        assert!(
            settings
                .text_of(&format!("{} .ds-row", row(1)))
                .is_some_and(|text| text.contains("Home")),
            "the first row is not Home"
        );
        // Home's grip, dropped on Travel's row: Home goes last.
        let from = centre(settings, &format!("{} .ds-list-grip", row(1)));
        let to = centre(settings, &row(3));
        drag(settings, from, to, |held| {
            assert_eq!(
                held.count("[*|data-drag=\"lifted\"]"),
                1,
                "the row did not lift"
            );
            assert_eq!(
                held.count("[*|data-drag=\"moved\"]"),
                2,
                "the rows it passed did not make room"
            );
        });
        until(settings, "the list is redrawn in its new order", |h| {
            h.text_of(&format!("{} .ds-row", row(3)))
                .is_some_and(|text| text.contains("Home"))
                && h.count("[*|data-drag]") == 0
        });
        // The row itself is still a press that opens the Space's page.
        click(settings, &format!("{} .ds-row", row(1)));
        until(settings, "Work's page opens", |h| {
            h.count("[*|value=\"Work\"]") == 1
        });
    });
    assert_eq!(names(&dirs), ["Work", "Travel", "Home"]);
}

#[test]
fn a_lone_space_has_no_grip() {
    let mut window = Window::open(|_| {});
    window.in_settings("Spaces", |settings| {
        until(settings, "the Spaces pane is drawn", |h| {
            h.count("[*|data-page=\"Spaces\"] .ds-row") >= 2
        });
        assert_eq!(
            settings.count(".ds-list-grip"),
            0,
            "a lone Space has a grip"
        );
    });
}
