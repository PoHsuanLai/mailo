//! General: mailo's settings schema, a group per section.
//!
//! Each row is the schema's ([`super::keys`]); what a section adds beside its rows (Refresh icons)
//! and what it says under them (whether there is a dictionary, whether a brand-logo root is
//! trusted) is said here, by section title, so the rows themselves stay the ones detent draws. The one key a row of the schema
//! cannot draw, the senders whose images load, is a row per sender with its Remove.

use super::keys::{SchemaSection, sections};
use crate::settings::{BrandLogos, LoadRemoteImages, Spelling};
use crate::ui::appearance::WindowDirs;
use crate::ui::compose::dictionaries;
use crate::ui::press::on_primary;
use dioxus::prelude::*;
use ds::components::controls::button_model::Bezel;
use ds::components::fields::field_row::FieldRow;
use ds::prelude::*;
use ds::root::common::Common;

#[component]
pub(super) fn General() -> Element {
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
    // What a section says about itself as a whole, under its group: that the system has no
    // dictionary for a language, or that no logo can be verified yet.
    let footer_of = |title: &str| match title {
        "Writing" => missing.clone(),
        "Reading" if settings.reading.brand_logos == BrandLogos::On && roots == 0 => {
            Some("Logos can't be verified on this computer yet.".to_owned())
        }
        _ => None,
    };
    rsx! {
        Form {
            // A change that could not be kept is the page's to say, above every group.
            if let Some(why) = failed() {
                InlineBanner { severity: Severity::Danger, text: "Not kept", detail: Some(TextLine::from(why)) }
            }
            for (title, keys) in drawn_sections() {
                SchemaSection {
                    key: "{title}",
                    title: title.clone(),
                    keys,
                    values: values.clone(),
                    onedit,
                    footer: footer_of(&title),
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
                        "Reading" => rsx! {
                            TrustedSenders {
                                senders: settings.reading.trusted_image_senders.clone(),
                                mode: settings.reading.remote_images,
                                onfail: move |why: String| failed.set(Some(why)),
                            }
                        },
                        _ => rsx! {},
                    }
                }
            }
        }
    }
}

/// The schema path of the senders whose images load.
const TRUSTED_KEY: &str = "reading.trusted_image_senders";

/// The schema's sections and the keys whose rows the schema draws: the trusted senders are rows
/// of their own ([`TrustedSenders`]), not the list's read-only value.
fn drawn_sections() -> Vec<(String, Vec<ds_settings::schema::KeySpec>)> {
    sections(&crate::settings::schema())
        .into_iter()
        .map(|(title, keys)| {
            let keys = keys
                .into_iter()
                .filter(|key| key.path.0 != TRUSTED_KEY)
                .collect();
            (title, keys)
        })
        .collect()
}

/// The senders whose images load, a row each with its Remove; while images load from trusted
/// senders and none is trusted yet, a row saying how one comes to be.
#[component]
fn TrustedSenders(
    senders: Vec<String>,
    mode: LoadRemoteImages,
    onfail: EventHandler<String>,
) -> Element {
    rsx! {
        if senders.is_empty() && mode == LoadRemoteImages::Trusted {
            FieldRow {
                label: "No trusted senders yet",
                help: Some(TextLine::from(
                    "Choose Always Load From on a message's image banner.",
                )),
            }
        }
        for sender in senders {
            FieldRow {
                key: "{sender}",
                label: sender.clone(),
                help: Some(TextLine::from("Images load from this sender")),
                Button {
                    label: "Remove",
                    common: Common {
                        aria_label: Some(format!("Stop loading images from {sender}")),
                        ..Common::default()
                    },
                    onclick: {
                        let sender = sender.clone();
                        on_primary(move || {
                            let gone = sender.clone();
                            let changed = crate::ui::prefs::change(move |settings| {
                                settings.reading.trusted_image_senders.retain(|kept| *kept != gone);
                            });
                            if let Err(why) = changed {
                                onfail.call(why);
                            }
                        })
                    },
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
            icons.set(crate::ui::provider_chip::read(&dir));
        }
    });
}
