//! The rules of one account, in the order they run: each with its switch, its place in the
//! order, Edit, Run on existing mail, and Delete.

use chrono::Utc;
use dioxus::prelude::*;
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::content::text_runs::RunTone;
use ds::components::controls::button_model::{Bezel, ButtonRole, ImagePosition};
use ds::components::controls::progress::model::{Progress, ProgressStyle};
use ds::components::controls::progress::view::ProgressIndicator;
use ds::components::lists::list::model::{ListItem, ListStyle};
use ds::prelude::*;
use ds::root::common::Common;
use mail_domain::{RuleId, RuleState};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

use super::super::files::work::grouped;
use super::super::files::{Phase, run};
use super::super::motion::{Follow, tell};
use super::super::press::{available, on_primary};
use super::editor::RuleEditor;
use super::work::{self, Draft, Listed, Step};

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
    let (rules, failed) = match work::listed(&store, account.clone()) {
        Ok(rules) => (rules, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let count = rules.len();
    let items: Vec<ListItem<RuleId>> = rules
        .into_iter()
        .enumerate()
        .map(|(at, listed)| {
            let id = listed.rule.id;
            let name = listed.rule.name.clone();
            let row = rsx! {
                RuleRow {
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
            };
            ListItem::row(id, name, row)
        })
        .collect();
    rsx! {
        section { class: "rules-part",
            SectionHeader { title: "Rules".to_owned() }
            Label {
                text: "Applied to new mail, in order.".to_owned(),
                role: LabelRole::Secondary,
                style: LabelStyle::Footnote,
            }
            List::<RuleId> {
                label: "Rules".to_owned(),
                items,
                style: ListStyle::Inset,
            }
            if count == 0 {
                Label {
                    text: failed.unwrap_or_else(|| "No rules on this account yet.".to_owned()),
                    role: LabelRole::Tertiary,
                }
            }
            RunProgress { phase: phase(), name: running() }
            match said() {
                Some(Ok(text)) => rsx! {
                    div { role: "status", Label { text, role: LabelRole::Secondary } }
                },
                Some(Err(why)) => rsx! {
                    div { role: "alert", Label { text: why, role: LabelRole::Primary, style: LabelStyle::Headline } }
                },
                None => rsx! {},
            }
            if editing.read().is_some() {
                RuleEditor { account, editing, changed, said }
            } else {
                div { class: "rules-acts",
                    Button {
                        label: "New rule".to_owned(),
                        common: Common { aria_label: Some("New rule".to_owned()), ..Common::default() },
                        icon: Icon::Plus,
                        onclick: on_primary(move || editing.set(Some(Draft::blank()))),
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
    let busy = phase.read().running();
    let write = move |done: Result<(), String>| {
        let (mut said, mut changed) = (said, changed);
        if let Err(why) = done {
            said.set(Some(Err(why)));
        }
        changed += 1;
    };
    let toggled = rule.clone();
    let (for_up, for_down) = (rule.account.clone(), rule.account.clone());
    let up = rule.id;
    let down = rule.id;
    let run_rule = rule.clone();
    let gone = rule.clone();
    let edit = listed.clone();
    let acts = rsx! {
        Toggle {
            label: format!("{name} on"),
            value: if on { Check::On } else { Check::Off },
            onchange: move |_| {
                let store = consume_context::<Arc<SqliteStore>>();
                let state = if on { RuleState::Disabled } else { RuleState::Enabled };
                write(work::switch(&store, &toggled, state));
            },
        }
        Button {
            bezel: Bezel::Toolbar,
            image: ImagePosition::Only,
            icon: Icon::ChevronUp,
            label: format!("Move {name} up"),
            availability: available(!first),
            onclick: on_primary(move || {
                let store = consume_context::<Arc<SqliteStore>>();
                write(work::reorder(&store, for_up.clone(), up, Step::Up));
            }),
        }
        Button {
            bezel: Bezel::Toolbar,
            image: ImagePosition::Only,
            icon: Icon::ChevronDown,
            label: format!("Move {name} down"),
            availability: available(!last),
            onclick: on_primary(move || {
                let store = consume_context::<Arc<SqliteStore>>();
                write(work::reorder(&store, for_down.clone(), down, Step::Down));
            }),
        }
        Button {
            bezel: Bezel::Toolbar,
            image: ImagePosition::Only,
            icon: Icon::Pen,
            label: format!("Edit {name}"),
            onclick: on_primary(move || {
                let mut editing = editing;
                editing.set(Some(Draft::of(&edit)));
            }),
        }
        Button {
            bezel: Bezel::Toolbar,
            image: ImagePosition::Only,
            icon: Icon::Play,
            label: format!("Run {name} on existing mail"),
            title: "Run on existing mail".to_owned(),
            availability: available(!busy),
            onclick: on_primary(move || {
                let store = consume_context::<Arc<SqliteStore>>();
                let of = work::conversations(&store, run_rule.account.clone(), Utc::now());
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
            }),
        }
        Button {
            bezel: Bezel::Toolbar,
            image: ImagePosition::Only,
            role: ButtonRole::Destructive,
            icon: Icon::Trash,
            label: format!("Delete {name}"),
            onclick: on_primary(move || {
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
            }),
        }
    };
    rsx! {
        Row {
            title: name.clone(),
            detail: Some(TextLine::Runs(vec![
                TextRun::new(listed.when.clone(), RunTone::Strong),
                TextRun::new(format!("  {}", listed.does), RunTone::Faint),
            ])),
            accessory: Accessory::Slot(acts),
        }
    }
}

/// "Run on existing mail" while it runs, and what it said.
#[component]
pub(super) fn RunProgress(phase: Phase, name: String) -> Element {
    match phase {
        Phase::Ready => rsx! {},
        Phase::Running { done, of } => {
            let share = (done.min(of) * 1000).checked_div(of).unwrap_or(0);
            rsx! {
                div { class: "files-progress", role: "status",
                    Label {
                        text: format!("Running “{name}”… {} of {} conversations", grouped(done), grouped(of)),
                        role: LabelRole::Secondary,
                    }
                    ProgressIndicator {
                        style: ProgressStyle::Bar,
                        progress: Progress::Known(Fraction(u16::try_from(share).unwrap_or(1000))),
                    }
                }
            }
        }
        Phase::Finished(said) => rsx! {
            div { role: "status", Label { text: said, role: LabelRole::Secondary } }
        },
        Phase::Failed(why) => rsx! {
            div { role: "alert", Label { text: why, role: LabelRole::Primary, style: LabelStyle::Headline } }
        },
    }
}
