use super::*;

/// The menu's rows as a person reads them.
fn read(rows: &[MenuItem<Pick>]) -> Vec<(String, bool)> {
    rows.iter()
        .map(|row| match row {
            MenuItem::Item { title, key, .. } => (title.clone(), key.is_some()),
            MenuItem::Separator => ("-".to_owned(), false),
            other => panic!("not an item: {other:?}"),
        })
        .collect()
}

/// The foot's menu holds Tidy Up…, then Settings and the sidebar's toggle with their keys, and
/// nothing else: what was opened lately is ⌘K's Recent, History is in ⌘K, New Space is in the
/// Space's menu.
#[test]
fn the_menu_holds_exactly_tidy_up_settings_and_the_sidebar_toggle() {
    assert_eq!(
        read(&menu_items(Shown::Visible)),
        [
            ("Tidy Up\u{2026}".to_owned(), false),
            ("-".to_owned(), false),
            ("Settings\u{2026}".to_owned(), true),
            ("Hide Sidebar".to_owned(), true),
        ]
    );
    assert_eq!(
        read(&menu_items(Shown::Hidden)),
        [
            ("Tidy Up\u{2026}".to_owned(), false),
            ("-".to_owned(), false),
            ("Settings\u{2026}".to_owned(), true),
            ("Show Sidebar".to_owned(), true),
        ]
    );
    let picks: Vec<Pick> = menu_items(Shown::Visible)
        .into_iter()
        .filter_map(|row| match row {
            MenuItem::Item { value, .. } => Some(value),
            _ => None,
        })
        .collect();
    assert_eq!(picks, [Pick::TidyUp, Pick::Settings, Pick::ToggleSidebar]);
}
