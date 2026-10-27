//! A rule's page, pushed over the Rules list: a name, its conditions ([`super::conditions`]), the
//! actions as rows chosen from the one [`Menu`], whether later rules still run, and whether to apply
//! it to the mail already here. Save keeps it, puts the account's rules on its server, starts the
//! run when asked and goes back to the list; the back button leaves it as it was.

use dioxus::prelude::*;
use ds::components::content::avatar::AvatarSize;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::segmented::Tracking;
use ds::components::fields::field_row::FieldRow;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::render::Glyph;
use mail_domain::{AfterMatch, Rule, RuleAction};
use porter_core::AccountId;

use super::super::common::classed;
use super::super::data::AccountRow;
use super::super::menu::{Floating, MenuItem, anchor_at, narrowed, palette_groups};
use super::super::move_to::destinations;
use super::super::press::{available, on_primary};
use super::conditions::{self, ConditionRows};
use super::when::Joined;
use super::work::{self, Draft};
use super::{Said, back};
use crate::ui::common::Problem;
use crate::ui::copy;
use crate::ui::view::Shell;
use mail_domain::RuleId;

/// Which menu the "Add an action" button has open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Adding {
    Closed,
    /// What kind of action.
    Kinds,
    /// Which label.
    Labels,
    /// Which folder.
    Folders,
    /// Which address to forward to.
    Forward,
    /// Which template to reply with.
    Reply,
}

fn item(key: &str, icon: Icon, name: &str, help: Option<&str>) -> MenuItem {
    MenuItem {
        help: help.map(str::to_owned),
        ..MenuItem::icon(key, icon, name)
    }
}

/// Every kind of action a rule can take, `folders` saying whether the account has any to move to
/// and `templates` whether it has any to reply with.
pub(in crate::ui) fn kinds(folders: bool, templates: bool) -> Vec<MenuItem> {
    let mut out = vec![item(
        "label",
        Icon::Tag,
        "Label…",
        Some("Put a label on it"),
    )];
    if folders {
        out.push(item("file", Icon::FolderInput, "Move to folder…", None));
    }
    out.extend([
        item(
            "read",
            Icon::MailOpen,
            &copy::menus::rule_action(&RuleAction::MarkRead),
            None,
        ),
        item(
            "unread",
            Icon::Mail,
            &copy::menus::rule_action(&RuleAction::MarkUnread),
            None,
        ),
        item(
            "star",
            Icon::Star,
            &copy::menus::rule_action(&RuleAction::Star),
            None,
        ),
        item(
            "archive",
            Icon::Archive,
            &copy::menus::rule_action(&RuleAction::Archive),
            Some("Out of the inbox"),
        ),
        item(
            "trash",
            Icon::Trash,
            &copy::menus::rule_action(&RuleAction::Trash),
            None,
        ),
        item(
            "spam",
            Icon::OctagonAlert,
            &copy::menus::rule_action(&RuleAction::Spam),
            None,
        ),
        item(
            "notify",
            Icon::Bell,
            copy::menus::RULE_NOTIFY,
            Some(copy::menus::RULE_SENDS_HELP),
        ),
        item(
            "forward",
            Icon::Forward,
            copy::menus::RULE_FORWARD,
            Some(copy::menus::RULE_SENDS_HELP),
        ),
    ]);
    if templates {
        out.push(item(
            "reply",
            Icon::Reply,
            copy::menus::RULE_REPLY,
            Some(copy::menus::RULE_SENDS_HELP),
        ));
    }
    out
}

/// The action a kind's key means, when it needs nothing more.
fn plain(key: &str) -> Option<RuleAction> {
    Some(match key {
        "read" => RuleAction::MarkRead,
        "unread" => RuleAction::MarkUnread,
        "notify" => RuleAction::Notify,
        "star" => RuleAction::Star,
        "archive" => RuleAction::Archive,
        "trash" => RuleAction::Trash,
        "spam" => RuleAction::Spam,
        _ => return None,
    })
}

fn icon_of(action: &RuleAction) -> Icon {
    match action {
        RuleAction::Label(_) => Icon::Tag,
        RuleAction::File(_) => Icon::FolderInput,
        RuleAction::Archive => Icon::Archive,
        RuleAction::Trash => Icon::Trash,
        RuleAction::Spam => Icon::OctagonAlert,
        RuleAction::MarkRead => Icon::MailOpen,
        RuleAction::Star => Icon::Star,
        RuleAction::MarkUnread => Icon::Mail,
        RuleAction::Notify => Icon::Bell,
        RuleAction::Forward(_) => Icon::Forward,
        RuleAction::Reply(_) => Icon::Reply,
    }
}

