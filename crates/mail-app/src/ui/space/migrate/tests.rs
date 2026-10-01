use crate::ui::space::{load, save};
use ds::prelude::Theme;

#[test]
fn a_space_with_no_theme_takes_the_windows_on_first_read() {
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
            {"name":"Own","theme":"light","grain":80,"motion":"extra"},
            {"name":"Half","theme":"light"}
        ]}"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let read = load(dir.path());
    let got: Vec<(&str, Theme)> = read
        .spaces
        .iter()
        .map(|space| (space.name.as_str(), space.look.theme))
        .collect();
    assert_eq!(
        got,
        [
            ("Old", Theme::Dark),
            ("Own", Theme::Light),
            ("Half", Theme::Light),
        ]
    );

    // Once saved, the Spaces carry their own: changing the window-wide file moves nothing.
    save(dir.path(), &read).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("appearance.json"), "{}").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(load(dir.path()), read);
}
