//! The page drawn in the real window's layout: every class styled, and a file to look at.

use base64::Engine as _;

use super::super::page::{CcRow, Float, Guard};
use super::*;
use crate::editor::{Check, Doc, ImageRef, Level, Object, ParaKind, Range, Table};
use crate::ui::app::App;
use crate::ui::fixtures::{key, work};

/// A small picture, embedded the way an image in a draft is: never fetched.
fn picture() -> String {
    let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='640' height='240'>\
        <defs><linearGradient id='g' x1='0' x2='1'><stop offset='0' stop-color='#7a9a6b'/>\
        <stop offset='1' stop-color='#d9c27a'/></linearGradient></defs>\
        <rect width='640' height='240' fill='url(#g)'/>\
        <circle cx='470' cy='90' r='42' fill='#f3e3b0'/>\
        <path d='M0 200 Q160 130 320 190 T640 170 V240 H0 Z' fill='#4f6b45'/></svg>";
    format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg)
    )
}

/// A composed message with a heading, a list, a quote, an image, the `/` menu open and the
/// warning bar showing.
fn composed(page: &mut Page) {
    page.subject = "Offsite, Friday".to_owned();
    page.to = vec![dana()];
    page.cc = vec![sam()];
    page.cc_row = CcRow::Shown;
    page.session.doc = Doc {
        nodes: vec![
            Node::plain(ParaKind::Heading(Level::One), "Plans for Friday"),
            Node::plain(
                ParaKind::Paragraph,
                "Here is where we landed, so nobody has to scroll back:",
            ),
            Node::plain(ParaKind::Bullet, "Leave at 08:30 from the north gate"),
            Node::plain(
                ParaKind::Bullet,
                "Lunch is on the terrace, weather allowing",
            ),
            Node::plain(ParaKind::Todo(Check::Done), "Book the minibus"),
            Node::plain(ParaKind::Todo(Check::Open), "Print the maps"),
            Node::plain(ParaKind::Quote, "Keep the talks short and the walks long."),
            Node::plain(ParaKind::Paragraph, "/"),
            Node::Object(Object::Image {
                src: ImageRef::new(picture()),
                alt: "The garden path, last spring".to_owned(),
            }),
            Node::plain(
                ParaKind::Paragraph,
                "I attached the agenda, and the rest is below.",
            ),
        ],
    };
    page.session.caret = crate::editor::Caret::at(7, 1);
    page.float = Float::Slash {
        anchor: Pos::new(7, 0),
        active: 0,
    };
    page.guard = Guard::Warn;
}

/// One way to dress the page before it is drawn.
type Dress = Box<dyn FnOnce(&mut Page)>;

/// The window with the composer open, laid out by the real app.
fn window_with_page(dress: impl FnOnce(&mut Page)) -> (String, tempfile::TempDir) {
    window_with(|page, _| dress(page))
}

/// The same, with the store to dress the page from.
pub(super) fn window_with(
    dress: impl FnOnce(&mut Page, &SqliteStore),
) -> (String, tempfile::TempDir) {
    crate::ui::fixtures::dispatching();
    let built = work();
    with_identities(&built);
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    dom.rebuild_in_place();
    key(&mut dom, "c");
    let desk = dom.in_scope(dioxus_core::ScopeId::APP, consume_context::<Desk>);
    let mut page = dom.in_runtime(|| {
        desk.current
            .peek()
            .unwrap_or_else(|| panic!("c opened no page"))
    });
    dom.in_runtime(|| dress(&mut page.write(), &built.store));
    crate::ui::fixtures::drain(&mut dom);
    (dioxus_ssr::render(&dom), built.root)
}

/// The Work Space's accounts have no identity rows; `account add` writes one for each.
pub(super) fn with_identities(built: &crate::ui::fixtures::Work) {
    let accounts: Vec<(String, String)> = crate::ui::data::account_rows(&built.store)
        .into_iter()
        .map(|row| (row.id.to_string(), row.address))
        .collect();
    for (account, address) in accounts {
        built
            .store
            .connection()
            .execute(
                "INSERT INTO identities (id, account, from_name, from_email, is_default)
                 VALUES (?1, ?2, 'Ada', ?3, '\"default\"')",
                rusqlite::params![IdentityId::generate().to_string(), account, address],
            )
            .unwrap_or_else(|why| panic!("an identity: {why}"));
    }
}