/// The key of the picker row that adds `action`: its kind, then the name or path. Only a label
/// and a folder are picked this way; the rest are kinds ([`kinds`]).
fn pick_key(action: &RuleAction) -> String {
    match action {
        RuleAction::Label(name) => format!("label:{name}"),
        RuleAction::File(path) => format!("file:{path}"),
        RuleAction::Forward(to) => format!("forward:{to}"),
        RuleAction::Reply(template) => format!("reply:{template}"),
        _ => String::new(),
    }
}

/// The action a picker row's key stands for: the inverse of [`pick_key`].
fn picked(key: &str) -> Option<RuleAction> {
    if let Some(name) = key.strip_prefix("label:") {
        Some(RuleAction::Label(name.to_owned()))
    } else if let Some(to) = key.strip_prefix("forward:") {
        Some(RuleAction::Forward(to.to_owned()))
    } else if let Some(template) = key.strip_prefix("reply:") {
        Some(RuleAction::Reply(template.to_owned()))
    } else {
        key.strip_prefix("file:")
            .map(|path| RuleAction::File(path.to_owned()))
    }
}

/// The label menu's rows: the account's labels, and "Create “…”" for a name it has not.
pub(in crate::ui) fn label_items(names: &[String], typed: &str) -> Vec<MenuItem> {
    let mut out: Vec<MenuItem> = names
        .iter()
        .map(|name| {
            item(
                &pick_key(&RuleAction::Label(name.clone())),
                Icon::Tag,
                name,
                None,
            )
        })
        .collect();
    let typed = typed.trim();
    if !typed.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(typed)) {
        out.push(item(
            &pick_key(&RuleAction::Label(typed.to_owned())),
            Icon::Plus,
            &format!("Create “{typed}”"),
            Some("Made when the rule first acts"),
        ));
    }
    out
}

/// The forward menu's row: the address typed, once it looks like one.
pub(in crate::ui) fn forward_items(typed: &str) -> Vec<MenuItem> {
    let typed = typed.trim();
    if typed.contains('@') && !typed.contains(char::is_whitespace) {
        vec![item(
            &pick_key(&RuleAction::Forward(typed.to_owned())),
            Icon::Forward,
            &copy::menus::forward_to(typed),
            None,
        )]
    } else {
        Vec::new()
    }
}

/// The page of the rule `id` on `row`'s account, or of a new one for `None`.
#[component]
pub(super) fn RulePage(
    shell: Signal<Shell>,
    row: AccountRow,
    id: Option<RuleId>,
    said: Said,
    pushed: Callback<AccountRow>,
    applied: Callback<Rule>,
) -> Element {
    let editing = use_signal(|| {
        let client = crate::ui::live::client();
        let kept = id.and_then(|id| {
            work::listed(&client, row.id.clone())
                .ok()?
                .into_iter()
                .find(|listed| listed.rule.id == id)
        });
        Some(kept.map_or_else(Draft::blank, |listed| {
            Draft::of(&mail_client::rules::ListedRule {
                rule: listed.rule,
                condition: listed.when,
            })
        }))
    });
    let account = row.id.clone();
    rsx! {
        Form {
            RuleEditor {
                account,
                editing,
                on_saved: move |(rule, apply): (Rule, Check)| {
                    pushed.call(row.clone());
                    if apply == Check::On {
                        applied.call(rule);
                    }
                    back(shell);
                },
            }
        }
    }
}

