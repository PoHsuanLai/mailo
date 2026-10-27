//! A label's editor: its ⋯ in the sidebar offers Edit Label…, and this popover hangs from the
//! row. It holds the label's colour, none or one of the seven a flag can be, as quire's row of
//! swatches; a pick is kept at once, and Escape or a click outside closes it.
//!
//! The colour is this computer's: no server keeps a label's colour where mailo can write it.

use mail_client::Client;

use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::radio_group::Arrangement;
use ds::components::overlays::popover::Arrow;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use mail_domain::{Change, ChangeId, Hue, Label, LabelId, Patch};

use crate::ui::copy;
use crate::ui::menu::anchor_at;
use crate::ui::motion::{Follow, tell};

/// The label `id`, on whichever account holds it.
pub(super) fn label_of(client: &Client, id: LabelId) -> Option<Label> {
    // No accounts read is no label to edit; the row stays as it is.
    let accounts = client.stored().list_accounts().unwrap_or_default();
    accounts.into_iter().find_map(|account| {
        client
            .stored()
            .labels(account.id)
            .ok()?
            .into_iter()
            .find(|label| label.id == id)
    })
}

/// The colour a label is drawn in, if it has one mailo can draw: an earlier build's `#rrggbb` is
/// none.
pub(super) fn hue_of(label: &Label) -> Option<Hue> {
    label.color.as_deref().and_then(Hue::parse)
}

/// The editor's swatches: none, then the seven in Apple Mail's order.
pub(super) fn choices() -> Vec<Choice<Option<Hue>>> {
    std::iter::once(Choice::new(None, copy::menus::NO_COLOUR).with_swatch("--fill"))
        .chain(Hue::ALL.into_iter().map(|hue| {
            Choice::new(Some(hue), copy::menus::hue(hue)).with_swatch(format!("--tag-{hue}"))
        }))
        .collect()
}

/// Keep `hue` as `label`'s colour, saying only a failure.
fn keep(label: &Label, hue: Option<Hue>) {
    let client = crate::ui::live::client();
    let changed = Label {
        color: hue.map(|hue| hue.as_str().to_owned()),
        ..label.clone()
    };
    let patch = Patch {
        id: ChangeId::generate(),
        changes: vec![Change::LabelUpsert(changed)],
    };
    if let Err(why) = client.stored().apply(label.account.clone(), &patch) {
        tell(why.to_string(), Follow::Nothing);
    }
}

/// The editor for `label`, hung from `anchor`. `on_change` hears each colour kept.
#[component]
pub(super) fn LabelEditor(
    label: Label,
    anchor: Option<MountedRef>,
    on_change: EventHandler<Option<Hue>>,
    on_close: EventHandler<()>,
) -> Element {
    let picked = hue_of(&label);
    let name = label.name.clone();
    rsx! {
        Popover {
            anchor: anchor_at(anchor),
            placement: Placement::new(Side::Bottom, Align::Start),
            gap: Px(4.0),
            arrow: Arrow::None,
            onclose: move |()| on_close.call(()),
            div { class: "label-editor",
                Label { text: name, role: LabelRole::Secondary, style: LabelStyle::Footnote }
                RadioGroup::<Option<Hue>> {
                    label: copy::fields::COLOUR.to_owned(),
                    choices: choices(),
                    value: picked,
                    arrangement: Arrangement::Swatches,
                    onchange: move |hue: Option<Hue>| {
                        keep(&label, hue);
                        on_change.call(hue);
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// None first, then the seven, each painted with its tag colour.
    #[test]
    fn the_swatches_are_none_and_the_seven_colours() {
        let swatches: Vec<(Option<Hue>, Option<String>)> = choices()
            .into_iter()
            .map(|choice| (choice.value, choice.swatch))
            .collect();
        assert_eq!(swatches.len(), 8);
        assert_eq!(swatches[0], (None, Some("--fill".to_owned())));
        assert_eq!(
            swatches[2],
            (Some(Hue::Orange), Some("--tag-orange".to_owned()))
        );
        assert_eq!(
            swatches[7],
            (Some(Hue::Gray), Some("--tag-gray".to_owned()))
        );
    }

    /// A label's colour is a hue's word; an earlier build's `#rrggbb` is no colour.
    #[test]
    fn a_label_wears_a_hue_word_and_nothing_else() {
        let label = |color: Option<&str>| Label {
            id: LabelId::generate(),
            account: mail_domain::testing::account(),
            name: "Work".to_owned(),
            color: color.map(str::to_owned),
            origin: mail_domain::LabelOrigin::User,
        };
        assert_eq!(hue_of(&label(Some("purple"))), Some(Hue::Purple));
        assert_eq!(hue_of(&label(Some("#ff0000"))), None);
        assert_eq!(hue_of(&label(None)), None);
    }
}
