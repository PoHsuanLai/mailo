//! Mail files in the window: "Import mail…" and "Export mail…" from Ctrl T.
//!
//! [`work`] is every question and every write, as functions of a store and a path; the two
//! sheets only draw what it answers. The path is typed, and a Maildir is a directory. The native
//! dialog beside the field (`ui::pick`) goes through the desktop portal.
//!
//! Both jobs run on a blocking thread, spawned from the click (F140), and their progress comes
//! back through a counter the task reads while it waits — the progress callback runs on that
//! thread, where no signal may be written.

mod export_sheet;
mod import_sheet;
pub(in crate::ui) mod work;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dioxus::prelude::*;

use super::motion::{Follow, tell};
use crate::view::{FileSheet, Shell};

/// Where the sheets suggest and start: the downloads directory, unless the window was handed
/// another one. Tests hand one, so nothing looks at the real downloads directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct SaveDir(pub std::path::PathBuf);

/// The directory [`SaveDir`] names.
pub(in crate::ui) fn save_dir() -> std::path::PathBuf {
    try_consume_context::<SaveDir>().map_or_else(crate::attach::downloads_dir, |dir| dir.0)
}

/// Open the Import sheet with an empty path, and put the cursor in it.
pub(in crate::ui) fn open_import(mut shell: Signal<Shell>) {
    shell.write().files = Some(FileSheet::Import {
        path: String::new(),
    });
    focus();
}

/// Open the Export sheet on what the window is showing, and put the cursor in its field.
pub(in crate::ui) fn open_export(mut shell: Signal<Shell>) {
    let query = work::prefill(&shell.peek());
    shell.write().files = Some(FileSheet::Export { query });
    focus();
}

fn focus() {
    crate::ui::host::Host::focus_next_frame(".files-main input");
}

/// Close the sheet and give the keyboard back to the window.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().files = None;
    crate::ui::host::Host::focus_app();
}

/// Whichever sheet is open. Mounted while `shell.files` is `Some`.
#[component]
pub(in crate::ui) fn FilesSheet(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    match shell.read().files {
        Some(FileSheet::Import { .. }) => rsx! { import_sheet::ImportSheet { shell, revision } },
        Some(FileSheet::Export { .. }) => rsx! { export_sheet::ExportSheet { shell } },
        None => rsx! {},
    }
}

/// Where a job stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Phase {
    /// Nothing started.
    Ready,
    /// Running: messages done so far, of how many.
    Running { done: usize, of: usize },
    /// Finished, and what it said.
    Finished(String),
    /// Stopped, and why.
    Failed(String),
}

impl Phase {
    pub(in crate::ui) fn running(&self) -> bool {
        matches!(self, Phase::Running { .. })
    }
}

/// How often a running job's counter is read into the sheet.
const TICK: std::time::Duration = std::time::Duration::from_millis(120);

/// Run `job` on a blocking thread, drawing its counter into `phase` while it runs, and say what
/// it said in the sheet and the toast. `after` runs once it has finished either way.
///
/// Call it from an event handler: that is where a spawned task is polled (F140).
pub(in crate::ui) fn run(
    mut phase: Signal<Phase>,
    of: usize,
    job: impl FnOnce(&(dyn Fn(usize) + Sync)) -> Result<String, String> + Send + 'static,
    mut after: impl FnMut() + 'static,
) {
    if phase.peek().running() {
        return;
    }
    phase.set(Phase::Running { done: 0, of });
    let seen = Arc::new(AtomicUsize::new(0));
    let writer = seen.clone();
    spawn(async move {
        let handle = tokio::task::spawn_blocking(move || {
            job(&|done: usize| writer.store(done, Ordering::Relaxed))
        });
        while !handle.is_finished() {
            tokio::time::sleep(TICK).await;
            let done = seen.load(Ordering::Relaxed);
            phase.set(Phase::Running { done, of });
        }
        match handle.await {
            Ok(Ok(said)) => {
                tell(said.clone(), Follow::Nothing);
                phase.set(Phase::Finished(said));
            }
            Ok(Err(why)) => phase.set(Phase::Failed(why)),
            Err(error) => phase.set(Phase::Failed(format!(
                "It stopped before it finished: {error}"
            ))),
        }
        after();
    });
}

/// The progress line and bar, or the result, under a sheet's fields.
#[component]
fn Progress(phase: Phase, verb: &'static str) -> Element {
    match phase {
        Phase::Ready => rsx! {},
        Phase::Running { done, of } => {
            let width = (done.min(of) * 100).checked_div(of).unwrap_or(0);
            let total = if of > 0 {
                format!("{} of {}", work::grouped(done), work::messages(of))
            } else {
                work::messages(done)
            };
            rsx! {
                div { class: "files-progress", role: "status",
                    span { "{verb}… {total}" }
                    div { class: "files-bar", span { style: "width:{width}%" } }
                }
            }
        }
        Phase::Finished(said) => rsx! { p { class: "capnote said", role: "status", "{said}" } },
        Phase::Failed(why) => rsx! { p { class: "capnote files-bad", role: "alert", "{why}" } },
    }
}

/// `path` with the home directory written `~`, as the fields show a hint. Compared by whole
/// components: `/home/ann2` is not inside `/home/ann`.
pub(in crate::ui) fn tilde(path: &std::path::Path, home: Option<&std::ffi::OsStr>) -> String {
    match home
        .map(std::path::Path::new)
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// [`tilde`] against this user's home directory.
pub(in crate::ui) fn tilde_here(path: &std::path::Path) -> String {
    tilde(path, std::env::var_os("HOME").as_deref())
}

#[cfg(test)]
mod sheet_tests;
#[cfg(test)]
mod work_tests;
