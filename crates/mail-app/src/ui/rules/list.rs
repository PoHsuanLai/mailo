//! The rules of one account, in the order they run: each with its switch, its place in the
//! order, Edit, Run on existing mail, and Delete.

use chrono::Utc;
use dioxus::prelude::*;
use mail_domain::{AccountId, RuleState};
use mail_store::SqliteStore;
use std::sync::Arc;

use super::super::files::work::grouped;
use super::super::files::{Phase, run};
use super::super::motion::{Follow, tell};
use super::editor::RuleEditor;
use super::work::{self, Draft, Listed, Step};
use ds::{Glyph, Icon};

/// The rules part of the sheet.
#[component]
pub(super) fn RulesPart(account: AccountId, revision: Signal<u64>) -> Element {
    // Bumped by every write, so the rules are read again.
    let changed = use_signal(|| 0u64);
    let mut editing = use_signal(|| None::<Draft>);
    let said = use_signal(|| None::<Result<String, String>>);
    let phase = use_signal(|| Phase::Ready);
    let running = use_signal(String::new);
    let _ = changed();
    let store = consume_context::<Arc<SqliteStore>>();
    let (rules, failed) = match work::listed(&store, account) {
        Ok(rules) => (rules, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let count = rules.len();
    let new_label = "New rule".to_owned();
    rsx! {
        section { class: "rules-part",
            h4 { "Rules" }
            p { class: "capnote",
                "They sort new mail as it arrives, first to last. A condition is written the way a search is."
            }
            ul { class: "rules-list",
                for (at, listed) in rules.into_iter().enumerate() {
                    RuleRow {
                        key: "{listed.rule.id}",
                        listed,
                        first: at == 0,
                        last: at + 1 == count,
                        changed,
                        editing,
                        said,
                        phase,
                        running,
                        revision,
                    }
                }
                if count == 0 {
                    li { class: "rules-none",
                        "{failed.clone().unwrap_or_else(|| \"No rules on this account yet.\".to_owned())}"
                    }
                }
            }
            RunProgress { phase: phase(), name: running() }
            match said() {
                Some(Ok(text)) => rsx! { p { class: "capnote said", role: "status", "{text}" } },
                Some(Err(why)) => rsx! { p { class: "capnote files-bad", role: "alert", "{why}" } },
                None => rsx! {},
            }
            if editing.read().is_some() {
                RuleEditor { account, editing, changed, said }
            } else {
                div { class: "rules-acts",
                    button {
                        class: "mini",
                        r#type: "button",
                        aria_label: "{new_label}",
                        onclick: move |_| editing.set(Some(Draft::blank())),
                        Glyph { icon: Icon::Plus }
                        "New rule"
                    }
                }
            }
        }
    }
}

/// One rule.
#[component]
fn RuleRow(
    listed: Listed,
    first: bool,
    last: bool,
    changed: Signal<u64>,
    editing: Signal<Option<Draft>>,
    said: Signal<Option<Result<String, String>>>,
    phase: Signal<Phase>,
    running: Signal<String>,
    revision: Signal<u64>,
) -> Element {
    let rule = listed.rule.clone();
    let name = rule.name.clone();
    let on = rule.state == RuleState::Enabled;
    let class = if on { "rules-row" } else { "rules-row off" };
    let busy = phase.read().running();
    let write = move |done: Result<(), String>| {
        let (mut said, mut changed) = (said, changed);
        if let Err(why) = done {
            said.set(Some(Err(why)));
        }
        changed += 1;
    };
    let toggled = rule.clone();
    let up = rule.id;
    let down = rule.id;
    let run_rule = rule.clone();
    let gone = rule.clone();
    let edit = listed.clone();
    rsx! {
        li { class: "{class}",
            button {
                class: "rules-switch",
                r#type: "button",
                role: "switch",
                aria_checked: if on { "true" } else { "false" },
                aria_label: "{name} on",
                title: if on { "On: it sorts new mail" } else { "Off: kept, and skipped" },
                onclick: move |_| {
                    let store = consume_context::<Arc<SqliteStore>>();
                    let state = if on { RuleState::Disabled } else { RuleState::Enabled };
                    write(work::switch(&store, &toggled, state));
                },
                span {}
            }
            div { class: "rules-text",
                b { "{name}" }
                code { class: "rules-when", "{listed.when}" }
                span { class: "rules-does", "{listed.does}" }
            }
            div { class: "rules-row-acts",
                button {
                    class: "ghost",
                    r#type: "button",
                    aria_label: "Move {name} up",
                    disabled: first,
                    onclick: move |_| {
                        let store = consume_context::<Arc<SqliteStore>>();
                        write(work::reorder(&store, rule.account, up, Step::Up));
                    },
                    "↑"
                }
                button {
                    class: "ghost",
                    r#type: "button",
                    aria_label: "Move {name} down",
                    disabled: last,
                    onclick: move |_| {
                        let store = consume_context::<Arc<SqliteStore>>();
                        write(work::reorder(&store, rule.account, down, Step::Down));
                    },
                    "↓"
                }
                button {
                    class: "ghost",
                    r#type: "button",
                    aria_label: "Edit {name}",
                    onclick: move |_| {
                        let mut editing = editing;
                        editing.set(Some(Draft::of(&edit)));
                    },
                    "Edit"
                }
                button {
                    class: "ghost",
                    r#type: "button",
                    aria_label: "Run {name} on existing mail",
                    title: "Run on existing mail",
                    disabled: busy,
                    onclick: move |_| {
                        let store = consume_context::<Arc<SqliteStore>>();
                        let of = work::conversations(&store, run_rule.account, Utc::now());
                        let rule = run_rule.clone();
                        let mut running = running;
                        running.set(rule.name.clone());
                        let mut revision = revision;
                        run(
                            phase,
                            of,
                            move |report| work::run(&store, &rule, Utc::now(), report),
                            move || revision += 1,
                        );
                    },
                    "Run"
                }
                button {
                    class: "ghost danger",
                    r#type: "button",
                    aria_label: "Delete {name}",
                    onclick: move |_| {
                        let store = consume_context::<Arc<SqliteStore>>();
                        match work::delete(&store, &gone) {
                            Ok(text) => tell(text, Follow::Nothing),
                            Err(why) => {
                                let mut said = said;
                                said.set(Some(Err(why)));
                            }
                        }
                        let mut changed = changed;
                        changed += 1;
                    },
                    "Delete"
                }
            }
        }
    }
}

/// "Run on existing mail" while it runs, and what it said.
#[component]
pub(super) fn RunProgress(phase: Phase, name: String) -> Element {
    match phase {
        Phase::Ready => rsx! {},
        Phase::Running { done, of } => {
            let width = (done.min(of) * 100).checked_div(of).unwrap_or(0);
            rsx! {
                div { class: "files-progress", role: "status",
                    span { "Running “{name}”… {grouped(done)} of {grouped(of)} conversations" }
                    div { class: "files-bar", span { style: "width:{width}%" } }
                }
            }
        }
        Phase::Finished(said) => rsx! { p { class: "capnote said", role: "status", "{said}" } },
        Phase::Failed(why) => rsx! { p { class: "capnote files-bad", role: "alert", "{why}" } },
    }
}
