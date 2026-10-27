//! An attachment in Peek: a stored picture, or a PDF a page at a time, larger; a calendar file,
//! a contact card or a forwarded message as mailo reads it (`own.rs`); with the `anyview` feature,
//! anyview's card for any other part (`route.rs` says who draws which). An SVG, and every other
//! part in a build without the feature, says there is no preview.
//!
//! The part is read and drawn on a blocking thread from a resource that runs again only when the
//! part or the page changes. Save writes the same part as the strip's Save; Open writes it to a
//! folder of its own and opens it in the app the system gives its kind, recording the copy so
//! `attachments.remove_unedited` can remove it again (`downloads::unedited`).

use std::path::PathBuf;
use std::sync::Arc;

use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::prelude::*;
use ds::style::appearance::peek::PeekMode;
use mail_client::Client;
use mail_domain::MessageId;

use super::super::route::{self, Own};
use super::own::{self, Sheet};
use crate::ui::copy;
use crate::ui::remedy::plainly;
use crate::ui::view::{Preview, Shell, Viewing};
use mail_core::attachment_image::{self, Kind, Page, Picture, Unavailable};

/// The most of a calendar, a contact or a message that is read whole to draw it.
const OWN_LIMIT: u64 = 16 << 20;

/// What Peek draws for a part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Drawn {
    Picture(Picture),
    Page(Page),
    /// A calendar, a contact or a message, as mailo reads it.
    Sheet(Sheet),
    /// What anyview made of a part mailo does not draw itself.
    #[cfg(feature = "anyview")]
    Peeked(Arc<anyview_peek::AnyPeeked>),
    /// Why nothing is drawn.
    Said(String),
}

/// Attachment `index` of `message`, page `page` if it is a PDF. Blocking: reads and decodes.
pub(super) fn draw(
    client: &Client,
    message: MessageId,
    index: usize,
    page: u32,
    clock: mail_core::when::Clock,
) -> Drawn {
    let stored = match client.stored().message(message) {
        Ok(stored) => stored,
        Err(why) => return unreadable(why),
    };
    let Some(attachment) = stored.attachments.get(index) else {
        return Drawn::Said(copy::status::NO_PREVIEW.to_owned());
    };
    let Some(blob) = attachment.blob() else {
        return Drawn::Said(copy::status::NOT_DOWNLOADED.to_owned());
    };
    let head = match client.blobs().head(blob, route::HEAD) {
        Ok(head) => head,
        Err(why) => return unreadable(why),
    };
    let read: fn(MessageId, &[u8], mail_core::when::Clock) -> Option<Sheet> =
        match route::own(&attachment.name, &head) {
            Some(Own::Svg) => return Drawn::Said(copy::status::NO_PREVIEW.to_owned()),
            Some(Own::Calendar) => own::calendar,
            Some(Own::Contact) => |_, bytes, _| own::contact(bytes),
            Some(Own::Message) => |_, bytes, clock| own::message(bytes, clock),
            Some(Own::Picture) | None => return picture(client, message, index, page),
        };
    let whole = match client.blobs().size(blob) {
        Ok(size) if size <= OWN_LIMIT => client.blobs().get(blob),
        Ok(_) => return Drawn::Said(copy::status::NO_PREVIEW.to_owned()),
        Err(why) => Err(why),
    };
    match whole {
        Ok(bytes) => read(message, &bytes, clock).map_or_else(
            || Drawn::Said(copy::status::NO_PREVIEW.to_owned()),
            Drawn::Sheet,
        ),
        Err(why) => unreadable(why),
    }
}

/// A part the client could not read: the reason goes to the log, and Peek says only that.
fn unreadable(why: impl std::fmt::Display) -> Drawn {
    log::warn!("an attachment could not be read for its preview: {why}");
    Drawn::Said(copy::status::no_preview(copy::status::UNREADABLE))
}

