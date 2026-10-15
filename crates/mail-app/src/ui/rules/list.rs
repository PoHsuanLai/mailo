//! The root of the Rules page: the account chosen, when there are several; its rules in the order
//! they run, a row each with its switch and its name, then New Rule…; how a run on existing mail
//! goes; and the vacation reply's row.
//!
//! A rule's row opens its page. Its right-click menu runs it on the mail already here, moves it
//! up or down the order (quire's lists have no drag to reorder), or deletes it.

use chrono::Utc;
use dioxus::prelude::*;
use ds::base::press::{PointerButton, Press};
use ds::components::controls::progress::model::{Progress, ProgressStyle};
use ds::components::controls::progress::view::ProgressIndicator;
use ds::components::fields::field_row::FieldRow;
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
use ds::components::menus::item::item::MenuItem as Choice;
use ds::components::menus::pop_up_button::PopUpButton;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::family::PlateFamily;
use mail_core::SqliteStore;
use mail_domain::{Rule, RuleState};
use porter_core::AccountId;
use std::sync::Arc;

use super::super::common::{classed, tile};
use super::super::data::AccountRow;
use super::super::files::work::grouped;
use super::super::files::{Phase, run};
use super::super::menu::{Floating, MenuItem, Right, Tile};
use super::super::motion::{Follow, tell};
use super::work::{self, Listed, Step};
use super::{Failed, Said, open, row_id};
use crate::ui::copy;
use crate::ui::view::{RulesAt, RulesPage as Showing, Shell};

/// The keys of the list's rows that are not a rule's.
const NEW: &str = " new";
const NONE: &str = " none";

