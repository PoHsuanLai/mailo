//! Send later and templates in the real window's layout: every class they draw is styled, and
//! files to look at.

use super::super::desk::Outgoing;
use super::super::life::{self, Anyway, Sent};
use super::super::page::{Float, When};
use super::super::templates::pick;
use super::render::with_identities;
use super::*;
use crate::ui::app::App;
use crate::ui::fixtures::{Work, key, work};

/// What happens in the window before the page is dressed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Before {
    Nothing,
    /// The page is sent for tomorrow 08:00, so the pill and Today show it.
    Schedule,
    /// Two templates are kept on the page's account, and the page is left empty.
    KeepTemplates,
}

/// The window with a page open and dressed, after `before`.
fn window(dress: impl FnOnce(&mut Page), before: Before) -> (String, Work) {
    let (schedule, templates) = (before == Before::Schedule, before == Before::KeepTemplates);
    crate::ui::fixtures::dispatching();
    let built = work();
    with_identities(&built);
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    dom.rebuild_in_place();
    key(&mut dom, "c");
    let mut desk = dom.in_scope(dioxus_core::ScopeId::APP, consume_context::<Desk>);
    let mut page = dom.in_runtime(|| {
        desk.current
            .peek()
            .unwrap_or_else(|| panic!("c opened no page"))
    });
    if templates {
        let kept = dom.in_runtime(|| {
            let mut write = page.write();
            write.subject = "Weekly notes".to_owned();
            type_text(
                &mut write,
                "Here is what moved this week, and what did not.",
            );
            write.float = Float::SaveTemplate("Weekly update".to_owned());
            super::super::templates::save_named(&built.store, &mut write, Utc::now())
        });
        kept.unwrap_or_else(|why| panic!("a template: {why}"));
        let other = crate::compose::draft_new(
            &built.store,
            dom.in_runtime(|| page.peek().from),
            &[],
            "Thanks for the interview",
            "Thank you for your time today.",
            Utc::now(),
        )
        .unwrap_or_else(|why| panic!("{why}"));
        crate::template::save(&built.store, other.id, "Interview thanks", Utc::now())
            .unwrap_or_else(|why| panic!("{why}"));
        // The page's own words were the template's; it starts empty again.
        dom.in_runtime(|| {
            let mut write = page.write();
            write.subject.clear();
            write.session.doc = crate::editor::Doc {
                nodes: vec![Node::plain(crate::editor::ParaKind::Paragraph, "")],
            };
            write.session.caret = crate::editor::Caret::at(0, 0);
            write.float = Float::Closed;
        });
    }
    if schedule {
        dom.in_scope(dioxus_core::ScopeId::APP, || {
            let mut write = page.write();
            write.to = vec![dana()];
            write.subject = "Offsite, Friday".to_owned();
            type_text(&mut write, "The plan for Friday, as promised.");
            write.when = When::Tomorrow;
            let sent = life::send(
                &built.store,
                &mut write,
                Anyway::Yes,
                Utc::now(),
                &chrono::Local,
            );
            let Ok(Sent::Queued { draft, due, when }) = sent else {
                panic!("not scheduled: {sent:?}");
            };
            let back = write.clone();
            drop(write);
            desk.outbox.set(Some(Outgoing {
                draft,
                due,
                when,
                page: back,
                refused: None,
            }));
        });
    }
    dom.in_runtime(|| dress(&mut page.write()));
    dom.render_immediate(&mut NoOpMutations);
    (dioxus_ssr::render(&dom), built)
}

fn styled(markup: &str) {
    let missing = crate::ui::style::tests::unstyled_classes(markup, crate::ui::style::STYLE);
    assert!(missing.is_empty(), "unstyled classes: {missing:?}");
}

#[tokio::test]
async fn every_class_send_later_and_templates_draw_is_styled() {
    let (markup, _built) = window(|_| {}, Before::Schedule);
    assert!(markup.contains("Scheduled for tomorrow 08:00"), "{markup}");
    assert!(
        markup.contains(r#"class="item today-item later""#),
        "{markup}"
    );
    styled(&markup);

    let (markup, _built) = window(
        |page| {
            type_text(page, "/");
            assert!(pick(page, super::super::templates::START_KEY));
        },
        Before::KeepTemplates,
    );
    assert!(markup.contains("Weekly update"), "{markup}");
    assert!(
        markup.contains(r#"class="rm""#),
        "no delete on the rows:\n{markup}"
    );
    styled(&markup);
}

#[tokio::test]
#[ignore = "writes target/later-*.html and their -dark twins for a person to look at"]
async fn render_send_later_and_templates_to_files() {
    let place = |markup: String| {
        // The glue puts a caret menu at the caret in the window; a file has none.
        markup.replacen(
            r#"class="c-float" data-anchor="below""#,
            r#"class="c-float" data-anchor="below" style="left:0;top:40px""#,
            1,
        )
    };
    let (sends, _a) = window(|page| page.float = Float::Sends, Before::Nothing);
    crate::ui::fixtures::dump("later-sends", &sends);
    let (pick_time, _b) = window(
        |page| page.float = Float::PickTime("fri 17:00".to_owned()),
        Before::Nothing,
    );
    crate::ui::fixtures::dump("later-pick", &pick_time);
    let (refused, _c) = window(
        |page| page.float = Float::PickTime("2026-01-05 09:00".to_owned()),
        Before::Nothing,
    );
    crate::ui::fixtures::dump("later-pick-refused", &refused);
    let (scheduled, _d) = window(|_| {}, Before::Schedule);
    crate::ui::fixtures::dump("later-scheduled", &scheduled);
    let (list, _e) = window(
        |page| {
            type_text(page, "/");
            pick(page, super::super::templates::START_KEY);
        },
        Before::KeepTemplates,
    );
    crate::ui::fixtures::dump("later-templates", &place(list));
    let (save, _f) = window(
        |page| page.float = Float::SaveTemplate("Weekly".to_owned()),
        Before::Nothing,
    );
    crate::ui::fixtures::dump("later-save", &place(save));
}
