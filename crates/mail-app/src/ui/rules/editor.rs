//! New rule and Edit: a name, a condition in the search language read as it is typed, the
//! actions as rows chosen from the one [`Menu`], and whether later rules still run.

use chrono::Utc;
use dioxus::prelude::*;
use mail_domain::{AccountId, AfterMatch, RuleAction};
use mail_store::SqliteStore;
use std::sync::Arc;

use super::super::field::{Field, FieldKind};
use super::super::menu::{Menu, MenuItem, Right, Tile};
use super::super::move_to::destinations;
use super::super::press::{available, on_primary};
use super::super::space_editor::Seg;
use super::work::{self, Draft};
use ds::{Glyph, Icon};

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
}

fn item(key: &str, icon: Icon, name: &str, help: Option<&str>) -> MenuItem {
    MenuItem {
        key: key.to_owned(),
        tile: Tile::Icon(icon),
        name: name.to_owned(),
        help: help.map(str::to_owned),
        right: Right::None,
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    }
}

/// Every kind of action a rule can take, `folders` saying whether the account has any to move to.
pub(in crate::ui) fn kinds(folders: bool) -> Vec<MenuItem> {
    let mut out = vec![item(
        "label",
        Icon::Tag,
        "Label…",
        Some("Put a label on it"),
    )];
    if folders {
        out.push(item(
            "file",
            Icon::FolderInput,
            "Move to folder…",
            Some("Out of the inbox, into one of your folders"),
        ));
    }
    out.extend([
        item("read", Icon::MailOpen, "Mark read", None),
        item("star", Icon::Star, "Star", None),
        item(
            "archive",
            Icon::Archive,
            "Archive",
            Some("Out of the inbox"),
        ),
        item("trash", Icon::Trash, "Move to Trash", None),
        item("spam", Icon::OctagonAlert, "Mark as spam", None),
    ]);
    out
}

/// The action a kind's key means, when it needs nothing more.
fn plain(key: &str) -> Option<RuleAction> {
    Some(match key {
        "read" => RuleAction::MarkRead,
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
    }
}

/// The label menu's rows: the account's labels, and "Create “…”" for a name it has not.
pub(in crate::ui) fn label_items(names: &[String], typed: &str) -> Vec<MenuItem> {
    let mut out: Vec<MenuItem> = names
        .iter()
        .map(|name| item(&format!("label:{name}"), Icon::Tag, name, None))
        .collect();
    let typed = typed.trim();
    if !typed.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(typed)) {
        out.push(item(
            &format!("label:{typed}"),
            Icon::Plus,
            &format!("Create “{typed}”"),
            Some("Made when the rule first acts"),
        ));
    }
    out
}

