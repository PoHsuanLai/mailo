//! The archive formats mail comes in and goes out as: an mbox, a Maildir, a single `.eml`, and
//! the Apple Mail and Thunderbird layouts that can only be read.
//!
//! [`Archive`] is what a format does when it is read ([`crate::import`]); [`Writer`] and
//! [`Sink`] are what it does when mail is written to it ([`crate::export`]). A format is one type
//! in [`ARCHIVES`], so a new one is a new type and a line there, not an edit in each ladder.

use crate::environment::Environment;
use crate::error::CoreError;
use crate::import::{AppleMail, Eml, Emlx, Items, Maildir, Mbox, Thunderbird};
use chrono::{DateTime, Utc};
use mail_domain::Message;
use mail_store::SqliteStore;
use std::fmt::Debug;
use std::path::Path;

/// A format mail is read from.
pub trait Archive: Debug + Sync {
    /// The format's stable name: `mbox`, `maildir`.
    fn slug(&self) -> &'static str;

    /// Whether `path` is of this format. Where two formats could claim a path, the earlier in
    /// [`ARCHIVES`] wins.
    fn claims(&self, path: &Path) -> Result<bool, CoreError>;

    /// The messages at `path`, one at a time.
    fn items(&self, path: &Path) -> Result<Items, CoreError>;

    /// How mail is written in this format. `None` for a format that is only read.
    fn writer(&self) -> Option<&'static dyn Writer> {
        None
    }
}

/// A format mail is written to.
pub trait Writer: Debug + Sync {
    /// The format's stable name: the command line's `--mbox`, `--maildir`, `--eml`.
    fn slug(&self) -> &'static str;

    /// The format's proper name.
    fn label(&self) -> &'static str;

    /// The name an export of the search `stem` is suggested, before anything is checked to be
    /// free.
    fn suggested(&self, stem: &str) -> String;

    /// Open `path` for writing.
    fn open(&self, path: &Path, ctx: &Ctx<'_>) -> Result<Box<dyn Sink>, CoreError>;
}

/// An open destination.
pub trait Sink {
    /// Write one message, with the bytes the server sent.
    fn write(&mut self, message: &Message, raw: &[u8]) -> Result<(), CoreError>;

    /// Write whatever is still held back.
    fn finish(self: Box<Self>) -> Result<(), CoreError> {
        Ok(())
    }
}

/// What a [`Writer`] may need to open its destination.
#[derive(Debug)]
pub struct Ctx<'a> {
    pub store: &'a SqliteStore,
    pub env: &'a Environment,
    pub now: DateTime<Utc>,
}

impl PartialEq for dyn Archive {
    fn eq(&self, other: &Self) -> bool {
        self.slug() == other.slug()
    }
}

impl Eq for dyn Archive {}

impl PartialEq for dyn Writer {
    fn eq(&self, other: &Self) -> bool {
        self.slug() == other.slug()
    }
}

impl Eq for dyn Writer {}

/// Every format, in the order they are tried when a path is looked at and the order the Export
/// sheet offers those that can be written. The files are told apart by their first bytes and
/// their extension; the directories are tried Maildir, Thunderbird, Apple Mail, as a directory
/// with `cur`, `new` or `tmp` in it is a Maildir whatever else it holds.
pub const ARCHIVES: &[&dyn Archive] = &[&Mbox, &Maildir, &Eml, &Thunderbird, &AppleMail, &Emlx];

/// Which format `path` is: the first of [`ARCHIVES`] to claim it.
pub fn detect(path: &Path) -> Result<&'static dyn Archive, CoreError> {
    std::fs::metadata(path).map_err(CoreError::at(path))?;
    for archive in ARCHIVES {
        if archive.claims(path)? {
            return Ok(*archive);
        }
    }
    Err(CoreError::NotMailFolder(path.to_owned()))
}

/// Every format that can be written, in [`ARCHIVES`] order.
pub fn writers() -> impl Iterator<Item = &'static dyn Writer> {
    ARCHIVES.iter().filter_map(|archive| archive.writer())
}

/// The writer called `slug`.
pub fn writer_named(slug: &str) -> Option<&'static dyn Writer> {
    writers().find(|writer| writer.slug() == slug)
}
