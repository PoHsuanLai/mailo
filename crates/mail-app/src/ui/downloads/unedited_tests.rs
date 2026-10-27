use super::{Ledger, NotRecorded, is_deleted};
use mail_core::Store as _;
use mail_domain::MessageId;
use std::path::{Path, PathBuf};

/// The folder Open writes into, and a folder of the person's beside it, in one scratch folder.
fn folders() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let root = scratch.path().join("mailo-open-test");
    let theirs = scratch.path().join("Downloads");
    std::fs::create_dir_all(&root).expect("Open's folder");
    std::fs::create_dir_all(&theirs).expect("the person's folder");
    (scratch, root, theirs)
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("a file");
    path
}

/// Each recorded copy as it was written is removed; one edited since, one never recorded and one
/// in the person's folder are left, and so is the folder itself.
#[test]
fn only_recorded_copies_as_they_were_written_are_removed() {
    let (_scratch, root, theirs) = folders();
    let message = MessageId::generate();
    let mut ledger = Ledger::default();

    let unedited = write(&root, "plans.pdf", b"%PDF-1.7 plans");
    let longer = write(&root, "notes.txt", b"notes");
    let same_size = write(&root, "list.txt", b"abcd");
    let unrecorded = write(&root, "other.txt", b"never recorded");
    let saved = write(&theirs, "plans.pdf", b"%PDF-1.7 plans");
    for path in [&unedited, &longer, &same_size] {
        ledger.record(&root, path, message).expect("recorded");
    }
    assert_eq!(
        ledger.record(&root, &saved, message),
        Err(NotRecorded::Outside),
        "a file outside Open's folder was recorded"
    );

    // Edited: one grew; one kept its size and was written again later.
    std::fs::write(&longer, b"notes, and more notes").expect("an edit");
    let file = std::fs::File::options()
        .write(true)
        .open(&same_size)
        .expect("the file");
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
        .expect("a later time");
    drop(file);

    let removed = ledger.remove_unedited(|_| true);

    assert_eq!(removed, std::slice::from_ref(&unedited));
    assert!(!unedited.exists(), "the unedited copy stayed");
    // (what stays, why)
    let stays = [
        (&longer, "edited to a new size"),
        (&same_size, "edited to a new time"),
        (&unrecorded, "never recorded"),
        (&saved, "in the person's folder"),
    ];
    for (path, why) in stays {
        assert!(path.exists(), "{} was removed: {why}", path.display());
    }
    assert!(root.is_dir(), "Open's folder was removed");
    assert!(ledger.is_empty(), "what was decided is still recorded");
}

/// Only the copies `due` picks are looked at; the rest stay recorded for later.
#[test]
fn copies_not_due_stay_recorded_and_in_place() {
    let (_scratch, root, _theirs) = folders();
    let (gone, kept) = (MessageId::generate(), MessageId::generate());
    let mut ledger = Ledger::default();
    let of_gone = write(&root, "a.txt", b"a");
    let of_kept = write(&root, "b.txt", b"b");
    ledger.record(&root, &of_gone, gone).expect("recorded");
    ledger.record(&root, &of_kept, kept).expect("recorded");

    let removed = ledger.remove_unedited(|written| written.message == gone);

    assert_eq!(removed, std::slice::from_ref(&of_gone));
    assert!(of_kept.exists());
    assert_eq!(ledger.messages(), [kept]);
    assert_eq!(ledger.remove_unedited(|_| true), [of_kept]);
}

/// Nothing outside the folder is recorded, however its path is spelt, and the folder itself is
/// not a file in it.
#[test]
fn a_path_that_leaves_open_s_folder_is_refused() {
    let (_scratch, root, theirs) = folders();
    let saved = write(&theirs, "plans.pdf", b"x");
    let message = MessageId::generate();
    let mut ledger = Ledger::default();
    let sideways = root.join("..").join("Downloads").join("plans.pdf");
    assert!(sideways.exists(), "the test's path does not reach the file");
    // (path, why it is refused)
    let cases = [
        (saved.clone(), "in another folder"),
        (sideways, "a step up and aside"),
        (root.clone(), "the folder itself"),
        (PathBuf::from("plans.pdf"), "relative"),
    ];
    for (path, why) in cases {
        assert_eq!(
            ledger.record(&root, &path, message),
            Err(NotRecorded::Outside),
            "{why}: {}",
            path.display()
        );
    }
    assert!(ledger.is_empty());
    assert!(saved.exists());
}

/// A link in Open's folder is not recorded, and what it points at is never removed.
#[cfg(unix)]
#[test]
fn a_link_in_open_s_folder_is_not_followed() {
    let (_scratch, root, theirs) = folders();
    let saved = write(&theirs, "plans.pdf", b"x");
    let link = root.join("plans.pdf");
    std::os::unix::fs::symlink(&saved, &link).expect("a link");
    let mut ledger = Ledger::default();
    assert!(matches!(
        ledger.record(&root, &link, MessageId::generate()),
        Err(NotRecorded::NotAFile)
    ));
    assert!(ledger.remove_unedited(|_| true).is_empty());
    assert!(saved.exists());
}

/// A recorded copy replaced by a link after it was written is left, and so is what the link
/// points at.
#[cfg(unix)]
#[test]
fn a_copy_replaced_by_a_link_is_left() {
    let (_scratch, root, theirs) = folders();
    let saved = write(&theirs, "plans.pdf", b"x");
    let copy = write(&root, "plans.pdf", b"x");
    let mut ledger = Ledger::default();
    ledger
        .record(&root, &copy, MessageId::generate())
        .expect("recorded");
    std::fs::remove_file(&copy).expect("the copy removed");
    std::os::unix::fs::symlink(&saved, &copy).expect("a link in its place");
    assert!(ledger.remove_unedited(|_| true).is_empty());
    assert!(saved.exists() && copy.symlink_metadata().is_ok());
}

/// A message still in the store and out of Trash is not deleted; one the store no longer has is.
#[test]
fn a_message_is_deleted_once_the_store_no_longer_has_it() {
    let (store, _dir) = crate::ui::fixtures::seeded();
    let thread = crate::ui::fixtures::thread_like(&store, "hi");
    let here = store.thread(thread).expect("the thread").messages[0];
    assert!(!is_deleted(&store, here), "a message in the inbox");
    assert!(
        is_deleted(&store, MessageId::generate()),
        "a message not stored"
    );
}
