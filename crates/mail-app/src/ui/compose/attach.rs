//! Files onto the draft: the Attach button's dialog, and files dropped on the page from a file
//! manager. Both hand paths to [`take`], so a dropped file is refused, read and stored exactly
//! as a picked one is.
//!
//! Pictures into the body: `/` › Image's dialog, a picture dropped on the page, and one pasted
//! from the clipboard. Each is stored on the draft as one of its parts and shown where the caret
//! is (`float::insert_image`); it goes out inline, in the HTML's `multipart/related`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::file_drop::drag::FileDrop;
use ds::file_drop::hook::{FileDropHandle, use_file_drop};
use ds::prelude::*;
use ds::root::common::Common;
use mail_core::{CoreError, SqliteStore};

use super::life;
use super::page::{Guard, Page};
use crate::settings::MessageFormat;
use crate::ui::press::on_primary;
use mail_core::compose::ATTACHMENT_BUDGET;

/// A file picker, as a button: the native dialog (`ui::pick`). Each file chosen lands on the
/// draft and in the strip at the body's end. `bezel` is the bar's it stands in: the composer's
/// foot (`Toolbar`, its glyph alone with a tip), or a banner's actions (`Push`, its words).
#[component]
pub(super) fn Attach(page: Signal<Page>, label: &'static str, bezel: Bezel) -> Element {
    let image = match bezel {
        Bezel::Toolbar => ImagePosition::Only,
        _ => ImagePosition::Leading,
    };
    rsx! {
        Button {
            bezel,
            label,
            image,
            icon: Icon::Paperclip,
            title: (image == ImagePosition::Only).then(|| label.to_owned()),
            onclick: on_primary(move || choose(page)),
            // The composer's tests find this button by its accessible name, which quire copies
            // onto aria-label only for an icon-only button; a labelled one names it here.
            common: Common { aria_label: Some(label.to_owned()), ..Common::default() },
        }
    }
}

/// Open the file picker; what is chosen is attached. The foot's Attach and `/` › Attachment both
/// come here.
pub(super) fn choose(page: Signal<Page>) {
    crate::ui::pick::choose(crate::ui::pick::Ask::Attachments, None, move |paths| {
        take(
            page,
            paths.into_iter().map(|path| (path, Lands::Files)).collect(),
        );
    });
}

/// Open the file picker for pictures; each chosen is shown in the body at the caret: `/` › Image.
pub(super) fn choose_image(page: Signal<Page>) {
    crate::ui::pick::choose(crate::ui::pick::Ask::Images, None, move |paths| {
        take(
            page,
            paths.into_iter().map(|path| (path, Lands::Body)).collect(),
        );
    });
}

/// The page as a drop target: files let go on it are attached, in the order they came, and a
/// picture the body can show is shown in it, at the caret. Its element takes the handle's
/// `onmounted` and `data-drop`.
pub(super) fn use_drop_target(page: Signal<Page>) -> FileDropHandle {
    // The drop handler is an event, not the component body, so reading can start here.
    use_file_drop(move |files: FileDrop| {
        let landing = files
            .paths
            .into_iter()
            .map(|path| {
                let picture = mail_core::compose::shows_inline(&crate::ui::pick::file_name(&path));
                (path, if picture { Lands::Body } else { Lands::Files })
            })
            .collect();
        take(page, landing);
    })
}

/// The picture on the clipboard, `png`, shown in the body at the caret: a paste the surface
/// brought nothing for (`crate::ui::pasted_image`).
pub(super) fn paste_image(mut page: Signal<Page>, png: Vec<u8>) {
    let name = crate::ui::copy::fields::PASTED_IMAGE;
    let placed = match shown_in(page.peek().format, Lands::Body) {
        Lands::Body => into_body(page, name, &png),
        Lands::Files => as_file(page, name, &png).map_err(BodyRefused::Said),
    };
    match placed {
        Ok(()) => {}
        Err(BodyRefused::Said(why)) => page.write().notice = Some(why),
        // A pasted picture the body cannot hold is kept as a file, as a dropped one is.
        Err(BodyRefused::AsFile) => {
            if let Err(why) = as_file(page, name, &png) {
                page.write().notice = Some(why);
            }
        }
    }
}

/// Where a file read in goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lands {
    /// Onto the draft, as a file in the strip at the body's end.
    Files,
    /// Into the body, as a picture at the caret.
    Body,
}

/// Where a file wanted `wanted` goes on a page written as `format`: a plain-text message has no
/// HTML to show a picture in, so everything on it is a file.
fn shown_in(format: MessageFormat, wanted: Lands) -> Lands {
    match format {
        MessageFormat::Plain => Lands::Files,
        MessageFormat::Rich => wanted,
    }
}

/// Why a picture was not put into the body: it goes as a file instead, or the person is told.
#[derive(Debug, PartialEq, Eq)]
enum BodyRefused {
    /// Not a picture the body can show, or too large for the pictures in one body: it is still a
    /// file that can be sent.
    AsFile,
    Said(String),
}

