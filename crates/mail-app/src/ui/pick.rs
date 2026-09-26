//! The native file dialog: the sheets' File… and Folder…, the composer's Attach and the
//! Contacts sheet's Import vCard….
//!
//! `rfd` through the desktop portal, so it opens the desktop's own dialog. The dialog blocks
//! until it is answered, so it is opened on a blocking thread, from the click.
//!
//! What opens it is a [`Dialogs`] seam, like `pgp::Seams`: a test hands the window its own
//! answers, and in this crate's tests the real one answers nothing, so no test ever opens a
//! dialog.

use std::path::PathBuf;
use std::sync::Arc;

use dioxus::prelude::*;

/// What to ask the dialog for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Ask {
    /// One file: an mbox or a `.eml`.
    File,
    /// A directory: a Maildir, or where an export goes.
    Folder,
    /// Any number of files of any kind, to attach to a draft.
    Attachments,
    /// Any number of vCard files, to import into Contacts.
    Cards,
}

/// The dialog: what was asked and where it starts, to what was chosen. Nothing, when it is
/// cancelled.
pub(in crate::ui) type Answer = dyn Fn(Ask, Option<PathBuf>) -> Vec<PathBuf> + Send + Sync;

/// Whatever answers the window's file dialogs, as a root context.
#[derive(Clone)]
pub(in crate::ui) struct Dialogs(pub(in crate::ui) Arc<Answer>);

impl Dialogs {
    /// The desktop's dialog. In this crate's tests, one that answers nothing: a window drawn by
    /// a test has no business opening a dialog.
    fn real() -> Dialogs {
        if cfg!(test) {
            return Dialogs(Arc::new(|_, _| Vec::new()));
        }
        Dialogs(Arc::new(|ask, start| {
            let mut dialog = rfd::FileDialog::new();
            if let Some(start) = start {
                dialog = dialog.set_directory(start);
            }
            match ask {
                Ask::File => dialog.pick_file().into_iter().collect(),
                Ask::Folder => dialog.pick_folder().into_iter().collect(),
                Ask::Attachments => dialog.pick_files().unwrap_or_default(),
                Ask::Cards => dialog
                    .add_filter("vCard", &["vcf", "vcard"])
                    .pick_files()
                    .unwrap_or_default(),
            }
        }))
    }
}

/// Every ask a test's dialog was handed, with where it was to start.
#[cfg(test)]
pub(in crate::ui) type Asked = Arc<std::sync::Mutex<Vec<(Ask, Option<PathBuf>)>>>;

#[cfg(test)]
impl Dialogs {
    /// A dialog that answers every ask with `paths`, and keeps what it was asked.
    pub(in crate::ui) fn answering(paths: Vec<PathBuf>) -> (Dialogs, Asked) {
        let asked = Asked::default();
        let kept = asked.clone();
        let dialogs = Dialogs(Arc::new(move |ask, start| {
            kept.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((ask, start));
            paths.clone()
        }));
        (dialogs, asked)
    }
}

/// Open the dialog for `ask` in `start`, and hand what was chosen to `chosen`. Nothing, when
/// it is cancelled.
pub(in crate::ui) fn choose(
    ask: Ask,
    start: Option<PathBuf>,
    chosen: impl FnOnce(Vec<PathBuf>) + 'static,
) {
    let dialogs = try_consume_context::<Dialogs>().unwrap_or_else(Dialogs::real);
    spawn(async move {
        let picked = tokio::task::spawn_blocking(move || (dialogs.0)(ask, start)).await;
        if let Ok(paths) = picked
            && !paths.is_empty()
        {
            chosen(paths);
        }
    });
}

/// The name a picked file goes by: its last component.
pub(in crate::ui) fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}
