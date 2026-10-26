//! Send later and templates in the real window's layout, drawn for the style lint.

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
    /// The page is sent for tomorrow 08:00, so the pill and the Scheduled place show it.
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
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone());
    let _ = crate::ui::fixtures::rebuild_into(&mut dom);
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
            super::super::templates::save_named(
                &crate::ui::fixtures::client(&built.store),
                &mut write,
                Utc::now(),
            )
        });
        kept.unwrap_or_else(|why| panic!("a template: {why}"));
        let other = mail_core::compose::draft_new(
            &built.store,
            dom.in_runtime(|| page.peek().from.clone()),
            &[],
            "Thanks for the interview",
            "Thank you for your time today.",
            Utc::now(),
        )
        .unwrap_or_else(|why| panic!("{why}"));
        mail_core::template::save(&built.store, other.id, "Interview thanks", Utc::now())
            .unwrap_or_else(|why| panic!("{why}"));
        // The page's own words were the template's; it starts empty again.
        dom.in_runtime(|| {
            let mut write = page.write();
            write.subject.clear();
            write.session.doc = mail_core::editor::Doc {
                nodes: vec![Node::plain(mail_core::editor::ParaKind::Paragraph, "")],
            };
            write.session.caret = mail_core::editor::Caret::at(0, 0);
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
                &crate::ui::fixtures::client(&built.store),
                &mail_core::testing::MapSigningStore::default(),
                &mut write,
                Anyway::Yes,
                Utc::now(),
                &chrono::Local,
                mail_core::when::Clock::H24,
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
    // A menu floats in the root's overlay, drawn on the renders after the one that asks.
    crate::ui::fixtures::drain(&mut dom);
    (dioxus_ssr::render(&dom), built)
}

/// Send-later's place and pill, and the templates menu, each in the window, for the style lint
/// in `style::tests`.
pub(in crate::ui) fn drawn_states() -> String {
    let (scheduled, _built) = window(|_| {}, Before::Schedule);
    assert!(
        scheduled.contains(r#"data-place="Scheduled""#),
        "no Scheduled place: {scheduled}"
    );
    let (templates, _built) = window(
        |page| {
            type_text(page, "/");
            assert!(pick(page, super::super::templates::START_KEY));
        },
        Before::KeepTemplates,
    );
    assert!(templates.contains("Weekly update"), "{templates}");
    scheduled + &templates
}
