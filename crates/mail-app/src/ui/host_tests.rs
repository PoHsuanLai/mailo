//! The seam sends the webview exactly the scripts the call sites used to send, and a test's
//! recorder keeps the asks without running any.

use super::{Ask, Drawn, Host, Recorder, When};
use crate::ui::fixtures::Scripts;
use dioxus::prelude::*;
use dioxus_core::{ScopeId, VirtualDom};

/// Every ask the window makes, beside the script its call site evaluated before the seam,
/// copied from that call site as it was.
fn before_the_seam() -> Vec<(Ask, &'static str)> {
    let focus = |selector, when| Ask::Focus { selector, when };
    vec![
        (Ask::FocusApp, "document.querySelector('.app')?.focus()"),
        (
            focus(".search input", When::Now),
            "document.querySelector('.search input')?.focus()",
        ),
        (
            focus(".acct-sheet .files-main input", When::NextFrame),
            "requestAnimationFrame(()=>document.querySelector('.acct-sheet .files-main input')?.focus())",
        ),
        (
            focus(".book-find .inp", When::NextFrame),
            "requestAnimationFrame(()=>document.querySelector('.book-find .inp')?.focus())",
        ),
        (
            focus(".files-main input", When::NextFrame),
            "requestAnimationFrame(()=>document.querySelector('.files-main input')?.focus())",
        ),
        (
            focus(".pick-field", When::AfterTask),
            "setTimeout(() => document.querySelector('.pick-field')?.focus())",
        ),
        (
            focus(".tpl-name", When::AfterTask),
            "setTimeout(() => document.querySelector('.tpl-name')?.focus())",
        ),
        (
            Ask::FocusAndSelect(Drawn::FindField),
            "(function focusFind(tries) {\
    const field = document.querySelector('.find input');\
    if (field) { field.focus(); field.select(); }\
    else if (tries > 0) { requestAnimationFrame(() => focusFind(tries - 1)); }\
})(20)",
        ),
        (
            Ask::FocusAndSelect(Drawn::FolderName),
            "(function focusName(tries) {\
    const input = document.querySelector('.fold-edit input');\
    if (input) { input.focus(); input.select(); }\
    else if (tries > 0) { requestAnimationFrame(() => focusName(tries - 1)); }\
})(20)",
        ),
        (
            Ask::ScrollIntoView("mark.hit.now"),
            "requestAnimationFrame(() => requestAnimationFrame(() => \
    document.querySelector('mark.hit.now')?.scrollIntoView({ block: 'center' })))",
        ),
        (
            Ask::Copy("a\"b@example.org".to_owned()),
            "navigator.clipboard && navigator.clipboard.writeText(\"a\\\"b@example.org\")",
        ),
    ]
}

#[test]
fn every_script_is_the_one_the_call_site_sent() {
    let wrong: Vec<String> = before_the_seam()
        .into_iter()
        .filter(|(ask, script)| ask.script() != *script)
        .map(|(ask, script)| format!("{ask:?}\n  sends {:?}\n  was   {script:?}", ask.script()))
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

fn empty() -> Element {
    rsx! {}
}

/// Every typed operation, in the order `before_the_seam` lists their asks.
fn ask_everything() {
    Host::focus_app();
    Host::focus(".search input");
    Host::focus_next_frame(".acct-sheet .files-main input");
    Host::focus_next_frame(".book-find .inp");
    Host::focus_next_frame(".files-main input");
    Host::focus_after_task(".pick-field");
    Host::focus_after_task(".tpl-name");
    Host::focus_and_select(Drawn::FindField);
    Host::focus_and_select(Drawn::FolderName);
    Host::scroll_into_view("mark.hit.now");
    Host::copy("a\"b@example.org");
}

#[test]
fn the_webview_evaluates_each_ask_in_the_page() {
    let scripts = Scripts::default();
    let mut dom = VirtualDom::new(empty).with_root_context(scripts.document());
    dom.rebuild_in_place();
    dom.in_scope(ScopeId::ROOT, ask_everything);
    let want: Vec<String> = before_the_seam()
        .into_iter()
        .map(|(_, script)| script.to_owned())
        .collect();
    assert_eq!(scripts.all(), want);
}

#[test]
fn a_recorder_keeps_each_ask_and_runs_none() {
    let scripts = Scripts::default();
    let recorder = Recorder::default();
    let mut dom = VirtualDom::new(empty)
        .with_root_context(scripts.document())
        .with_root_context(recorder.host());
    dom.rebuild_in_place();
    dom.in_scope(ScopeId::ROOT, ask_everything);
    let want: Vec<Ask> = before_the_seam().into_iter().map(|(ask, _)| ask).collect();
    assert_eq!(recorder.asked(), want);
    assert!(scripts.all().is_empty(), "{:?}", scripts.all());
}
