//! The book in and out as vCard, from ⌘K's Import Contacts… and Export Contacts….
//!
//! Import takes files through the native dialog Attach uses (`ui::pick`), each read on a blocking
//! thread; export writes `contacts.vcf` into the downloads directory the way Save writes an
//! attachment, never over a file already there. Each says what it came to once, in a toast.

use dioxus::prelude::*;

use super::book;
use crate::ui::copy;
use crate::ui::motion::{Follow, tell};
use crate::ui::pick::{Ask, choose, file_name};

/// Ask for vCard files and read each into the book, in order, saying what the last one did.
pub(in crate::ui) fn import_cards() {
    choose(Ask::Cards, None, |paths| {
        spawn(async move {
            let mut said = None;
            for path in paths {
                let name = file_name(&path);
                let read = tokio::task::spawn_blocking(move || std::fs::read(path)).await;
                said = Some(match read {
                    Ok(Ok(bytes)) => {
                        let client = crate::ui::live::client();
                        book::import(&client, &bytes)
                    }
                    _ => Err(copy::errors::cannot_read(&name)),
                });
            }
            if let Some(said) = said {
                tell(said.unwrap_or_else(|why| why), Follow::Nothing);
            }
        });
    });
}

/// Write the whole book into the downloads directory and say where it went.
pub(in crate::ui) fn export_cards() {
    let client = crate::ui::live::client();
    let dir = crate::ui::files::save_dir();
    match book::export(&client, &dir) {
        Ok(path) => {
            crate::ui::downloads::Saving::quick().end(Some(&path), None);
            tell(copy::toasts::saved_to(&path), Follow::Nothing);
        }
        Err(why) => tell(why, Follow::Nothing),
    }
}
