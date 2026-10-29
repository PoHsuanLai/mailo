use super::*;

/// The menu's rows as a person reads them.
fn read(rows: &[MenuItem<&'static dyn Command>]) -> Vec<(String, bool)> {
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
    use crate::ui::copy::menus::{HIDE_SIDEBAR, SETTINGS, SHOW_SIDEBAR, TIDY_UP};
    for (shown, toggle) in [
        (Shown::Visible, HIDE_SIDEBAR),
        (Shown::Hidden, SHOW_SIDEBAR),
    ] {
        assert_eq!(
            read(&menu_items(shown)),
            [
                (TIDY_UP.to_owned(), false),
                ("-".to_owned(), false),
                (SETTINGS.to_owned(), true),
                (toggle.to_owned(), true),
            ],
            "{toggle}"
        );
    }
    let picks: Vec<&str> = menu_items(Shown::Visible)
        .into_iter()
        .filter_map(|row| match row {
            MenuItem::Item { value, .. } => Some(value.key()),
            _ => None,
        })
        .collect();
    assert_eq!(picks, ["tidy-up", "settings", "sidebar"]);
}
