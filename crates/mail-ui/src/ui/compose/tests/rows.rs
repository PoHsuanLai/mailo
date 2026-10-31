//! The header rows say only what is not the default (docs/design-language.md §2.1): a new message
//! is Subject, To, the body and the foot; every other row is drawn while its choice is set and goes
//! when it is set back, and every choice is one step away under Send ▾ or ⋯. Rows are found by
//! their `data-row`, controls by their accessible names.

use mail_core::Store;

use super::super::foot::{More, more_items, pick_more, pick_send, send_items};
use super::super::page::{Float, List};
use super::super::props::sends_items;
use super::super::protection::{self, Protection};
use super::super::receipt;
use super::super::remind::{self, Remind};
use super::*;
use crate::ui::fixtures::{acct_account, seeded};
use ds::components::menus::item::item::MenuItem as Line;
use mail_core::SqliteStore;

/// Every row a choice or a list can draw, by its `data-row`.
const OPTIONAL: [&str; 8] = [
    "from", "cc", "bcc", "sends", "remind", "security", "receipt", "attached",
];

fn fresh_draft(store: &SqliteStore) -> Draft {
    mail_core::compose::draft_new(store, acct_account(), &[], "", "", Utc::now())
        .unwrap_or_else(|why| panic!("a new draft: {why}"))
}

fn row(name: &str) -> String {
    format!("data-row=\"{name}\"")
}

/// The names of a menu's top lines: a command's title, a submenu's, `—` for a rule.
fn titles(items: &[Line<String>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Line::Item { title, .. } | Line::Submenu { title, .. } => title.clone(),
            Line::Separator => "—".to_owned(),
            Line::Header(title) => format!("# {title}"),
            Line::Info { title, .. } => format!("i {title}"),
            Line::Swatches(row) => format!("o {}", row.title),
        })
        .collect()
}

#[tokio::test]
async fn a_new_message_draws_subject_to_the_body_and_the_foot_and_nothing_else() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let (mut window, _seen) = Window::open(crate::ui::fixtures::client(&store), draft, None);
    let markup = window.render();
    assert!(markup.contains("c-title"), "no subject:\n{markup}");
    assert!(markup.contains(&row("to")), "no To row:\n{markup}");
    for name in OPTIONAL {
        assert!(
            !markup.contains(&row(name)),
            "a new message draws {name}:\n{markup}"
        );
    }
    // No status, no second home for keeping or deleting the draft, no emoji or plain-text button,
    // and Remind is nowhere but Send ▾, which is shut.
    for gone in [
        "Draft · ",
        r#"aria-label="Discard""#,
        r#"aria-label="Keep for later""#,
        r#"aria-label="Emoji""#,
        "Plain text",
        "Remind",
    ] {
        assert!(!markup.contains(gone), "{gone} is drawn:\n{markup}");
    }
    // The foot: Attach, ⋯, Send ▾, and one filled button.
    for name in [
        crate::ui::copy::controls::ATTACH,
        crate::ui::copy::aria::MORE,
        crate::ui::copy::controls::SEND,
        crate::ui::copy::aria::SEND_OPTIONS,
        "Add Cc",
        "Add Bcc",
    ] {
        assert!(
            markup.contains(&format!("aria-label=\"{name}\"")),
            "no {name}:\n{markup}"
        );
    }
    assert_eq!(
        markup.matches(r#"data-answers="return""#).count(),
        1,
        "one filled button:\n{markup}"
    );
    // Labels have no colons.
    assert!(!markup.contains("To:"), "{markup}");
}

/// One choice: how to set it, how to set it back, and the row it draws while set.
struct Choice {
    row: &'static str,
    set: fn(&mut Page),
    reset: fn(&mut Page),
}

#[tokio::test]
async fn setting_a_choice_draws_its_row_and_setting_it_back_removes_it() {
    let choices = [
        Choice {
            row: "sends",
            set: |page| pick_send(page, "later:tomorrow"),
            reset: |page| super::super::props::pick_sends(page, "now"),
        },
        Choice {
            row: "remind",
            set: |page| pick_send(page, "remind:three-days"),
            reset: |page| remind::pick_remind(page, remind::OFF_KEY),
        },
        Choice {
            row: "security",
            set: |page| {
                assert_eq!(pick_more(page, "security:smime-sign"), More::Done);
            },
            reset: |page| protection::pick(page, "none"),
        },
        Choice {
            row: "receipt",
            set: |page| pick_send(page, receipt::KEY),
            reset: |page| receipt::pick_row(page, receipt::OFF_KEY),
        },
    ];
    for choice in choices {
        let (store, _dir) = seeded();
        let (mut window, _seen) = Window::open(
            crate::ui::fixtures::client(&store.clone()),
            fresh_draft(&store),
            None,
        );
        let mut page = window.page();
        assert!(
            !window.render().contains(&row(choice.row)),
            "{}",
            choice.row
        );
        window.dom.in_runtime(|| (choice.set)(&mut page.write()));
        let markup = window.render();
        assert!(
            markup.contains(&row(choice.row)),
            "{} is not drawn:\n{markup}",
            choice.row
        );
        // Every other optional row stays away.
        for other in OPTIONAL.iter().filter(|other| **other != choice.row) {
            assert!(!markup.contains(&row(other)), "{} drew {other}", choice.row);
        }
        window.dom.in_runtime(|| (choice.reset)(&mut page.write()));
        let markup = window.render();
        assert!(
            !markup.contains(&row(choice.row)),
            "{} stayed:\n{markup}",
            choice.row
        );
    }
}