impl BodyRefused {
    fn of(why: CoreError) -> Self {
        match why {
            CoreError::NotAnInlineImage { .. } | CoreError::InlineImagesTooBig { .. } => {
                BodyRefused::AsFile
            }
            other => BodyRefused::Said(crate::ui::remedy::told(other)),
        }
    }
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

/// Store each of `paths` on the draft, in order, each read on a blocking thread, where it
/// [`Lands`]. What could not be stored is named in one note once all have been tried.
fn take(mut page: Signal<Page>, paths: Vec<(PathBuf, Lands)>) {
    spawn(async move {
        let mut refused = Vec::new();
        for (path, lands) in paths {
            let name = crate::ui::pick::file_name(&path);
            let picked = tokio::task::spawn_blocking(move || read_picked(&path))
                .await
                .unwrap_or(Picked::Unreadable);
            let bytes = match picked {
                Picked::Read(bytes) => bytes,
                Picked::TooLarge => {
                    refused.push(format!("{name} is too large to send."));
                    continue;
                }
                Picked::Folder => {
                    refused.push(format!("{name} is a folder. Attach its files instead."));
                    continue;
                }
                Picked::Unreadable => {
                    refused.push(format!("Can’t read {name}"));
                    continue;
                }
            };
            if shown_in(page.peek().format, lands) == Lands::Body {
                match into_body(page, &name, &bytes) {
                    Ok(()) => continue,
                    Err(BodyRefused::Said(why)) => {
                        refused.push(why);
                        continue;
                    }
                    // A photo too large to embed, or a type the body does not draw, is attached.
                    Err(BodyRefused::AsFile) => {}
                }
            }
            if let Err(why) = as_file(page, &name, &bytes) {
                refused.push(why);
            }
        }
        if !refused.is_empty() {
            page.write().notice = Some(refused.join("; "));
        }
    });
}

/// Store `bytes`, named `name`, on the draft as a file in the strip at the body's end.
fn as_file(mut page: Signal<Page>, name: &str, bytes: &[u8]) -> Result<(), String> {
    let store = consume_context::<Arc<SqliteStore>>();
    let draft = page.peek().draft;
    let now = chrono::Utc::now();
    let stored = life::save(&store, &mut page.write(), now).and_then(|_| {
        mail_core::compose::attach_bytes(&store, draft, name, bytes, now)
            .map_err(crate::ui::remedy::told)
    })?;
    let mut write = page.write();
    write.attached = mail_core::compose::attached_to(&store, &stored);
    write.guard = Guard::Clear;
    Ok(())
}

/// Store the picture `bytes`, named `name`, on the draft, and show it in the body at the caret,
/// as one undo step. The page is saved first, so the store's draft is the page's when the
/// picture is added to it. A picture the body cannot take is [`BodyRefused::AsFile`].
fn into_body(mut page: Signal<Page>, name: &str, bytes: &[u8]) -> Result<(), BodyRefused> {
    let store = consume_context::<Arc<SqliteStore>>();
    let draft = page.peek().draft;
    let now = chrono::Utc::now();
    life::save(&store, &mut page.write(), now).map_err(BodyRefused::Said)?;
    let (stored, cid) = mail_core::compose::attach_inline(&store, draft, name, bytes, now)
        .map_err(BodyRefused::of)?;
    let mime = stored
        .attachments
        .iter()
        .find(|part| part.cid.as_deref() == Some(cid.as_str()))
        .map_or("", |part| part.mime.as_str());
    let mut write = page.write();
    write.images.insert(&cid, mime, bytes);
    super::float::insert_image(&mut write, &cid);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BodyRefused, Lands, Picked, read_picked, shown_in};
    use crate::settings::MessageFormat;
    use mail_core::CoreError;
    use mail_core::compose::ATTACHMENT_BUDGET;

    /// A picture the body cannot take is attached; every other refusal is said.
    #[test]
    fn a_picture_the_body_cannot_hold_goes_as_a_file() {
        assert_eq!(
            BodyRefused::of(CoreError::InlineImagesTooBig {
                limit: "6 MB".to_owned()
            }),
            BodyRefused::AsFile
        );
        assert_eq!(
            BodyRefused::of(CoreError::NotAnInlineImage {
                name: "scan.tiff".to_owned()
            }),
            BodyRefused::AsFile
        );
        assert!(matches!(
            BodyRefused::of(CoreError::DraftOnItsWay),
            BodyRefused::Said(_)
        ));
    }

    /// A plain-text message has no HTML to show a picture in: all of it goes as files.
    #[test]
    fn on_a_plain_text_message_every_file_is_a_file() {
        assert_eq!(shown_in(MessageFormat::Plain, Lands::Body), Lands::Files);
        assert_eq!(shown_in(MessageFormat::Plain, Lands::Files), Lands::Files);
        assert_eq!(shown_in(MessageFormat::Rich, Lands::Body), Lands::Body);
        assert_eq!(shown_in(MessageFormat::Rich, Lands::Files), Lands::Files);
    }

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
