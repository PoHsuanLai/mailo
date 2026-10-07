//! New rule and Edit: a name, a condition in the search language read as it is typed, the
//! actions as rows chosen from the one [`Menu`], and whether later rules still run.

use chrono::Utc;
use dioxus::prelude::*;
use ds::components::content::avatar::AvatarSize;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::segmented::Tracking;
use ds::components::fields::field_row::{FieldRow, RowLayout};
use ds::components::fields::text_field_model::Invalid;
use ds::host::measure::MountedRef;
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::render::Glyph;
use mail_domain::{AfterMatch, RuleAction};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

use super::super::menu::{Floating, MenuItem, Right, Tile, anchor_at, narrowed, palette_groups};
use super::super::move_to::destinations;
use super::super::press::{available, on_primary};
use super::work::{self, Draft};

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
        out.push(item("file", Icon::FolderInput, "Move to folder…", None));
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
    // The Add button, which the action menu floats against.
    let mut add_at = use_signal(|| None::<MountedRef>);
    let mut refused = use_signal(|| None::<String>);
    let Some(draft) = editing() else {
        return rsx! {};
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let index = work::labels(&store, account.clone());
    let names: Vec<String> = index.iter().map(|(name, _)| name.clone()).collect();
    let folders: Vec<String> = destinations(&store, account.clone())
        .into_iter()
        .map(|d| d.path)
        .collect();
    // Read as it is typed: the refusal in words, or how much it matches now.
    let (validity, look, readable) =
        match work::read_condition(&draft.query, &index, &chrono::Local) {
            Err(why) => (refusal(&why), String::new(), false),
            Ok(filter) => match work::matching(&store, account.clone(), filter, Utc::now()) {
                Ok(count) => (Validity::Valid, work::matching_words(count), true),
                Err(why) => (refusal(&why), String::new(), true),
            },
        };
    let title = if draft.id.is_some() {
        "Edit rule"
    } else {
        "New rule"
    };
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
        FormSection { title: Some(title.to_owned()),
            FieldRow {
                label: "Name",
                layout: RowLayout::Form,
                TextField {
                    label: "Name".to_owned(),
                    value: draft.name.clone(),
                    placeholder: "Bills".to_owned(),
                    oninput: move |value: String| {
                        if let Some(draft) = editing.write().as_mut() {
                            draft.name = value;
                        }
                    },
                }
            }
            FieldRow {
                label: "When a message matches",
                help: Some(look.into()),
                layout: RowLayout::Form,
                TextField {
                    label: "Condition".to_owned(),
                    value: draft.query.clone(),
                    placeholder: "from:bank.example subject:statement".to_owned(),
                    validity,
                    oninput: move |value: String| {
                        if let Some(draft) = editing.write().as_mut() {
                            draft.query = value;
                        }
                    },
                }
            }
            FieldRow {
                label: "Do",
                layout: RowLayout::Form,
                div { class: "rules-actions",
                    for (at, action) in draft.actions.iter().enumerate() {
                        div { key: "{at}", class: "rules-action",
                            Glyph { icon: icon_of(action), size: IconSize::Small }
                            Label { text: work::action_words(action) }
                            Button {
                                bezel: Bezel::Toolbar,
                                image: ImagePosition::Only,
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
                    Button {
                        label: "Add an action",
                        icon: Icon::Plus,
                        shown: Some(if kinds_open { Shown::Visible } else { Shown::Hidden }),
                        onclick: on_primary(move || {
                            let next = if adding() == Adding::Closed { Adding::Kinds } else { Adding::Closed };
                            adding.set(next);
                        }),
                        common: Common {
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
                            title: "Add an action".to_owned(),
                            items: kinds(!folders.is_empty()),
                            on_pick: move |key: String| {
                                if key == "label" {
                                    adding.set(Adding::Labels);
                                } else if key == "file" {
                                    adding.set(Adding::Folders);
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
                    if matches!(adding(), Adding::Labels | Adding::Folders) {
                        ActionPicker {
                            anchor: add_at(),
                            adding: adding(),
                            names: names.clone(),
                            folders: folders.clone(),
                            on_pick: move |key: String| {
                                if let Some(name) = key.strip_prefix("label:") {
                                    add(RuleAction::Label(name.to_owned()));
                                } else if let Some(path) = key.strip_prefix("file:") {
                                    add(RuleAction::File(path.to_owned()));
                                }
                            },
                            on_close: move |()| adding.set(Adding::Closed),
                        }
                    }
                }
            }
            FieldRow {
                label: "After it matches",
                layout: RowLayout::Form,
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
                label: "",
                help: refused().map(TextLine::from),
                layout: RowLayout::Form,
                Button {
                    label: "Cancel".to_owned(),
                    onclick: on_primary(move || {
                        editing.set(None);
                        refused.set(None);
                    }),
                }
                Button {
                    label: "Save".to_owned(),
                    availability: available(readable),
                    onclick: on_primary(move || {
                        let Some(draft) = editing.peek().clone() else { return };
                        let store = consume_context::<Arc<SqliteStore>>();
                        match work::save(&store, account.clone(), &draft, &chrono::Local) {
                            Ok(rule) => {
                                said.set(Some(Ok(format!("Kept the rule “{}”.", rule.name))));
                                refused.set(None);
                                editing.set(None);
                                changed += 1;
                            }
                            Err(why) => refused.set(Some(why)),
                        }
                    }),
                    common: Common { aria_label: Some(format!("Save {title}")), ..Common::default() },
                }
            }
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
        _ => (
            "Move to",
            "Find a folder",
            folders
                .iter()
                .map(|path| item(&format!("file:{path}"), Icon::FolderInput, path, None))
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

/// A refusal quire's field draws under itself: the words, and one stamp for each phrasing.
fn refusal(why: &str) -> Validity {
    Validity::Invalid(Invalid {
        message: why.to_owned().into(),
        stamp: EventStamp(u32::try_from(why.len()).unwrap_or(0)),
    })
}
