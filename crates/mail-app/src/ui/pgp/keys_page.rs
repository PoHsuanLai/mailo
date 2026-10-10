//! The Keys and certificates page of Settings: OpenPGP keys, then S/MIME certificates, each a row
//! of quire's list with its actions, the user's own first; a key to make for each sending account
//! that has none; Import in each group; and the questions asked before an act that cannot be
//! undone (`key_row::Asking`).

use dioxus::prelude::*;
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::family::PlateFamily;
use mail_core::{SqliteStore, Store};
use std::sync::Arc;

use super::super::common::{Told, tile};
use super::super::press::{available, on_primary};
use super::certs::{CertPart, IMPORT, quiet_item};
use super::key_row::{Asking, Confirm, KeyRow};
use super::keys::{Done, Job, keyless, ordered, work};
use super::{Busy, seams, who};

/// The page.
#[component]
pub(in crate::ui) fn KeysPage() -> Element {
    // Bumped by every change, so the keys are read again.
    let mut changed = use_signal(|| 0u64);
    let mut said = use_signal(|| None::<Result<String, String>>);
    let mut busy = use_signal(|| Busy::Idle);
    let mut confirm = use_signal(|| Confirm::Nothing);
    let _ = changed();
    let store = consume_context::<Arc<SqliteStore>>();
    let (keys, failed) = match store.pgp_keys() {
        Ok(keys) => (ordered(keys), None),
        Err(why) => (Vec::new(), Some(why.to_string())),
    };
    let (certs, certs_failed) = match store.smime_certs() {
        Ok(certs) => (super::certs::ordered(certs), None),
        Err(why) => (Vec::new(), Some(why.to_string())),
    };
    let keyless = keyless(&store);
    // Made here, so the task belongs to the page and not to a row a deletion takes away.
    let run = use_callback(move |job: Job| {
        if *busy.peek() == Busy::Working {
            return;
        }
        busy.set(Busy::Working);
        said.set(None);
        let store = consume_context::<Arc<SqliteStore>>();
        let seams = seams();
        spawn(async move {
            let done = tokio::task::spawn_blocking(move || work(&store, &seams, job))
                .await
                .unwrap_or_else(|error| Err(format!("It stopped before it finished: {error}")));
            match done {
                Ok(Done::Said(text)) => said.set(Some(Ok(text))),
                Ok(Done::Password(path)) => confirm.set(Confirm::Password(path)),
                Err(why) => said.set(Some(Err(why))),
            }
            busy.set(Busy::Idle);
            changed += 1;
        });
    });
    let working = busy() == Busy::Working;
    let import_label = "Import from a file…";
    let mut items: Vec<ListItem<String>> = keys
        .into_iter()
        .map(|key| {
            let id = key.fingerprint.to_string();
            let name = who(&key);
            ListItem::row(
                id.clone(),
                name,
                rsx! { KeyRow { key: "{id}", pgp: key, confirm, run, busy: busy() } },
            )
        })
        .collect();
    if items.is_empty() {
        items.push(quiet_item(
            failed.unwrap_or_else(|| "No OpenPGP keys yet".to_owned()),
        ));
    }
    for address in keyless {
        let label = format!("Make a key for {address}");
        let make = address.clone();
        items.push(ListItem::row(
            format!(" make {address}"),
            label.clone(),
            rsx! {
                Row {
                    leading: tile(Icon::Plus, PlateFamily::Green),
                    title: address.clone(),
                    detail: Some(TextLine::from("No key of your own yet")),
                    size: RowSize::Settings,
                    accessory: Accessory::Slot(rsx! {
                        Button {
                            label: "Make a Key",
                            availability: available(!working),
                            onclick: on_primary(move || run.call(Job::Generate(make.clone()))),
                            common: Common { aria_label: Some(label.clone()), ..Common::default() },
                        }
                    }),
                }
            },
        ));
    }
    items.push(ListItem::row(
        IMPORT.to_owned(),
        import_label.to_owned(),
        rsx! {
            Row {
                leading: tile(Icon::Plus, PlateFamily::Green),
                title: "Import",
                detail: Some(TextLine::from("Yours sign and decrypt; theirs encrypt and verify.")),
                size: RowSize::Settings,
                accessory: Accessory::Slot(rsx! {
                    Button {
                        label: "Import…",
                        availability: available(!working),
                        onclick: on_primary(move || run.call(Job::Import)),
                        common: Common { aria_label: Some(import_label.to_owned()), ..Common::default() },
                    }
                }),
            }
        },
    ));
    rsx! {
        Form {
            Told { said: said() }
            FormSection { title: Some("OpenPGP".to_owned()),
                List::<String> { label: "OpenPGP keys", items, style: ListStyle::Grouped }
            }
            CertPart { certs, failed: certs_failed, confirm, run, busy: busy() }
            Asking { confirm, run, busy: busy() }
        }
    }
}
