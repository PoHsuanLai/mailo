//! General: mailo's settings schema, a group per section, and under the groups the sheets that
//! hold more than a switch.
//!
//! Each row is the schema's ([`super::keys`]); what a section adds beside its rows (Refresh icons,
//! whether there is a dictionary, how many brand-logo roots are trusted) is said here, by section
//! title, so the rows themselves stay the ones detent draws.

use super::keys::{SchemaSection, sections};
use crate::settings::{BrandLogos, Spelling};
use crate::ui::appearance::WindowDirs;
use crate::ui::compose::dictionaries;
use crate::ui::press::on_primary;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::controls::button_model::Bezel;
use ds::components::fields::field_row::{FieldGroup, FieldRow};
use ds::prelude::*;
use ds::root::common::Common;

#[component]
pub(super) fn General(shell: Signal<Shell>) -> Element {
    let settings = crate::ui::prefs::current();
    let values =
        toml::Value::try_from(&settings).unwrap_or_else(|_| toml::Value::Table(toml::Table::new()));
    let mut failed = use_signal(|| None::<String>);
    // Read once, as the page opens: the dictionary directories, and the trusted logo roots.
    let found = use_hook(dictionaries);
    let roots = use_hook(|| {
        try_consume_context::<WindowDirs>()
            .map_or(0, |dirs| mail_core::bimi::anchors(&dirs.config).len())
    });
    let missing = match settings.compose.spelling {
        Spelling::On => found.missing(),
        Spelling::Off => None,
    };
    let onedit = move |(path, value): (String, toml::Value)| {
        failed.set(crate::ui::prefs::change_key(&path, value).err());
    };
    rsx! {
        for (title, keys) in sections(&crate::settings::schema()) {
            SchemaSection { key: "{title}", title: title.clone(), keys, values: values.clone(), onedit,
                match title.as_str() {
                    "Mail list" => rsx! {
                        FieldRow { label: "Provider icons",
                            Button {
                                bezel: Bezel::Inline,
                                label: "Refresh Icons",
                                onclick: on_primary(refresh_icons),
                            }
                        }
                    },
                    "Writing" => rsx! {
                        if let Some(missing) = missing.clone() {
                            p { class: "capnote", "{missing}" }
                        }
                    },
                    "Reading" => rsx! {
                        if settings.reading.brand_logos == BrandLogos::On && roots == 0 {
                            p { class: "capnote",
                                "No mark verifying authority's root is installed, so no logo can be verified yet. Roots can be added to {mail_core::bimi::USER_ROOTS} in the config directory."
                            }
                        }
                    },
                    _ => rsx! {},
                }
            }
        }
        if let Some(why) = failed() {
            p { class: "capnote", "{why}" }
        }
        FieldGroup { title: "More",
            FieldRow { label: "Contacts",
                Button { label: "Contacts\u{2026}", onclick: on_primary(move || crate::ui::contacts::open(shell)) }
            }
            FieldRow { label: "Rules",
                Button { label: "Rules\u{2026}", onclick: on_primary(move || crate::ui::rules::open(shell)) }
            }
            FieldRow { label: "Keys and certificates",
                Button {
                    label: "Keys and Certificates\u{2026}",
                    onclick: on_primary(move || crate::ui::pgp::keys::open(shell)),
                }
            }
            FieldRow { label: "Keyboard",
                Button {
                    label: "Keyboard Shortcuts\u{2026}",
                    common: Common { aria_label: Some("Keyboard shortcuts".to_owned()), ..Common::default() },
                    onclick: on_primary(move || crate::ui::keyboard::open(shell)),
                }
            }
        }
    }
}

/// Fetch every provider's icon again, then show the new ones.
fn refresh_icons() {
    let store = consume_context::<std::sync::Arc<mail_store::SqliteStore>>();
    let icons = try_consume_context::<Signal<mail_core::provider::icon::Loaded>>();
    spawn(async move {
        let Some(root) = mail_core::config::cache_dir() else {
            eprintln!("provider icon: no cache directory");
            return;
        };
        let dir = root.join("providers");
        let providers = match mail_core::provider::icon::providers_of(&store) {
            Ok(providers) => providers,
            Err(err) => {
                eprintln!("provider icon: {err}");
                return;
            }
        };
        let results = mail_core::provider::icon::refresh(&dir, &providers).await;
        for (provider, result) in &results {
            if let Err(err) = result
                && !matches!(err, mail_core::provider::icon::IconError::Unmapped)
            {
                eprintln!("provider icon: {provider:?}: {err}");
            }
        }
        if let Some(mut icons) = icons {
            icons.set(mail_core::provider::icon::Loaded::read(&dir));
        }
    });
}