#[tokio::test]
async fn cc_and_bcc_are_drawn_once_added_or_while_someone_is_on_them() {
    for (list, name) in [(List::Cc, "cc"), (List::Bcc, "bcc")] {
        let (store, _dir) = seeded();
        let (mut window, _seen) = Window::open(
            crate::ui::fixtures::client(&store.clone()),
            fresh_draft(&store),
            None,
        );
        let mut page = window.page();
        window.dom.in_runtime(|| page.write().reveal(list));
        let markup = window.render();
        assert!(markup.contains(&row(name)), "{name} once added:\n{markup}");
        let word = super::super::props::list_label(list);
        assert!(
            !markup.contains(&format!("aria-label=\"Add {word}\"")),
            "Add {word} is still offered:\n{markup}"
        );

        // A draft that already has someone on it opens with the row.
        let mut draft = fresh_draft(&store);
        let someone = Address {
            name: None,
            email: "sam@example.test".to_owned(),
        };
        match list {
            List::Cc => draft.cc = vec![someone],
            _ => draft.bcc = vec![someone],
        }
        mail_core::compose::save(&store, &draft).unwrap_or_else(|why| panic!("{why}"));
        let (mut window, _seen) =
            Window::open(crate::ui::fixtures::client(&store.clone()), draft, None);
        assert!(
            window.render().contains(&row(name)),
            "{name} while someone is on it"
        );
    }
}

#[tokio::test]
async fn bcc_is_written_to_the_draft() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let (window, _seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        write.typed_bcc = "hidden@example.test".to_owned();
        assert!(super::super::recipients::commit_typed(
            &mut write,
            List::Bcc
        ));
        super::super::life::save(&crate::ui::fixtures::client(&store), &mut write, Utc::now())
            .unwrap_or_else(|why| panic!("{why}"));
    });
    let stored = store.draft(draft.id).unwrap_or_else(|why| panic!("{why}"));
    let bcc: Vec<&str> = stored.bcc.iter().map(|one| one.email.as_str()).collect();
    assert_eq!(bcc, ["hidden@example.test"]);
    assert!(stored.to.is_empty() && stored.cc.is_empty());
}

#[tokio::test]
async fn from_is_drawn_only_with_more_than_one_address_to_send_from() {
    // The Work Space sends from several; the seeded store from one (the test above).
    let (markup, _root) = super::render::window_with(|_, _| {});
    assert!(markup.contains(&row("from")), "{markup}");
}

#[test]
fn each_rows_menu_sets_it_back_first() {
    let now = at(0);
    let mut page = page_of("");
    page.when = super::super::page::When::Tomorrow;
    let first = |items: Vec<crate::ui::menu::MenuItem>| {
        items
            .first()
            .map(|item| (item.key.clone(), item.name.clone()))
    };
    assert_eq!(
        first(sends_items(&page, now, &Utc, mail_logic::when::Clock::H24)),
        Some(("now".to_owned(), "Now".to_owned()))
    );
    assert_eq!(
        first(remind::remind_items(
            Remind::After("three-days"),
            now,
            &Utc,
            mail_logic::when::Clock::H24
        )),
        Some((remind::OFF_KEY.to_owned(), "Off".to_owned()))
    );
    assert_eq!(
        first(protection::items(Protection::OpenPgp(Mode::Sign))),
        Some(("none".to_owned(), "None".to_owned()))
    );
    let receipt_keys: Vec<String> = receipt::row_items(ReceiptRequest::Requested)
        .into_iter()
        .map(|item| item.key)
        .collect();
    assert!(receipt_keys.contains(&receipt::OFF_KEY.to_owned()));
}

#[test]
fn send_menu_and_more_hold_every_choice_once() {
    let page = page_of("");
    assert_eq!(
        titles(&send_items(
            &page,
            at(0),
            &Utc,
            mail_logic::when::Clock::H24
        )),
        [
            crate::ui::copy::menus::SEND_LATER,
            crate::ui::copy::menus::REMIND_IF_NO_REPLY,
            "Ask for Read Receipt"
        ]
    );
    assert_eq!(
        titles(&more_items(&page, true)),
        [
            crate::ui::copy::menus::SECURITY,
            crate::ui::copy::menus::PRIORITY,
            crate::ui::copy::menus::REQUEST_DELIVERY_RECEIPT,
            "Show Plain Text",
            crate::ui::copy::menus::SAVE_AS_TEMPLATE,
            "—",
            crate::ui::copy::menus::KEEP_FOR_LATER,
            crate::ui::copy::menus::DELETE_DRAFT,
        ]
    );
    // An account that cannot ask for a report is not offered the choice.
    assert!(
        !titles(&more_items(&page, false))
            .contains(&crate::ui::copy::menus::REQUEST_DELIVERY_RECEIPT.to_owned()),
        "no Request Delivery Receipt where it does nothing"
    );
    let mut page = page;
    assert_eq!(pick_more(&mut page, "plain"), More::Done);
    assert_eq!(page.plain, super::super::page::Fold::Open);
    assert_eq!(pick_more(&mut page, "keep"), More::Keep);
    assert_eq!(pick_more(&mut page, "delete"), More::Delete);
    assert_eq!(page.float, Float::Closed);
}

