//! The keys and certificates Settings lists: an account's own on its page (its Encryption
//! group), and every other on the Other Keys page under Accounts. OpenPGP keys come first, then
//! S/MIME certificates, each a row of quire's list with its actions; under them, a list of its
//! own of what adds to them, a key to make when the account has none and Import for each kind;
//! and the questions asked before an act that cannot be undone (`key_row::Asking`).

use dioxus::prelude::*;
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
use ds::prelude::*;
use ds::root::common::Common;
use mail_core::{SqliteStore, Store};
use mail_domain::SecretHeld;
use std::sync::Arc;

use super::super::common::{Told, add_mark};
use super::super::data::account_rows;
use super::super::press::{available, on_primary};
use super::certs::{IMPORT, cert_items, quiet_item};
use super::key_row::{Asking, Confirm, KeyRow};
use super::keys::{Done, Job, keyless, ordered, work};
use super::{Busy, seams, who};
use crate::ui::copy;

/// Whose keys and certificates a [`KeysPage`] lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Whose {
    /// One account's own, by its address: what signs and decrypts its mail.
    Account(String),
    /// Every other: correspondents', authorities', and the user's own for an address no
    /// account here has.
    Others,
}

impl Whose {
    /// Whether a key or certificate for `emails`, whose secret half is `secret`, is listed here,
    /// `accounts` being the addresses of the accounts here, lowercased.
    pub(in crate::ui) fn lists(
        &self,
        emails: &[String],
        secret: SecretHeld,
        accounts: &[String],
    ) -> bool {
        let own = |address: &str| secret == SecretHeld::Held && emails.iter().any(|e| e == address);
        match self {
            Whose::Account(address) => own(&address.to_lowercase()),
            Whose::Others => !accounts.iter().any(|address| own(address)),
        }
    }
}

/// The group: its rows, then what the last act came to under them.
#[component]
pub(in crate::ui) fn KeysPage(
    whose: Whose,
    /// The shared revision, bumped by a toast's Undo that puts a deleted key back.
    #[props(default)]
    revision: Option<Signal<u64>>,
) -> Element {
    // Bumped by every change, so the keys are read again.
    let mut changed = use_signal(|| 0u64);
    let _ = revision.map(|revision| revision());
    let mut said = use_signal(|| None::<Result<String, String>>);
    let mut busy = use_signal(|| Busy::Idle);
    let mut confirm = use_signal(|| Confirm::Nothing);
    let _ = changed();
    let store = consume_context::<Arc<SqliteStore>>();
    let accounts: Vec<String> = account_rows(&store)
        .into_iter()
        .filter(|row| !row.is_local())
        .map(|row| row.address.to_lowercase())
        .collect();
    let (keys, failed) = match store.pgp_keys() {
        Ok(keys) => (ordered(keys), None),
        Err(why) => (
            Vec::new(),
            Some(crate::ui::remedy::plainly(
                crate::ui::copy::errors::NOT_READ_HERE,
                why,
            )),
        ),
    };
    let (certs, certs_failed) = match store.smime_certs() {
        Ok(certs) => (super::certs::ordered(certs), None),
        Err(why) => (
            Vec::new(),
            Some(crate::ui::remedy::plainly(
                crate::ui::copy::errors::NOT_READ_HERE,
                why,
            )),
        ),
    };
    let keys: Vec<_> = keys
        .into_iter()
        .filter(|key| whose.lists(&key.emails, key.secret, &accounts))
        .collect();
    let certs: Vec<_> = certs
        .into_iter()
        .filter(|cert| whose.lists(&cert.emails, cert.secret, &accounts))
        .collect();
    let make = match &whose {
        Whose::Account(address) => keyless(&store)
            .into_iter()
            .filter(|keyless| keyless.eq_ignore_ascii_case(address))
            .collect(),
        Whose::Others => Vec::new(),
    };
    // Made here, so the task belongs to the page and not to a row a deletion takes away.
    let run = use_callback(move |job: Job| {
        if *busy.peek() == Busy::Working {
            return;
        }
        busy.set(Busy::Working);
        said.set(None);
        let store = consume_context::<Arc<SqliteStore>>();
        let seams = seams();
        let motion = crate::ui::motion::motion();
        spawn(async move {
            let done = tokio::task::spawn_blocking(move || work(&store, &seams, job))
                .await
                .unwrap_or_else(|error| Err(format!("Stopped early: {error}")));
            match done {
                Ok(Done::Said(text)) => said.set(Some(Ok(text))),
                Ok(Done::Password(path)) => confirm.set(Confirm::Password(path)),
                // Said in the toast with its Undo; on the page only with no window to toast in.
                Ok(Done::Gone(text, restore)) => {
                    if let Err(text) = crate::ui::motion::offer_restore(motion, text, *restore) {
                        said.set(Some(Ok(text)));
                    }
                }
                Err(why) => said.set(Some(Err(why))),
            }
            busy.set(Busy::Idle);
            changed += 1;
        });
    });
    let working = busy() == Busy::Working;
    let import_label = "Import from a file…";
    let nothing = keys.is_empty() && certs.is_empty() && make.is_empty();
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
    // Why a list could not be read, or that the others have nothing, is a row of its own.
    let quiet = failed.or(certs_failed).or_else(|| {
        (nothing && whose == Whose::Others).then(|| copy::empty::NO_OTHER_KEYS.to_owned())
    });
    if let Some(quiet) = quiet {
        items.push(quiet_item(quiet));
    }
    let (cert_rows, cert_import) = cert_items(certs, confirm, run, busy());
    items.extend(cert_rows);
    // What adds to the list is a list of its own under it, as Add Account… is under the
    // accounts: a key to make, then Import for each kind.
    let mut adds: Vec<ListItem<String>> = Vec::new();
    for address in make {
        let label = format!("Make a key for {address}");
        let make = address.clone();
        adds.push(ListItem::row(
            format!(" make {address}"),
            label.clone(),
            rsx! {
                Row {
                    leading: add_mark(),
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
    adds.push(ListItem::row(
        IMPORT.to_owned(),
        import_label.to_owned(),
        rsx! {
            Row {
                leading: add_mark(),
                title: copy::fields::OPENPGP_KEY,
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
    adds.push(cert_import);
    let title = match whose {
        Whose::Account(_) => Some(copy::headers::ENCRYPTION.to_owned()),
        Whose::Others => None,
    };
    let empty = items.is_empty();
    rsx! {
        Told { title: crate::ui::copy::titles::KEYS_FAILED, said }
        // The group's title heads its first list: the keys, or with none, what adds one.
        if !items.is_empty() {
            FormSection { title: title.clone(),
                List::<String> { label: copy::headers::ENCRYPTION, items, style: ListStyle::Grouped }
            }
        }
        FormSection { title: if empty { title } else { None },
            List::<String> { label: copy::aria::ADD_A_KEY, items: adds, style: ListStyle::Grouped }
        }
        Asking { confirm, run, busy: busy() }
    }
}
