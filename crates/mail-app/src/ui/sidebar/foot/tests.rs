use super::*;

/// The menu's rows as a person reads them.
fn read(rows: &[MenuItem<Pick>]) -> Vec<(String, bool)> {
    rows.iter()
        .map(|row| match row {
            MenuItem::Item { title, key, .. } => (title.clone(), key.is_some()),
            other => panic!("not an item: {other:?}"),
        })
        .collect()
}

/// The foot's menu holds Settings and the sidebar's toggle and nothing else, each with its key.
/// Today, the drafts put aside and the messages waiting head the sidebar; History is in ⌘K;
/// New Space is in the Space's menu.
#[test]
fn the_menu_holds_exactly_settings_and_the_sidebar_toggle() {
    assert_eq!(
        read(&menu_items(Shown::Visible)),
        [
            ("Settings\u{2026}".to_owned(), true),
            ("Hide Sidebar".to_owned(), true),
        ]
    );
    assert_eq!(
        read(&menu_items(Shown::Hidden)),
        [
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
    assert_eq!(picks, [Pick::Settings, Pick::ToggleSidebar]);
}
