//! A rule's condition on its page: rows built from menus ([`super::when`]), every one having to
//! match or any one of them, or the search language as text for a condition the rows cannot
//! hold. Either way the rule keeps the text; the rows write it as they change.

use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::segmented::Tracking;
use ds::components::fields::text_field_model::Invalid;
use ds::components::menus::item::item::MenuItem as Choice;
use ds::components::menus::pop_up_button::PopUpButton;
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use ds::root::common::Common;
use mail_domain::LabelId;

use super::super::common::classed;
use super::super::press::{available, on_primary};
use super::when::{self, Condition, Field, Joined, STATES, Test};
use super::work;
use crate::ui::copy;

/// How the condition is being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Writing {
    Rows,
    Text,
}

/// The rows, how they join and the way of writing a rule's page opens on for `query`: rows when
/// they can hold it (one empty From row for a new rule), else text.
pub(super) fn opened(query: &str) -> (Writing, Joined, Vec<Condition>) {
    match when::rows_of(query) {
        Some((joined, rows)) if rows.is_empty() => {
            (Writing::Rows, joined, vec![Field::From.fresh()])
        }
        Some((joined, rows)) => (Writing::Rows, joined, rows),
        None => (Writing::Text, Joined::All, Vec::new()),
    }
}

/// Whether the condition as written can be kept: every row has its value and the text reads.
pub(super) fn ready(writing: Writing, rows: &[Condition], readable: bool) -> bool {
    readable && (writing == Writing::Text || when::complete(rows))
}

/// The condition's rows, a line each in the group the page puts them in, or its text, with the
/// switch between them on the last line. `query` is the text; every
/// change is handed to `on_query`.
#[component]
pub(super) fn ConditionRows(
    writing: Signal<Writing>,
    joined: Signal<Joined>,
    rows: Signal<Vec<Condition>>,
    query: String,
    labels: Vec<(String, LabelId)>,
    on_query: EventHandler<String>,
) -> Element {
    let names: Vec<String> = labels.iter().map(|(name, _)| name.clone()).collect();
    let mut set = move |next: Vec<Condition>| {
        on_query.call(when::query_of(&next, *joined.peek()));
        rows.set(next);
    };
    if writing() == Writing::Text {
        // Whether it reads, not what it matches: `is:vip` is a term whoever the VIPs are.
        let vips = mail_core::vip::Vips::default();
        let validity = match work::read_condition(&query, &labels, &vips, &chrono::Local) {
            Err(why) => refusal(&why),
            Ok(_) => Validity::Valid,
        };
        let back = when::rows_of(&query);
        return rsx! {
            div { class: "rules-line",
                TextField {
                    label: copy::fields::CONDITION.to_owned(),
                    value: query.clone(),
                    placeholder: copy::fields::CONDITION_EXAMPLE.to_owned(),
                    validity,
                    common: classed("rules-value"),
                    oninput: move |value: String| on_query.call(value),
                }
            }
            div { class: "rules-line",
                Button {
                    label: copy::controls::EDIT_AS_CONDITIONS,
                    common: Common { aria_label: Some(copy::controls::EDIT_AS_CONDITIONS.to_owned()), ..classed("rules-end") },
                    bezel: Bezel::Inline,
                    availability: available(back.is_some()),
                    onclick: on_primary(move || {
                        let (_, how, opened) = opened(&query);
                        joined.set(how);
                        rows.set(opened);
                        writing.set(Writing::Rows);
                    }),
                }
            }
        };
    }
    let shown = rows();
    let several = shown.len() > 1;
    rsx! {
                if several {
                    div { class: "rules-line",
                        SegmentedControl::<Joined> {
                            label: copy::fields::MATCHES.to_owned(),
                            choices: vec![
                                ds::prelude::Choice::new(Joined::All, copy::fields::ALL_OF_THEM),
                                ds::prelude::Choice::new(Joined::Any, copy::fields::ANY_OF_THEM),
                            ],
                            tracking: Tracking::SelectOne(joined()),
                            common: Common { id: Some("rule-joined".to_owned()), ..Common::default() },
                            onchange: move |how: Joined| {
                                joined.set(how);
                                on_query.call(when::query_of(&rows.peek(), how));
                            },
                        }
                    }
                }
                for (at, row) in shown.iter().cloned().enumerate() {
                    ConditionRow {
                        key: "{at}",
                        row: row.clone(),
                        names: names.clone(),
                        validity: row_validity(&row, &labels),
                        removable: several,
                        on_change: move |next: Option<Condition>| {
                            let mut all = rows.peek().clone();
                            match next {
                                Some(next) if at < all.len() => all[at] = next,
                                None if at < all.len() => {
                                    all.remove(at);
                                }
                                _ => {}
                            }
                            set(all);
                        },
                    }
                }
                div { class: "rules-line",
                    Button {
                        label: copy::controls::ADD_CONDITION,
                    common: Common { aria_label: Some(copy::controls::ADD_CONDITION.to_owned()), ..Common::default() },
                        icon: Icon::Plus,
                        onclick: on_primary(move || {
                            let mut all = rows.peek().clone();
                            all.push(Field::Subject.fresh());
                            set(all);
                        }),
                    }
                    Button {
                        label: copy::controls::EDIT_AS_TEXT,
                    common: Common { aria_label: Some(copy::controls::EDIT_AS_TEXT.to_owned()), ..classed("rules-end") },
                        bezel: Bezel::Inline,
                        onclick: on_primary(move || writing.set(Writing::Text)),
                    }
                }
    }
}

