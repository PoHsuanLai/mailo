//! The editor's own controls around quire's: a segment and the provider marks.

use crate::appearance::WindowDirs;
use crate::view::{Appearance, Marks as MarksKind, Shell};
use dioxus::prelude::*;
use ds::SegmentedControl;

/// A row of mutually exclusive buttons, each saying with `aria-pressed` whether it is the one:
/// quire's `SegmentedControl`, choosing by position. The caller's options say which is on.
#[component]
pub(in crate::ui) fn Seg(
    label: String,
    options: Vec<(String, bool)>,
    on_pick: EventHandler<usize>,
) -> Element {
    let value = options.iter().position(|(_, on)| *on).unwrap_or(usize::MAX);
    let options: Vec<(usize, String)> = options
        .into_iter()
        .enumerate()
        .map(|(index, (name, _))| (index, name))
        .collect();
    rsx! {
        SegmentedControl::<usize> { label, options, value, onchange: move |index| on_pick.call(index) }
    }
}

/// Provider marks: the window's, not the Space's, so a choice here is kept at once.
#[component]
pub(super) fn Marks(shell: Signal<Shell>) -> Element {
    let now = shell.read().appearance.marks;
    rsx! {
        div {
            div { class: "ed-label", "Provider marks" }
            SegmentedControl::<MarksKind> {
                label: "Provider marks",
                options: MarksKind::ALL.into_iter().map(|marks| (marks, marks.label().to_owned())).collect::<Vec<_>>(),
                value: now,
                onchange: move |marks| {
                    let look = Appearance { marks };
                    shell.write().appearance = look;
                    if let Some(dirs) = try_consume_context::<WindowDirs>() {
                        let _ = crate::appearance::save(&dirs.config, look);
                    }
                },
            }
            span { class: "marks-refresh",
                ds::Button {
                    variant: ds::ButtonVariant::Quiet,
                    label: "Refresh icons",
                    onclick: super::super::press::on_primary(refresh_icons),
                }
            }
        }
    }
}

/// Fetch every provider's icon again, then show the new ones.
fn refresh_icons() {
    let store = consume_context::<std::sync::Arc<mail_store::SqliteStore>>();
    let icons = try_consume_context::<Signal<crate::provider::icon::Loaded>>();
    spawn(async move {
        let Some(root) = crate::appearance::cache_dir() else {
            eprintln!("provider icon: no cache directory");
            return;
        };
        let dir = root.join("providers");
        let providers = match crate::provider::icon::providers_of(&store) {
            Ok(providers) => providers,
            Err(err) => {
                eprintln!("provider icon: {err}");
                return;
            }
        };
        let results = crate::provider::icon::refresh(&dir, &providers).await;
        for (provider, result) in &results {
            if let Err(err) = result
                && !matches!(err, crate::provider::icon::IconError::Unmapped)
            {
                eprintln!("provider icon: {provider:?}: {err}");
            }
        }
        if let Some(mut icons) = icons {
            icons.set(crate::provider::icon::Loaded::read(&dir));
        }
    });
}
