//! A test's recorder keeps the window's asks without running any, and a window with no host
//! drops them.

use super::{Ask, Drawn, Host, Recorder, When};
use crate::ui::fixtures::Scripts;
use dioxus::prelude::*;
use dioxus_core::{ScopeId, VirtualDom};

/// Every ask the window makes, one of each shape.
fn every_ask() -> Vec<Ask> {
    let focus = |selector, when| Ask::Focus { selector, when };
    vec![
        Ask::FocusApp,
        focus(".search input", When::Now),
        focus(".acct-sheet .files-main input", When::NextFrame),
        focus(".book-find .inp", When::NextFrame),
        focus(".files-main input", When::NextFrame),
        focus(".pick-field", When::AfterTask),
        focus(".tpl-name", When::AfterTask),
        Ask::FocusAndSelect(Drawn::FindField),
        Ask::FocusAndSelect(Drawn::FolderName),
        Ask::ScrollIntoView("mark.hit.now"),
        Ask::Copy("a\"b@example.org".to_owned()),
    ]
}

fn empty() -> Element {
    rsx! {}
}

/// Every typed operation, in the order `every_ask` lists their asks.
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
fn a_window_with_no_host_evaluates_no_script() {
    let scripts = Scripts::default();
    let mut dom = VirtualDom::new(empty).with_root_context(scripts.document());
    dom.rebuild_in_place();
    dom.in_scope(ScopeId::ROOT, ask_everything);
    assert!(scripts.all().is_empty(), "{:?}", scripts.all());
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
    assert_eq!(recorder.asked(), every_ask());
    assert!(scripts.all().is_empty(), "{:?}", scripts.all());
}