/// A picture or a PDF page `mail_core::attachment_image` decodes; anything else is anyview's.
fn picture(client: &Client, message: MessageId, index: usize, page: u32) -> Drawn {
    match client.preview().load(message, index) {
        Ok((Kind::Pdf, bytes)) => {
            match attachment_image::pdf_page(bytes, page, attachment_image::PDF_PAGE) {
                Ok(page) => Drawn::Page(page),
                Err(refusal) => Drawn::Said(refusal.sentence()),
            }
        }
        Ok((kind, bytes)) => match attachment_image::picture(&bytes, kind, attachment_image::LARGE)
        {
            Ok(picture) => Drawn::Picture(picture),
            Err(refusal) => Drawn::Said(refusal.sentence()),
        },
        Err(Unavailable::Refused(refusal)) => Drawn::Said(refusal.sentence()),
        Err(Unavailable::NotHere) => Drawn::Said(copy::status::NOT_DOWNLOADED.to_owned()),
        Err(Unavailable::NotAPicture) => beyond_mailo(client, message, index),
        Err(Unavailable::Store(why)) => unreadable(why),
    }
}

/// A part mailo does not draw itself, as anyview draws it.
#[cfg(feature = "anyview")]
fn beyond_mailo(client: &Client, message: MessageId, index: usize) -> Drawn {
    use super::super::anyview::{Seen, see};
    match see(client, message, index) {
        Seen::Ready(peeked) => Drawn::Peeked(peeked),
        Seen::NotHere => Drawn::Said(copy::status::NOT_DOWNLOADED.to_owned()),
        Seen::Own => Drawn::Said(copy::status::NO_PREVIEW.to_owned()),
        Seen::Failed(why) => Drawn::Said(copy::status::no_preview(&why)),
    }
}

/// Without anyview, a part mailo does not draw itself has no preview.
#[cfg(not(feature = "anyview"))]
fn beyond_mailo(_store: &Client, _message: MessageId, _index: usize) -> Drawn {
    Drawn::Said(copy::status::NO_PREVIEW.to_owned())
}

/// The stored attachment `by` places on from `viewing`'s among its message's, held to the ones
/// there are; `None` when there is none that way. A part still on the server is passed over:
/// Peek opens only what is here, as the strip does.
pub(super) fn stepped(client: &Client, viewing: Viewing, by: i32) -> Option<Viewing> {
    let attachments = client.stored().message(viewing.message).ok()?.attachments;
    let here = |index: &usize| attachments.get(*index).and_then(|a| a.blob()).is_some();
    let next = if by > 0 {
        (viewing.index + 1..attachments.len()).find(here)
    } else {
        (0..viewing.index).rev().find(here)
    }?;
    Some(Viewing::of(viewing.message, next))
}

/// What Peek shows, when it is an attachment.
fn viewing_of(shell: &Shell) -> Option<Viewing> {
    match &shell.preview {
        Some(Preview::Part(viewing)) => Some(*viewing),
        _ => None,
    }
}

/// The attachment's name, as it would be written.
fn name_of(client: &Client, viewing: Viewing) -> String {
    client
        .stored()
        .message(viewing.message)
        .ok()
        .and_then(|message| {
            message
                .attachments
                .get(viewing.index)
                .map(|a| mail_core::attach::safe_name(&a.name))
        })
        .unwrap_or_default()
}

type Toasts = Option<ds::stack::toast_hub::ToastHub>;

/// Save the part to the downloads folder, as the strip's Save does, and say where in a toast.
fn save(viewing: Viewing, toasts: Toasts) {
    let client = crate::ui::live::client();
    let dir = crate::ui::files::save_dir();
    let said = match client.attach().save(viewing.message, viewing.index, &dir) {
        Ok(path) => {
            let origin = crate::ui::downloads::origin(&client, viewing.message);
            crate::ui::downloads::Saving::quick().end(Some(&path), origin);
            super::super::fetch::saved_toast(&path)
        }
        Err(why) => attachment_failure(why, copy::errors::ATTACHMENT_NOT_SAVED),
    };
    if let Some(toasts) = toasts {
        toasts.push(said, None);
    }
}

