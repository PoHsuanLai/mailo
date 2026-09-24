//! Local folders are not somewhere to send from: the From menu leaves them out.

use super::super::page::Float;
use super::render::window_with;

#[tokio::test]
async fn the_from_menu_does_not_offer_local_folders() {
    let mut offered = Vec::new();
    let (markup, _root) = window_with(|page, store| {
        crate::account::local(store, chrono::Utc::now())
            .unwrap_or_else(|why| panic!("local folders: {why}"));
        offered = crate::ui::data::account_rows(store)
            .into_iter()
            .map(|row| (row.is_local(), row.address))
            .map(|(local, address)| (address, local))
            .collect();
        page.float = Float::From;
    });
    assert!(
        markup.contains("Send from"),
        "the menu did not open: {markup}"
    );
    assert!(offered.iter().any(|(_, local)| *local), "no local account");
    for (address, local) in offered {
        let listed = markup.contains(&format!(">{address}<"));
        assert_eq!(listed, !local, "{address} listed: {listed}");
    }
}
