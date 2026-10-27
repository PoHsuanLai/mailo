//! Lane 15: sorting the list. The list's ⋯ › Sort By sorts the inbox by subject, then the other
//! way, and the order is kept in `settings.toml` under the inbox's key; a window opened over a
//! kept order lists the inbox in it from its first rows.

use ds_harness::Harness;
use mail_domain::{Property, Sort, SortDir};
use mail_ui::settings::ListOrder;
use mail_ui::ui::copy;

use super::look::row;
use super::seed::INBOX;
use super::window::Window;

/// The inbox's key in `window.sorts` (`mail_core::conversation::place_key`).
const INBOX_KEY: &str = "place:Inbox";

fn more() -> String {
    format!(".list-head button[*|aria-label=\"{}\"]", copy::aria::MORE)
}

/// The subjects of the list's rows, top to bottom.
fn subjects(harness: &Harness) -> Vec<String> {
    (1..=harness.count(".list .ds-list-item"))
        .filter_map(|n| harness.text_of(&format!("{} .ds-thread-sub", row(n))))
        .map(|subject| subject.trim().to_owned())
        .filter(|subject| !subject.is_empty())
        .collect()
}

/// The seeded inbox's subjects, A to Z.
fn a_to_z() -> Vec<String> {
    let mut subjects: Vec<String> = INBOX.iter().map(|(_, s)| (*s).to_owned()).collect();
    subjects.sort_by_key(|s| s.to_lowercase());
    subjects
}

/// Pick `name` under the list's ⋯ › Sort By.
fn sort_by(window: &mut Window, name: &str) {
    window.click(&more());
    window.until("the ⋯ opens", |h| h.count(".ds-menu") >= 1);
    window.submenu_item(copy::menus::SORT_BY, name);
    window.until("the ⋯ closes", |h| h.count(".ds-menu") == 0);
}

#[test]
fn the_list_sorts_from_its_more_menu_and_keeps_the_order() {
    let mut window = Window::open(|_| {});
    let newest: Vec<String> = INBOX.iter().map(|(_, s)| (*s).to_owned()).collect();
    window.until("the inbox is newest first", |h| subjects(h) == newest);

    sort_by(&mut window, copy::menus::sort_key(Property::Subject));
    let want = a_to_z();
    window.until("the inbox is A to Z", |h| subjects(h) == want);

    let [_, z_to_a] = copy::menus::sort_dirs(Property::Subject);
    sort_by(&mut window, z_to_a);
    let want: Vec<String> = a_to_z().into_iter().rev().collect();
    window.until("the inbox is Z to A", |h| subjects(h) == want);

    let root = mail_ui::settings::root_for(&window.dirs().config);
    assert_eq!(
        mail_ui::settings::load(&root).window.sorts.get(INBOX_KEY),
        Some(&ListOrder(Sort {
            property: Property::Subject,
            dir: SortDir::Desc,
        })),
        "the order is kept under the inbox's key"
    );

    // Back to the date, which runs newest first when picked.
    sort_by(&mut window, copy::menus::sort_key(Property::Date));
    window.until("the inbox is newest first again", |h| subjects(h) == newest);
}

#[test]
fn a_kept_order_is_the_list_s_when_the_window_opens() {
    let kept = Sort {
        property: Property::Subject,
        dir: SortDir::Asc,
    };
    let mut window = Window::open_with(
        chrono::Utc::now(),
        |_| {},
        |dir, contexts| {
            let root = mail_ui::settings::root_for(&dir.join("config"));
            mail_ui::settings::change(&root, |settings| {
                settings
                    .window
                    .sorts
                    .insert(INBOX_KEY.to_owned(), ListOrder(kept));
            })
            .expect("settings.toml is written");
            contexts
        },
    );
    let want = a_to_z();
    window.until("the inbox opens A to Z", |h| subjects(h) == want);
}