/// What the window says when an attachment could not be written: core's own sentence when it is
/// one for people (the part is still on the server, or there is no such part), else `said`, with
/// core's words in the log.
#[track_caller]
fn attachment_failure(failure: mail_core::CoreError, said: &str) -> String {
    use mail_core::CoreError;
    match failure {
        CoreError::AttachmentOnServer { .. } | CoreError::NoAttachment { .. } => {
            crate::ui::remedy::told(failure)
        }
        other => plainly(said, other),
    }
}

/// Where Open writes parts before the system opens them, so nothing lands among the person's
/// downloads unasked: a folder made for this run under the system's temporary folder with a name
/// no one can guess and only its owner may enter, and in it a folder for each message. A shared,
/// fixed name is one another user could make first, or leave a link in.
fn open_root() -> std::io::Result<PathBuf> {
    static OPENED: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let root = match OPENED.get() {
        Some(root) => root,
        None => {
            let made = tempfile::Builder::new().prefix(OPEN_PREFIX).tempdir()?;
            OPENED.get_or_init(|| made)
        }
    };
    Ok(root.path().to_owned())
}

/// What the folder [`open_root`] makes for a run is named with.
const OPEN_PREFIX: &str = "mailo-open-";

/// How long a folder an earlier run's Open left must have gone unchanged before a window clears
/// it away: a window still running may be showing a file from one newer than this.
const LEFT_OPEN_FOR: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Clear away the folders earlier runs' Open left in the system's temporary folder: each run
/// makes one ([`open_root`]) and nothing removes it, since its files may still be open in their
/// apps when the window closes. Only this user's, only folders (never a link to one), and only
/// those older than [`LEFT_OPEN_FOR`]. Best-effort: one that cannot be removed is left.
pub(in crate::ui) fn sweep_left_open() {
    sweep_in(&std::env::temp_dir(), std::time::SystemTime::now());
}

fn sweep_in(temp: &std::path::Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(temp) else {
        return;
    };
    let mine = Owner::of_new_folder_in(temp);
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(OPEN_PREFIX) {
            continue;
        }
        let path = entry.path();
        // Not followed: a link named like ours is left as it is, and so is what it points at.
        let Ok(meta) = path.symlink_metadata() else {
            continue;
        };
        let old = meta
            .modified()
            .ok()
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age > LEFT_OPEN_FOR);
        if meta.is_dir() && mine.owns(&meta) && old {
            // Best-effort: what cannot be removed now is tried again at the next start.
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Who a folder this process makes belongs to, to tell this user's folders from another's: the
/// owner's id on Unix, and `None` where there is none to compare or it could not be learnt.
struct Owner(
    #[cfg_attr(
        not(unix),
        expect(dead_code, reason = "owners are compared on Unix only")
    )]
    Option<u32>,
);

impl Owner {
    /// The owner of a folder made in `temp` now, and removed again.
    fn of_new_folder_in(temp: &std::path::Path) -> Owner {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let made = tempfile::Builder::new()
                .prefix("mailo-probe-")
                .tempdir_in(temp);
            Owner(
                made.ok()
                    .and_then(|dir| dir.path().metadata().ok())
                    .map(|meta| meta.uid()),
            )
        }
        #[cfg(not(unix))]
        {
            let _ = temp;
            Owner(None)
        }
    }

    /// Whether `meta` is of something this user owns. Where there are no owners to compare (not
    /// Unix), the temporary folder is the user's own already.
    fn owns(&self, meta: &std::fs::Metadata) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            self.0 == Some(meta.uid())
        }
        #[cfg(not(unix))]
        {
            let _ = meta;
            true
        }
    }
}