/// One row: what it looks at, how, the value, and a way to take it away while there are others.
#[component]
fn ConditionRow(
    row: Condition,
    names: Vec<String>,
    validity: Validity,
    removable: bool,
    on_change: EventHandler<Option<Condition>>,
) -> Element {
    let fields: Vec<Choice<Field>> = Field::ALL
        .iter()
        .map(|field| Choice::new(*field, field.name()))
        .collect();
    let tests: Vec<Choice<Test>> = row
        .field
        .tests()
        .into_iter()
        .map(|(test, words)| Choice::new(test, words))
        .collect();
    let picked = row.clone();
    let tested = row.clone();
    let typed = row.clone();
    let chosen = row.clone();
    let first_label = names.first().cloned();
    rsx! {
        div { class: "rules-line",
            PopUpButton::<Field> {
                items: fields,
                value: Some(row.field),
                title: Some(copy::aria::CONDITION_FIELD.to_owned()),
                common: Common { aria_label: Some(copy::aria::CONDITION_FIELD.to_owned()), ..Common::default() },
                onpick: move |field: Field| {
                    if field == picked.field {
                        return;
                    }
                    let mut next = field.fresh();
                    // A typed value carries over between the fields that take words.
                    if words(field) && words(picked.field) {
                        next.value = picked.value.clone();
                    }
                    if field == Field::Label && let Some(name) = first_label.clone() {
                        next.value = name;
                    }
                    on_change.call(Some(next));
                },
            }
            if !tests.is_empty() {
                PopUpButton::<Test> {
                    items: tests,
                    value: Some(row.test),
                    title: Some(copy::aria::CONDITION_TEST.to_owned()),
                    common: Common { aria_label: Some(copy::aria::CONDITION_TEST.to_owned()), ..Common::default() },
                    onpick: move |test: Test| on_change.call(Some(Condition { test, ..tested.clone() })),
                }
            }
            match row.field {
                Field::Attachment => rsx! {},
                Field::Status => rsx! {
                    PopUpButton::<String> {
                        items: STATES.iter().map(|(state, words)| Choice::new((*state).to_owned(), *words)).collect::<Vec<_>>(),
                        value: Some(row.value.clone()),
                        title: Some(copy::fields::STATUS.to_owned()),
                        onpick: move |value: String| on_change.call(Some(Condition { value, ..chosen.clone() })),
                    }
                },
                Field::Label if !names.is_empty() => rsx! {
                    PopUpButton::<String> {
                        items: names.iter().map(|name| Choice::new(name.clone(), name.clone())).collect::<Vec<_>>(),
                        value: Some(row.value.clone()),
                        title: Some(copy::fields::LABEL.to_owned()),
                        onpick: move |value: String| on_change.call(Some(Condition { value, ..chosen.clone() })),
                    }
                },
                field => rsx! {
                    TextField {
                        label: field.name().to_owned(),
                        value: row.value.clone(),
                        placeholder: match field {
                            Field::Before | Field::After => copy::fields::A_DATE.to_owned(),
                            Field::Size => copy::fields::A_SIZE.to_owned(),
                            Field::Age => copy::fields::AN_AGE.to_owned(),
                            _ => String::new(),
                        },
                        validity,
                        common: classed("rules-value"),
                        oninput: move |value: String| on_change.call(Some(Condition { value, ..typed.clone() })),
                    }
                },
            }
            if removable {
                Button {
                    bezel: Bezel::Toolbar,
                    image: ImagePosition::Only,
                    icon: Icon::Minus,
                    label: copy::aria::REMOVE_CONDITION,
                    common: Common { aria_label: Some(copy::aria::REMOVE_CONDITION.to_owned()), ..classed("rules-end") },
                    onclick: on_primary(move || on_change.call(None)),
                }
            }
        }
    }
}

/// Whether `field` takes typed words, which carry over when the field changes.
fn words(field: Field) -> bool {
    matches!(
        field,
        Field::From | Field::To | Field::Subject | Field::Text
    )
}

/// A row's own refusal: its term read alone, once it has a value.
fn row_validity(row: &Condition, labels: &[(String, LabelId)]) -> Validity {
    match when::term(row).map(|term| {
        work::read_condition(
            &term,
            labels,
            &mail_core::vip::Vips::default(),
            &chrono::Local,
        )
    }) {
        Some(Err(why)) => refusal(&why),
        _ => Validity::Valid,
    }
}

/// A refusal quire's field draws under itself: the words, and one stamp for each phrasing.
pub(super) fn refusal(why: &str) -> Validity {
    Validity::Invalid(Invalid {
        message: why.to_owned().into(),
        stamp: EventStamp(u32::try_from(why.len()).unwrap_or(0)),
    })
}