/// ⋯ › Priority and Request Delivery Receipt: each pick is checked in the menu and lands on the
/// draft the page saves, and Normal and a second press take them back off.
#[test]
fn priority_and_a_delivery_receipt_are_picked_from_more_and_saved_on_the_draft() {
    let mut page = page_of("");
    let base = draft_of("");
    // What is checked under Priority ▸ and at the top of ⋯ (Security's own choices aside).
    let checked = |page: &Page| -> Vec<String> {
        let on = |item: &Line<String>| match item {
            Line::Item { title, check, .. } if *check == Some(Check::On) => Some(title.clone()),
            _ => None,
        };
        more_items(page, true)
            .iter()
            .flat_map(|item| match item {
                Line::Submenu {
                    title, children, ..
                } if title == crate::ui::copy::menus::PRIORITY => {
                    children.iter().filter_map(on).collect()
                }
                item => on(item).into_iter().collect::<Vec<_>>(),
            })
            .collect()
    };
    assert_eq!(checked(&page), [crate::ui::copy::menus::PRIORITY_NORMAL]);
    let before = page.edits;

    assert_eq!(pick_more(&mut page, "priority:high"), More::Done);
    assert_eq!(pick_more(&mut page, "delivery"), More::Done);
    assert_eq!(
        checked(&page),
        [
            crate::ui::copy::menus::PRIORITY_HIGH,
            crate::ui::copy::menus::REQUEST_DELIVERY_RECEIPT
        ]
    );
    assert_eq!(
        page.edits,
        before + 2,
        "the picks did not mark the page to save"
    );
    let saved = page.apply_to(&base, at(1));
    assert_eq!(saved.priority, Priority::High);
    assert_eq!(saved.delivery, DeliveryReport::Requested);

    assert_eq!(pick_more(&mut page, "priority:normal"), More::Done);
    assert_eq!(pick_more(&mut page, "delivery"), More::Done);
    let saved = page.apply_to(&base, at(2));
    assert_eq!(saved.priority, Priority::Normal);
    assert_eq!(saved.delivery, DeliveryReport::Unrequested);
    assert_eq!(page.float, Float::Closed);
}

#[tokio::test]
async fn delete_draft_can_be_taken_back() {
    let (store, _dir) = seeded();
    let mut draft = fresh_draft(&store);
    draft.subject = "Lunch on Friday".to_owned();
    mail_core::compose::save(&store, &draft).unwrap_or_else(|why| panic!("{why}"));
    let (mut window, _seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    let (page, shell, desk) = (window.page(), window.shell, window.desk);
    window.dom.in_scope(dioxus_core::ScopeId::APP, || {
        super::super::delete_draft(page, shell, desk);
    });
    window.render();
    assert!(store.draft(draft.id).is_err(), "the draft is still there");
    let entry = window
        .dom
        .in_runtime(|| {
            let mut shell = shell;
            shell.write().undo.pop()
        })
        .unwrap_or_else(|| panic!("nothing to take back"));
    for part in &entry {
        assert!(mail_core::act::take_back(&store, part), "{part:?}");
    }
    let back = store
        .draft(draft.id)
        .unwrap_or_else(|why| panic!("not back: {why}"));
    assert_eq!(back.subject, "Lunch on Friday");
}

/// What is typed that is not an address stays typed and marks its field, not the page; typing in
/// the field takes the mark off, and an address committed there leaves no mark.
#[test]
fn what_is_not_an_address_marks_its_field() {
    use super::super::recipients::{commit_typed, typed};
    let (store, _dir) = seeded();
    let mut page = page_of("");
    page.typed_to = "/".to_owned();
    assert!(!commit_typed(&mut page, List::To));
    assert_eq!(page.typed_to, "/", "the entry was dropped");
    assert!(page.misread(List::To).is_some(), "To is not marked");
    assert!(page.misread(List::Cc).is_none(), "Cc is marked");
    assert_eq!(page.notice, None, "the page put up a notice");

    typed(
        &mut page,
        List::To,
        "ada@example.test".to_owned(),
        &crate::ui::fixtures::client(&store),
    );
    assert!(page.misread(List::To).is_none(), "typing left the mark");
    page.typed_to = "ada@".to_owned();
    assert!(!commit_typed(&mut page, List::To));
    assert!(!commit_typed(&mut page, List::To));
    let count = page.misread(List::To).map(|misread| misread.count);
    page.typed_to = "ada@example.test".to_owned();
    assert!(commit_typed(&mut page, List::To));
    assert!(page.misread.is_none(), "an address left the mark");
    assert_eq!(count, Some(2), "a second refusal is not a new one");
}