/// Open the part in the app the system gives its kind. From a press or a key: the write and the
/// opener run off the thread that draws, and only a failure is said.
pub(super) fn open_in_app(viewing: Viewing) {
    let client = crate::ui::live::client();
    let toasts = try_consume_context::<ds::stack::toast_hub::ToastHub>();
    spawn(async move {
        let said = copy::errors::ATTACHMENT_NOT_OPENED;
        let opened = tokio::task::spawn_blocking(move || {
            let root = open_root().map_err(|why| plainly(said, why))?;
            let dir = root.join(viewing.message.to_string());
            std::fs::create_dir_all(&dir).map_err(|why| plainly(said, why))?;
            let path = client
                .attach()
                .save(viewing.message, viewing.index, &dir)
                .map_err(|why| attachment_failure(why, said))?;
            // Recorded, so `attachments.remove_unedited` can remove it while it is unchanged.
            crate::ui::downloads::unedited::record(&root, &path, viewing.message);
            crate::ui::downloads::open_file(&path).map_err(|why| why.told(said))
        })
        .await
        .unwrap_or_else(|why| Err(plainly(said, why)));
        if let (Err(said), Some(toasts)) = (opened, toasts) {
            toasts.push(said, None);
        }
    });
}

/// Peek on an attachment. Mounted while `shell.preview` is a part.
#[component]
pub(super) fn PartPeek(shell: Signal<Shell>) -> Element {
    let client = crate::ui::live::use_client();
    let toasts = use_hook(try_consume_context::<ds::stack::toast_hub::ToastHub>);
    // The part and the page, and nothing else of the shell: the count written back below must
    // not draw the page again.
    let wanted = use_memo(move || {
        viewing_of(&shell.read()).map(|viewing| (viewing.message, viewing.index, viewing.page))
    });
    let hours = crate::ui::clock::hours();
    let drawn = use_resource(move || {
        let client = crate::ui::live::client();
        let wanted = wanted();
        async move {
            let (message, index, page) = wanted?;
            let drawn =
                tokio::task::spawn_blocking(move || draw(&client, message, index, page, hours))
                    .await
                    .unwrap_or_else(unreadable);
            if let Drawn::Page(page) = &drawn {
                let mut shell = shell;
                let known = viewing_of(&shell.peek());
                if let Some(viewing) = known.filter(|v| (v.message, v.index) == (message, index)) {
                    let settled = Viewing {
                        page: page.number,
                        pages: Some(page.count),
                        ..viewing
                    };
                    if settled != viewing {
                        shell.write().preview = Some(Preview::Part(settled));
                    }
                }
            }
            Some(drawn)
        }
    });
    let Some(viewing) = viewing_of(&shell.read()) else {
        return rsx! {};
    };
    let name = name_of(&client, viewing);
    // What was drawn for another part (the arrows moved on) is not shown under this one's name.
    let drawn = drawn
        .read()
        .clone()
        .flatten()
        .filter(|_| wanted() == Some((viewing.message, viewing.index, viewing.page)));
    let pages = matches!(drawn, Some(Drawn::Page(_)))
        .then_some(viewing.pages)
        .flatten();
    let main = match drawn {
        None => rsx! { p { class: "peek-note", {copy::status::LOADING} } },
        Some(Drawn::Picture(picture)) => rsx! {
            img { class: "peek-picture", alt: "{name}", src: "{picture.uri}" }
        },
        Some(Drawn::Page(page)) => rsx! {
            img {
                class: "peek-picture peek-sheet",
                alt: copy::aria::page_picture(&name, page.number + 1),
                src: "{page.picture.uri}",
            }
        },
        Some(Drawn::Sheet(sheet)) => rsx! { SheetView { sheet } },
        #[cfg(feature = "anyview")]
        Some(Drawn::Peeked(peeked)) => rsx! {
            div { class: "peek-card",
                anyview_peek::Card {
                    peeked,
                    // The name is in the head already.
                    parts: anyview_peek::Parts::of([
                        anyview_peek::Part::Media,
                        anyview_peek::Part::Facts,
                    ]),
                }
            }
        },
        Some(Drawn::Said(why)) => rsx! { EmptyState { title: why } },
    };
    rsx! {
        ds::components::app::peek::Peek {
            mode: PeekMode::Center,
            label: name.clone(),
            onclose: move |()| super::close(shell),
            div { class: "peek-body",
                div { class: "peek-head",
                    h3 { class: "peek-name", "{name}" }
                    if let Some(count) = pages {
                        Button {
                            bezel: Bezel::Toolbar,
                            image: ImagePosition::Only,
                            icon: Some(IconSource::Glyph(Icon::ChevronUp)),
                            label: copy::aria::PREVIOUS_PAGE.to_owned(),
                            title: Some(copy::aria::PREVIOUS_PAGE.to_owned()),
                            title_shortcut: Some(ds::prelude::Shortcut(vec![ShortcutKey::Up])),
                            availability: crate::ui::press::available(viewing.page > 0),
                            onclick: crate::ui::press::on_primary(move || super::pressed(|| super::turn(shell, -1))),
                        }
                        span { class: "peek-page", {copy::status::page_of(viewing.page + 1, count)} }
                        Button {
                            bezel: Bezel::Toolbar,
                            image: ImagePosition::Only,
                            icon: Some(IconSource::Glyph(Icon::ChevronDown)),
                            label: copy::aria::NEXT_PAGE.to_owned(),
                            title: Some(copy::aria::NEXT_PAGE.to_owned()),
                            title_shortcut: Some(ds::prelude::Shortcut(vec![ShortcutKey::Down])),
                            availability: crate::ui::press::available(viewing.page + 1 < count),
                            onclick: crate::ui::press::on_primary(move || super::pressed(|| super::turn(shell, 1))),
                        }
                    }
                    Button {
                        label: copy::controls::SAVE.to_owned(),
                        onclick: crate::ui::press::on_primary(move || super::pressed(|| save(viewing, toasts))),
                    }
                    Button {
                        label: copy::controls::OPEN.to_owned(),
                        title_shortcut: crate::ui::actions::open_in_app_tip(),
                        onclick: crate::ui::press::on_primary(move || super::pressed(|| open_in_app(viewing))),
                    }
                }
                div { class: "peek-main", {main} }
            }
        }
    }
}