/// The editor for `editing`, while it holds a draft. `on_saved` hears a rule kept, and whether it
/// is to be applied to the mail already here.
#[component]
fn RuleEditor(
    account: AccountId,
    editing: Signal<Option<Draft>>,
    on_saved: EventHandler<(Rule, Check)>,
) -> Element {
    let opened = use_hook(|| {
        conditions::opened(
            &editing
                .peek()
                .as_ref()
                .map(|draft| draft.query.clone())
                .unwrap_or_default(),
        )
    });
    let writing = use_signal(|| opened.0);
    let joined = use_signal(|| opened.1);
    let rows = use_signal(|| opened.2.clone());
    let mut apply = use_signal(|| Check::Off);
    let mut adding = use_signal(|| Adding::Closed);
    // The Add button, which the action menu floats against.
    let mut add_at = use_signal(|| None::<MountedRef>);
    let mut refused = use_signal(|| None::<String>);
    let Some(draft) = editing() else {
        return rsx! {};
    };
    let client = crate::ui::live::client();
    let index = client.rules_manage().labels(account.clone());
    let names: Vec<String> = index.iter().map(|(name, _)| name.clone()).collect();
    let folders: Vec<String> = destinations(&client, account.clone())
        .into_iter()
        .map(|d| d.path)
        .collect();
    let vips = client.query().vips();
    let templates = client.rules_manage().templates(account.clone());
    // Read as it is written: a condition that cannot be read is its field's error, and Save waits
    // for one that can, with every row's value in.
    let filter = work::read_condition(&draft.query, &index, &vips, &chrono::Local).ok();
    let readable = conditions::ready(writing(), &rows(), filter.is_some());
    let matching = filter
        .filter(|_| readable)
        .and_then(|filter| work::matching(&client, account.clone(), filter, chrono::Utc::now()));
    let mut add = move |action: RuleAction| {
        if let Some(draft) = editing.write().as_mut()
            && !draft.actions.contains(&action)
        {
            draft.actions.push(action);
        }
        adding.set(Adding::Closed);
    };
    let kinds_open = adding() == Adding::Kinds;
    rsx! {
        FormSection {
            FieldRow {
                label: "Name",
                TextField {
                    label: "Name".to_owned(),
                    value: draft.name.clone(),
                    placeholder: "Bills".to_owned(),
                    common: classed("rules-field"),
                    oninput: move |value: String| {
                        if let Some(draft) = editing.write().as_mut() {
                            draft.name = value;
                        }
                    },
                }
            }
        }
        FormSection { title: Some(match joined() {
                Joined::All => copy::fields::WHEN_ALL_MATCH,
                Joined::Any => copy::fields::WHEN_ANY_MATCH,
            }.to_owned()),
            ConditionRows {
                writing,
                joined,
                rows,
                query: draft.query.clone(),
                labels: index.clone(),
                on_query: move |query: String| {
                    if let Some(draft) = editing.write().as_mut() {
                        draft.query = query;
                    }
                },
            }
        }
        // What the rule does, a line each, then the line that adds one.
        FormSection { title: Some("Do".to_owned()),
                    for (at, action) in draft.actions.iter().enumerate() {
                        div { key: "{at}", class: "rules-line",
                            Glyph { icon: icon_of(action), size: IconSize::Compact }
                            Label { text: copy::menus::rule_action(action) }
                            Button {
                                bezel: Bezel::Toolbar,
                                image: ImagePosition::Only,
                                icon: Icon::Minus,
                                label: format!("Remove {}", copy::menus::rule_action(action)),
                                title: Some("Remove".to_owned()),
                                common: classed("rules-end"),
                                onclick: on_primary(move || {
                                    if let Some(draft) = editing.write().as_mut()
                                        && at < draft.actions.len()
                                    {
                                        draft.actions.remove(at);
                                    }
                                }),
                            }
                        }
                    }
                div { class: "rules-line",
                    Button {
                        label: copy::aria::ADD_ACTION,
                        icon: Icon::Plus,
                        shown: Some(if kinds_open { Shown::Visible } else { Shown::Hidden }),
                        onclick: on_primary(move || {
                            let next = if adding() == Adding::Closed { Adding::Kinds } else { Adding::Closed };
                            adding.set(next);
                        }),
                        common: Common {
                            aria_label: Some(copy::aria::ADD_ACTION.to_owned()),
                            mounted: Some(EventHandler::new(move |event: MountedEvent| add_at.set(Some(MountedRef(event.data()))))),
                            ..Common::default()
                        },
                    }
                    // What kind of action opens a menu; a label or a folder is then chosen by
                    // typing, in a picker, so the menu closes on the kind it hands over.
                    if kinds_open {
                        Floating {
                            placement: MenuPlacement::Popup,
                            anchor: add_at(),
                            title: copy::aria::ADD_ACTION.to_owned(),
                            items: kinds(!folders.is_empty(), !templates.is_empty()),
                            on_pick: move |key: String| {
                                if key == "label" {
                                    adding.set(Adding::Labels);
                                } else if key == "file" {
                                    adding.set(Adding::Folders);
                                } else if key == "forward" {
                                    adding.set(Adding::Forward);
                                } else if key == "reply" {
                                    adding.set(Adding::Reply);
                                } else if let Some(action) = plain(&key) {
                                    add(action);
                                }
                            },
                            on_close: move |_| {
                                if adding() == Adding::Kinds {
                                    adding.set(Adding::Closed);
                                }
                            },
                        }
                    }
                    if !matches!(adding(), Adding::Closed | Adding::Kinds) {
                        ActionPicker {
                            anchor: add_at(),
                            adding: adding(),
                            names: names.clone(),
                            folders: folders.clone(),
                            templates: templates.clone(),
                            on_pick: move |key: String| {
                                if let Some(action) = picked(&key) {
                                    add(action);
                                }
                            },
                            on_close: move |()| adding.set(Adding::Closed),
                        }
                    }
                }
        }
        FormSection {
            FieldRow {
                label: "After it matches",
                SegmentedControl::<AfterMatch> {
                    label: "After it matches".to_owned(),
                    choices: vec![
                        Choice::new(AfterMatch::Continue, "Later rules run too"),
                        Choice::new(AfterMatch::Stop, "No later rule runs"),
                    ],
                    tracking: Tracking::SelectOne(draft.after),
                    onchange: move |after: AfterMatch| {
                        if let Some(draft) = editing.write().as_mut() {
                            draft.after = after;
                        }
                    },
                }
            }
            FieldRow {
                label: copy::fields::apply_to_existing(matching),
                Toggle {
                    label: copy::fields::apply_to_existing(matching),
                    value: apply(),
                    onchange: move |next: Check| apply.set(next),
                }
            }
        }
        // Save stands alone under the groups, at their end, as a Space's Create Space does.
        div { class: "acct-actions",
                Button {
                    label: copy::controls::SAVE,
                    availability: available(readable),
                    onclick: on_primary(move || {
                        let Some(draft) = editing.peek().clone() else { return };
                        let client = crate::ui::live::client();
                        match work::save(&client, account.clone(), &draft, &chrono::Local) {
                            Ok(rule) => {
                                refused.set(None);
                                on_saved.call((rule, apply()));
                            }
                            Err(why) => refused.set(Some(why)),
                        }
                    }),
                    common: Common { aria_label: Some(copy::aria::SAVE_RULE.to_owned()), ..Common::default() },
                }
        }
        if let Some(why) = refused() {
            Problem { title: copy::titles::NOT_SAVED, why, onclose: move |()| refused.set(None) }
        }
    }
}