/// The list, for `row` (`None` before there is an account), `rows` being every account.
#[component]
pub(super) fn RulesList(
    shell: Signal<Shell>,
    revision: Signal<u64>,
    row: Option<AccountRow>,
    rows: Vec<AccountRow>,
    said: Said,
    changed: Signal<u64>,
    pushed: Callback<AccountRow>,
) -> Element {
    let _ = changed();
    let phase = use_signal(|| Phase::Ready);
    let running = use_signal(String::new);
    // The rule whose menu is open, and where the right click was.
    let menu = use_signal(|| None::<(Listed, Rect)>);
    let several = rows.len() > 1;
    // What a rule's menu asks, done here: a run outlives the menu, which closes as it is picked.
    let acted = use_callback({
        let row = row.clone();
        move |(key, rule): (String, Rule)| {
            if let Some(row) = &row {
                act(
                    &key, &rule, row, said, changed, pushed, phase, running, revision,
                );
            }
        }
    });
    let Some(row) = row else {
        return rsx! {
            Form {
                FormSection {
                    FieldRow { label: copy::empty::ADD_AN_ACCOUNT_FIRST }
                }
            }
        };
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let (rules, failed) = match work::listed(&store, row.id.clone()) {
        Ok(rules) => (rules, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let mut items: Vec<ListItem<String>> = Vec::new();
    if let Some(failed) = failed {
        items.push(ListItem::row(
            NONE.to_owned(),
            failed.clone(),
            rsx! { Row { title: failed, size: RowSize::Settings } },
        ));
    }
    for listed in rules {
        let key = listed.rule.id.to_string();
        let name = listed.rule.name.clone();
        items.push(ListItem::row(
            key.clone(),
            name,
            rsx! {
                RuleRow {
                    key: "{key}",
                    listed,
                    shell,
                    row: row.clone(),
                    said,
                    changed,
                    pushed,
                    menu,
                }
            },
        ));
    }
    let new_id = row_id(&RulesAt::Rule(None));
    items.push(ListItem::row(
        NEW.to_owned(),
        copy::controls::NEW_RULE.to_owned(),
        rsx! {
            Row {
                leading: tile(Icon::Plus, PlateFamily::Blue),
                title: copy::controls::NEW_RULE,
                size: RowSize::Settings,
                accessory: Accessory::Chevron,
                onclick: move |_| open(shell, RulesAt::Rule(None)),
                common: Common { id: new_id, aria_label: Some(copy::controls::NEW_RULE.to_owned()), ..Common::default() },
            }
        },
    ));
    let vacation = vec![ListItem::row(
        "vacation".to_owned(),
        copy::titles::VACATION_REPLY.to_owned(),
        rsx! {
            Row {
                title: copy::titles::VACATION_REPLY,
                size: RowSize::Settings,
                accessory: Accessory::Chevron,
                onclick: move |_| open(shell, RulesAt::Vacation),
                common: Common { id: row_id(&RulesAt::Vacation), ..Common::default() },
            }
        },
    )];
    let accounts: Vec<Choice<AccountId>> = rows
        .iter()
        .map(|row| Choice::new(row.id.clone(), row.shown()))
        .collect();
    rsx! {
        Form {
            Failed { said }
            if several {
                FormSection {
                    FieldRow { label: copy::fields::ACCOUNT,
                        PopUpButton::<AccountId> {
                            items: accounts,
                            value: Some(row.id.clone()),
                            title: Some(copy::fields::ACCOUNT.to_owned()),
                            onpick: move |id: AccountId| {
                                let mut said = said;
                                said.set(None);
                                let mut shell = shell;
                                shell.write().rules = Showing { account: Some(id), ..Showing::default() };
                            },
                        }
                    }
                }
            }
            FormSection { footer: Some(copy::fields::RULES_RUN_IN_ORDER.to_owned()),
                List::<String> { label: copy::titles::RULES, items, style: ListStyle::Grouped }
            }
            RunProgress { phase: phase(), name: running() }
            FormSection {
                List::<String> { label: copy::titles::VACATION_REPLY, items: vacation, style: ListStyle::Grouped }
            }
        }
        if let Some((listed, at)) = menu() {
            RuleMenu { listed, at, row, menu, phase, acted }
        }
    }
}

/// One rule: its name, and its switch. A press opens its page; a right click, its menu.
#[component]
fn RuleRow(
    listed: Listed,
    shell: Signal<Shell>,
    row: AccountRow,
    said: Said,
    changed: Signal<u64>,
    pushed: Callback<AccountRow>,
    menu: Signal<Option<(Listed, Rect)>>,
) -> Element {
    let rule = listed.rule.clone();
    let on = rule.state == RuleState::Enabled;
    let id = rule.id;
    let toggled = rule.clone();
    let opened = listed.clone();
    rsx! {
        Row {
            title: rule.name.clone(),
            size: RowSize::Settings,
            accessory: Accessory::Toggle {
                value: if on { Check::On } else { Check::Off },
                on_toggle: EventHandler::new(move |_| {
                    let store = consume_context::<Arc<SqliteStore>>();
                    let state = if on { RuleState::Disabled } else { RuleState::Enabled };
                    wrote(work::switch(&store, &toggled, state), &row, said, changed, pushed);
                }),
            },
            onclick: move |press: Press| {
                if press.button == PointerButton::Secondary {
                    let mut menu = menu;
                    menu.set(Some((opened.clone(), Rect { origin: press.at, size: Size::default() })));
                } else {
                    open(shell, RulesAt::Rule(Some(id)));
                }
            },
            common: Common { id: row_id(&RulesAt::Rule(Some(id))), ..Common::default() },
        }
    }
}

/// A write's end: a failure said, the list read again, and the script put on the server.
pub(super) fn wrote(
    done: Result<(), String>,
    row: &AccountRow,
    mut said: Said,
    mut changed: Signal<u64>,
    pushed: Callback<AccountRow>,
) {
    match done {
        Ok(()) => pushed.call(row.clone()),
        Err(why) => said.set(Some(Err(why))),
    }
    changed += 1;
}

/// The keys of a rule's menu.
const RUN: &str = "run";
const UP: &str = "up";
const DOWN: &str = "down";
const DELETE: &str = "delete";

fn item(key: &str, icon: Icon, name: &str) -> MenuItem {
    MenuItem {
        key: key.to_owned(),
        tile: Tile::Icon(icon),
        name: name.to_owned(),
        help: None,
        right: Right::None,
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    }
}

/// A rule's menu's rows: Move Up and Move Down only where it can move.
pub(super) fn rule_items(first: bool, last: bool) -> Vec<MenuItem> {
    let mut items = vec![item(RUN, Icon::Play, copy::menus::RUN_ON_EXISTING_MAIL)];
    if !first {
        items.push(item(UP, Icon::ChevronUp, copy::menus::MOVE_UP));
    }
    if !last {
        items.push(item(DOWN, Icon::ChevronDown, copy::menus::MOVE_DOWN));
    }
    items.push(item(DELETE, Icon::Trash, copy::menus::DELETE_RULE));
    items
}

/// The menu a right click on a rule opens, at the pointer; `acted` does what is picked.
#[component]
fn RuleMenu(
    listed: Listed,
    at: Rect,
    row: AccountRow,
    menu: Signal<Option<(Listed, Rect)>>,
    phase: Signal<Phase>,
    acted: Callback<(String, Rule)>,
) -> Element {
    let store = consume_context::<Arc<SqliteStore>>();
    let rules = work::listed(&store, row.id.clone()).unwrap_or_default();
    let place = rules.iter().position(|one| one.rule.id == listed.rule.id);
    let first = place == Some(0);
    let last = place.is_some_and(|at| at + 1 == rules.len());
    let mut items = rule_items(first, last);
    // A run is one at a time.
    if phase.read().running() {
        items.retain(|item| item.key != RUN);
    }
    let rule = listed.rule.clone();
    rsx! {
        Floating {
            anchor: None,
            placed: Some(at),
            title: rule.name.clone(),
            items,
            on_pick: move |key: String| {
                let mut menu = menu;
                menu.set(None);
                acted.call((key, rule.clone()));
            },
            on_close: move |()| {
                let mut menu = menu;
                menu.set(None);
            },
        }
    }
}

/// Do what `key` in a rule's menu asks.
#[allow(clippy::too_many_arguments)]
fn act(
    key: &str,
    rule: &Rule,
    row: &AccountRow,
    said: Said,
    changed: Signal<u64>,
    pushed: Callback<AccountRow>,
    phase: Signal<Phase>,
    running: Signal<String>,
    mut revision: Signal<u64>,
) {
    let store = consume_context::<Arc<SqliteStore>>();
    match key {
        RUN => {
            let of = work::conversations(&store, rule.account.clone(), Utc::now());
            let rule = rule.clone();
            let mut running = running;
            running.set(rule.name.clone());
            run(
                phase,
                of,
                move |report| work::run(&store, &rule, Utc::now(), report),
                move || revision += 1,
            );
        }
        UP | DOWN => {
            let step = if key == UP { Step::Up } else { Step::Down };
            let moved = work::reorder(&store, rule.account.clone(), rule.id, step);
            wrote(moved, row, said, changed, pushed);
        }
        DELETE => {
            let deleted = work::delete(&store, rule).map(|text| tell(text, Follow::Nothing));
            wrote(deleted, row, said, changed, pushed);
        }
        _ => {}
    }
}

/// "Run on existing mail" while it runs, and what it said.
#[component]
pub(super) fn RunProgress(phase: Phase, name: String) -> Element {
    let line = match phase {
        Phase::Ready => return rsx! {},
        Phase::Running { done, of } => {
            let share = (done.min(of) * 1000).checked_div(of).unwrap_or(0);
            rsx! {
                FieldRow {
                    label: format!("Running “{name}”…"),
                    help: Some(TextLine::from(format!("{} of {} conversations", grouped(done), grouped(of)))),
                    ProgressIndicator {
                        style: ProgressStyle::Bar,
                        progress: Progress::Known(Fraction(u16::try_from(share).unwrap_or(1000))),
                        common: classed("rules-progress"),
                    }
                }
            }
        }
        Phase::Finished(said) => rsx! {
            FieldRow { label: format!("Ran “{name}”"), help: Some(TextLine::from(said)) }
        },
        Phase::Failed(why) => rsx! {
            FieldRow { label: format!("“{name}” did not run"), help: Some(TextLine::from(why)) }
        },
    };
    rsx! {
        FormSection { {line} }
    }
}
