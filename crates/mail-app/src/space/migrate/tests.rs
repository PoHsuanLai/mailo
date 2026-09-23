use crate::space::{load, save};
use crate::view::{Appearance, Motion, Theme};

#[test]
fn a_space_with_no_theme_or_motion_takes_the_windows_on_first_read() {
    // The window-wide values are not the defaults, so a loader that ignored appearance.json
    // would fail every row; and the Space that chose its own must keep them, so a loader
    // that overwrote every Space fails too.
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(
        dir.path().join("appearance.json"),
        r#"{"theme":"dark","accent":"pine","motion":"calm"}"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(
        dir.path().join("spaces.json"),
        r#"{"spaces":[
            {"name":"Old"},
            {"name":"Own","theme":"light","motion":"extra"},
            {"name":"Half","theme":"light"}
        ]}"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let read = load(dir.path());
    let got: Vec<(&str, Theme, Motion)> = read
        .spaces
        .iter()
        .map(|space| (space.name.as_str(), space.theme, space.motion))
        .collect();
    assert_eq!(
        got,
        [
            ("Old", Theme::Dark, Motion::Calm),
            ("Own", Theme::Light, Motion::Extra),
            ("Half", Theme::Light, Motion::Calm),
        ]
    );

    // Once saved, the Spaces carry their own: changing the window-wide file moves nothing.
    save(dir.path(), &read).unwrap_or_else(|e| panic!("{e}"));
    crate::appearance::save(dir.path(), Appearance::default()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(load(dir.path()), read);
}
