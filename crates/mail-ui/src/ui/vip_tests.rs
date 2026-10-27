//! VIPs in the real window: a sender marked from the reader's name menu wears the mark there and
//! in their rows, the VIPs place appears and lists their conversations, and a row's menu takes
//! the mark off again.

use super::app::App;
use super::fixtures::{
    Seen, click, dispatching, listed_subjects, pick_named, rebuild_into, right_click, settle,
    settle_quiet_seen, work,
};
use crate::ui::copy;
use dioxus::prelude::*;
use mail_core::Store as _;
use mail_domain::{Filter, PageReq, Property, Query, Sort, SortDir};

const DANA: &str = "dana@example.com";
const UIDL: &str = "Re: UIDL stability across a UIDVALIDITY change";

/// Render until the dom is quiet, gathering what it drew.
async fn quiet(dom: &mut VirtualDom, seen: Seen) -> Seen {
    seen.merge(settle_quiet_seen(dom).await)
}

#[tokio::test]
async fn a_sender_marked_vip_in_the_reader_is_marked_and_listed_in_the_vips_place() {
    dispatching();
    let built = work();
    let mut dom = VirtualDom::new(App)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    let seen = quiet(&mut dom, seen).await;
    let page = dioxus_ssr::render(&dom);
    assert!(!page.contains("data-vip"), "a mark before anyone is a VIP");
    assert!(
        !page.contains(r#"data-place="VIPs""#),
        "the VIPs place before anyone is one"
    );

    // Open Dana's conversation, and mark her from her name in the reader.
    let opened = click(&mut dom, seen.one("aria-label", &format!("Open {UIDL}")));
    let seen = quiet(&mut dom, seen.merge(opened)).await;
    let name = *seen
        .fixed("class", "sender-name")
        .last()
        .expect("the reader names the sender");
    let menu = right_click(&mut dom, name);
    let menu = settle(&mut dom, menu);
    let marked = pick_named(&mut dom, &menu, copy::menus::ADD_TO_VIPS).await;
    let seen = quiet(&mut dom, seen.merge(marked)).await;

    let vips = built.store.vips().unwrap();
    assert_eq!(
        vips.iter()
            .map(|vip| vip.address.as_str())
            .collect::<Vec<_>>(),
        [DANA],
        "the store holds her"
    );
    assert_eq!(vips[0].name.as_deref(), Some("Dana Okafor"));
    let page = dioxus_ssr::render(&dom);
    let reader = &page[page.find("sender-name").expect("the reader")..];
    assert!(
        reader[..reader.find("msg-when").unwrap_or(reader.len())].contains("data-vip"),
        "no mark beside her name in the reader"
    );
    let rows = page.matches(r#"data-vip="true""#).count();
    assert!(rows >= 2, "no mark in her rows: {rows}");

    // The VIPs place is drawn now, and lists her conversations and nobody else's.
    let place = seen.one("data-place", "VIPs");
    let listed = click(&mut dom, place);
    // What it drew is read from the page below.
    let _ = quiet(&mut dom, listed).await;
    let page = dioxus_ssr::render(&dom);
    let want: Vec<String> = built
        .store
        .threads(
            &Query {
                filter: Filter::FromAddress(DANA.to_owned()),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 50,
                },
            },
            chrono::Utc::now(),
        )
        .unwrap()
        .items
        .into_iter()
        .map(|summary| summary.subject)
        .collect();
    assert!(want.len() >= 2, "the fixture has two of Dana's: {want:?}");
    assert_eq!(listed_subjects(&page), want, "the VIPs place lists her");
}

#[tokio::test]
async fn a_row_s_menu_marks_its_sender_and_takes_the_mark_off() {
    dispatching();
    let built = work();
    let mut dom = VirtualDom::new(App)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    let mut seen = quiet(&mut dom, seen).await;
    for (pick, want) in [
        (copy::menus::ADD_TO_VIPS, vec![DANA.to_owned()]),
        (copy::menus::REMOVE_FROM_VIPS, Vec::new()),
    ] {
        let row = *seen
            .all("aria-label", &format!("Open {UIDL}"))
            .last()
            .expect("her row");
        let menu = right_click(&mut dom, row);
        let menu = settle(&mut dom, menu);
        let picked = pick_named(&mut dom, &menu, pick).await;
        seen = quiet(&mut dom, seen.merge(menu).merge(picked)).await;
        let held: Vec<String> = built
            .store
            .vips()
            .unwrap()
            .into_iter()
            .map(|vip| vip.address)
            .collect();
        assert_eq!(held, want, "{pick}");
    }
}

/// The person's own addresses are never marked: it is mail to be heard of from others.
#[test]
fn an_own_address_cannot_be_marked_a_vip() {
    let shell = crate::ui::view::Shell {
        accounts: vec![(
            "Me@Example.test".to_owned(),
            mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(1)),
        )],
        ..Default::default()
    };
    assert!(!crate::ui::vip::markable(&shell, "me@example.test"));
    assert!(crate::ui::vip::markable(&shell, "bob@example.test"));
}

/// With the last VIP removed while the VIPs place is chosen, the choice goes to the Inbox: the
/// sidebar no longer draws the VIPs place, and its list would be empty with nothing chosen in it.
#[test]
fn the_last_vip_removed_leaves_the_vips_place_for_the_inbox() {
    use crate::ui::view::{is_vips, own_places, selected_after};
    use mail_client::vip::Vips;

    let some = Vips::new(["dana@example.test"]);
    let was = own_places(&some);
    let at = was.iter().position(is_vips).expect("the VIPs place");
    let next = own_places(&Vips::default());
    let inbox = next
        .iter()
        .position(|place| place.name == "Inbox")
        .expect("the Inbox");
    assert_eq!(selected_after(&was, at, &next, &Vips::default()), inbox);
    // While somebody is a VIP the place stays chosen, and another place stays as it was.
    assert_eq!(selected_after(&was, at, &own_places(&some), &some), at);
    assert_eq!(selected_after(&was, 1, &next, &Vips::default()), 1);
}