/// Which label or folder an action names: quire's `PickList` under the Add button, the rows
/// narrowed as the person types. A label typed that the account lacks is offered as "Create".
#[component]
fn ActionPicker(
    anchor: Option<MountedRef>,
    adding: Adding,
    names: Vec<String>,
    folders: Vec<String>,
    templates: Vec<String>,
    on_pick: EventHandler<String>,
    on_close: EventHandler<()>,
) -> Element {
    let mut query = use_signal(String::new);
    let (label, placeholder, items) = match adding {
        Adding::Labels => (
            "Label",
            "Find or create a label",
            label_items(&names, &query()),
        ),
        Adding::Forward => (
            copy::menus::FORWARD_TO,
            copy::menus::AN_ADDRESS,
            forward_items(&query()),
        ),
        Adding::Reply => (
            copy::menus::REPLY_WITH,
            copy::menus::FIND_A_TEMPLATE,
            templates
                .iter()
                .map(|name| {
                    item(
                        &pick_key(&RuleAction::Reply(name.clone())),
                        Icon::Reply,
                        name,
                        None,
                    )
                })
                .collect(),
        ),
        _ => (
            "Move to",
            "Find a folder",
            folders
                .iter()
                .map(|path| {
                    item(
                        &pick_key(&RuleAction::File(path.clone())),
                        Icon::FolderInput,
                        path,
                        None,
                    )
                })
                .collect(),
        ),
    };
    let shown = narrowed(&items, &query());
    rsx! {
        PickList::<String> {
            anchor: anchor_at(anchor),
            label,
            placeholder,
            query: query(),
            groups: palette_groups(&shown, AvatarSize::Size22, None),
            empty: "Nothing matches.".to_owned(),
            oninput: move |text: String| query.set(text),
            onpick: on_pick,
            onclose: on_close,
        }
    }
}
