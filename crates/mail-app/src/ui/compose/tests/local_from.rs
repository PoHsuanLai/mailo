//! What the From menu offers: every address an account sends as, its aliases too, and never
//! local folders, which send nothing.

use super::super::page::Float;
use super::render::window_with;

#[tokio::test]
async fn the_from_menu_does_not_offer_local_folders() {
    let mut offered = Vec::new();
    let (markup, _root) = window_with(|page, store| {
        mail_core::account::local(store, chrono::Utc::now())
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

/// An alias is somewhere to send from: the From menu offers it beside every account's own
/// address.
#[tokio::test]
async fn the_from_menu_offers_each_alias_beside_the_accounts_own_addresses() {
    const ALIAS: &str = "press@work.example";
    let mut own = Vec::new();
    let (markup, _root) = window_with(|page, store| {
        own = crate::ui::data::account_rows(store)
            .into_iter()
            .filter(|row| !row.is_local())
            .map(|row| (row.id, row.address))
            .collect();
        mail_core::identity::add_alias(
            store,
            own[0].0.clone(),
            &mail_core::identity::Typed {
                name: "The Press Desk".to_owned(),
                email: ALIAS.to_owned(),
                reply_to: String::new(),
            },
        )
        .unwrap_or_else(|why| panic!("the alias: {why}"));
        page.float = Float::From;
    });
    for address in own
        .iter()
        .map(|(_, address)| address.as_str())
        .chain([ALIAS])
    {
        assert!(
            markup.contains(&format!(">{address}<")),
            "{address} is not offered: {markup}"
        );
    }
}