/// The editor for `editing`, while it holds a draft.
#[component]
pub(super) fn RuleEditor(
    account: AccountId,
    editing: Signal<Option<Draft>>,
    changed: Signal<u64>,
    said: Signal<Option<Result<String, String>>>,
) -> Element {
    let mut adding = use_signal(|| Adding::Closed);
    let mut typed = use_signal(String::new);
    let mut refused = use_signal(|| None::<String>);
    let Some(draft) = editing() else {
        return rsx! {};
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let index = work::labels(&store, account);
    let names: Vec<String> = index.iter().map(|(name, _)| name.clone()).collect();
    let folders: Vec<String> = destinations(&store, account)
        .into_iter()
        .map(|d| d.path)
        .collect();
    // Read as it is typed: the refusal in words, or how much it matches now.
    let (look_class, look, readable) =
        match work::read_condition(&draft.query, &index, &chrono::Local) {
            Err(why) => ("rules-look refused", why, false),
            Ok(filter) => match work::matching(&store, account, filter, Utc::now()) {
                Ok(count) => ("rules-look found", work::matching_words(count), true),
                Err(why) => ("rules-look refused", why, true),
            },
        };
    let title = if draft.id.is_some() {
        "Edit rule"
    } else {
        "New rule"
    };
    let stops = draft.after == AfterMatch::Stop;
    let menu = match adding() {
        Adding::Closed => None,
        Adding::Kinds => Some(("Add an action", kinds(!folders.is_empty()), false)),
        Adding::Labels => Some(("Label", label_items(&names, &typed()), true)),
        Adding::Folders => Some((
            "Move to",
            folders
                .iter()
                .map(|path| item(&format!("file:{path}"), Icon::FolderInput, path, None))
                .collect(),
            folders.len() > 7,
        )),
    };
    let mut add = move |action: RuleAction| {
        if let Some(draft) = editing.write().as_mut()
            && !draft.actions.contains(&action)
        {
            draft.actions.push(action);
        }
        adding.set(Adding::Closed);
        typed.set(String::new());
    };
    rsx! {
        div { class: "rules-edit", role: "group", aria_label: "{title}",
            h4 { "{title}" }
            span { class: "files-k", "Name" }
            Field {
                kind: FieldKind::Boxed,
                value: draft.name.clone(),
                placeholder: "Bills".to_owned(),
                extra: Some("rules-in".to_owned()),
                on_input: move |value: String| {
                    if let Some(draft) = editing.write().as_mut() {
                        draft.name = value;
                    }
                },
                on_focus: |_| {},
                on_blur: |_| {},
            }
            span { class: "files-k", "When a message matches" }
            Field {
                kind: FieldKind::Boxed,
                value: draft.query.clone(),
                placeholder: "from:bank.example subject:statement".to_owned(),
                extra: Some("rules-in".to_owned()),
                on_input: move |value: String| {
                    if let Some(draft) = editing.write().as_mut() {
                        draft.query = value;
                    }
                },
                on_focus: |_| {},
                on_blur: |_| {},
            }
            p { class: "{look_class}", aria_live: "polite", "{look}" }
            span { class: "files-k", "Do" }
            ul { class: "rules-actions",
                for (at, action) in draft.actions.iter().enumerate() {
                    li { key: "{at}", class: "rules-action",
                        Glyph { icon: icon_of(action), size: ds::IconSize::Small }
                        span { "{work::action_words(action)}" }
                        ds::IconButton {
                            variant: ds::IconButtonVariant::Strip,
                            icon: Icon::X,
                            label: format!("Remove {}", work::action_words(action)),
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
                li { class: "rules-add",
                    ds::Button {
                        variant: ds::ButtonVariant::Mini,
                        label: "Add an action",
                        icon: Icon::Plus,
                        expanded: if adding() == Adding::Closed { ds::Expanded::Closed } else { ds::Expanded::Open },
                        onclick: on_primary(move || {
                            let next = if adding() == Adding::Closed { Adding::Kinds } else { Adding::Closed };
                            adding.set(next);
                        }),
                    }
                    if let Some((menu_title, items, filterable)) = menu {
                        div { class: "rules-menu",
                            Menu {
                                title: menu_title.to_owned(),
                                items,
                                filterable,
                                on_pick: move |key: String| {
                                    if let Some(name) = key.strip_prefix("label:") {
                                        add(RuleAction::Label(name.to_owned()));
                                    } else if let Some(path) = key.strip_prefix("file:") {
                                        add(RuleAction::File(path.to_owned()));
                                    } else if key == "label" {
                                        adding.set(Adding::Labels);
                                    } else if key == "file" {
                                        adding.set(Adding::Folders);
                                    } else if let Some(action) = plain(&key) {
                                        add(action);
                                    }
                                },
                                on_close: move |_| adding.set(Adding::Closed),
                                on_query: move |value: String| typed.set(value),
                                slim: true,
                                active: None,
                            }
                        }
                    }
                }
            }
            span { class: "files-k", "After it matches" }
            Seg {
                label: "After it matches".to_owned(),
                options: vec![
                    ("Later rules run too".to_owned(), !stops),
                    ("No later rule runs".to_owned(), stops),
                ],
                on_pick: move |index: usize| {
                    if let Some(draft) = editing.write().as_mut() {
                        draft.after = if index == 1 { AfterMatch::Stop } else { AfterMatch::Continue };
                    }
                },
            }
            div { class: "rules-acts",
                if let Some(why) = refused() {
                    p { class: "capnote files-bad", role: "alert", "{why}" }
                }
                ds::Button {
                    variant: ds::ButtonVariant::Mini,
                    label: "Cancel".to_owned(),
                    onclick: on_primary(move || {
                        editing.set(None);
                        refused.set(None);
                    }),
                }
                ds::Button {
                    variant: ds::ButtonVariant::Primary,
                    label: "Save".to_owned(),
                    aria_label: format!("Save {title}"),
                    availability: available(!(!readable)),
                    onclick: on_primary(move || {
                        let Some(draft) = editing.peek().clone() else { return };
                        let store = consume_context::<Arc<SqliteStore>>();
                        match work::save(&store, account, &draft, &chrono::Local) {
                            Ok(rule) => {
                                said.set(Some(Ok(format!("Kept the rule “{}”.", rule.name))));
                                refused.set(None);
                                editing.set(None);
                                changed += 1;
                            }
                            Err(why) => refused.set(Some(why)),
                        }
                    }),
                }
            }
        }
    }
}
