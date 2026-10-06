//! A segmented choice drawn from labelled options, for rows that pick by position.

use dioxus::prelude::*;
use ds::components::controls::segmented::Tracking;
use ds::prelude::{Choice, SegmentedControl};

/// A row of mutually exclusive buttons. The caller's options say which is on, and the
/// pick comes back as that option's position. Quire's `SegmentedControl` is the control.
#[component]
pub(in crate::ui) fn Seg(
    label: String,
    options: Vec<(String, bool)>,
    on_pick: EventHandler<usize>,
) -> Element {
    let value = options.iter().position(|(_, on)| *on).unwrap_or(0);
    let choices = options
        .into_iter()
        .enumerate()
        .map(|(index, (name, _))| Choice::new(index, name))
        .collect::<Vec<_>>();
    rsx! {
        SegmentedControl::<usize> {
            label,
            choices,
            tracking: Tracking::SelectOne(value),
            onchange: move |index| on_pick.call(index),
        }
    }
}
