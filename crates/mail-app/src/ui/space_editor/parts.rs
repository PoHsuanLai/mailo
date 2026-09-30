//! The editor's own control around quire's: the provider marks.

use crate::appearance::WindowDirs;
use crate::view::{Appearance, Marks as MarksKind, Shell};
use dioxus::prelude::*;
use ds::components::controls::button_model::Bezel;
use ds::components::controls::segmented::Tracking;
use ds::components::fields::field_row::FieldRow;
use ds::prelude::{Button, Choice, SegmentedControl};

/// Provider marks: the window's, not the Space's, so a choice here is kept at once.
#[component]
pub(super) fn Marks(shell: Signal<Shell>) -> Element {
    let now = shell.read().appearance.marks;
    rsx! {
        FieldRow {
            label: "Provider marks",
            SegmentedControl::<MarksKind> {
                label: "Provider marks",
                choices: MarksKind::ALL.into_iter().map(|marks| Choice::new(marks, marks.label())).collect::<Vec<_>>(),
                tracking: Tracking::SelectOne(now),
                onchange: move |marks| {
                    let look = Appearance { marks };
                    shell.write().appearance = look;
                    if let Some(dirs) = try_consume_context::<WindowDirs>() {
                        let _ = crate::appearance::save(&dirs.config, look);
                    }
                },
            }
            Button {
                bezel: Bezel::Inline,
                label: "Refresh icons",
                onclick: super::super::press::on_primary(refresh_icons),
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
