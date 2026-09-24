//! The composer page, without a browser: the glue's messages replayed through the page's own
//! handler, the guards, the draft's life against a real store, and the pill's faces.

mod faces;
mod later;
mod later_render;
mod life;
mod local_from;
mod openpgp;
mod people;
mod receipt;
mod render;
mod smime;
mod templates;
mod wire;

use std::cell::Cell;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_domain::*;
use mail_store::SqliteStore;

use super::body::Body;
use super::desk::{Desk, use_desk};
use super::page::Page;
use super::wire::{hear, parse};
use super::{ComposerPage, SendPill, composing};
use crate::appearance::WindowDirs;
use crate::editor::{Node, Person, Pos};
use crate::view::Shell;

/// A fixed instant, so nothing here depends on the clock.
fn at(minutes: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 23, 10, 0, 0)
        .single()
        .unwrap_or_else(|| panic!("a real instant"))
        + chrono::TimeDelta::minutes(minutes)
}

/// A draft holding `text`, belonging to nobody in particular.
fn draft_of(text: &str) -> Draft {
    Draft {
        id: DraftId::generate(),
        account: AccountId::generate(),
        identity: IdentityId::generate(),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "Plans".to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: text.to_owned(),
        html: None,
        attachments: Vec::new(),
        receipt: ReceiptRequest::Unrequested,
        openpgp: OpenPgp::None,
        smime: mail_domain::Smime::None,
        state: SendState::Editing,
        updated: at(0),
    }
}

fn dana() -> Person {
    Person {
        name: "Dana Whitfield".to_owned(),
        address: "dana@example.test".to_owned(),
    }
}

fn sam() -> Person {
    Person {
        name: "Sam Okafor".to_owned(),
        address: "sam@example.test".to_owned(),
    }
}

fn page_of(text: &str) -> Page {
    Page::of(&draft_of(text), vec![dana(), sam()], Vec::new())
}

/// One message exactly as the glue writes it.
fn message(
    k: u64,
    input_type: &str,
    data: Option<&str>,
    ranges: &[[usize; 4]],
    sel: Option<[usize; 4]>,
) -> String {
    serde_json::json!({
        "k": k,
        "t": input_type,
        "data": data,
        "ranges": ranges,
        "sel": sel,
        "html": null,
    })
    .to_string()
}

/// Replay one raw message through the page's handler, as the wire's `oninput` does.
fn feed(page: &mut Page, raw: &str, at_ms: u64) {
    let heard = parse(raw).unwrap_or_else(|| panic!("the glue's message did not parse: {raw}"));
    hear(page, heard, at_ms);
}

/// Type `text` one grapheme at a time, each event at the caret, the way an up-to-date page
/// reports it.
fn type_text(page: &mut Page, text: &str) {
    use unicode_segmentation::UnicodeSegmentation;
    for grapheme in text.graphemes(true) {
        let caret = page.session.caret.pos;
        let here = [caret.node, caret.offset, caret.node, caret.offset];
        let k = page.wire.seq + 1;
        let raw = message(k, "insertText", Some(grapheme), &[here], Some(here));
        feed(page, &raw, k * 40);
    }
}

/// The paragraphs' text, `|` between them.
fn body(page: &Page) -> String {
    page.session
        .doc
        .nodes
        .iter()
        .map(|node| match node {
            Node::Para { runs, .. } => crate::editor::runs_text(runs),
            Node::Object(_) => "[object]".to_owned(),
        })
        .collect::<Vec<_>>()
        .join("|")
}

thread_local! {
    static BODY_PAGE: Cell<Option<Signal<Page>>> = const { Cell::new(None) };
    static DESK: Cell<Option<Desk>> = const { Cell::new(None) };
    static SHELL: Cell<Option<Signal<Shell>>> = const { Cell::new(None) };
}

/// The body alone, on a page the test holds.
#[component]
fn BodyHarness(initial: Page) -> Element {
    let page = use_signal(|| initial.clone());
    BODY_PAGE.with(|slot| slot.set(Some(page)));
    let shell = use_signal(Shell::default);
    rsx! { Body { page, shell, on_attach: |_| {} } }
}

fn body_dom(page: Page) -> (VirtualDom, Signal<Page>) {
    let mut dom = VirtualDom::new_with_props(BodyHarness, BodyHarnessProps { initial: page });
    dom.rebuild_in_place();
    let signal = BODY_PAGE
        .with(Cell::get)
        .unwrap_or_else(|| panic!("the harness did not render"));
    (dom, signal)
}

