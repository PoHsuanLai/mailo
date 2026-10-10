//! Files onto the draft: the Attach button's dialog, and files dropped on the page from a file
//! manager. Both hand paths to [`attach`], so a dropped file is refused, read and stored exactly
//! as a picked one is.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dioxus::prelude::*;
use ds::components::controls::button_model::Bezel;
use ds::file_drop::drag::FileDrop;
use ds::file_drop::hook::{FileDropHandle, use_file_drop};
use ds::prelude::*;
use ds::root::common::Common;
use mail_core::SqliteStore;

use super::life;
use super::page::{Guard, Page};
use crate::ui::press::on_primary;
use mail_core::compose::ATTACHMENT_BUDGET;

/// A file picker, as a button: the native dialog (`ui::pick`). Each file chosen lands on the
/// draft and in the Attached row. `bezel` is the bar's it stands in: the composer's own bar
/// (`Toolbar`), or a banner's actions (`Push`).
#[component]
pub(super) fn Attach(page: Signal<Page>, label: &'static str, bezel: Bezel) -> Element {
    rsx! {
        Button {
            bezel,
            label,
            icon: Icon::Paperclip,
            onclick: on_primary(move || {
                crate::ui::pick::choose(crate::ui::pick::Ask::Attachments, None, move |paths| {
                    attach(page, paths);
                });
            }),
            // A plain label is the visible name. The composer's tests find this button by its
            // accessible name, which quire copies onto aria-label only for an icon-only button.
            common: Common { aria_label: Some(label.to_owned()), ..Common::default() },
        }
    }
}

/// The page as a drop target: files let go on it are attached, in the order they came. Its
/// element takes the handle's `onmounted` and `data-drop`.
pub(super) fn use_drop_target(page: Signal<Page>) -> FileDropHandle {
    // The drop handler is an event, not the component body, so reading can start here.
    use_file_drop(move |files: FileDrop| attach(page, files.paths))
}

/// A file asked for, read when it is a regular file within the budget.
enum Picked {
    Read(Vec<u8>),
    TooLarge,
    Folder,
    Unreadable,
}

/// Read `path`. A symbolic link is followed, and read only if what it names is a regular file:
/// a folder is refused rather than walked, and a pipe or a device is never opened, since opening
/// one can wait for good. The size is checked before reading, so a 4 GB file is refused rather
/// than read, and the read stops past the budget in case the file grew in between.
fn read_picked(path: &Path) -> Picked {
    let Ok(meta) = std::fs::metadata(path) else {
        return Picked::Unreadable;
    };
    if meta.is_dir() {
        return Picked::Folder;
    }
    if !meta.is_file() {
        return Picked::Unreadable;
    }
    if meta.len() > ATTACHMENT_BUDGET {
        return Picked::TooLarge;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return Picked::Unreadable;
    };
    let mut bytes = Vec::new();
    match file.take(ATTACHMENT_BUDGET + 1).read_to_end(&mut bytes) {
        Ok(_) if bytes.len() as u64 > ATTACHMENT_BUDGET => Picked::TooLarge,
        Ok(_) => Picked::Read(bytes),
        Err(_) => Picked::Unreadable,
    }
}

/// Attach each of `paths` to the draft, in order, each read on a blocking thread. What could not
/// be attached is named in one note once all have been tried.
fn attach(mut page: Signal<Page>, paths: Vec<PathBuf>) {
    spawn(async move {
        let mut refused = Vec::new();
        for path in paths {
            let name = crate::ui::pick::file_name(&path);
            let picked = tokio::task::spawn_blocking(move || read_picked(&path))
                .await
                .unwrap_or(Picked::Unreadable);
            let bytes = match picked {
                Picked::Read(bytes) => bytes,
                Picked::TooLarge => {
                    refused.push(format!("{name} is too large to send"));
                    continue;
                }
                Picked::Folder => {
                    refused.push(format!(
                        "{name} is a folder; attach the files in it instead"
                    ));
                    continue;
                }
                Picked::Unreadable => {
                    refused.push(format!("cannot read {name}"));
                    continue;
                }
            };
            let store = consume_context::<Arc<SqliteStore>>();
            let draft = page.peek().draft;
            let now = chrono::Utc::now();
            let saved = life::save(&store, &mut page.write(), now).and_then(|_| {
                mail_core::compose::attach_bytes(&store, draft, &name, &bytes, now)
                    .map_err(String::from)
            });
            match saved {
                Ok(stored) => {
                    let mut write = page.write();
                    write.attached = mail_core::compose::attached_to(&store, &stored);
                    write.guard = Guard::Clear;
                }
                Err(why) => refused.push(why),
            }
        }
        if !refused.is_empty() {
            page.write().notice = Some(refused.join("; "));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{Picked, read_picked};
    use mail_core::compose::ATTACHMENT_BUDGET;

    fn kind(picked: &Picked) -> &'static str {
        match picked {
            Picked::Read(_) => "read",
            Picked::TooLarge => "too large",
            Picked::Folder => "folder",
            Picked::Unreadable => "unreadable",
        }
    }

    #[test]
    fn only_a_regular_file_within_the_budget_is_read() {
        let dir = tempfile::tempdir().unwrap_or_else(|why| panic!("a temp dir: {why}"));
        let at = |name: &str| dir.path().join(name);
        std::fs::write(at("note.txt"), "hello").unwrap_or_else(|why| panic!("{why}"));
        std::fs::create_dir(at("photos")).unwrap_or_else(|why| panic!("{why}"));
        std::fs::File::create(at("huge.bin"))
            .and_then(|file| file.set_len(ATTACHMENT_BUDGET + 1))
            .unwrap_or_else(|why| panic!("{why}"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(at("note.txt"), at("to-note")).unwrap_or_else(|why| panic!("{why}"));
            symlink(at("photos"), at("to-photos")).unwrap_or_else(|why| panic!("{why}"));
            symlink(at("gone"), at("to-gone")).unwrap_or_else(|why| panic!("{why}"));
        }
        let mut cases = vec![
            ("note.txt", "read"),
            ("photos", "folder"),
            ("huge.bin", "too large"),
            ("missing", "unreadable"),
        ];
        if cfg!(unix) {
            cases.extend([
                ("to-note", "read"),
                ("to-photos", "folder"),
                ("to-gone", "unreadable"),
            ]);
        }
        for (name, expected) in cases {
            assert_eq!(kind(&read_picked(&at(name))), expected, "{name}");
        }
        #[cfg(unix)]
        match read_picked(&at("to-note")) {
            Picked::Read(bytes) => assert_eq!(bytes, b"hello", "the link read as its file"),
            other => panic!("the link to a file was {}", kind(&other)),
        }
    }
}