/// A calendar, a contact or a message: its title, its facts and its text.
#[component]
fn SheetView(sheet: Sheet) -> Element {
    let facts: Vec<ds::components::fields::fact_list::Fact> = sheet
        .facts
        .iter()
        .map(|(label, value)| ds::components::fields::fact_list::Fact::new(*label, value.as_str()))
        .collect();
    rsx! {
        div { class: "peek-own",
            h2 { class: "peek-own-title", "{sheet.title}" }
            ds::components::fields::fact_list::FactList { facts }
            if let Some(text) = sheet.text {
                p { class: "peek-own-text", "{text}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LEFT_OPEN_FOR, OPEN_PREFIX, sweep_in};
    use std::time::SystemTime;

    /// A folder an earlier run's Open left, once old enough, is cleared away; a new one, a file
    /// named like one, and anything else in the temporary folder are left.
    #[test]
    fn old_folders_open_left_are_swept_and_nothing_else() {
        let temp = tempfile::tempdir().expect("a scratch folder");
        let left = temp.path().join(format!("{OPEN_PREFIX}abc"));
        std::fs::create_dir_all(left.join("1234")).expect("a folder Open made");
        std::fs::write(left.join("1234").join("plans.pdf"), b"%PDF").expect("its file");
        let file = temp.path().join(format!("{OPEN_PREFIX}file"));
        std::fs::write(&file, b"x").expect("a file named like one");
        let other = temp.path().join("someone-else");
        std::fs::create_dir(&other).expect("another folder");

        // Now: all of it is too new.
        sweep_in(temp.path(), SystemTime::now());
        assert!(left.exists(), "a folder from now was swept");

        let later = SystemTime::now() + LEFT_OPEN_FOR * 2;
        sweep_in(temp.path(), later);
        assert!(!left.exists(), "an old folder was left");
        assert!(file.exists() && other.exists(), "something else was swept");
    }
}