/// The whole page for `draft`, with a desk, a shell and the pill, as the app lays them out.
#[component]
fn PageHarness(draft: Draft) -> Element {
    let mut shell = use_signal(Shell::default);
    let revision = use_signal(|| 0u64);
    let today = use_signal(crate::today::Today::default);
    let spaces = use_signal(crate::space::Spaces::default);
    let side = use_signal(|| false);
    let dirs = try_consume_context::<WindowDirs>();
    let desk = use_desk(today, spaces, dirs, side);
    use_hook(|| shell.write().compose(&draft));
    DESK.with(|slot| slot.set(Some(desk)));
    SHELL.with(|slot| slot.set(Some(shell)));
    let open = composing(&shell.read());
    rsx! {
        div { class: "app",
            if let Some((id, _)) = open {
                ComposerPage { key: "{id}", draft: id, shell, revision }
            }
            SendPill { shell }
            super::ScheduledDrafts { shell }
        }
    }
}

struct Window {
    dom: VirtualDom,
    desk: Desk,
    shell: Signal<Shell>,
}

impl Window {
    fn open(
        store: Arc<SqliteStore>,
        draft: Draft,
        dirs: Option<WindowDirs>,
    ) -> (Self, crate::ui::fixtures::Seen) {
        crate::ui::fixtures::dispatching();
        let mut dom = VirtualDom::new_with_props(PageHarness, PageHarnessProps { draft })
            .with_root_context(store);
        if let Some(dirs) = dirs {
            dom = dom.with_root_context(dirs);
        }
        let seen = crate::ui::fixtures::rebuild_into(&mut dom);
        let desk = DESK.with(Cell::get).unwrap_or_else(|| panic!("no desk"));
        let shell = SHELL.with(Cell::get).unwrap_or_else(|| panic!("no shell"));
        (Self { dom, desk, shell }, seen)
    }

    /// The page on screen.
    fn page(&self) -> Signal<Page> {
        self.dom.in_runtime(|| {
            self.desk
                .current
                .peek()
                .unwrap_or_else(|| panic!("no page is open"))
        })
    }

    fn render(&mut self) -> String {
        self.dom.render_immediate(&mut NoOpMutations);
        dioxus_ssr::render(&self.dom)
    }
}

#[test]
fn the_glue_stays_small_and_never_writes_markup() {
    let lines = super::GLUE.lines().count();
    assert!(
        lines < 100,
        "the glue script is {lines} lines; it must stay under 100"
    );
    for forbidden in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "execCommand",
        "document.write",
    ] {
        assert!(
            !super::GLUE.contains(forbidden),
            "the glue uses {forbidden}, which builds markup or edits text"
        );
    }
}

#[test]
fn when_a_scheduled_send_is_due() {
    use super::page::When;
    // 2026-09-23 is a Wednesday.
    let now = at(0);
    let cases: &[(When, Option<&str>)] = &[
        (When::Now, None),
        (When::Tomorrow, Some("2026-09-24 08:00")),
        (When::Monday, Some("2026-09-28 09:00")),
        (When::At(at(90)), Some("2026-09-23 11:30")),
    ];
    for (when, want) in cases {
        let got = when
            .due(now, &Utc)
            .map(|due| due.format("%Y-%m-%d %H:%M").to_string());
        assert_eq!(got.as_deref(), *want, "{when:?}");
    }
}

#[test]
fn a_reply_text_opens_as_paragraphs_a_signature_and_a_folded_original() {
    use crate::editor::Object;
    let text = "Sounds good.\r\n\r\n-- \r\nDana\r\n\r\nOn Wed, 23 Sep 2026 at 09:02, Sam Okafor wrote:\r\n> first line\r\n> second line\r\n";
    let nodes = super::opening::doc_from_text(text);
    let shape: Vec<String> = nodes
        .iter()
        .map(|node| match node {
            Node::Para { runs, .. } => format!("p:{}", crate::editor::runs_text(runs)),
            Node::Object(Object::Signature) => "sig".to_owned(),
            Node::Object(Object::QuotedMessage { who, when, body }) => {
                format!("quoted:{who}/{when}/{}", body.len())
            }
            Node::Object(_) => "object".to_owned(),
        })
        .collect();
    assert_eq!(
        shape,
        [
            "p:Sounds good.",
            "sig",
            "p:Dana",
            "quoted:Sam Okafor/Wed, 23 Sep 2026 at 09:02/2",
        ]
    );
}

#[tokio::test]
async fn the_composer_can_be_opened_and_closed_repeatedly() {
    // Opening and closing runs the page's hooks each way round, parking on the way out.
    let (mut dom, toggle, _dir) = crate::ui::fixtures::harness(false);
    dom.rebuild_in_place();
    for open in [true, false, true, false] {
        toggle.0.store(open, std::sync::atomic::Ordering::SeqCst);
        dom.mark_dirty(dioxus_core::ScopeId::APP);
        dom.render_immediate(&mut NoOpMutations);
    }
}

#[test]
fn the_caret_attribute_is_the_selection_else_the_caret() {
    let mut page = page_of("hello");
    page.session.caret = crate::editor::Caret::at(0, 3);
    assert_eq!(super::wire::caret_attr(&page), "0:3:0:3");
    page.selection = Some(crate::editor::Range {
        start: Pos::new(0, 1),
        end: Pos::new(0, 4),
    });
    assert_eq!(super::wire::caret_attr(&page), "0:1:0:4");
}