#[tokio::test]
async fn every_class_the_composer_draws_is_styled() {
    let states: Vec<Dress> = vec![
        Box::new(composed),
        Box::new(|page: &mut Page| {
            composed(page);
            page.float = Float::Closed;
            page.selection = Some(Range {
                start: Pos::new(1, 0),
                end: Pos::new(1, 7),
            });
        }),
        Box::new(|page: &mut Page| {
            composed(page);
            page.float = Float::Turn;
            page.selection = Some(Range {
                start: Pos::new(1, 0),
                end: Pos::new(1, 7),
            });
        }),
        Box::new(|page: &mut Page| {
            composed(page);
            page.float = Float::Object(8);
            page.session
                .doc
                .nodes
                .push(Node::Object(Object::Table(Table {
                    rows: vec![vec!["Item".to_owned(), "Detail".to_owned()]],
                })));
            page.session.doc.nodes.push(Node::Object(Object::Divider));
            page.session.doc.nodes.push(Node::Object(Object::Signature));
            page.session
                .doc
                .nodes
                .push(Node::plain(ParaKind::Code, "cargo run"));
        }),
        Box::new(|page: &mut Page| {
            composed(page);
            page.float = Float::Sends;
            page.guard = Guard::Shake(1);
        }),
        Box::new(|page: &mut Page| page.float = Float::PickTime("tomorrow 9".to_owned())),
        Box::new(|page: &mut Page| page.float = Float::PickTime("2020-01-01 10:00".to_owned())),
        Box::new(|page: &mut Page| page.float = Float::SaveTemplate("Weekly".to_owned())),
        Box::new(|page: &mut Page| page.float = Float::Templates { active: 0 }),
    ];
    for dress in states {
        let (markup, _root) = window_with_page(dress);
        let missing = crate::ui::style::tests::unstyled_classes(
            &markup,
            &crate::ui::style::tests::full_css(),
        );
        assert!(missing.is_empty(), "unstyled classes: {missing:?}");
    }
}

/// Every kind of node, marked for quire's `EditSurface`: `data-edit-node` on each, every object
/// an atom, the surface itself `.c-body`, and no `contenteditable` and no wire at all.
#[tokio::test]
async fn the_composed_message_draws_every_kind_of_node_on_the_surface() {
    let (markup, _root) = window_with_page(composed);
    for needle in [
        r#"<h2 data-edit-node="0">"#,
        r#"<ul><li data-edit-node="2">"#,
        r#"<ul class="todo"><li data-edit-node="4" class="done">"#,
        r#"<blockquote data-edit-node="6">"#,
        r#"class="obj o-img" data-edit-node="8" data-edit-kind="atom""#,
        // The surface is `.c-body` itself: quire's class, then mailo's, and no wrapper inside.
        r#"class="ds-edit c-body""#,
        r#"class="c-float" data-anchor="below""#,
        r#"class="c-warn""#,
    ] {
        assert!(markup.contains(needle), "no {needle} in:\n{markup}");
    }
    for gone in [
        "data-n=",
        r#"contenteditable="true""#,
        r#"class="c-wire""#,
        r#"<div class="c-body">"#,
    ] {
        assert!(!markup.contains(gone), "{gone} in:\n{markup}");
    }
}

#[tokio::test]
#[ignore = "writes target/composer.html and target/composer-dark.html for a person to look at"]
async fn render_the_composer_to_a_file() {
    let (markup, _root) = window_with_page(composed);
    // The surface puts the menu at the caret in the window. A file has no surface, so it is
    // placed by hand under the line where the "/" is.
    let placed = markup.replacen(
        r#"class="c-float" data-anchor="below""#,
        r#"class="c-float" data-anchor="below" style="left:0;top:246px""#,
        1,
    );
    crate::ui::fixtures::dump("composer", &placed);
}
