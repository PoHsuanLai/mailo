//! The root of the Rules page: the account chosen, when there are several; its rules in the order
//! they run, a row each with its switch and its name, then New Rule…; and how a rule being applied
//! to the mail already here goes. On an account whose server cannot run rules, one line says they
//! run only while mailo is open.
//!
//! A rule's row opens its page. Its right-click menu moves it up or down the order (quire's lists
//! have no drag to reorder), or deletes it.

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
use mail_domain::{Rule, RuleState};
use porter_core::AccountId;

use super::super::common::{add_mark, classed};
use super::super::data::AccountRow;
use super::super::files::Phase;
use super::super::menu::{Floating, MenuItem};
use super::super::motion::{Follow, Restore, tell};
use super::server::reach;
use super::work::{self, Listed, Step};
use super::{Said, open, row_id};
use crate::ui::common::Problem;
use crate::ui::copy;
use crate::ui::view::{RulesAt, RulesPage as Showing, Shell};

/// The keys of the list's rows that are not a rule's.
const NEW: &str = " new";
const NONE: &str = " none";

/// The list, for `row` (`None` before there is an account), `rows` being every account.
#[component]
pub(super) fn RulesList(
    shell: Signal<Shell>,
    row: Option<AccountRow>,
    rows: Vec<AccountRow>,
    said: Said,
    pushed: Callback<AccountRow>,
    phase: Signal<Phase>,
    running: Signal<String>,
) -> Element {
    // A write of ours, or the toast's Undo putting a deleted rule back, moves the store.
    let _ = crate::ui::live::moved();
    // The rule whose menu is open, and where the right click was.
    let menu = use_signal(|| None::<(Listed, Rect)>);
    // What a rule's menu asks, done here, as the menu closes on the pick.
    let acted = use_callback({
        let row = row.clone();
        move |(key, rule): (String, Rule)| {
            if let Some(row) = &row {
                act(&key, &rule, row, said, pushed);
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
    let client = crate::ui::live::client();
    let (rules, failed) = match work::listed(&client, row.id.clone()) {
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
                    pushed,
                    menu,
                }
            },
        ));
    }
    let new_id = row_id(&RulesAt::Rule(None));
    // New Rule is a list of its own under the rules, as New Space is under the Spaces.
    let add = vec![ListItem::row(
        NEW.to_owned(),
        copy::controls::NEW_RULE.to_owned(),
        rsx! {
            Row {
                leading: add_mark(),
                title: copy::controls::NEW_RULE,
                size: RowSize::Settings,
                accessory: Accessory::Chevron,
                onclick: move |_| open(shell, RulesAt::Rule(None)),
                common: Common { id: new_id, aria_label: Some(copy::controls::NEW_RULE.to_owned()), ..Common::default() },
            }
        },
    )];
    let empty = items.is_empty();
    // Said only where the server will not run them: elsewhere they run there too, unremarked.
    let here_only = reach(&row.plan)
        .is_err()
        .then(|| copy::status::RULES_RUN_HERE.to_owned());
    rsx! {
        AccountChoice { shell, rows, row: row.clone(), said }
        Form {
            // With no rules yet, New Rule stands alone and says where they would run.
            if !empty {
                FormSection { footer: here_only.clone(),
                    List::<String> { label: copy::titles::RULES, items, style: ListStyle::Grouped }
                }
            }
            FormSection { footer: if empty { here_only } else { None },
                List::<String> { label: copy::aria::ADD_A_RULE, items: add, style: ListStyle::Grouped }
            }
            RunProgress { phase, name: running() }
        }
        if let Some((listed, at)) = menu() {
            RuleMenu { listed, at, row, menu, acted }
        }
    }
}

/// The account whose rules and vacation reply are shown, when there are several: the Rules and
/// Vacation Reply pages share it.
#[component]
pub(super) fn AccountChoice(
    shell: Signal<Shell>,
    rows: Vec<AccountRow>,
    row: AccountRow,
    said: Said,
) -> Element {
    if rows.len() < 2 {
        return rsx! {};
    }
    let accounts: Vec<Choice<AccountId>> = rows
        .iter()
        .map(|row| Choice::new(row.id.clone(), row.shown()))
        .collect();
    rsx! {
        Form {
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
    }
}

/// One rule: its name, and its switch. A press opens its page; a right click, its menu.
#[component]
fn RuleRow(
    listed: Listed,
    shell: Signal<Shell>,
    row: AccountRow,
    said: Said,
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
                    let client = crate::ui::live::client();
                    let state = if on { RuleState::Disabled } else { RuleState::Enabled };
                    wrote(work::switch(&client, &toggled, state), &row, said, pushed);
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

/// A write's end: a failure said, or the script put on the server. The list reads the store
/// again as it moves.
pub(super) fn wrote(
    done: Result<(), String>,
    row: &AccountRow,
    mut said: Said,
    pushed: Callback<AccountRow>,
) {
    match done {
        Ok(()) => pushed.call(row.clone()),
        Err(why) => said.set(Some(Err(why))),
    }
}

/// The keys of a rule's menu.
const UP: &str = "up";
const DOWN: &str = "down";
const DELETE: &str = "delete";

fn item(key: &str, icon: Icon, name: &str) -> MenuItem {
    MenuItem::icon(key, icon, name)
}

/// A rule's menu's rows: Move Up and Move Down only where it can move.
pub(super) fn rule_items(first: bool, last: bool) -> Vec<MenuItem> {
    let mut items = Vec::new();
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
    acted: Callback<(String, Rule)>,
) -> Element {
    let client = crate::ui::live::client();
    let rules = work::listed(&client, row.id.clone()).unwrap_or_default();
    let place = rules.iter().position(|one| one.rule.id == listed.rule.id);
    let first = place == Some(0);
    let last = place.is_some_and(|at| at + 1 == rules.len());
    let items = rule_items(first, last);
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
fn act(
    key: &str,
    rule: &Rule,
    row: &AccountRow,
    said: Said,
    pushed: Callback<AccountRow>,
) {
    let client = crate::ui::live::client();
    match key {
        UP | DOWN => {
            let step = if key == UP { Step::Up } else { Step::Down };
            let moved = work::reorder(&client, rule.account.clone(), rule.id, step);
            wrote(moved, row, said, pushed);
        }
        DELETE => {
            let deleted = work::delete(&client, rule)
                .map(|text| tell(text, Follow::Restore(Restore::Rule(rule.clone()))));
            wrote(deleted, row, said, pushed);
        }
        _ => {}
    }
}

/// A rule applied to the mail already here, while it runs, as a bar; a run that stopped is said in a dialog, which
/// closing puts back to nothing. A run that finished says nothing: what it did is in the mail.
#[component]
pub(super) fn RunProgress(phase: Signal<Phase>, name: String) -> Element {
    match phase() {
        Phase::Ready | Phase::Finished(_) => rsx! {},
        Phase::Running { done, of } => {
            let share = (done.min(of) * 1000).checked_div(of).unwrap_or(0);
            rsx! {
                FormSection {
                    FieldRow {
                        label: format!("Running “{name}”…"),
                        ProgressIndicator {
                            style: ProgressStyle::Bar,
                            progress: Progress::Known(Fraction(u16::try_from(share).unwrap_or(1000))),
                            common: classed("rules-progress"),
                        }
                    }
                }
            }
        }
        Phase::Failed(why) => {
            let mut phase = phase;
            rsx! {
                Problem {
                    title: copy::titles::did_not_run(&name),
                    why,
                    onclose: move |()| phase.set(Phase::Ready),
                }
            }
        }
    }
}
